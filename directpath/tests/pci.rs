//! Fold #2 (re-fixed) — the BAR0 dword-access bound is overflow-safe AND rejects len<4. Host-proven
//! non-vacuously (the previous `saturating_sub(4)` regressed len<4 to an OOB read).

use directpath::pci::dword_in_bounds;

#[test]
fn dword_bound_accepts_only_wholly_in_range() {
    // Real BAR0 (512 KiB) — a valid register offset.
    assert!(dword_in_bounds(0x0, 0x80000));
    assert!(dword_in_bounds(0x042a4, 0x80000)); // LINKS
    assert!(dword_in_bounds(0x7fffc, 0x80000), "last valid dword (offset+4 == len)");
    assert!(!dword_in_bounds(0x7fffd, 0x80000), "offset+4 > len → out");
    assert!(!dword_in_bounds(0x80000, 0x80000), "offset == len → out");
}

#[test]
fn dword_bound_rejects_len_below_4_no_oob() {
    // The regression the re-fix closes: for len in 0..3, saturating_sub(4)==0 let offset 0 through.
    for len in 0..4usize {
        assert!(!dword_in_bounds(0, len), "len={len} (<4) must reject a 4-byte access at offset 0");
    }
    assert!(dword_in_bounds(0, 4), "len==4 admits exactly one dword at offset 0");
}

#[test]
fn dword_bound_is_overflow_safe() {
    assert!(!dword_in_bounds(usize::MAX, 0x80000), "offset+4 overflows → reject, not wrap");
    assert!(!dword_in_bounds(usize::MAX - 3, usize::MAX), "near-max offset overflows → reject");
}
