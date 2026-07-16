//! The physical **arena** for this NIC's DMA — the sprint's primary security-adjacent target, and
//! **pure** (safe Rust, host-tested). Given one UEFI-usable region, it lays out a single **contiguous,
//! DMAR-confinable** extent holding exactly this 82599's RX/TX descriptor rings + packet buffers, and
//! nothing else. The layout is identity-mapped (virt == phys), so a `base` here is a physical address
//! the device is later pointed at; Sprint 5 confines exactly this one extent behind the IOMMU.
//!
//! No allocation, no `unsafe`: `plan()` is arithmetic over addresses, so its invariants — every
//! sub-region inside the extent, the extent inside the usable region, no overlaps, DMAR-page-aligned —
//! are proven by host `cargo test`, not by staring at a running machine.

/// Minimum IOMMU/DMAR mapping granularity (a 4 KiB page). The extent length is a whole number of these.
pub const DMAR_PAGE: u64 = 4096;
/// Extent base alignment: 2 MiB, so the whole arena is confinable with 4 KiB *or* 2 MiB DMAR pages.
pub const EXTENT_ALIGN: u64 = 2 * 1024 * 1024;
/// Per-sub-region alignment (page). Satisfies the 82599's 128-byte ring-base requirement with margin.
pub const REGION_ALIGN: u64 = 4096;
/// Advanced descriptor size (RX writeback / TX), bytes.
pub const DESC_SIZE: u64 = 16;

/// A physical extent `[base, base + len)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub base: u64,
    pub len: u64,
}

impl Region {
    pub const fn end(&self) -> u64 {
        self.base + self.len
    }
    /// Whether `self` lies wholly inside `outer`.
    pub const fn within(&self, outer: &Region) -> bool {
        self.base >= outer.base && self.end() <= outer.end()
    }
    /// Whether `self` and `other` share any byte.
    pub const fn overlaps(&self, other: &Region) -> bool {
        self.base < other.end() && other.base < self.end()
    }
}

/// This NIC's ring/buffer sizing — the only knobs, all fixed to the 82599 the substrate binds.
#[derive(Clone, Copy, Debug)]
pub struct ArenaConfig {
    pub rx_depth: u64,
    pub tx_depth: u64,
    pub rx_buf_len: u64,
    pub tx_buf_len: u64,
}

impl ArenaConfig {
    /// The 82599 data-path sizing this substrate uses (power-of-two ring depths, 2 KiB buffers).
    pub const NIC_82599: ArenaConfig = ArenaConfig {
        rx_depth: 256,
        tx_depth: 256,
        rx_buf_len: 2048,
        tx_buf_len: 2048,
    };
}

/// The planned layout: one extent, four sub-regions, no overlap, all identity-mapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArenaLayout {
    pub extent: Region,
    pub rx_ring: Region,
    pub tx_ring: Region,
    pub rx_bufs: Region,
    pub tx_bufs: Region,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaError {
    /// The usable region is too small to hold the (aligned) extent.
    RegionTooSmall { need: u64, have: u64 },
}

const fn align_up(value: u64, align: u64) -> u64 {
    (value + align - 1) & !(align - 1)
}

/// Lay the arena inside `usable`: bump the four sub-regions (each `REGION_ALIGN`-aligned) from an
/// `EXTENT_ALIGN`-aligned base, size the extent up to `EXTENT_ALIGN`, and verify it fits. Deterministic.
pub fn plan(usable: Region, cfg: &ArenaConfig) -> Result<ArenaLayout, ArenaError> {
    let base = align_up(usable.base, EXTENT_ALIGN);

    // Bump-allocate within the extent, page-aligning each sub-region.
    let mut cursor = base;
    let mut carve = |bytes: u64| -> Region {
        let start = align_up(cursor, REGION_ALIGN);
        cursor = start + bytes;
        Region { base: start, len: bytes }
    };
    let rx_ring = carve(cfg.rx_depth * DESC_SIZE);
    let tx_ring = carve(cfg.tx_depth * DESC_SIZE);
    let rx_bufs = carve(cfg.rx_depth * cfg.rx_buf_len);
    let tx_bufs = carve(cfg.tx_depth * cfg.tx_buf_len);

    // The extent spans base..cursor, rounded up to the extent alignment (a whole number of DMAR pages).
    let extent = Region { base, len: align_up(cursor - base, EXTENT_ALIGN) };

    if !extent.within(&usable) {
        return Err(ArenaError::RegionTooSmall { need: extent.end() - usable.base, have: usable.len });
    }
    Ok(ArenaLayout { extent, rx_ring, tx_ring, rx_bufs, tx_bufs })
}

impl ArenaLayout {
    /// All four sub-regions, for iteration in checks/reports.
    pub fn regions(&self) -> [Region; 4] {
        [self.rx_ring, self.tx_ring, self.rx_bufs, self.tx_bufs]
    }

    /// Every sub-region lies within the extent (bounds).
    pub fn all_within_extent(&self) -> bool {
        self.regions().iter().all(|r| r.within(&self.extent))
    }

    /// No two sub-regions overlap.
    pub fn no_overlap(&self) -> bool {
        let rs = self.regions();
        for i in 0..rs.len() {
            for j in (i + 1)..rs.len() {
                if rs[i].overlaps(&rs[j]) {
                    return false;
                }
            }
        }
        true
    }

    /// The extent is a single DMAR-confinable block: base aligned to a large DMAR page, length a whole
    /// number of DMAR pages. Sprint 5 confines exactly `self.extent`.
    pub fn dmar_confinable(&self) -> bool {
        self.extent.base.is_multiple_of(EXTENT_ALIGN) && self.extent.len.is_multiple_of(DMAR_PAGE) && self.extent.len > 0
    }

    /// The full security predicate for the arena: inside a usable region, self-consistent, confinable.
    pub fn sound(&self, usable: &Region) -> bool {
        self.extent.within(usable) && self.all_within_extent() && self.no_overlap() && self.dmar_confinable()
    }
}
