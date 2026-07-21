//! U5 part 2 + part 4 — the compile-fail corpus (C1) and the bypass audit, as `trybuild` cases
//! (proto's `tests/trybuild.rs` pattern). Each must fail to compile against its pinned `.stderr`.
//! The POSITIVE (a valid register compiles + is consumed) lives in `tests/register.rs`, so this
//! negative gate is not vacuous.
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/malformed_route.rs");
    t.compile_fail("tests/ui/malformed_tool_identity.rs");
    t.compile_fail("tests/ui/route_struct_literal_bypass.rs");
    // Sprint 6 S1: SafeSegmentStack has no bypass constructor (leaf_validate_prescanned soundness).
    t.compile_fail("tests/ui/safesegmentstack_no_bypass_constructor.rs");
}
