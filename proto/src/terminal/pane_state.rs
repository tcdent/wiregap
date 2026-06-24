//! `pane-state` (`box → head`, [`model.md`] §8 Input events): scrollbar extent + input-interest.
//! Body `[scrollback_total u32][viewport_offset u32][input_flags u32]` — the one genuinely
//! all-integer body, decoded with the fixed-record primitive (`split_first_chunk` + `from_le_bytes`)
//! and nothing else.
//!
//! `input_flags` bit 0 is `wants-free-motion`; **bits 1–31 are reserved and dropped** (Decided Fork
//! #8(ii)) — the box→head render-relay surface tolerates any reserved value (strict would hand the
//! hostile box a denial-of-rendering lever). Re-encode is canonical: bit 0 only, the rest zero.
//!
//! [`model.md`]: ../../docs/proto/model.md

use alloc::vec::Vec;

use crate::reject::Reject;
use crate::verb;
use crate::wire::Wire;

/// `input_flags` bit 0 — the only defined bit (DECSET 1003 hover interest).
const WANTS_FREE_MOTION: u32 = 1;

/// The body is exactly three `u32`s and no tail.
const BODY_LEN: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PaneState {
    /// Scrollbar extent — total scrollback lines.
    pub scrollback_total: u32,
    /// Current clamped viewport offset (`0` = live).
    pub viewport_offset: u32,
    /// Bit 0 of `input_flags`. Reserved bits 1–31 are dropped on decode and emitted zero on encode.
    pub wants_free_motion: bool,
}

impl Wire for PaneState {
    const VERB: u32 = verb::PANE_STATE;

    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.scrollback_total.to_le_bytes());
        out.extend_from_slice(&self.viewport_offset.to_le_bytes());
        // Canonical: only the defined bit; reserved bits zero.
        let input_flags = u32::from(self.wants_free_motion) * WANTS_FREE_MOTION;
        out.extend_from_slice(&input_flags.to_le_bytes());
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        let short = || Reject::ShortBody {
            needed: BODY_LEN,
            got: body.len(),
        };
        let (scrollback_total, rest) = body.split_first_chunk::<4>().ok_or_else(short)?;
        let (viewport_offset, rest) = rest.split_first_chunk::<4>().ok_or_else(short)?;
        let (input_flags, rest) = rest.split_first_chunk::<4>().ok_or_else(short)?;
        if !rest.is_empty() {
            return Err(Reject::TrailingBytes { extra: rest.len() });
        }
        let input_flags = u32::from_le_bytes(*input_flags);
        Ok(PaneState {
            scrollback_total: u32::from_le_bytes(*scrollback_total),
            viewport_offset: u32::from_le_bytes(*viewport_offset),
            // Read the defined bit; drop reserved bits 1–31 (any value decodes Ok).
            wants_free_motion: (input_flags & WANTS_FREE_MOTION) != 0,
        })
    }
}
