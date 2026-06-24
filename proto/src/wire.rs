//! The `Wire` codec trait — one implementation per message type (per verb).
//!
//! Each seed message is a struct in its service module implementing `Wire`: it knows its verb id and
//! how its **body** (everything after the `[verb u32]`) encodes and decodes. The verb prefix itself
//! is read and written once, by [`crate::message`] — `Wire` is body-only, so the two-level
//! organization stays Rust code structure, not a second wire layer (law 1 intact).
//!
//! Decode is owned, field-by-field (`split_first_chunk` + `from_le_bytes`), validating every non-POD
//! field — no whole-struct overlay (Decided Fork #2). It is total: any body yields `Ok` or a typed
//! [`Reject`], never a panic or over-read.

use alloc::vec::Vec;

use crate::reject::Reject;

/// The encode/decode contract for one message body.
pub trait Wire: Sized {
    /// The verb id (low byte; upper three bytes are reserved and written zero).
    const VERB: u32;

    /// Append this message's body bytes to `out`. Encoding is canonical: spare/reserved bits are
    /// written zero (the dropped bits never round-tripped into the value).
    fn encode(&self, out: &mut Vec<u8>);

    /// Decode a message body (the bytes after the verb, bounded by the frame). Returns a typed
    /// [`Reject`] on any structural fault; never panics or reads past `body`.
    fn decode(body: &[u8]) -> Result<Self, Reject>;
}

/// Decode a **repeated-fixed-record tail**: a run of exact `N`-byte records to the frame end, each
/// passed to `parse`. The boundary is the slice end — never a box-authored count (law 11).
///
/// - An **empty** tail is `Ok(vec![])` — a structurally valid run of zero records (e.g. a no-op
///   display repaint); contrast the segment stack's `EmptyStack`, where an empty *address* is
///   meaningless.
/// - A **partial trailing record** (`tail.len() % N != 0`) is [`Reject::PartialRecord`].
/// - Allocation is frame-bounded: the `Vec` is reserved to exactly the record count
///   (`tail.len() / N`), never to an input-supplied number.
///
/// `parse` is fallible (`-> Result<T, Reject>`) so a future record kind can reject a malformed field;
/// for an infallible record (e.g. a display cell carried raw) it simply returns `Ok`.
//
// Live consumer: `display` (0x11) `Display::decode` routes through this decoder (seam 2B).
pub(crate) fn decode_record_tail<const N: usize, T, F>(
    tail: &[u8],
    mut parse: F,
) -> Result<Vec<T>, Reject>
where
    F: FnMut(&[u8; N]) -> Result<T, Reject>,
{
    // `as_chunks` splits into whole `[u8; N]` records plus a remainder, all bounds-checked — no
    // indexing, no `split_at`, no `unwrap`/`expect` on input.
    let (records, remainder) = tail.as_chunks::<N>();
    if !remainder.is_empty() {
        return Err(Reject::PartialRecord {
            record_len: N,
            remainder: remainder.len(),
        });
    }
    let mut out = Vec::with_capacity(records.len());
    for record in records {
        out.push(parse(record)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::decode_record_tail;
    use crate::reject::Reject;
    use alloc::vec;
    use alloc::vec::Vec;

    /// An infallible 4-byte record parse: little-endian `u32`.
    fn le_u32(record: &[u8; 4]) -> Result<u32, Reject> {
        Ok(u32::from_le_bytes(*record))
    }

    #[test]
    fn empty_tail_is_zero_records_ok() {
        assert_eq!(decode_record_tail::<4, _, _>(&[], le_u32), Ok(Vec::new()));
    }

    #[test]
    fn one_and_n_records() {
        assert_eq!(
            decode_record_tail::<4, _, _>(&[1, 0, 0, 0], le_u32),
            Ok(vec![1u32])
        );
        assert_eq!(
            decode_record_tail::<4, _, _>(&[1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0], le_u32),
            Ok(vec![1u32, 2, 3])
        );
    }

    #[test]
    fn every_nonzero_residue_mod_4_is_partial_record() {
        // lengths 1,2,3 (and 5,6,7) — one whole record's worth offset by each residue
        for len in [1usize, 2, 3, 5, 6, 7] {
            let buf = vec![0u8; len];
            let remainder = len % 4;
            assert_eq!(
                decode_record_tail::<4, _, _>(&buf, le_u32),
                Err(Reject::PartialRecord {
                    record_len: 4,
                    remainder,
                }),
                "len {len} should be a partial record with remainder {remainder}"
            );
        }
    }

    #[test]
    fn parse_error_propagates() {
        // a fallible parse that rejects the second record proves the closure's Err is propagated
        let mut seen = 0u32;
        let parse = |_: &[u8; 4]| -> Result<u32, Reject> {
            seen += 1;
            if seen == 2 {
                Err(Reject::TrailingBytes { extra: 0 })
            } else {
                Ok(seen)
            }
        };
        assert_eq!(
            decode_record_tail::<4, _, _>(&[0; 8], parse),
            Err(Reject::TrailingBytes { extra: 0 })
        );
    }
}
