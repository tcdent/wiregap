//! The typed rejection.
//!
//! Parsing any input yields `Ok(_)` or `Err(Reject)` — never a panic, over-read, or
//! input-controlled allocation (totality, the security property). `Reject` is a small value-type:
//! `Debug` + `Display` only, **no** `Error` impl (smaller surface; additive later — Decided Fork
//! #7b). Variants carry a bad byte/offset for diagnostics but **never** an owned copy of the input
//! payload.

use core::fmt;

/// Why a decode was rejected. **Every variant is structural** — a verb's low byte is unknown, a
/// body is too short or too long, or a segment byte is out of alphabet. There is deliberately **no
/// value-policy variant**: reserved bits are dropped (not rejected) and enum reserved value-ranges
/// are carried raw (not rejected) — Decided Fork #8. That absence is the coherence tell that the
/// library never rejects on a *value* the consumer owns.
///
/// Seam 1A defined the segment-scan rejections; seam 1B adds the message-geometry rejections; seam
/// 2A adds the record-tail geometry reject. New variants are **appended last** so existing
/// discriminants are preserved (the scan-surface codegen that constructs them stays byte-identical).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reject {
    /// The verb's low byte is not a supported id. Keys on `verb & 0xFF` only — the reserved upper
    /// three bytes are dropped, so a *known* verb with nonzero reserved bytes still dispatches.
    UnsupportedVerb { verb: u32 },
    /// The body (or the leading verb) was shorter than the fixed structure required.
    ShortBody { needed: usize, got: usize },
    /// A fixed-shape body (no frame-bounded tail) had `extra` bytes past its fixed structure.
    TrailingBytes { extra: usize },
    /// The segment stack was empty — a stack must hold ≥1 segment.
    EmptyStack,
    /// The segment at `index` had zero bytes (a leading, trailing, or doubled `0x1F`).
    EmptySegment { index: usize },
    /// The safe segment at `index` had no alphanumeric byte ([`file.md`] §0 requires ≥1).
    ///
    /// [`file.md`]: ../../docs/proto/file.md
    SafeSegmentNotAlnum { index: usize },
    /// The segment at `index` was `.` or `..` — a traversal component, structurally unsayable.
    TraversalSegment { index: usize },
    /// `byte` at `offset` within the segment at `index` is outside the segment's alphabet.
    BadSegmentByte {
        index: usize,
        offset: usize,
        byte: u8,
    },
    /// A record-tail body had `remainder` bytes past the last whole `record_len`-byte record — a
    /// partial trailing record (`frame_len % record_len != 0`). Structural geometry reject, distinct
    /// from `TrailingBytes` (which is bytes past a *fixed single* body). Appended last (seam 2A).
    PartialRecord { record_len: usize, remainder: usize },
    /// A combined-record (`list-entry`) name had no `0x1F` terminator before the frame end — the
    /// record-internal `0x1F` is a **terminator** (the frame does not end an unterminated name), unlike
    /// the segment stack's separator. `offset` is the name's start in the body. Appended last (seam 3A).
    UnterminatedRecord { offset: usize },
    /// A multi-frame reassembly would exceed the capacity `cap` (the fail-closed total-request bound).
    /// **Structural / resource** — it rejects on the accumulated byte-count alone, never on any value
    /// the consumer owns, so the no-value-policy coherence tell holds. This is the proxy's TCB defense:
    /// a payload that exceeds the cap, or never ends (the cap trips), fails closed. Appended last (5A).
    ReassemblyOverflow { cap: usize },
}

impl fmt::Display for Reject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Reject::UnsupportedVerb { verb } => write!(f, "unsupported verb id 0x{verb:02x}"),
            Reject::ShortBody { needed, got } => {
                write!(f, "body too short: needed {needed} bytes, got {got}")
            }
            Reject::TrailingBytes { extra } => {
                write!(f, "{extra} trailing bytes past the fixed body")
            }
            Reject::EmptyStack => f.write_str("segment stack is empty"),
            Reject::EmptySegment { index } => write!(f, "empty segment at index {index}"),
            Reject::SafeSegmentNotAlnum { index } => {
                write!(f, "safe segment at index {index} has no alphanumeric byte")
            }
            Reject::TraversalSegment { index } => {
                write!(f, "traversal segment ('.' or '..') at index {index}")
            }
            Reject::BadSegmentByte {
                index,
                offset,
                byte,
            } => write!(
                f,
                "byte 0x{byte:02x} not allowed in segment {index} at offset {offset}"
            ),
            Reject::PartialRecord {
                record_len,
                remainder,
            } => write!(
                f,
                "partial trailing record: {remainder} bytes past the last {record_len}-byte record"
            ),
            Reject::UnterminatedRecord { offset } => {
                write!(f, "unterminated record name at offset {offset} (no 0x1F)")
            }
            Reject::ReassemblyOverflow { cap } => {
                write!(f, "reassembly exceeds the {cap}-byte capacity bound")
            }
        }
    }
}
