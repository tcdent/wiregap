//! G4 binding negative gate: the safe-to-log capability is a *compile-time* guarantee. Each case
//! below must fail to compile against its pinned `.stderr`. The G4 positive (the capability used on
//! a `SafeSegment`) lives in `segment_rejects.rs` so this gate is not vacuous.

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    // type boundary: an UnsafeSegmentStack cannot stand in for the &SafeSegmentStack a logger wants
    t.compile_fail("tests/ui/log_unsafe_segment.rs");
    // capability absence: an UnsafeSegment has no `as_str`
    t.compile_fail("tests/ui/unsafe_segment_has_no_as_str.rs");
}
