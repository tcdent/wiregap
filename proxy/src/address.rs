//! U2 — the address recognizer (`spec/address-recognizer.md`): the net/address service's route
//! gate as **pure functions** over a decoded `proto::NetAddress`. Consumes the shared vocabulary
//! ([`crate::vocab`]) and the pinned [`assemble`] format; produces the canonical form U3 matches
//! against the register (Slice 4), or a legible denial. No I/O.
//!
//! The per-leaf/structural and alphabet floors are **inherited** — an incoming `net-address` is
//! already `scan_safe`-decoded (`proto::NetAddress::decode`), so `:` (port), `@` (userinfo), `#`
//! (fragment), `%` (percent-encoding), `?` (query), and `..`/`.` are **unsayable** upstream and can
//! never reach `recognize` (they fail to *decode*). This module adds the **scheme gate** and the
//! **byte-exact match**.

use alloc::string::String;

use crate::vocab::{Address, Route, Scheme, assemble};

/// Why an address was denied by the recognizer (before register lookup — that is U3's).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AddressDenial {
    /// The scheme is not https (a reserved value) → deny the whole request
    /// (`spec/address-recognizer.md` §4). Carries the raw reserved value for a legible denial.
    ReservedScheme { value: u32 },
    /// An empty address stack — unreachable for a `scan_safe`-decoded address (≥1 segment); handled
    /// so `recognize` is total.
    EmptyAddress,
}

/// Recognize a decoded `net-address`: gate the scheme (https only — every reserved value denies,
/// `spec/address-recognizer.md` §4), then produce its canonical [`assemble`] form. The register
/// lookup is U3's; this yields the canonical string U3 byte-exact matches.
///
/// Total: every `(scheme, stack)` maps to exactly one `Ok(canonical)` or `Err(denial)`.
pub fn recognize(addr: &Address) -> Result<String, AddressDenial> {
    match addr.scheme {
        Scheme::Https => {}
        Scheme::Other(reserved) => {
            return Err(AddressDenial::ReservedScheme {
                value: reserved.get(),
            });
        }
    }
    assemble(addr).ok_or(AddressDenial::EmptyAddress)
}

/// The byte-exact match primitive (`spec/address-recognizer.md` §5): a canonical address matches a
/// route iff their bytes are **identical** — no case folding, no dot-segment resolution, no
/// percent-decoding, no trailing-slash tolerance (verify-and-deny, never rewrite). U3 applies this
/// over the whole register with deny-by-default.
#[must_use]
pub fn matches(canonical: &str, route: &Route) -> bool {
    canonical == route.as_str()
}
