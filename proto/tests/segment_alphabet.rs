//! Exhaustive byte-alphabet proof for the 011 segment-scan `match` refactor (behavior-identical). For
//! **every** `u8`, the refactored `scan_safe`/`scan_unsafe` accept/reject the byte exactly as the
//! pre-refactor logic did — the oracle is the OLD per-byte classification (the stdlib
//! `is_ascii_alphanumeric` + the literal alphabet), so this proves `new == old`, not `new == itself`. A
//! mis-ranged `match` arm (one of 256 byte values flipped) fails here.
//!
//! Via the public API (`scan_safe`/`scan_unsafe`) — no in-anchor test code. `0x1F` is the **separator**,
//! never a content byte (its doubled/leading/trailing behavior → `EmptySegment` is covered by
//! `segment_rejects.rs`), so it is skipped in this per-byte alphabet sweep.

use proto::{Reject, scan_safe, scan_unsafe};

const US: u8 = 0x1F;

#[test]
fn scan_safe_accepts_exactly_the_safe_alphabet_over_all_256() {
    for b in 0u8..=255 {
        if b == US {
            continue; // the separator — not a content byte (see segment_rejects.rs)
        }
        // `[b'a', b]`: the leading `'a'` satisfies the ≥1-alphanumeric rule and is never `.`/`..`, so
        // the only variable is whether byte `b` (at offset 1) is in the alphabet.
        let seg = [b'a', b];
        // OLD-logic oracle: the safe alphabet is `[A-Za-z0-9._-]`.
        let accepted = b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-');
        let got = scan_safe(&seg);
        if accepted {
            assert!(got.is_ok(), "byte {b:#04x} must be ACCEPTED by the safe alphabet");
        } else {
            assert_eq!(
                got,
                Err(Reject::BadSegmentByte {
                    index: 0,
                    offset: 1,
                    byte: b,
                }),
                "byte {b:#04x} must be REJECTED (BadSegmentByte) by the safe alphabet",
            );
        }
    }
}

#[test]
fn scan_unsafe_accepts_exactly_the_unsafe_rule_over_all_256() {
    for b in 0u8..=255 {
        if b == US {
            continue; // the separator — not a content byte
        }
        let seg = [b'a', b];
        // OLD-logic oracle: the unsafe rule rejects only `/` and NUL (`0x1F` is structural).
        let accepted = b != b'/' && b != 0x00;
        let got = scan_unsafe(&seg);
        if accepted {
            assert!(got.is_ok(), "byte {b:#04x} must be ACCEPTED by the unsafe rule");
        } else {
            assert_eq!(
                got,
                Err(Reject::BadSegmentByte {
                    index: 0,
                    offset: 1,
                    byte: b,
                }),
                "byte {b:#04x} must be REJECTED (BadSegmentByte) by the unsafe rule",
            );
        }
    }
}
