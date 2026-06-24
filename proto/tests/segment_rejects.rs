//! Seam 1A behavioural gates: G5 (traversal/structural safety), the G1 segment reject-edges
//! (empty stack / empty segment / safe-with-0-alnum as typed `Err`), the accept paths and alphabet
//! boundary bytes, and the G4 **positive** (the safe-to-log capability compiles and works on a
//! `SafeSegment`, so the compile-fail negative is not vacuous).
//!
//! Integration tests link `std`; the library under test is `no_std` (proven separately by the
//! lib-only build, G6.d).

use proto::{Reject, SafeSegmentStack, scan_safe, scan_unsafe};

const US: u8 = 0x1F;

// ----- G1 reject-edge: empty stack -------------------------------------------------------------

#[test]
fn empty_input_is_empty_stack() {
    assert_eq!(scan_safe(b""), Err(Reject::EmptyStack));
    assert_eq!(scan_unsafe(b""), Err(Reject::EmptyStack));
}

// ----- G1 reject-edge: empty segment (leading / trailing / doubled delimiter) -------------------

#[test]
fn empty_segment_is_rejected() {
    // leading delimiter -> empty segment at 0
    assert_eq!(scan_safe(b"\x1Fa"), Err(Reject::EmptySegment { index: 0 }));
    // trailing delimiter -> empty segment at the tail (no trailing-delimiter canonical form)
    assert_eq!(scan_safe(b"a\x1F"), Err(Reject::EmptySegment { index: 1 }));
    // doubled delimiter -> empty middle segment
    assert_eq!(
        scan_safe(b"a\x1F\x1Fb"),
        Err(Reject::EmptySegment { index: 1 })
    );
    // same structural rule on the unsafe scan
    assert_eq!(
        scan_unsafe(b"a\x1F"),
        Err(Reject::EmptySegment { index: 1 })
    );
}

// ----- G1 reject-edge: safe segment with zero alphanumerics -------------------------------------

#[test]
fn safe_segment_needs_an_alphanumeric() {
    assert_eq!(
        scan_safe(b"_-"),
        Err(Reject::SafeSegmentNotAlnum { index: 0 })
    );
    // three dots: not the `.`/`..` traversal forms, but still no alphanumeric
    assert_eq!(
        scan_safe(b"..."),
        Err(Reject::SafeSegmentNotAlnum { index: 0 })
    );
    // the failing segment's index is reported
    assert_eq!(
        scan_safe(b"ok\x1F_"),
        Err(Reject::SafeSegmentNotAlnum { index: 1 })
    );
}

// ----- G5: traversal is structurally unsayable --------------------------------------------------

#[test]
fn dot_and_dotdot_are_traversal_rejects() {
    // unsafe alphabet (real names) — the rule that matters most
    assert_eq!(scan_unsafe(b"."), Err(Reject::TraversalSegment { index: 0 }));
    assert_eq!(scan_unsafe(b".."), Err(Reject::TraversalSegment { index: 0 }));
    assert_eq!(
        scan_unsafe(b"dir\x1F.."),
        Err(Reject::TraversalSegment { index: 1 })
    );
    // safe alphabet — checked before the alphabet pass, so the reason is traversal, not no-alnum
    assert_eq!(scan_safe(b"."), Err(Reject::TraversalSegment { index: 0 }));
    assert_eq!(scan_safe(b".."), Err(Reject::TraversalSegment { index: 0 }));
}

// ----- G5: the forbidden bytes (`/`, NUL) — `0x1F` is structural (delimiter, never content) ------

#[test]
fn slash_and_nul_are_rejected() {
    assert_eq!(
        scan_unsafe(b"a/b"),
        Err(Reject::BadSegmentByte {
            index: 0,
            offset: 1,
            byte: b'/'
        })
    );
    assert_eq!(
        scan_unsafe(b"a\0b"),
        Err(Reject::BadSegmentByte {
            index: 0,
            offset: 1,
            byte: 0
        })
    );
    // safe alphabet rejects `/` and NUL too (outside `[A-Za-z0-9._-]`)
    assert_eq!(
        scan_safe(b"a/b"),
        Err(Reject::BadSegmentByte {
            index: 0,
            offset: 1,
            byte: b'/'
        })
    );
}

