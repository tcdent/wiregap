//! VT-d DMA-remapping **table construction** — the security core, and (like [`crate::ecam::decode_bar`])
//! **pure**: given the arena extent + a table-region base + the NIC's BDF + the level count, build the
//! root / context / second-level tables that map **only** the arena (identity `iova == phys`), and a
//! [`walk`] that resolves an IOVA to its physical address or `None`. So the confinement — in-arena
//! resolves, **everything else is unmapped** — is proven in host `cargo-test`, deterministically, before
//! any hardware. The register programming that installs these tables is the confined `unsafe`
//! ([`crate::iommu`]); this module has **zero** `unsafe`.
//!
//! Slice 1 proves the table BYTES, not enforcement — the induced-fault enforcement proof is Slice 2.
//!
//! Structure (VT-d, legacy/non-scalable mode): a 256-entry **root table** (indexed by bus, 16 B entries)
//! → a 256-entry **context table** (indexed by `devfn`, 16 B entries) → **second-level page tables**
//! (8 B entries, `R`/`W`/`PS` bits). The arena, one large-page-aligned extent, maps as 2 MiB large-page
//! leaves at level 2 — minimal tables.
//!
//! **The tables live in a NIC-UNREACHABLE region:** [`build`] rejects a table region that overlaps the
//! arena, and because only the arena is in the SL map, no IOVA resolves to the table region — a device
//! cannot rewrite its own confinement. Both are asserted by the host tests.

extern crate alloc;
use crate::arena::Region;
use alloc::vec;
use alloc::vec::Vec;

/// A 2 MiB second-level large page (the leaf granularity the arena is aligned to).
pub const LARGE_PAGE: u64 = 2 * 1024 * 1024;
const TABLE_SIZE: u64 = 4096;
/// Physical-address bits [51:12] — the pointer/frame field shared by every entry type.
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

const ROOT_PRESENT: u64 = 1 << 0;
const CTX_PRESENT: u64 = 1 << 0;
const SL_R: u64 = 1 << 0; // second-level read
const SL_W: u64 = 1 << 1; // second-level write
const SL_PS: u64 = 1 << 7; // page size (large page) at this level

/// The confine-this-NIC-to-this-arena parameters. No baked physical base — everything is an input.
#[derive(Clone, Copy, Debug)]
pub struct DmarConfig {
    /// The one region the NIC may reach (identity-mapped).
    pub arena: Region,
    /// Where the DMAR tables themselves live — must be disjoint from `arena` (NIC-unreachable).
    pub table_region: Region,
    pub bus: u8,
    /// `(device << 3) | function`.
    pub devfn: u8,
    /// Second-level page-table levels (3 for 39-bit AGAW, 4 for 48-bit), from the IOMMU CAP/SAGAW.
    pub levels: u8,
    pub domain_id: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmarError {
    BadLevels,
    ArenaNotLargePageAligned,
    /// The tables would sit where the NIC can DMA — the self-referential containment hole.
    TableRegionOverlapsArena,
    /// The arena crosses a level-2 table's 1 GiB span (Slice 1 maps it within one).
    ArenaCrossesL2Boundary,
    TableRegionTooSmall,
}

/// The constructed table image (to be copied into the physical table region) + the root pointer to
/// program into `RTADDR`.
pub struct DmarImage {
    bytes: Vec<u8>,
    region_base: u64,
    root_phys: u64,
}

impl DmarImage {
    /// The raw table bytes to write to the physical table region.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// The physical address of the root table (for `RTADDR`).
    pub fn root_phys(&self) -> u64 {
        self.root_phys
    }
}

/// One Interrupt Remap Table Entry (IRTE), decoded — for the (B) interrupt-remapping (i)-source verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Irte {
    pub present: bool,
    pub vector: u8,
    pub dest_apic: u8,
    /// Source id the entry is validated against (SVT source validation) — the field that makes the IRT
    /// source-correct: a device with a different SID cannot use this interrupt index.
    pub sid: u16,
    /// Source-validation type (01 = verify SID). Non-zero = source validation enabled.
    pub svt: u8,
}

