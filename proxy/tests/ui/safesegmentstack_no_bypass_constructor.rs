// Type-construction audit (Sprint 6 S1, leaf_validate_prescanned soundness): a SafeSegmentStack can
// be obtained ONLY via proto::scan_safe. A struct-literal construction must fail — the interior is a
// private field, so scan_safe is the sole constructor and a prescanned stack is provably scanned.
fn main() {
    let _bypass = proto::SafeSegmentStack(std::vec::Vec::new());
    let _ = _bypass;
}
