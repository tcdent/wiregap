//! The segment stack — one addressing format shared by file and net ([`file.md`] §0, [`model.md`]
//! §6), and its safe/unsafe **type split**.
//!
//! A stack is `0x1F`-delimited segments, **scanned**: every byte is verified against the segment's
//! alphabet or the input is rejected, and `0x1F` (excluded from every alphabet) separates segments
//! while the slice end ends the stack. The scan **verifies, it never transforms** — there is no
//! normalization, no case-folding, no unescaping. Traversal (`/`, `.`, `..`) is *unrepresentable*,
//! not caught: a receiver joins the segments under its own root and `/`/`..` simply cannot occur.
//!
//! The split is a **type**, not a runtime flag: [`SafeSegment`] (restricted alphabet, loggable) and
//! [`UnsafeSegment`] (real names, display-only) are distinct, with private interiors and no public
//! constructor — the only way to obtain one is through [`scan_safe`] / [`scan_unsafe`]. Only
//! [`SafeSegment`] exposes [`SafeSegment::as_str`], the safe-to-log capability, so a real filename
//! or domain physically cannot reach a log line (gate G4).
//!
//! [`file.md`]: ../../docs/proto/file.md
//! [`model.md`]: ../../docs/proto/model.md

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::reject::Reject;

/// ASCII unit separator — the one structural segment delimiter ([`model.md`] §6). Excluded from
/// both alphabets, so it can only ever delimit, never appear inside a verified segment.
///
/// [`model.md`]: ../../docs/proto/model.md
const US: u8 = 0x1F;

/// A segment over the **safe** alphabet `[A-Za-z0-9._-]` with ≥1 alphanumeric.
///
/// Loggable by construction: these bytes cannot form a terminal escape even in a log piped straight
/// to a terminal. For names the box *chooses* (a quarantine destination, a net address). The safe
/// alphabet is pure ASCII, so the interior is a `str`. Constructed only by [`scan_safe`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SafeSegment(Box<str>);

/// A segment over the **unsafe** structural rule (any byte but `/`, `0x1F`, NUL; never `.`/`..`).
///
/// A *real* name — a source filename, a listing entry — holding spaces and arbitrary (possibly
/// non-UTF-8) bytes. Traversal-safe and safe to *display* (the receiver interprets no control in
/// it), but **not** verified safe to log: it carries no `as_str` / log capability by construction.
/// Constructed only by [`scan_unsafe`].
#[derive(Clone, PartialEq, Eq)]
pub struct UnsafeSegment(Box<[u8]>);

impl SafeSegment {
    /// The **safe-to-log capability** — the loggable view (gate G4). Only [`SafeSegment`] carries
    /// it; [`UnsafeSegment`] deliberately does not, so a consumer's logger that requires this method
    /// physically cannot be handed a real name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The segment's bytes (a subset of ASCII).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl UnsafeSegment {
    /// The segment's raw bytes, for **display only**. May not be valid UTF-8. There is deliberately
    /// no `as_str` and no log capability — that is the type split.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

// Redacting `Debug`: a real (unsafe) name must not reach a log line even via `{:?}` (file.md §0
// makes unsafe names display-only, never logged). Length only, never the bytes. Deliberate — and
// flagged for review as a faithfulness choice beyond G4's type-level requirement.
impl fmt::Debug for UnsafeSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UnsafeSegment(<{} bytes>)", self.0.len())
    }
}

/// A verified stack of [`SafeSegment`]s (≥1). Constructed only by [`scan_safe`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SafeSegmentStack(Vec<SafeSegment>);

/// A verified stack of [`UnsafeSegment`]s (≥1). Constructed only by [`scan_unsafe`].
#[derive(Clone, PartialEq, Eq)]
pub struct UnsafeSegmentStack(Vec<UnsafeSegment>);

impl SafeSegmentStack {
    /// The segments, in wire order (first is the leading component).
    #[must_use]
    pub fn segments(&self) -> &[SafeSegment] {
        &self.0
    }

    /// The number of segments (always ≥1 for a value produced by [`scan_safe`]).
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Always `false` for a scanned stack; present for API hygiene.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Append the stack's wire form to `out`: segment bytes joined by `0x1F`, **no trailing
    /// delimiter** (the separator/canonical form — model.md §6). The inverse of [`scan_safe`]:
    /// `scan_safe(encoded) == self`.
    pub fn encode(&self, out: &mut Vec<u8>) {
        for (i, seg) in self.0.iter().enumerate() {
            if i > 0 {
                out.push(US);
            }
            out.extend_from_slice(seg.as_bytes());
        }
    }
}

impl UnsafeSegmentStack {
    /// The segments, in wire order.
    #[must_use]
    pub fn segments(&self) -> &[UnsafeSegment] {
        &self.0
    }

