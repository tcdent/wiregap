//! Mitigation Gate 1 host tests — the pure leg-2 core proven fail-closed with NO hardware. The pagemap
//! parse and the in-hugepage bounds are the pre-hardware gate: a wrong physaddr on the no-IOMMU direct
//! path DMAs to arbitrary host RAM, so each fail-closed condition is positively asserted (non-vacuous).

use directpath::pagemap::{buffer_in_hugepage, parse_pagemap_entry, phys_addr, PagemapError, HUGE_2MB};

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