/// Build one source-validated IRTE `(lo, hi)`: present, fixed delivery, `vector` to `dest_apic`, with
/// SVT=01 verifying `sid`. Pure — host-tested, so the source-correctness of the IRT is proven in bytes.
pub fn build_irte(vector: u8, dest_apic: u8, sid: u16) -> (u64, u64) {
    let lo = 1u64                       // Present
        | ((vector as u64) << 16)       // Vector [23:16]
        | ((dest_apic as u64) << 40);   // Destination (xAPIC id)
    let hi = (sid as u64)               // SID [15:0]
        | (0b01u64 << 18);              // SVT = 01 (verify source id)
    (lo, hi)
}

/// Decode an IRTE `(lo, hi)`. Pure — host-tested.
pub fn decode_irte(lo: u64, hi: u64) -> Irte {
    Irte {
        present: lo & 1 != 0,
        vector: ((lo >> 16) & 0xFF) as u8,
        dest_apic: ((lo >> 40) & 0xFF) as u8,
        sid: (hi & 0xFFFF) as u16,
        svt: ((hi >> 18) & 0x3) as u8,
    }
}

/// Build a minimal IRT image: `entry_count` IRTEs, index 0 present + source-validated for the NIC, the
/// rest absent. Pure. The IRT lives in the NIC-unreachable table region (like the DMAR tables).
pub fn build_irt(entry_count: usize, vector: u8, dest_apic: u8, sid: u16) -> Vec<u8> {
    let mut bytes = vec![0u8; entry_count * 16];
    let (lo, hi) = build_irte(vector, dest_apic, sid);
    write_u64(&mut bytes, 0, lo);
    write_u64(&mut bytes, 8, hi);
    bytes
}

/// A decoded VT-d fault record (from a Fault Recording register pair) — the evidence that the IOMMU (not
/// the device) blocked a DMA: which IOVA faulted, from which source (BDF), why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultRecord {
    /// The faulting IOVA (page-aligned).
    pub iova: u64,
    /// Source identifier `(bus << 8) | devfn` — the device that issued the faulting DMA.
    pub sid: u16,
    /// Fault reason code.
    pub reason: u8,
    /// Whether the faulting access was a read (vs a write).
    pub is_read: bool,
}

/// Decode a VT-d Fault Recording register pair `(lo, hi)`. `None` when no fault is recorded (F bit clear).
/// Pure — host-tested, so the "it's the IOMMU" interpretation is proven independent of hardware.
///
/// Layout: `lo[63:12]` = faulting address; `hi[63]` = F (fault recorded); `hi[62]` = T (1 = read);
/// `hi[39:32]` = fault reason; `hi[15:0]` = source id.
pub fn decode_frcd(lo: u64, hi: u64) -> Option<FaultRecord> {
    if hi & (1 << 63) == 0 {
        return None;
    }
    Some(FaultRecord {
        iova: lo & ADDR_MASK,
        sid: (hi & 0xFFFF) as u16,
        reason: ((hi >> 32) & 0xFF) as u8,
        is_read: hi & (1 << 62) != 0,
    })
}

fn aw_field(levels: u8) -> u64 {
    // Context-entry Address Width: 001 = 39-bit (3 level), 010 = 48-bit (4 level).
    if levels == 3 { 1 } else { 2 }
}

fn write_u64(bytes: &mut [u8], off: u64, val: u64) {
    let o = off as usize;
    bytes[o..o + 8].copy_from_slice(&val.to_le_bytes());
}

fn read_u64(bytes: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&bytes[off..off + 8]);
    u64::from_le_bytes(a)
}

