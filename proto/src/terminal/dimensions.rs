//! The shared `Dimensions` grammar — a fixed `[rows u16][cols u16]` body ([`model.md`] §8). Reused by
//! `create-pty` (0x10, this sprint) and `resize` (0x13, the input sprint) — the §8 grammar-reuse
//! decision. A **free verb body**, so it is byte-aligned u16|u16 (not the cell-op `RowCol` 12|12
//! packing, which is payload-constrained — Decided Fork #3). `rows`/`cols` are carried **raw**; the
//! box clamps an oversize request (like the display `goto` out-of-grid clamp) — no value reject.
//!
//! [`model.md`]: ../../docs/proto/model.md

use alloc::vec::Vec;

use crate::reject::Reject;

/// A terminal size: rows × cols, each a raw `u16`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Dimensions {
    pub rows: u16,
    pub cols: u16,
}

impl Dimensions {
    /// The fixed body width: two `u16`s, no tail.
    const BODY_LEN: usize = 4;

    /// Decode `[rows u16][cols u16]` — exactly 4 bytes. Fixed-only geometry: `< 4` → `ShortBody`,
    /// `> 4` → `TrailingBytes`.
    pub fn decode_body(body: &[u8]) -> Result<Self, Reject> {
        let short = || Reject::ShortBody {
            needed: Self::BODY_LEN,
            got: body.len(),
        };
        let (rows, rest) = body.split_first_chunk::<2>().ok_or_else(short)?;
        let (cols, rest) = rest.split_first_chunk::<2>().ok_or_else(short)?;
        if !rest.is_empty() {
            return Err(Reject::TrailingBytes { extra: rest.len() });
        }
        Ok(Dimensions {
            rows: u16::from_le_bytes(*rows),
            cols: u16::from_le_bytes(*cols),
        })
    }

    /// Append `[rows u16][cols u16]` to `out`.
    pub fn encode_body(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.rows.to_le_bytes());
        out.extend_from_slice(&self.cols.to_le_bytes());
    }
}
