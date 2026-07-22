//! **Mitigation Gate 1** — the pure leg-2 core, brought forward from the driver retarget (Slice 3) as the
//! **pre-hardware** gate. On the pure-direct path with **no IOMMU**, the physical address the device
//! DMAs to is resolved from `/proc/self/pagemap`; a wrong physaddr DMAs to arbitrary host RAM (no
//! backstop). So the pagemap parse + the in-hugepage bounds are proven **fail-closed** here, in host
//! `cargo test`, with **no hardware and no device** — before any register or DMA op runs.
//!
//! Everything in this module is pure (no I/O): the caller reads the 8-byte pagemap entry from the file
//! and the hugepage physaddr; these functions decide whether the resolved DMA target is trustworthy, and
//! **Refuse** (never silently pass) on any doubt — present-clear, swapped, PFN==0 (a genuine PFN-0, or
//! the kernel zeroing it for an unprivileged reader), a short read, or an out-of-hugepage buffer.

/// Page shift / size (the pagemap is per-4 KiB page, even inside a 2 MiB hugepage — the PFNs are the
/// consecutive 4 KiB frames of the physically-contiguous huge page).
pub const PAGE_SHIFT: u64 = 12;
pub const PAGE_SIZE: u64 = 1 << PAGE_SHIFT;
/// A 2 MiB hugepage (hugetlbfs default) — the DMA leaf.
pub const HUGE_2MB: u64 = 2 * 1024 * 1024;

/// `/proc/self/pagemap` entry bit layout (Linux `Documentation/admin-guide/mm/pagemap.rst`).
const BIT_PRESENT: u64 = 1 << 63;
const BIT_SWAPPED: u64 = 1 << 62;
/// PFN occupies bits 0..=54 when the page is present and not swapped.
const PFN_MASK: u64 = (1 << 55) - 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PagemapError {
    /// Fewer than 8 bytes were read for the entry.
    ShortRead,
    /// The present bit (63) is clear — the page has no resident physical frame.
    NotPresent,
    /// The swapped bit (62) is set — the PFN field is a swap entry, not a physical frame.
    Swapped,
    /// PFN == 0: either a genuine frame 0 (never our DMA memory) or the kernel zeroed it because the
    /// reader is unprivileged (`CAP_SYS_ADMIN` absent). Either way it is not a usable DMA physaddr.
    PfnZeroOrUnprivileged,
}

/// Parse a `/proc/self/pagemap` entry into a PFN, fail-closed. `Ok(pfn)` only for a present, non-swapped,
/// nonzero PFN.
pub fn parse_pagemap_entry(bytes: &[u8]) -> Result<u64, PagemapError> {
    if bytes.len() < 8 {
        return Err(PagemapError::ShortRead);
    }
    let entry = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    if entry & BIT_PRESENT == 0 {
        return Err(PagemapError::NotPresent);
    }
    if entry & BIT_SWAPPED != 0 {
        return Err(PagemapError::Swapped);
    }
    let pfn = entry & PFN_MASK;
    if pfn == 0 {
        return Err(PagemapError::PfnZeroOrUnprivileged);
    }
    Ok(pfn)
}

/// The physical address of virtual address `va`, given the PFN of the 4 KiB page containing it:
/// `phys = (PFN << 12) | (va & 0xFFF)`. Pure.
pub const fn phys_addr(pfn: u64, va: u64) -> u64 {
    (pfn << PAGE_SHIFT) | (va & (PAGE_SIZE - 1))
}

/// Whether a DMA buffer `[buf_phys, buf_phys + len)` lies wholly within the 2 MiB hugepage at
/// `huge_phys` and does not cross the 2 MiB leaf. Fail-closed: a `huge_phys` that is not 2 MiB-aligned
/// (not a real hugepage base — a red flag for a bad pagemap read), a zero length, an overflow, a buffer
/// below/above the hugepage, or a buffer straddling the leaf boundary all return `false`.
pub fn buffer_in_hugepage(buf_phys: u64, len: u64, huge_phys: u64) -> bool {
    if !huge_phys.is_multiple_of(HUGE_2MB) || len == 0 {
        return false;
    }
    let Some(end) = buf_phys.checked_add(len) else {
        return false;
    };
    // Wholly within the hugepage extent (checked add so a pathological base fail-closes uniformly, not
    // a debug-panic — matches the `buf_phys + len` handling above).
    let Some(huge_end) = huge_phys.checked_add(HUGE_2MB) else {
        return false;
    };
    if buf_phys < huge_phys || end > huge_end {
        return false;
    }
    // And not crossing the 2 MiB leaf (first and last byte in the same leaf).
    buf_phys / HUGE_2MB == (end - 1) / HUGE_2MB
}

