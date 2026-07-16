#![allow(unsafe_code)]

//! VT-d IOMMU **register programming** — the confined `unsafe` that installs the pure [`crate::dmar`]
//! tables and enables translation. The table *bytes* are proven in host `cargo-test`; this module only
//! does the ORDER-SENSITIVE register dance: set the root-table pointer, invalidate the context-cache +
//! IOTLB, then enable translation. Skipping the invalidation leaves stale/absent translations that
//! silently don't take. The register base comes from the DMAR ACPI table (`acpi::dmar_drhd_reg_base`).
//!
//! Slice 1 observes only that translation is ENABLED (`GSTS.TES`). Enforcement (the induced fault) is
//! Slice 2. Each MMIO access is a single aligned read/write of a fixed IOMMU register with a SAFETY note.

// Register offsets from the DRHD base.
const CAP: u64 = 0x08; // Capability (SAGAW at bits [12:8])
const ECAP: u64 = 0x10; // Extended Capability (IRO at bits [17:8])
const GCMD: u64 = 0x18; // Global Command (write)
const GSTS: u64 = 0x1C; // Global Status (read)
const RTADDR: u64 = 0x20; // Root Table Address (write)
const CCMD: u64 = 0x28; // Context Command (invalidate)
const FSTS: u64 = 0x34; // Fault Status (read/write-1-clear)

const FSTS_PPF: u32 = 1 << 1; // primary pending fault

const IRTA: u64 = 0xB8; // Interrupt Remap Table Address (write)

const GCMD_SIRTP: u32 = 1 << 24; // set interrupt-remap table pointer
const GCMD_IRE: u32 = 1 << 25; // interrupt remapping enable
const GCMD_QIE: u32 = 1 << 26; // queued invalidation enable
const GCMD_SRTP: u32 = 1 << 30; // set root table pointer
const GCMD_TE: u32 = 1 << 31; // translation enable
const GSTS_IRTPS: u32 = 1 << 24; // interrupt-remap table pointer status
const GSTS_IRES: u32 = 1 << 25; // interrupt remapping enable status
const GSTS_QIES: u32 = 1 << 26; // queued invalidation enable status
const GSTS_RTPS: u32 = 1 << 30; // root table pointer status
const GSTS_TES: u32 = 1 << 31; // translation enable status

const CCMD_ICC: u64 = 1 << 63; // invalidate context cache
const CCMD_CIRG_GLOBAL: u64 = 1 << 61; // context invalidation request granularity = global
const IOTLB_IVT: u64 = 1 << 63; // invalidate IOTLB
const IOTLB_IIRG_GLOBAL: u64 = 1 << 60; // IOTLB invalidation request granularity = global

/// Bound on the status-poll spins so a non-responding IOMMU cannot wedge the boot; the verdict is read
/// from `GSTS.TES` afterwards regardless.
const POLL_MAX: u32 = 1 << 24;

/// One VT-d remapping unit, addressed by its DRHD register base.
pub struct Iommu {
    base: u64,
}

impl Iommu {
    pub fn new(base: u64) -> Self {
        Self { base }
    }

    fn r32(&self, off: u64) -> u32 {
        // SAFETY: `base + off` is a fixed, aligned IOMMU register (base from the DMAR ACPI table); a
        // volatile read has no side effect and touches no Rust-owned memory.
        unsafe { core::ptr::read_volatile((self.base + off) as *const u32) }
    }
    fn w32(&self, off: u64, val: u32) {
        // SAFETY: as `r32` — a fixed, aligned IOMMU command/address register; a volatile write affects
        // only the IOMMU.
        unsafe { core::ptr::write_volatile((self.base + off) as *mut u32, val) }
    }
    fn r64(&self, off: u64) -> u64 {
        // SAFETY: as `r32`, 64-bit.
        unsafe { core::ptr::read_volatile((self.base + off) as *const u64) }
    }
    fn w64(&self, off: u64, val: u64) {
        // SAFETY: as `w32`, 64-bit.
        unsafe { core::ptr::write_volatile((self.base + off) as *mut u64, val) }
    }

    pub fn cap(&self) -> u64 {
        self.r64(CAP)
    }

    /// Second-level page-table levels from CAP.SAGAW (bit1 = 39-bit/3-level, bit2 = 48-bit/4-level).
    /// Prefer 3-level when supported (fewer tables); fall back to 4-level.
    pub fn levels(&self) -> u8 {
        let sagaw = (self.cap() >> 8) & 0x1F;
        if sagaw & 0b010 != 0 {
            3
        } else if sagaw & 0b100 != 0 {
            4
        } else {
            3
        }
    }

    /// Byte offset of the IOTLB register block: ECAP.IRO (bits [17:8]) × 16.
    fn iotlb_base(&self) -> u64 {
        ((self.r64(ECAP) >> 8) & 0x3FF) * 16
    }

    fn poll32(&self, off: u64, mask: u32, want_set: bool) {
        let mut n = 0;
        while ((self.r32(off) & mask != 0) != want_set) && n < POLL_MAX {
            core::hint::spin_loop();
            n += 1;
        }
    }
    fn poll64_clear(&self, off: u64, mask: u64) {
        let mut n = 0;
        while self.r64(off) & mask != 0 && n < POLL_MAX {
            core::hint::spin_loop();
            n += 1;
        }
    }

