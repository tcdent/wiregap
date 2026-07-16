//! Host `cargo test` of the Slice-2 pure logic (harness surface (a)): the ECAM config-address
//! arithmetic and the physical-arena layout planner + its security invariants. These are the exact
//! functions the UEFI binary calls; the raw MMIO/ACPI derefs around them are exercised in QEMU.

use substrate::arena::{self, ArenaConfig, ArenaError, ArenaLayout, Region, DMAR_PAGE, EXTENT_ALIGN};
use substrate::ecam::{config_address, decode_bar, Bar};

// ---- ECAM addressing --------------------------------------------------------------------------

#[test]
fn config_address_packs_bdf_and_offset() {
    let base = 0xB000_0000;
    // bus<<20 | dev<<15 | func<<12 | (offset & 0xfff)
    assert_eq!(config_address(base, 0, 3, 0, 0x10), base + (3 << 15) + 0x10);
    assert_eq!(config_address(base, 1, 0, 0, 0x00), base + (1 << 20));
    assert_eq!(config_address(base, 0, 0, 1, 0x04), base + (1 << 12) + 0x04);
    // Offset is masked to 12 bits (stays within the 4 KiB per-function window).
    assert_eq!(config_address(base, 0, 0, 0, 0x1004), base + 0x004);
}

#[test]
fn decode_bar_32bit_non_prefetchable() {
    // e1000e/82574L-style BAR0 (the QEMU stand-in): memory, 32-bit, non-prefetchable.
    let bar = decode_bar(0xC106_0000, 0);
    assert_eq!(bar, Bar { base: 0xC106_0000, is_64bit: false, prefetchable: false });
}

#[test]
fn decode_bar_64bit_prefetchable_the_real_82599_path() {
    // The real 82599's BAR0: 64-bit, prefetchable. low = base_low | type-64(0x4) | prefetch(0x8).
    let low = 0xC000_0000 | 0x4 | 0x8; // 0xC000_000C
    let high = 0x0000_0004;
    let bar = decode_bar(low, high);
    assert_eq!(bar, Bar { base: 0x0000_0004_C000_0000, is_64bit: true, prefetchable: true });
    // The high dword is only folded in for a 64-bit BAR — a 32-bit BAR ignores it.
    assert_eq!(decode_bar(0xC000_0000, 0xDEAD_BEEF).base, 0xC000_0000);
}

// ---- Arena layout -----------------------------------------------------------------------------

fn big_region() -> Region {
    // 8 MiB starting a little above 1 MiB, deliberately unaligned to exercise the base alignment.
    Region { base: 0x0010_1000, len: 8 * 1024 * 1024 }
}

#[test]
fn arena_is_sound_in_a_large_region() {
    let usable = big_region();
    let layout = arena::plan(usable, &ArenaConfig::NIC_82599).expect("fits");
    assert!(layout.sound(&usable), "layout must be sound: {layout:?}");
    assert!(layout.extent.within(&usable));
    assert!(layout.all_within_extent());
    assert!(layout.no_overlap());
    assert!(layout.dmar_confinable());
}

#[test]
fn extent_is_2mib_aligned_and_whole_dmar_pages() {
    let layout = arena::plan(big_region(), &ArenaConfig::NIC_82599).unwrap();
    assert_eq!(layout.extent.base % EXTENT_ALIGN, 0, "extent base is 2 MiB-aligned");
    assert_eq!(layout.extent.len % DMAR_PAGE, 0, "extent length is a whole number of DMAR pages");
    assert!(layout.extent.len > 0);
}

#[test]
fn sub_regions_have_the_82599_sizes_and_page_alignment() {
    let cfg = ArenaConfig::NIC_82599;
    let layout = arena::plan(big_region(), &cfg).unwrap();
    assert_eq!(layout.rx_ring.len, cfg.rx_depth * 16);
    assert_eq!(layout.tx_ring.len, cfg.tx_depth * 16);
    assert_eq!(layout.rx_bufs.len, cfg.rx_depth * cfg.rx_buf_len);
    assert_eq!(layout.tx_bufs.len, cfg.tx_depth * cfg.tx_buf_len);
    for r in layout.regions() {
        assert_eq!(r.base % 4096, 0, "each sub-region is page-aligned: {r:?}");
    }
}

#[test]
fn too_small_region_is_rejected_not_silently_truncated() {
    // Smaller than one extent alignment: cannot hold the 2 MiB-rounded extent.
    let tiny = Region { base: 0x0010_0000, len: 64 * 1024 };
    match arena::plan(tiny, &ArenaConfig::NIC_82599) {
        Err(ArenaError::RegionTooSmall { .. }) => {}
        other => panic!("expected RegionTooSmall, got {other:?}"),
    }
}

#[test]
fn region_within_and_overlap_predicates() {
    let outer = Region { base: 0x1000, len: 0x1000 };
    assert!(Region { base: 0x1000, len: 0x800 }.within(&outer));
    assert!(Region { base: 0x1800, len: 0x800 }.within(&outer));
    assert!(!Region { base: 0x1800, len: 0x801 }.within(&outer), "past the end is not within");
    assert!(Region { base: 0x1000, len: 0x400 }.overlaps(&Region { base: 0x1200, len: 0x400 }));
    assert!(!Region { base: 0x1000, len: 0x200 }.overlaps(&Region { base: 0x1200, len: 0x200 }), "adjacent, no overlap");
}

/// A hand-built layout with a deliberate overlap must fail the security predicate — proving the check
/// is real, not vacuous.
#[test]
fn overlap_is_detected() {
    let extent = Region { base: 0x20_0000, len: EXTENT_ALIGN };
    let overlapping = ArenaLayout {
        extent,
        rx_ring: Region { base: 0x20_0000, len: 0x1000 },
        tx_ring: Region { base: 0x20_0800, len: 0x1000 }, // overlaps rx_ring
        rx_bufs: Region { base: 0x21_0000, len: 0x1000 },
        tx_bufs: Region { base: 0x22_0000, len: 0x1000 },
    };
    assert!(!overlapping.no_overlap(), "overlap must be caught");
    assert!(!overlapping.sound(&Region { base: 0x20_0000, len: EXTENT_ALIGN }));
}
