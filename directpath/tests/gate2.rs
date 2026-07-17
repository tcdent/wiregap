//! Mitigation Gate 2 host tests — the pure physaddr-validation logic, fail-closed. The /proc/iomem
//! System-RAM parse + the range check are proven non-vacuously with no hardware; the VA↔PA self-check's
//! refuse-on-mismatch is the runtime half (reviewed-by-reading + a sudo self-run).

use directpath::gate2::{in_system_ram, parse_system_ram};

const IOMEM: &str = "\
00000000-00000fff : Reserved
00001000-0009ffff : System RAM
000a0000-000fffff : Reserved
00100000-7fffffff : System RAM
80000000-8fffffff : PCI Bus 0000:00
fed90000-fed93fff : dmar0
";

#[test]
fn parse_extracts_only_system_ram_ranges() {
    let r = parse_system_ram(IOMEM);
    // Two System RAM ranges; /proc/iomem ends are inclusive → we store one-past.
    assert_eq!(r, vec![(0x1000, 0xa0000), (0x100000, 0x80000000)]);
}

#[test]
fn in_system_ram_accepts_only_wholly_contained() {
    let r = parse_system_ram(IOMEM);
    // A 2 MiB buffer wholly inside the big System-RAM range.
    assert!(in_system_ram(0x0010_0000, 0x200000, &r), "inside System RAM");
    assert!(in_system_ram(0x7fe0_0000, 0x200000, &r), "ending exactly at the range end");
}

#[test]
fn out_of_system_ram_is_refused() {
    let r = parse_system_ram(IOMEM);
    assert!(!in_system_ram(0x8000_0000, 0x1000, &r), "in the PCI-bus (MMIO) range → refuse");
    assert!(!in_system_ram(0x000a_0000, 0x1000, &r), "in a Reserved range → refuse");
    assert!(!in_system_ram(0x7fff_f000, 0x200000, &r), "straddles the range end → refuse");
    assert!(!in_system_ram(u64::MAX - 4, 0x200000, &r), "overflow → refuse");
    assert!(!in_system_ram(0x1000, 0x1000, &[]), "no ranges → refuse everything (fail-closed)");
}
