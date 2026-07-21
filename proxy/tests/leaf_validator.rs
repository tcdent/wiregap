//! U1 leaf-validator certification (Sprint 5, CB1–CB4). Enumeration against an **independent
//! oracle** — the oracle is the spec's own count bound (`1 ≤ n ≤ MAX_ELEMENTS`) and proto's own
//! reject arms, computed here from the *construction* of each input, never read back from
//! `leaf_validate`'s output. This is the `segment_alphabet.rs` model (stdlib/spec oracle, not
//! the code's own output), so a mis-ranged bound (`>=` vs `>`, an off-by-one) fails here.
//!
//! The per-BYTE domain (all 256 values) is **inherited** — swept in proto's `segment_alphabet.rs`
//! and proven in `spec/scan-safe.md`; this suite certifies the array-LEVEL bound and the correct
//! delegation of the inherited floor, not the byte alphabet again.

use proto::Reject;
use proxy::leaf::{LeafDenial, MAX_ELEMENTS, leaf_validate};

const US: u8 = 0x1F;

/// Build a destination of exactly `k` valid single-char segments (`a\x1fa\x1f…a`). Each segment
/// is `"a"` — one alphanumeric, so it satisfies the ≥1-alnum rule and is never `.`/`..`; the
/// only variable across the sweep is the segment *count*.
fn dest_with_segments(k: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for i in 0..k {
        if i > 0 {
            v.push(US);
        }
        v.push(b'a');
    }
    v
}

/// LEG-A: the count boundary, swept `k ∈ [0, MAX+2]` against the independent oracle.
#[test]
fn count_bound_boundary_against_independent_oracle() {
    for k in 0..=(MAX_ELEMENTS + 2) {
        let dest = dest_with_segments(k);
        let got = leaf_validate(&dest);

        // INDEPENDENT ORACLE — derived from the spec's stated bound and proto's stated rejects,
        // from `k` (the construction), never from `leaf_validate`'s own output.
        if k == 0 {
            // Empty dest → no stack → the inherited ≥1-element floor (scan_safe EmptyStack).
            assert_eq!(
                got,
                Err(LeafDenial::Scan(Reject::EmptyStack)),
                "k=0 must deny via the inherited EmptyStack floor",
            );
        } else if k <= MAX_ELEMENTS {
            // 1 ≤ k ≤ MAX → admit, and the stack carries exactly `k` segments.
            let stack = got.expect("k in [1, MAX] must admit");
            assert_eq!(stack.len(), k, "admitted stack must carry exactly k segments");
        } else {
            // k > MAX → the one new arm, reporting the exact count and cap.
            assert_eq!(
                got,
                Err(LeafDenial::TooManyElements {
                    count: k,
                    max: MAX_ELEMENTS,
                }),
                "k>MAX must deny via TooManyElements with the exact count",
            );
        }
    }
}

/// The inherited scan floor is correctly *delegated* — a per-leaf/structural reject flows
/// through as `LeafDenial::Scan(..)`, unchanged. Representative rows only (the exhaustive
/// per-byte sweep is proto's `segment_alphabet.rs`, cited); the point here is delegation, not
/// re-sweeping the alphabet.
#[test]
fn inherited_scan_floor_is_delegated() {
    // Traversal — unsayable, surfaces as TraversalSegment.
    assert!(matches!(
        leaf_validate(b".."),
        Err(LeafDenial::Scan(Reject::TraversalSegment { .. }))
    ));
    assert!(matches!(
        leaf_validate(b"."),
        Err(LeafDenial::Scan(Reject::TraversalSegment { .. }))
    ));
    // Path separator inside a segment — out of alphabet.
    assert!(matches!(
        leaf_validate(b"a/b"),
        Err(LeafDenial::Scan(Reject::BadSegmentByte { .. }))
    ));
    // Control byte — out of alphabet.
    assert!(matches!(
        leaf_validate(&[b'a', 0x1b]),
        Err(LeafDenial::Scan(Reject::BadSegmentByte { .. }))
    ));
    // Empty segment (doubled delimiter) — EmptySegment.
    assert!(matches!(
        leaf_validate(b"a\x1f\x1fb"),
        Err(LeafDenial::Scan(Reject::EmptySegment { .. }))
    ));
    // All-punctuation segment (no alphanumeric) — SafeSegmentNotAlnum.
    assert!(matches!(
        leaf_validate(b"..."),
        Err(LeafDenial::Scan(Reject::SafeSegmentNotAlnum { .. }))
    ));
}

/// The admit path returns the real proto stack (CB4 — bind to the real primitive's output, not
/// a hand-computed expected): a valid multi-segment destination admits with its segments intact.
#[test]
fn admit_returns_the_real_scanned_stack() {
    let stack = leaf_validate(b"dir1\x1fsub_dir\x1ffile.txt").expect("valid destination admits");
    assert_eq!(stack.len(), 3);
    assert_eq!(stack.segments()[0].as_str(), "dir1");
    assert_eq!(stack.segments()[1].as_str(), "sub_dir");
    assert_eq!(stack.segments()[2].as_str(), "file.txt");
}

// ─── Sprint 6 S1: the tier-1 leaf_validate_prescanned re-cert (three pieces) ─────────────────
// (1) CB1 coherence is `cargo test` compiling-then-running (the run is the current source's).
// (2) count-bound enumeration DIRECTLY on SafeSegmentStack input.
// (3) the type-construction audit — SafeSegmentStack only via scan_safe — proving no floor is
//     dropped by skipping the scan (also asserted as a trybuild compile-fail, tests/ui/).

use proxy::leaf::leaf_validate_prescanned;

/// (2) The count boundary swept directly over `SafeSegmentStack` inputs (built via `scan_safe`, the
/// only constructor). Oracle: admit iff `k <= MAX_ELEMENTS` — from the construction, independent of
/// the code. `k=0` is *unrepresentable* at this entry (no empty stack exists), so the ≥1 lower bound
/// is inherited from the type, not re-checked — the honest-boundary discipline, at the type level.
#[test]
fn prescanned_count_bound_over_safesegmentstack_input() {
    for k in 1..=(MAX_ELEMENTS + 2) {
        let stack = proto::scan_safe(dest_with_segments(k).as_slice()).expect("k>=1 scans");
        let got = leaf_validate_prescanned(&stack);
        if k <= MAX_ELEMENTS {
            assert_eq!(got, Ok(()), "k={k} in [1,MAX] must admit");
        } else {
            assert_eq!(
                got,
                Err(LeafDenial::TooManyElements { count: k, max: MAX_ELEMENTS }),
                "k={k}>MAX must deny TooManyElements with the exact count",
            );
        }
    }
}

/// (2+3) The two entry points AGREE: for the same destination bytes, `leaf_validate(&[u8])` (scan +
/// count) and `leaf_validate_prescanned(&scan_safe(bytes))` reach the same verdict — proving the
/// refactor is behavior-preserving and the count-bound impl is shared, not duplicated.
#[test]
fn both_entry_points_agree() {
    for k in [1usize, 2, MAX_ELEMENTS, MAX_ELEMENTS + 1] {
        let dest = dest_with_segments(k);
        let full = leaf_validate(&dest).map(|_| ());
        let stack = proto::scan_safe(&dest).expect("scans");
        let pre = leaf_validate_prescanned(&stack);
        assert_eq!(full, pre, "the two entry points must agree at k={k}");
    }
}
