//! G1 — host `cargo test` of the DMAR confinement in PURE logic (the real shipped `dmar::build`/`walk`).
//! Proves the table BYTES map only the arena: an in-arena IOVA resolves to its identity phys, and the
//! NEGATIVE space is POSITIVELY unmapped — a hostile-IOVA sweep, a different BDF, and the DMAR-table
//! region itself all resolve `None`. Slice 1 proves bytes, not enforcement (that is Slice 2 / QEMU).

use substrate::arena::Region;
use substrate::dmar::{
    build, build_irt, decode_frcd, decode_irte, walk, DmarConfig, DmarError, FaultRecord, LARGE_PAGE,
};

const BUS: u8 = 0;
const DEVFN: u8 = 0x18; // device 3, function 0 (the Sprint-4 NIC BDF 00:03.0)

/// A config mapping a 2 MiB arena, with the tables in a disjoint region just above it.
fn cfg(levels: u8) -> DmarConfig {
    let arena = Region { base: 0x0160_0000, len: LARGE_PAGE }; // the real Sprint-4 extent
    let table_region = Region { base: 0x0180_0000, len: 64 * 1024 }; // disjoint, above the arena
    DmarConfig { arena, table_region, bus: BUS, devfn: DEVFN, levels, domain_id: 1 }
}

#[test]
fn in_arena_iova_resolves_to_identity_phys() {
    let img = build(&cfg(3)).unwrap();
    let a = 0x0160_0000u64;
    assert_eq!(walk(&img, BUS, DEVFN, a), Some(a), "arena base → identity");
    assert_eq!(walk(&img, BUS, DEVFN, a + 0x1000), Some(a + 0x1000), "mid-page → identity");
    assert_eq!(walk(&img, BUS, DEVFN, a + LARGE_PAGE - 1), Some(a + LARGE_PAGE - 1), "last byte → identity");
}

#[test]
fn negative_space_is_positively_unmapped() {
    let img = build(&cfg(3)).unwrap();
    let base = 0x0160_0000u64;
    let end = base + LARGE_PAGE;
    // The hostile-IOVA sweep the contract names: every one must resolve None.
    for (name, iova) in [
        ("zero", 0u64),
        ("just-below-arena", base - 1),
        ("arena-end (one past)", end),
        ("double-arena-base", base * 2),
        ("high-bit-set", 1u64 << 47),
        ("u64-max", u64::MAX),
        ("u64-page-max", 0xFFFF_FFFF_FFFF_F000),
    ] {
        assert_eq!(walk(&img, BUS, DEVFN, iova), None, "{name} must be unmapped");
    }
}

#[test]
fn a_different_bdf_is_unmapped() {
    let img = build(&cfg(3)).unwrap();
    let a = 0x0160_0000u64;
    assert_eq!(walk(&img, BUS, DEVFN, a), Some(a), "our BDF maps the arena");
    assert_eq!(walk(&img, BUS, 0x20, a), None, "a different devfn has no context entry");
    assert_eq!(walk(&img, 1, DEVFN, a), None, "a different bus has no root entry");
}

#[test]
fn the_dmar_table_region_itself_is_unmapped() {
    // The self-referential-containment guard: the region holding the tables is NOT in the SL map, so no
    // IOVA resolves into it — a device cannot DMA to (and rewrite) its own page tables.
    let c = cfg(3);
    let img = build(&c).unwrap();
    assert_eq!(walk(&img, BUS, DEVFN, c.table_region.base), None, "table-region base is unmapped");
    assert_eq!(walk(&img, BUS, DEVFN, c.table_region.base + 0x1000), None, "into the table region: unmapped");
}

#[test]
fn overlapping_table_region_is_rejected_not_silently_built() {
    let arena = Region { base: 0x0160_0000, len: LARGE_PAGE };
    let overlapping = Region { base: 0x0160_0000, len: 0x1000 }; // inside the arena
    let c = DmarConfig { arena, table_region: overlapping, bus: BUS, devfn: DEVFN, levels: 3, domain_id: 1 };
    assert!(
        matches!(build(&c), Err(DmarError::TableRegionOverlapsArena)),
        "tables inside the arena = a containment hole"
    );
}

#[test]
fn four_level_geometry_also_confines() {
    // A 48-bit / 4-level config maps the same arena and confines identically — the geometry is a
    // parameter (CAP/SAGAW), not a baked 3-level assumption.
    let img = build(&cfg(4)).unwrap();
    let a = 0x0160_0000u64;
    assert_eq!(walk(&img, BUS, DEVFN, a), Some(a), "4-level: arena maps");
    assert_eq!(walk(&img, BUS, DEVFN, a + LARGE_PAGE), None, "4-level: just past the arena is unmapped");
    assert_eq!(walk(&img, BUS, DEVFN, 0), None, "4-level: zero is unmapped");
}

#[test]
fn frcd_decode_extracts_iova_sid_reason() {
    // F set: a read fault at IOVA 0x1A0_0000 from source-id 0x18 (BDF 00:03.0), reason 0x06.
    let lo = 0x01A0_0000u64; // faulting address (page-aligned)
    let hi = (1u64 << 63) | (1u64 << 62) | (0x06u64 << 32) | 0x0018;
    assert_eq!(
        decode_frcd(lo, hi),
        Some(FaultRecord { iova: 0x01A0_0000, sid: 0x0018, reason: 0x06, is_read: true }),
        "decodes the faulting IOVA + source id + reason"
    );
    // F clear: no fault recorded.
    assert_eq!(decode_frcd(lo, 0), None, "no F bit → no fault");
    // A write fault (T=0).
    let write_hi = (1u64 << 63) | (0x05u64 << 32) | 0x0018;
    assert!(!decode_frcd(lo, write_hi).unwrap().is_read, "T=0 → write");
}

#[test]
fn irt_entry_is_present_and_source_validated() {
    // The (B) interrupt-remapping (i)-source property: index 0 is present, for the NIC's vector, and
    // source-validated against the NIC's SID (SVT != 0); every other index is absent.
    let irt = build_irt(256, 0x40, 0x00, 0x0018);
    let e0 = decode_irte(
        u64::from_le_bytes(irt[0..8].try_into().unwrap()),
        u64::from_le_bytes(irt[8..16].try_into().unwrap()),
    );
    assert!(e0.present, "IRTE[0] present");
    assert_eq!(e0.vector, 0x40, "the NIC's vector");
    assert_eq!(e0.sid, 0x0018, "validated against the NIC SID (BDF 00:03.0)");
    assert_ne!(e0.svt, 0, "source validation ENABLED (a different SID cannot use this entry)");
    let e1 = decode_irte(
        u64::from_le_bytes(irt[16..24].try_into().unwrap()),
        u64::from_le_bytes(irt[24..32].try_into().unwrap()),
    );
    assert!(!e1.present, "every other IRTE is absent");
}

#[test]
fn bad_levels_and_unaligned_arena_are_rejected() {
    let arena = Region { base: 0x0160_0000, len: LARGE_PAGE };
    let table_region = Region { base: 0x0180_0000, len: 64 * 1024 };
    let base = DmarConfig { arena, table_region, bus: BUS, devfn: DEVFN, levels: 3, domain_id: 1 };
    assert!(matches!(build(&DmarConfig { levels: 2, ..base }), Err(DmarError::BadLevels)));
    let bad_arena = Region { base: 0x0160_1000, len: LARGE_PAGE }; // not 2 MiB-aligned
    assert!(matches!(build(&DmarConfig { arena: bad_arena, ..base }), Err(DmarError::ArenaNotLargePageAligned)));
}