#[test]
fn unit_separator_only_delimits_never_appears_in_content() {
    // `0x1F` splits; it can never be a verified byte inside a segment.
    let stack = scan_unsafe(b"a\x1Fb").expect("two segments");
    assert_eq!(stack.len(), 2);
    assert_eq!(stack.segments()[0].as_bytes(), b"a");
    assert_eq!(stack.segments()[1].as_bytes(), b"b");
    assert!(!stack.segments().iter().any(|s| s.as_bytes().contains(&US)));
}

// ----- G5: alphabet boundary bytes (safe scan) --------------------------------------------------

#[test]
fn safe_alphabet_boundaries() {
    // every accepted class, plus `.`/`_`/`-`, in one segment (has alphanumerics)
    let stack = scan_safe(b"AZaz09._-").expect("accepted");
    assert_eq!(stack.len(), 1);
    assert_eq!(stack.segments()[0].as_str(), "AZaz09._-");

    // bytes just outside each alphanumeric run are rejected
    for bad in [
        b'/', // 0x2F, below '0'
        b':', // 0x3A, above '9'
        b'@', // 0x40, below 'A'
        b'[', // 0x5B, above 'Z'
        b'`', // 0x60, below 'a'
        b'{', // 0x7B, above 'z'
        b' ', // 0x20, space
    ] {
        let input = [b'a', bad];
        assert_eq!(
            scan_safe(&input),
            Err(Reject::BadSegmentByte {
                index: 0,
                offset: 1,
                byte: bad
            }),
            "byte {bad:#04x} should be rejected by the safe alphabet"
        );
    }
}

// ----- accept paths: the scan verifies, it never transforms -------------------------------------

#[test]
fn safe_scan_preserves_bytes_exactly() {
    // no case-folding, no normalization
    let stack = scan_safe(b"AbC.1_2-3").expect("accepted");
    assert_eq!(stack.segments()[0].as_str(), "AbC.1_2-3");
}

#[test]
fn safe_multi_segment() {
    let stack = scan_safe(b"api\x1Fv1\x1Fmessages").expect("three segments");
    assert_eq!(stack.len(), 3);
    let got: Vec<&str> = stack.segments().iter().map(|s| s.as_str()).collect();
    assert_eq!(got, ["api", "v1", "messages"]);
}

#[test]
fn unsafe_scan_accepts_real_names_including_non_utf8() {
    // spaces and UTF-8 are real filenames; the unsafe alphabet allows them
    let stack = scan_unsafe("Résumé final.pdf".as_bytes()).expect("accepted");
    assert_eq!(stack.segments()[0].as_bytes(), "Résumé final.pdf".as_bytes());

    // even non-UTF-8 bytes are carried verbatim (display-only); the scan never transforms
    let stack = scan_unsafe(b"weird\xff\xfename").expect("accepted");
    assert_eq!(stack.segments()[0].as_bytes(), b"weird\xff\xfename");

    let stack = scan_unsafe(b"dir\x1Fsub\x1Ffile name.txt").expect("three segments");
    assert_eq!(stack.len(), 3);
    assert_eq!(stack.segments()[2].as_bytes(), b"file name.txt");
}

// ----- G4 positive: the safe-to-log capability compiles and works on a SafeSegment --------------

/// A representative consumer logger: it accepts only what is safe to log and uses the
/// `SafeSegment::as_str` capability. The compile-fail negative (`tests/ui/`) feeds an
/// `UnsafeSegmentStack` to this exact shape and must fail to compile.
fn audit_log(address: &SafeSegmentStack) -> String {
    let mut out = String::new();
    for (i, seg) in address.segments().iter().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(seg.as_str()); // <- the capability only SafeSegment carries
    }
    out
}

#[test]
fn safe_to_log_capability_is_usable_on_safe_segments() {
    let address = scan_safe(b"api\x1Fv1\x1Fmessages").expect("safe address");
    assert_eq!(audit_log(&address), "api/v1/messages");
}
