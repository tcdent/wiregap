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
/// the field is private and the only constructor ([`Allowed::mint`]) is private to this module and
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