    /// The number of segments (always ≥1 for a value produced by [`scan_unsafe`]).
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Always `false` for a scanned stack; present for API hygiene.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Append the stack's wire form to `out`: segment bytes joined by `0x1F`, **no trailing
    /// delimiter** (model.md §6). The inverse of [`scan_unsafe`]: `scan_unsafe(encoded) == self`.
    pub fn encode(&self, out: &mut Vec<u8>) {
        for (i, seg) in self.0.iter().enumerate() {
            if i > 0 {
                out.push(US);
            }
            out.extend_from_slice(seg.as_bytes());
        }
    }
}

// Redacting `Debug` for the same reason as `UnsafeSegment` — segment count only, never the names.
impl fmt::Debug for UnsafeSegmentStack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UnsafeSegmentStack(<{} segments>)", self.0.len())
    }
}

/// Scan a frame-bounded slice into a [`SafeSegmentStack`], verifying every byte against the safe
/// alphabet `[A-Za-z0-9._-]` (≥1 alphanumeric per segment). **Verifies, never transforms.**
///
/// `0x1F` separates segments; the slice end ends the stack. Rejects: an empty stack
/// ([`Reject::EmptyStack`]), an empty segment ([`Reject::EmptySegment`] — a leading/trailing/doubled
/// delimiter), a `.`/`..` segment ([`Reject::TraversalSegment`]), a byte outside the alphabet
/// ([`Reject::BadSegmentByte`] — including `/`, `0x1F` in content is impossible, NUL), or a segment
/// with no alphanumeric ([`Reject::SafeSegmentNotAlnum`]).
pub fn scan_safe(input: &[u8]) -> Result<SafeSegmentStack, Reject> {
    if input.is_empty() {
        return Err(Reject::EmptyStack);
    }
    let mut stack = Vec::new();
    for (index, part) in input.split(|&b| b == US).enumerate() {
        if part.is_empty() {
            return Err(Reject::EmptySegment { index });
        }
        if part == b"." || part == b".." {
            return Err(Reject::TraversalSegment { index });
        }
        let mut text = String::with_capacity(part.len());
        let mut has_alnum = false;
        for (offset, &byte) in part.iter().enumerate() {
            // The safe alphabet `[A-Za-z0-9._-]`, explicit in the arms (file.md §0 / model §6). The
            // alphanumeric range sets `has_alnum`; `.`/`_`/`-` are accepted but do not; every other
            // byte is out of alphabet. (`0x1F` is the separator and never reaches here.)
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => has_alnum = true,
                b'.' | b'_' | b'-' => {}
                _ => {
                    return Err(Reject::BadSegmentByte {
                        index,
                        offset,
                        byte,
                    });
                }
            }
            // Every accepted byte is in `[A-Za-z0-9._-]`, hence ASCII (< 0x80), so `as char` is an
            // exact, lossless widening — no `unsafe`, no `from_utf8` fallibility.
            text.push(byte as char);
        }
        if !has_alnum {
            return Err(Reject::SafeSegmentNotAlnum { index });
        }
        stack.push(SafeSegment(text.into_boxed_str()));
    }
    Ok(SafeSegmentStack(stack))
}

/// Scan a frame-bounded slice into an [`UnsafeSegmentStack`], verifying the **unsafe** structural
/// rule: any byte but `/`, `0x1F`, and NUL; a segment is never `.` or `..`. **Verifies, never
/// transforms.**
///
/// `0x1F` separates segments (and so can never appear inside one); the slice end ends the stack.
/// Rejects an empty stack, an empty segment, a `.`/`..` segment, or a `/`/NUL byte
/// ([`Reject::BadSegmentByte`]).
pub fn scan_unsafe(input: &[u8]) -> Result<UnsafeSegmentStack, Reject> {
    if input.is_empty() {
        return Err(Reject::EmptyStack);
    }
    let mut stack = Vec::new();
    for (index, part) in input.split(|&b| b == US).enumerate() {
        if part.is_empty() {
            return Err(Reject::EmptySegment { index });
        }
        if part == b"." || part == b".." {
            return Err(Reject::TraversalSegment { index });
        }
        for (offset, &byte) in part.iter().enumerate() {
            // The unsafe rule (file.md §0): any byte but `/` and NUL. `0x1F` cannot appear here — it
            // is the delimiter that bounded `part`.
            match byte {
                b'/' | 0x00 => {
                    return Err(Reject::BadSegmentByte {
                        index,
                        offset,
                        byte,
                    });
                }
                _ => {}
            }
        }
        stack.push(UnsafeSegment(Box::from(part)));
    }
    Ok(UnsafeSegmentStack(stack))
}
