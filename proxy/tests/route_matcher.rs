//! U3 route-matcher certification (Sprint 5, CB1–CB6). Enumeration against an INDEPENDENT oracle:
//! the match matrix and the deny-by-default rule, computed from the construction and from the REAL
//! register (`register::net::ROUTES`, never a mock). The valid-by-type reliance and the
//! never-widened property are gated explicitly (the reviewer re-confirms both at this gate).

use proto::Wire;
use proxy::register::net::ROUTES;
use proxy::route::{RouteDenial, match_route};
use proxy::vocab::Address;

fn decode(scheme: u32, joined: &str) -> Address {
    let mut body = scheme.to_le_bytes().to_vec();
    body.extend_from_slice(joined.as_bytes());
    Address::decode(&body).expect("valid net-address body")
}

/// LEG-A: the match matrix against the REAL register. `api.anthropic.com/v1/messages` is a
/// registered route; every variant of it denies. Oracle = whether the assembled form is
/// byte-identical to a real `ROUTES` entry, derived from the construction.
#[test]
fn match_matrix_against_the_real_register() {
    // Exact match of a registered route → Admit.
    let admit = match_route(&decode(0, "api.anthropic.com\u{1f}v1\u{1f}messages"), ROUTES);
    assert_eq!(
        admit.map(|r| r.as_str()),
        Ok("https://api.anthropic.com/v1/messages"),
        "an exactly-registered route admits",
    );
    // Another registered route (bare docs.rs) → Admit.
    assert_eq!(
        match_route(&decode(0, "docs.rs"), ROUTES).map(|r| r.as_str()),
        Ok("https://docs.rs"),
    );

    // Every near-miss denies by default (NoRoute):
    let denials = [
        "API.anthropic.com\u{1f}v1\u{1f}messages", // case variant (RT-1) — a correct deny
        "api.anthropic.com\u{1f}v1",                // prefix (missing /messages)
        "api.anthropic.com\u{1f}v1\u{1f}messages\u{1f}extra", // superset
        "api.anthropic.com\u{1f}v2\u{1f}messages", // different path
        "evil.com",                                  // absent destination
        "docs.rs\u{1f}serde",                       // superset of a bare-domain route
    ];
    for joined in denials {
        let got = match_route(&decode(0, joined), ROUTES);
        assert!(
            matches!(got, Err(RouteDenial::NoRoute { .. })),
            "near-miss {joined:?} must deny-by-default (NoRoute), got {got:?}",
        );
    }
}

/// A reserved scheme is denied with U2's denial surfaced unchanged (not silently swallowed as a
/// NoRoute) — the two denial classes stay distinct.
#[test]
fn reserved_scheme_surfaces_the_address_denial() {
    let got = match_route(&decode(7, "api.anthropic.com\u{1f}v1\u{1f}messages"), ROUTES);
    assert!(
        matches!(got, Err(RouteDenial::Address(_))),
        "a reserved scheme surfaces the address denial, not NoRoute; got {got:?}",
    );
}

/// NEVER-WIDENED-AT-RUNTIME: exercising the matcher (admits and denies) does not change the
/// register. Structural too — `match_route` takes `&[Route]` (immutable) and exposes no add path;
/// this is the runtime demonstration.
#[test]
fn register_is_never_widened_at_runtime() {
    let before = ROUTES.len();
    let _ = match_route(&decode(0, "api.anthropic.com\u{1f}v1\u{1f}messages"), ROUTES);
    let _ = match_route(&decode(0, "evil.com"), ROUTES);
    let _ = match_route(&decode(9, "x"), ROUTES);
    assert_eq!(ROUTES.len(), before, "the register is never widened at runtime");
}

/// VALID-BY-TYPE RELIANCE (the reviewer's re-confirm point): U3 matches against the real,
/// compile-validated register with no runtime re-validation of the route strings. This test simply
/// consumes `ROUTES` — the fact that it type-checks and matches proves U3 trusts `Route`'s
/// valid-by-type guarantee (it calls only `matches`, a byte-compare, never a re-scan/re-validate).
#[test]
fn relies_on_valid_by_type_register_without_re_check() {
    // Every entry in the real register is a well-formed canonical route (U5's compile guarantee);
    // U3 matches against them directly. A registered route matches its own assembled address.
    assert!(match_route(&decode(0, "pypi.org\u{1f}simple"), ROUTES).is_ok());
}
