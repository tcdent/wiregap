//! `display` (0x11, box→head, [`display.md`]): the cell stream that paints a terminal — a flat run of
//! `u32` cells applied in order onto the head's held grid. There is no keyframe/delta distinction on
//! the wire (display.md §0); the proxy relays it inert (display.md §4).
//!
//! [`display.md`]: ../../docs/proto/display.md

use alloc::vec::Vec;

use crate::reject::Reject;
use crate::terminal::cell::Cell;
use crate::terminal::dimensions::Dimensions;
use crate::verb;
use crate::wire::{Wire, decode_record_tail};

/// `create-pty` (0x10, head→box): run a PTY on a session at the given size. A fixed-only `Dimensions`
/// body (no tail).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CreatePty(pub Dimensions);

impl Wire for CreatePty {
    const VERB: u32 = verb::CREATE_PTY;

    fn encode(&self, out: &mut Vec<u8>) {
        self.0.encode_body(out);
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        Dimensions::decode_body(body).map(CreatePty)
    }
}

/// A display payload: an ordered run of cell ops.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Display(pub Vec<Cell>);

impl Wire for Display {
    const VERB: u32 = verb::DISPLAY;

    fn encode(&self, out: &mut Vec<u8>) {
        for cell in &self.0 {
            out.extend_from_slice(&cell.to_u32().to_le_bytes());
        }
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        // The live decode routes through the 2A repeated-fixed-record tail decoder (N=4) — the boundary
        // is the frame, a partial trailing record is `PartialRecord`, an empty stream is `Ok(empty)`.
        // The cell parse is infallible (every `u32` is a `Cell`, unknown ops carried as `Reserved`).
        let cells = decode_record_tail::<4, Cell, _>(body, |record| {
            Ok(Cell::from_u32(u32::from_le_bytes(*record)))
        })?;
        Ok(Display(cells))
    }
}
