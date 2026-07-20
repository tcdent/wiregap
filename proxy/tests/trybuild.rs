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
}
