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