    /// The currently-enabled *persistent* GCMD bits, reconstructed from GSTS. GCMD sets every persistent
    /// function to the written bit, so any command write must OR its new bit onto these — otherwise it
    /// would silently disable an already-enabled feature (e.g. writing IRE alone would clear TE).
    fn persistent_gcmd(&self) -> u32 {
        let gsts = self.r32(GSTS);
        let mut cmd = 0;
        if gsts & GSTS_TES != 0 {
            cmd |= GCMD_TE;
        }
        if gsts & GSTS_IRES != 0 {
            cmd |= GCMD_IRE;
        }
        if gsts & GSTS_QIES != 0 {
            cmd |= GCMD_QIE;
        }
        cmd
    }

    /// Install the root table and enable translation, in the required order:
    /// RTADDR + SRTP → global context-cache invalidate → global IOTLB invalidate → set TE.
    pub fn enable(&self, root_phys: u64) {
        // 1. root table pointer (legacy mode: RTT = 0), then Set-Root-Table-Pointer.
        self.w64(RTADDR, root_phys & 0xFFFF_FFFF_FFFF_F000);
        self.w32(GCMD, self.persistent_gcmd() | GCMD_SRTP);
        self.poll32(GSTS, GSTS_RTPS, true);

        // 2. global context-cache invalidate.
        self.w64(CCMD, CCMD_ICC | CCMD_CIRG_GLOBAL);
        self.poll64_clear(CCMD, CCMD_ICC);

        // 3. global IOTLB invalidate.
        let iotlb = self.iotlb_base() + 8;
        self.w64(iotlb, IOTLB_IVT | IOTLB_IIRG_GLOBAL);
        self.poll64_clear(iotlb, IOTLB_IVT);

        // 4. enable translation (preserving any persistent bits).
        self.w32(GCMD, self.persistent_gcmd() | GCMD_TE);
        self.poll32(GSTS, GSTS_TES, true);
    }

    /// Install the interrupt-remap table and enable interrupt remapping, preserving TE. `n_entries` is a
    /// power of two. Order: IRTA + Set-IRT-Pointer → set IRE. (The (B) interrupt-remapping (i)-source
    /// step; the (ii) rogue-MSI-blocked proof is un-inducible in QEMU and stays honest-open.)
    pub fn enable_ir(&self, irt_phys: u64, n_entries: u32) {
        let size = (31 - n_entries.leading_zeros()).saturating_sub(1); // log2(n_entries) - 1
        self.w64(IRTA, (irt_phys & 0xFFFF_FFFF_FFFF_F000) | (size as u64 & 0xF));
        self.w32(GCMD, self.persistent_gcmd() | GCMD_SIRTP);
        self.poll32(GSTS, GSTS_IRTPS, true);
        self.w32(GCMD, self.persistent_gcmd() | GCMD_IRE);
        self.poll32(GSTS, GSTS_IRES, true);
    }

    /// Whether translation is enabled (GSTS.TES) — the Slice-1 observed fact.
    pub fn translation_enabled(&self) -> bool {
        self.r32(GSTS) & GSTS_TES != 0
    }

    /// Whether interrupt remapping is enabled (GSTS.IRES) — the (B) IR (i)-source observed fact.
    pub fn ir_enabled(&self) -> bool {
        self.r32(GSTS) & GSTS_IRES != 0
    }

    /// Whether the IOMMU hardware advertises Device-TLB (ATS) support (ECAP.DT) — informational for the
    /// ATS-off verdict: even where the hardware supports ATS, our config does not enable it.
    pub fn supports_ats(&self) -> bool {
        self.r64(ECAP) & (1 << 2) != 0
    }

    /// Byte offset of the Fault Recording register block: CAP.FRO (bits [33:24]) × 16.
    fn fro(&self) -> u64 {
        ((self.cap() >> 24) & 0x3FF) * 16
    }

    /// Clear any pending fault: read the primary fault record's F bit and the FSTS pending bits, and
    /// write them back (both are write-1-to-clear). Call before an induced-fault test to start clean.
    pub fn clear_faults(&self) {
        let fsts = self.r32(FSTS);
        if fsts & FSTS_PPF != 0 {
            let fri = ((fsts >> 8) & 0xFF) as u64;
            // Clear the fault record's F bit (bit 63 of the high qword, write-1-to-clear).
            self.w64(self.fro() + fri * 16 + 8, 1 << 63);
        }
        // Clear the FSTS pending bits (write-1-to-clear).
        self.w32(FSTS, fsts);
    }

    /// Read + clear the primary fault record if one is pending (FSTS.PPF). Returns the decoded record
    /// (the IOMMU's own account of *which* IOVA faulted, from *which* source) — the "it's the IOMMU, not
    /// the device" evidence.
    pub fn take_fault(&self) -> Option<crate::dmar::FaultRecord> {
        let fsts = self.r32(FSTS);
        if fsts & FSTS_PPF == 0 {
            return None;
        }
        let fri = ((fsts >> 8) & 0xFF) as u64;
        let frcd = self.fro() + fri * 16;
        let lo = self.r64(frcd);
        let hi = self.r64(frcd + 8);
        let record = crate::dmar::decode_frcd(lo, hi);
        // Clear the record (F bit) + the FSTS pending bit.
        self.w64(frcd + 8, 1 << 63);
        self.w32(FSTS, fsts);
        record
    }
}
