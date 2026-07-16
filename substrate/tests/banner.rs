//! Host `cargo test` of the pure Slice-1 logic (harness surface (a)) — the boot-banner formatter and
//! the handoff predicate. These run on the host with `std` available; they exercise the *exact*
//! functions the UEFI binary calls, with no `uefi`/`x86_64` in the build (those deps are gated to the
//! bare-metal target). Establishes the no_std-lib + host-test idiom the pure reactor logic uses in
//! Slice 4.

use substrate::banner::{write_banner, BootFacts};

fn render(facts: &BootFacts) -> String {
    let mut s = String::new();
    write_banner(&mut s, facts).expect("String write is infallible");
    s
}

#[test]
fn handoff_ok_requires_rsdp_and_usable_memory() {
    let good = BootFacts { build_id: "abc", rustc: "r", rsdp: 0xE0000, usable_regions: 3, usable_pages: 1000 };
    assert!(good.handoff_ok());

    // Each necessary condition, negated independently.
    assert!(!BootFacts { rsdp: 0, ..good_like() }.handoff_ok(), "no ACPI root => not ok");
    assert!(!BootFacts { usable_regions: 0, ..good_like() }.handoff_ok(), "no usable regions => not ok");
    assert!(!BootFacts { usable_pages: 0, ..good_like() }.handoff_ok(), "no usable pages => not ok");
}

#[test]
fn banner_reports_observed_values_and_pass() {
    let facts = BootFacts { build_id: "deadbeef-dirty", rustc: "rustc 1.94.1", rsdp: 0xE0000, usable_regions: 5, usable_pages: 262144 };
    let out = render(&facts);

    // Self-pinning identity is present.
    assert!(out.contains("build-id : deadbeef-dirty"), "banner pins build-id:\n{out}");
    assert!(out.contains("rustc    : rustc 1.94.1"), "banner pins rustc:\n{out}");
    // Observed handoff facts, not a bare PASS.
    assert!(out.contains("ExitBootServices reached"), "banner states the handoff:\n{out}");
    assert!(out.contains("acpi-rsdp: 0x00000000000e0000"), "banner reports the observed rsdp:\n{out}");
    assert!(out.contains("usable-regions=5 usable-pages=262144"), "banner reports the observed memmap:\n{out}");
    // Observed -> expected assertion lines.
    assert!(out.contains("observed=0x00000000000e0000 expected!=0"), "rsdp assertion is observed->expected:\n{out}");
    assert!(out.contains("RESULT   : PASS"), "all conditions met => PASS:\n{out}");
}

#[test]
fn banner_reports_fail_when_handoff_incomplete() {
    let facts = BootFacts { build_id: "x", rustc: "x", rsdp: 0, usable_regions: 0, usable_pages: 0 };
    let out = render(&facts);
    assert!(out.contains("RESULT   : FAIL"), "a dead handoff => FAIL:\n{out}");
    assert!(out.contains("observed=0 expected>0"), "surfaces which observation failed:\n{out}");
}

/// A baseline `handoff_ok`-true facts value the struct-update tests negate one field at a time.
fn good_like() -> BootFacts<'static> {
    BootFacts { build_id: "abc", rustc: "r", rsdp: 0xE0000, usable_regions: 3, usable_pages: 1000 }
}
