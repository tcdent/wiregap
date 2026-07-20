//! U3 — route matcher / register enforcement (`spec/address-recognizer.md` §1, §5;
//! `contract/box-proxy.md` route-matcher row). Runs U2's [`recognize`] (scheme gate + assemble),
//! then byte-exact [`matches`] the canonical form against the operator's **real** register (U5,
//! [`crate::register`]), admitting the first match or denying by default. Pure — no I/O.
//!
//! Three properties, each gated:
//! - **Deny-by-default.** No matching route → [`RouteDenial::NoRoute`]. The *absence* of a match is
//!   the deny; nothing is admitted that was not explicitly registered.
//! - **Never widened at runtime.** The register is an immutable `&[Route]`; U3 exposes **no** path
//!   that adds to it — structural (an immutable borrow cannot be mutated), so a widening can only
//!   happen at the source, by edit + recompile + re-review (U5's C4).
//! - **Relies on valid-by-type, no runtime re-check.** U3 trusts every `&Route` is already a
//!   well-formed canonical form (U5's compile-time guarantee) and does **not** re-validate it — it
//!   only byte-compares. This is sound *because* U5's bypass audit proves `Route::new` is the only
//!   constructor, so no ill-formed `Route` can exist to be trusted.

use alloc::string::String;

use crate::address::{AddressDenial, matches, recognize};
use crate::vocab::{Address, Route};

/// Why a request was denied at the register.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RouteDenial {
    /// The address failed U2's recognition (a reserved scheme, etc.) — surfaced unchanged.
    Address(AddressDenial),
    /// The address recognized but matched no route in the register — **deny-by-default**.
    NoRoute { assembled: String },
}

/// Match a decoded `net-address` against the operator's route register. Admits the first exact
/// match (returning the matched [`Route`], `Copy`); denies by default if none matches. `register`
/// is borrowed **immutably** — U3 never widens it.
///
/// Total: every `(addr, register)` maps to exactly one `Ok(route)` or `Err(denial)`.
pub fn match_route(addr: &Address, register: &[Route]) -> Result<Route, RouteDenial> {
    // U2 recognizes the incoming address (scheme gate + assemble). Note: recognition runs on the
    // box-authored ADDRESS; the registered routes are trusted valid-by-type and never re-checked.
    let canonical = recognize(addr).map_err(RouteDenial::Address)?;
    for route in register {
        if matches(&canonical, route) {
            return Ok(*route);
        }
    }
    Err(RouteDenial::NoRoute {
        assembled: canonical,
    })
}
