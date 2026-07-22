//! The **safety gate** — the primary safety leg of the pure-direct path (Sprint 6). Running as a
//! userspace process on a live host with **no IOMMU**, a state-changing op on the wrong device (unbind /
//! bus-master / `resource0` mmap / DMA) is unconfined, so **every** such op routes through this one pure
//! choke-point first, and acts **only** on `Allow`. Fail-closed: any mismatch or short read is `Refuse`,
//! and the caller must not attempt the op. Pure (no I/O), so the whole invariant is host-provable with no
//! hardware — the caller supplies the target BDF, the frozen bench allowlist, and the config-space bytes
//! it read from sysfs.

/// A PCI address (domain:bus:dev.func), e.g. `0000:41:00.0`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bdf {
    pub domain: u16,
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
}

impl Bdf {
    /// The canonical sysfs form `DDDD:BB:DD.F` (used to build `/sys/bus/pci/devices/<bdf>/...` paths).
    pub fn sysfs(&self) -> String {
        format!("{:04x}:{:02x}:{:02x}.{:x}", self.domain, self.bus, self.dev, self.func)
    }

    /// Parse a canonical `DDDD:BB:DD.F` BDF string (as sysfs / lspci present it). Pure; `None` on any
    /// malformation (fail-closed — a BDF we cannot parse is never allowlisted).
    pub fn parse(s: &str) -> Option<Bdf> {
        // DDDD:BB:DD.F
        let (domain, rest) = s.split_once(':')?;
        let (bus, rest) = rest.split_once(':')?;
        let (dev, func) = rest.split_once('.')?;
        Some(Bdf {
            domain: u16::from_str_radix(domain, 16).ok()?,
            bus: u8::from_str_radix(bus, 16).ok()?,
            dev: u8::from_str_radix(dev, 16).ok()?,
            func: u8::from_str_radix(func, 16).ok()?,
        })
    }
}

/// The bench 82599 identity every allowed device must match (confirmed read-only, 2026-07-17).
pub const EXPECTED_VENDOR: u16 = 0x8086;
pub const EXPECTED_DEVICE: u16 = 0x10fb;
pub const EXPECTED_SUBSYS_VENDOR: u16 = 0x8086;
pub const EXPECTED_SUBSYS_DEVICE: u16 = 0x0003;

/// The production NIC's subsystem-device id — an **additive** denylist (defense-in-depth, never a
/// substitute for the positive allowlist + identity match). The production `61:00.0` is `0x7a12`.
pub const DENY_SUBSYS_DEVICE: u16 = 0x7a12;

/// Config-space offsets the gate reads (little-endian u16 each).
const OFF_VENDOR: usize = 0x00;
const OFF_DEVICE: usize = 0x02;
const OFF_SUBSYS_VENDOR: usize = 0x2c;
const OFF_SUBSYS_DEVICE: usize = 0x2e;
/// Minimum config bytes needed to read through the subsystem id.
const MIN_CONFIG_LEN: usize = 0x30;

/// An **authorization witness** — proof that [`gate`] returned `Allow` for a specific BDF. UN-FORGEABLE:
/// the field is private and the only constructor (`Allowed::mint`) is private to this module and
/// called **only** on `gate`'s allow arm — there is no `pub` constructor, no `Default`, no pub-field
/// literal, and deliberately no `Clone`/`Copy` (a freely-replicable witness is weaker). BOUND to its BDF:
/// every state-changing op takes `&Allowed` and derives its target from [`Allowed::bdf`], so a token for
/// `41:00.0` can authorize an op only on `41:00.0` — structurally, not by assertion. This makes
/// "no state-changing op without a passed gate" a compiler-enforced invariant, not convention.
#[derive(Debug)]
pub struct Allowed {
    bdf: Bdf,
}

impl Allowed {
    /// The ONLY constructor — private, called only inside [`gate`] on the allow arm.
    fn mint(bdf: Bdf) -> Allowed {
        Allowed { bdf }
    }

    /// The authorized BDF. State-changing ops act on exactly this.
    pub fn bdf(&self) -> Bdf {
        self.bdf
    }
}

/// The gate's verdict. Only `Allow` — carrying the un-forgeable [`Allowed`] witness — may proceed to a
/// state-changing op.
#[derive(Debug)]
pub enum Decision {
    Allow(Allowed),
    Refuse(RefuseReason),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefuseReason {
    /// The target BDF is not in the frozen bench allowlist.
    BdfNotAllowlisted,
    /// The config read was too short to confirm identity (fail-closed on a short read).
    ShortConfig,
    WrongVendor,
    WrongDevice,
    WrongSubsysVendor,
    WrongSubsysDevice,
    /// The additive denylist tripped — the config identifies the production NIC.
    ProductionDenylisted,
}

fn le16(config: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([config[off], config[off + 1]])
}

/// The pure safety gate. `Allow` **iff** `target` is in the frozen `allowlist` **and** `config` (the
/// device's own config-space bytes) identifies the bench 82599 (`8086:10FB` / subsys `8086:0003`) **and**
/// it is not the production denylist. Assert-then-act: membership first, then identity, then the additive
/// denylist. Any failure ⇒ `Refuse` (the caller performs no op).
pub fn gate(target: Bdf, allowlist: &[Bdf], config: &[u8]) -> Decision {
    if !allowlist.contains(&target) {
        return Decision::Refuse(RefuseReason::BdfNotAllowlisted);
    }
    if config.len() < MIN_CONFIG_LEN {
        return Decision::Refuse(RefuseReason::ShortConfig);
    }
    // Additive denylist first (defense-in-depth): never act on the production identity, even if the
    // allowlist were somehow wrong.
    if le16(config, OFF_SUBSYS_DEVICE) == DENY_SUBSYS_DEVICE {
        return Decision::Refuse(RefuseReason::ProductionDenylisted);
    }
    if le16(config, OFF_VENDOR) != EXPECTED_VENDOR {
        return Decision::Refuse(RefuseReason::WrongVendor);
    }
    if le16(config, OFF_DEVICE) != EXPECTED_DEVICE {
        return Decision::Refuse(RefuseReason::WrongDevice);
    }
    if le16(config, OFF_SUBSYS_VENDOR) != EXPECTED_SUBSYS_VENDOR {
        return Decision::Refuse(RefuseReason::WrongSubsysVendor);
    }
    if le16(config, OFF_SUBSYS_DEVICE) != EXPECTED_SUBSYS_DEVICE {
        return Decision::Refuse(RefuseReason::WrongSubsysDevice);
    }
    // The single mint site: only here, only after every check passed, bound to the checked target.
    Decision::Allow(Allowed::mint(target))
}

#[cfg(test)]
mod integration_tests {
//! Host tests of the pure safety gate + the `Allowed` token (Part A / Part B source half) — the primary
//! safety leg, proven fail-closed with no hardware. `Allow` only for an allowlisted bench BDF whose
//! config identifies the bench 82599, carrying a token bound to THAT BDF; `Refuse` (no op, no token) on
//! every other case. The token is UN-FORGEABLE (no `pub` constructor) — a test cannot fabricate one, it
//! can only receive it from `gate()`, which is exactly the invariant.

use super::{gate, Bdf, Decision, RefuseReason};

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
}
