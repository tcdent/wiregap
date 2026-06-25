//! Coverage closeout (009): targeted tests for the reachable library lines the conformance/round-trip
//! suite never exercises — public-facing `Display`/`Debug` impls and accessors. All via the **public
//! API** (`proto::…`), so the anchored files (`segment.rs`/`cell.rs`) stay byte-identical. No new
//! behavior — these assert what the crate already does.

use proto::{Reject, RowCol, decode_message, scan_safe, scan_unsafe};

/// `Reject::Display` over **every** variant — the conformance suite compares rejects by `PartialEq`
/// and never formats them, so the whole `Display` impl was uncovered. (Reject's variants are public
/// and constructible from a downstream crate.)
#[test]
fn reject_display_formats_every_variant() {
    let variants = [
        Reject::UnsupportedVerb { verb: 0x99 },
        Reject::ShortBody { needed: 4, got: 1 },
        Reject::TrailingBytes { extra: 2 },
        Reject::EmptyStack,
        Reject::EmptySegment { index: 0 },
        Reject::SafeSegmentNotAlnum { index: 1 },
        Reject::TraversalSegment { index: 2 },
        Reject::BadSegmentByte {
            index: 0,
            offset: 1,
            byte: 0x2F,
        },
        Reject::PartialRecord {
            record_len: 4,
            remainder: 3,
        },
        Reject::UnterminatedRecord { offset: 5 },
        Reject::ReassemblyOverflow { cap: 1024 },
    ];
    for r in variants {
        let s = alloc_format(r);
        assert!(!s.is_empty(), "Display for {r:?} must be non-empty");
    }
}

fn alloc_format(r: Reject) -> String {
    format!("{r}")
}

/// `UnsafeSegment`'s redacting `Debug` — also a security guard: the real name must NOT appear, only a
/// byte count. Reached via the public `scan_unsafe` + `UnsafeSegmentStack::segments`.
#[test]
fn unsafe_segment_debug_redacts() {
    let stack = scan_unsafe(b"secret-name.txt").expect("a 0x1F-free name scans");
    let seg = &stack.segments()[0];
    let shown = format!("{seg:?}");
    assert!(shown.contains("bytes"), "redacted Debug shows a byte count: {shown}");
    assert!(
        !shown.contains("secret"),
        "redacted Debug must NOT leak the real name: {shown}"
    );
    // the stack's own redacting Debug (the sibling impl) — a segment count, never the names.
    let stack_shown = format!("{stack:?}");
    assert!(
        stack_shown.contains("segments"),
        "stack Debug shows a segment count: {stack_shown}"
    );
    assert!(
        !stack_shown.contains("secret"),
        "stack Debug must NOT leak names: {stack_shown}"
    );
}

/// The `is_empty` accessors on both segment stacks (public API, never called by the suite). A scanned
/// stack always holds ≥ 1 segment, so both assert `!is_empty()` — the point is to execute the line.
#[test]
fn segment_stacks_is_empty() {
    let safe = scan_safe(b"docs").expect("an alnum name scans safe");
    assert!(!safe.is_empty());
    let unsafe_stack = scan_unsafe(b"my file").expect("a 0x1F-free name scans unsafe");
    assert!(!unsafe_stack.is_empty());
}

/// `RowCol::row()`/`col()` accessors (public API; the codec reads the packed `RowCol` but never the
/// individual accessors).
#[test]
fn rowcol_row_col_accessors() {
    let rc = RowCol::new(3, 5).expect("12+12 bits fit 3/5");
    assert_eq!(rc.row(), 3);
    assert_eq!(rc.col(), 5);
}

/// `focus` with a > 1-byte body → `TrailingBytes`, via `decode_message` dispatch — the suite only
/// tested 0/1-byte focus bodies, leaving the trailing-bytes reject path uncovered.
#[test]
fn focus_trailing_bytes_via_dispatch() {
    assert_eq!(
        decode_message(&[0x17, 0x00, 0x00, 0x00, 0x01, 0x00]),
        Err(Reject::TrailingBytes { extra: 1 })
    );
}
