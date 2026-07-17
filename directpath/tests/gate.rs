//! Host tests of the pure safety gate + the `Allowed` token (Part A / Part B source half) — the primary
//! safety leg, proven fail-closed with no hardware. `Allow` only for an allowlisted bench BDF whose
//! config identifies the bench 82599, carrying a token bound to THAT BDF; `Refuse` (no op, no token) on
//! every other case. The token is UN-FORGEABLE (no `pub` constructor) — a test cannot fabricate one, it
//! can only receive it from `gate()`, which is exactly the invariant.

use directpath::gate::{gate, Bdf, Decision, RefuseReason};

fn allowlist() -> Vec<Bdf> {
    vec![Bdf::parse("0000:41:00.0").unwrap(), Bdf::parse("0000:63:00.0").unwrap()]
}

fn bench_config() -> Vec<u8> {
    let mut c = vec![0u8; 0x40];
    c[0x00..0x02].copy_from_slice(&0x8086u16.to_le_bytes()); // vendor
    c[0x02..0x04].copy_from_slice(&0x10fbu16.to_le_bytes()); // device
    c[0x2c..0x2e].copy_from_slice(&0x8086u16.to_le_bytes()); // subsys vendor
    c[0x2e..0x30].copy_from_slice(&0x0003u16.to_le_bytes()); // subsys device
    c
}

/// Assert the gate refused with a specific reason.
fn assert_refused(d: Decision, reason: RefuseReason) {
    match d {
        Decision::Refuse(r) => assert_eq!(r, reason),
        Decision::Allow(a) => panic!("expected Refuse({reason:?}), got Allow({:?})", a.bdf()),
    }
}

#[test]
fn bdf_parse_and_sysfs_roundtrip_fail_closed_on_garbage() {
    let b = Bdf::parse("0000:41:00.0").unwrap();
    assert_eq!(b, Bdf { domain: 0, bus: 0x41, dev: 0, func: 0 });
    assert_eq!(b.sysfs(), "0000:41:00.0", "sysfs path form round-trips");
    assert_eq!(Bdf::parse("0000:63:00.0").unwrap().sysfs(), "0000:63:00.0");
    assert_eq!(Bdf::parse("garbage"), None);
    assert_eq!(Bdf::parse("0000:41:00"), None, "missing .func → unparseable → never allowlisted");
    assert_eq!(Bdf::parse("zzzz:41:00.0"), None, "non-hex → None");
}

#[test]
fn allowed_token_is_bound_to_the_gated_bdf() {
    for s in ["0000:41:00.0", "0000:63:00.0"] {
        let t = Bdf::parse(s).unwrap();
        match gate(t, &allowlist(), &bench_config()) {
            Decision::Allow(a) => assert_eq!(a.bdf(), t, "the token authorizes exactly the gated BDF"),
            Decision::Refuse(r) => panic!("{s} should Allow, got Refuse({r:?})"),
        }
    }
}

#[test]
fn non_allowlisted_bdf_is_refused_even_with_bench_identity() {
    let prod = Bdf::parse("0000:61:00.0").unwrap();
    assert_refused(gate(prod, &allowlist(), &bench_config()), RefuseReason::BdfNotAllowlisted);
}

#[test]
fn production_identity_trips_the_additive_denylist() {
    let mut prod_cfg = bench_config();
    prod_cfg[0x2e..0x30].copy_from_slice(&0x7a12u16.to_le_bytes());
    let t = Bdf::parse("0000:41:00.0").unwrap();
    assert_refused(gate(t, &allowlist(), &prod_cfg), RefuseReason::ProductionDenylisted);
}

#[test]
fn wrong_identity_fields_each_refuse() {
    let t = Bdf::parse("0000:41:00.0").unwrap();
    let al = allowlist();

    let mut c = bench_config();
    c[0x00..0x02].copy_from_slice(&0x1234u16.to_le_bytes());
    assert_refused(gate(t, &al, &c), RefuseReason::WrongVendor);

    let mut c = bench_config();
    c[0x02..0x04].copy_from_slice(&0x1533u16.to_le_bytes());
    assert_refused(gate(t, &al, &c), RefuseReason::WrongDevice);

    let mut c = bench_config();
    c[0x2c..0x2e].copy_from_slice(&0x1043u16.to_le_bytes());
    assert_refused(gate(t, &al, &c), RefuseReason::WrongSubsysVendor);

    let mut c = bench_config();
    c[0x2e..0x30].copy_from_slice(&0x0004u16.to_le_bytes());
    assert_refused(gate(t, &al, &c), RefuseReason::WrongSubsysDevice);
}

#[test]
fn short_config_fails_closed() {
    let t = Bdf::parse("0000:41:00.0").unwrap();
    assert_refused(gate(t, &allowlist(), &[0u8; 0x10]), RefuseReason::ShortConfig);
    assert_refused(gate(t, &allowlist(), &[]), RefuseReason::ShortConfig);
}