/// Build the DMAR tables that map **only** `cfg.arena`. Pure — same inputs → same bytes.
pub fn build(cfg: &DmarConfig) -> Result<DmarImage, DmarError> {
    if cfg.levels != 3 && cfg.levels != 4 {
        return Err(DmarError::BadLevels);
    }
    if !cfg.arena.base.is_multiple_of(LARGE_PAGE) || !cfg.arena.len.is_multiple_of(LARGE_PAGE) || cfg.arena.len == 0 {
        return Err(DmarError::ArenaNotLargePageAligned);
    }
    if cfg.arena.overlaps(&cfg.table_region) {
        return Err(DmarError::TableRegionOverlapsArena);
    }
    // The arena must fall within a single level-2 table's 1 GiB span (one L2 table maps it).
    let l2_span = 1u64 << 30;
    if cfg.arena.base / l2_span != (cfg.arena.end() - 1) / l2_span {
        return Err(DmarError::ArenaCrossesL2Boundary);
    }

    // Layout: root, context, then the SL tables from the top level down to level 2 (levels-1 of them).
    let n_sl = (cfg.levels - 1) as u64;
    let need = (2 + n_sl) * TABLE_SIZE;
    if cfg.table_region.len < need {
        return Err(DmarError::TableRegionTooSmall);
    }

    let base = cfg.table_region.base;
    let mut bytes = vec![0u8; cfg.table_region.len as usize];

    let root_phys = base;
    let ctx_phys = base + TABLE_SIZE;
    // sl_phys[0] = top level (cfg.levels); the last = level 2.
    let mut sl_phys = [0u64; 3];
    for (i, slot) in sl_phys.iter_mut().enumerate().take(n_sl as usize) {
        *slot = base + (2 + i as u64) * TABLE_SIZE;
    }
    let top_sl = sl_phys[0];

    // Root entry for the bus → the context table.
    write_u64(&mut bytes, (root_phys - base) + (cfg.bus as u64) * 16, ROOT_PRESENT | (ctx_phys & ADDR_MASK));
    // Context entry for devfn → the top SL table; qword1 carries the address width + domain id.
    let ctx_off = (ctx_phys - base) + (cfg.devfn as u64) * 16;
    write_u64(&mut bytes, ctx_off, CTX_PRESENT | (top_sl & ADDR_MASK));
    write_u64(&mut bytes, ctx_off + 8, aw_field(cfg.levels) | ((cfg.domain_id as u64) << 8));

    // Wire the non-leaf SL path (top level down to level 3): one present entry each, pointing down.
    for i in 0..n_sl.saturating_sub(1) {
        let level = cfg.levels as u64 - i;
        let shift = 12 + 9 * (level - 1);
        let idx = (cfg.arena.base >> shift) & 0x1FF;
        let next = sl_phys[(i + 1) as usize];
        write_u64(&mut bytes, (sl_phys[i as usize] - base) + idx * 8, SL_R | SL_W | (next & ADDR_MASK));
    }

    // Level-2 leaves: a 2 MiB large page per 2 MiB of the arena (identity iova == phys).
    let l2 = sl_phys[(n_sl - 1) as usize];
    let mut pa = cfg.arena.base;
    while pa < cfg.arena.end() {
        let idx = (pa >> 21) & 0x1FF;
        write_u64(&mut bytes, (l2 - base) + idx * 8, SL_R | SL_W | SL_PS | (pa & ADDR_MASK));
        pa += LARGE_PAGE;
    }

    Ok(DmarImage { bytes, region_base: base, root_phys })
}

/// Resolve `iova` for source `(bus, devfn)` through the constructed tables. `None` = unmapped → the
/// hardware would fault. Pure — the confinement predicate the host tests assert.
pub fn walk(image: &DmarImage, bus: u8, devfn: u8, iova: u64) -> Option<u64> {
    let base = image.region_base;
    let read = |phys: u64| -> Option<u64> {
        let off = phys.checked_sub(base)? as usize;
        if off + 8 > image.bytes.len() {
            return None;
        }
        Some(read_u64(&image.bytes, off))
    };

    let root = read(image.root_phys + (bus as u64) * 16)?;
    if root & ROOT_PRESENT == 0 {
        return None;
    }
    let ctp = root & ADDR_MASK;

    let ctx0 = read(ctp + (devfn as u64) * 16)?;
    if ctx0 & CTX_PRESENT == 0 {
        return None;
    }
    let ctx1 = read(ctp + (devfn as u64) * 16 + 8)?;
    let levels = match ctx1 & 0x7 {
        1 => 3u64,
        2 => 4u64,
        _ => return None,
    };
    let mut table = ctx0 & ADDR_MASK;

    // Reject an IOVA beyond the adjusted guest address width (the hardware faults on it).
    let max_bits = 12 + 9 * levels;
    if max_bits < 64 && (iova >> max_bits) != 0 {
        return None;
    }

    let mut level = levels;
    loop {
        let shift = 12 + 9 * (level - 1);
        let idx = (iova >> shift) & 0x1FF;
        let e = read(table + idx * 8)?;
        if e & SL_R == 0 {
            return None;
        }
        if level == 1 || e & SL_PS != 0 {
            let offset_mask = (1u64 << shift) - 1;
            let frame = e & ADDR_MASK & !offset_mask;
            return Some(frame | (iova & offset_mask));
        }
        table = e & ADDR_MASK;
        level -= 1;
    }
}