#[cfg(test)]
mod integration_tests {
//! Mitigation Gate 1 host tests — the pure leg-2 core proven fail-closed with NO hardware. The pagemap
//! parse and the in-hugepage bounds are the pre-hardware gate: a wrong physaddr on the no-IOMMU direct
//! path DMAs to arbitrary host RAM, so each fail-closed condition is positively asserted (non-vacuous).

use super::{buffer_in_hugepage, parse_pagemap_entry, phys_addr, PagemapError, HUGE_2MB};

const PRESENT: u64 = 1 << 63;
const SWAPPED: u64 = 1 << 62;

/// Build a pagemap entry from flags + PFN.
fn entry(flags: u64, pfn: u64) -> [u8; 8] {
    (flags | (pfn & ((1 << 55) - 1))).to_le_bytes()
}

// ---- pagemap parse (fail-closed) --------------------------------------------------------------

#[test]
fn present_nonzero_pfn_parses() {
    let pfn = 0x1_2345;
    assert_eq!(parse_pagemap_entry(&entry(PRESENT, pfn)), Ok(pfn));
}

#[test]
fn phys_addr_composes_pfn_and_offset() {
    // phys = (pfn << 12) | (va & 0xFFF)
    assert_eq!(phys_addr(0x1_2345, 0x0000_0ABC), (0x1_2345 << 12) | 0xABC);
    assert_eq!(phys_addr(0x800, 0x1000), 0x800 << 12, "page-aligned va → zero offset");
}

#[test]
fn present_clear_refuses() {
    // No present bit, even with a plausible PFN → NotPresent (no resident frame).
    assert_eq!(parse_pagemap_entry(&entry(0, 0x1_2345)), Err(PagemapError::NotPresent));
}

#[test]
fn swapped_refuses() {
    // Present + swapped: the PFN field is a swap entry, not a frame → Swapped.
    assert_eq!(parse_pagemap_entry(&entry(PRESENT | SWAPPED, 0x1_2345)), Err(PagemapError::Swapped));
}

#[test]
fn pfn_zero_or_unprivileged_refuses() {
    // Present but PFN==0 — the kernel zeroes the PFN for an unprivileged reader (and frame 0 is never our
    // DMA memory) → refuse rather than DMA to physical 0.
    assert_eq!(parse_pagemap_entry(&entry(PRESENT, 0)), Err(PagemapError::PfnZeroOrUnprivileged));
}

#[test]
fn short_read_refuses() {
    assert_eq!(parse_pagemap_entry(&[]), Err(PagemapError::ShortRead));
    assert_eq!(parse_pagemap_entry(&[0xff; 7]), Err(PagemapError::ShortRead), "7 bytes < 8 → refuse");
}

// ---- in-hugepage bounds (fail-closed) ---------------------------------------------------------

#[test]
fn buffer_within_hugepage_is_ok() {
    let huge = 0x1_2000_0000u64; // 2 MiB-aligned
    assert!(buffer_in_hugepage(huge, 2048, huge), "buffer at the base");
    assert!(buffer_in_hugepage(huge + 0x1000, 2048, huge), "buffer mid-page");
    assert!(buffer_in_hugepage(huge + HUGE_2MB - 2048, 2048, huge), "buffer ending exactly at the leaf end");
}

#[test]
fn buffer_outside_or_straddling_refuses() {
    let huge = 0x1_2000_0000u64;
    assert!(!buffer_in_hugepage(huge - 1, 8, huge), "starts below the hugepage");
    assert!(!buffer_in_hugepage(huge + HUGE_2MB - 4, 8, huge), "runs past the leaf end (straddles)");
    assert!(!buffer_in_hugepage(huge, HUGE_2MB + 1, huge), "len exceeds the leaf");
    assert!(!buffer_in_hugepage(huge, 0, huge), "zero-length buffer");
    assert!(!buffer_in_hugepage(u64::MAX - 4, 8, huge), "address overflow");
}

#[test]
fn pathological_hugepage_base_near_u64_max_fails_closed() {
    // huge_phys + 2 MiB would overflow — must fail-closed (checked add), not panic.
    let near_max = u64::MAX - (HUGE_2MB / 2); // 2 MiB-aligned? force alignment below
    let aligned = near_max & !(HUGE_2MB - 1);
    assert!(!buffer_in_hugepage(aligned, 8, aligned), "base whose +2MiB overflows → refuse");
}

#[test]
fn non_2mib_aligned_hugepage_base_refuses() {
    // A hugepage base that is not 2 MiB-aligned is not a real hugepage — a red flag for a bad pagemap
    // read; refuse rather than trust it.
    let bad = 0x1_2000_1000u64; // 4 KiB-aligned, NOT 2 MiB-aligned
    assert!(!buffer_in_hugepage(bad, 2048, bad), "non-2MiB-aligned base → refuse");
}
}
