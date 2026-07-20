//! U5 positive gate + downstream-reliance check (`005` Method parts 2/5). The sample reviewed
//! register compiles (proving all sample entries are valid) and is consumed as typed values —
//! keeping the trybuild negative gate non-vacuous (proto's positive-paired pattern). U3/U4 rely on
//! "valid by type" without a runtime re-check; that reliance is sound because the bypass audit
//! (trybuild `route_struct_literal_bypass`) proves `Route::new` is the only construction path.

use proxy::register::{anthropic, net};
use proxy::vocab::{Route, ToolIdentity};

#[test]
fn net_register_is_populated_and_typed() {
    // The register compiled — so every entry passed the compile-time validator. Consume them as
    // the typed values U3 will match against.
    assert!(!net::ROUTES.is_empty());
    let routes: &[Route] = net::ROUTES;
    assert!(routes.iter().any(|r| r.as_str() == "https://api.anthropic.com/v1/messages"));
    // Every canonical route begins with the scheme structure the format guarantees.
    for r in routes {
        assert!(r.as_str().starts_with("https://"), "every route is canonical https");
    }
}

#[test]
fn anthropic_register_is_populated_and_typed() {
    assert!(!anthropic::VETTED_TOOLS.is_empty());
    let tools: &[ToolIdentity] = anthropic::VETTED_TOOLS;
    assert!(tools.iter().any(|t| t.as_str() == "text_editor_20250728"));
}

#[test]
fn valid_route_and_identity_construct_at_runtime_too() {
    // `new` is a const fn but also callable at runtime; a valid input round-trips.
    let r = Route::new("https://docs.rs/serde");
    assert_eq!(r.as_str(), "https://docs.rs/serde");
    let t = ToolIdentity::new("memory_20250818");
    assert_eq!(t.as_str(), "memory_20250818");
}
