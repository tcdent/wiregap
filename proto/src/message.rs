//! Message geometry and dispatch — the one flat layer (law 1): a message is `[verb u32][body]`, read
//! by reading the verb and dispatching to its decoder.
//!
//! Dispatch keys on the **low byte** of the verb (`verb & 0xFF`); the upper three reserved bytes are
//! dropped regardless of value (Decided Fork #8(ii)). A *known* id with nonzero reserved bytes still
//! dispatches — rejecting it would wrongly deny a structurally valid message (a G2 totality
//! violation). Only an unknown low byte is [`Reject::UnsupportedVerb`] (law 5). Re-encode writes the
//! id in the low byte with the upper three zero.

use alloc::vec::Vec;

use crate::file::{FileChunk, FileError, FileRead};
use crate::net::NetAddress;
use crate::reject::Reject;
use crate::terminal::pane_state::PaneState;
use crate::verb;
use crate::wire::Wire;

/// A decoded message: one variant per **seed** verb. The other 17 verbs are declared in [`verb`] but
/// have no variant or decode arm in sprint 001 — they resolve to [`Reject::UnsupportedVerb`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Message {
    PaneState(PaneState),
    FileChunk(FileChunk),
    FileRead(FileRead),
    NetAddress(NetAddress),
    FileError(FileError),
}

impl Message {
    /// The verb id this message encodes under (low byte; upper three bytes zero).
    #[must_use]
    pub fn verb(&self) -> u32 {
        match self {
            Message::PaneState(_) => PaneState::VERB,
            Message::FileChunk(_) => FileChunk::VERB,
            Message::FileRead(_) => FileRead::VERB,
            Message::NetAddress(_) => NetAddress::VERB,
            Message::FileError(_) => FileError::VERB,
        }
    }
}

/// Decode one frame `[verb u32][body]`. Total: any byte slice yields `Ok(Message)` or `Err(Reject)`,
/// never a panic or a read past `frame`.
pub fn decode_message(frame: &[u8]) -> Result<Message, Reject> {
    let (verb, body) = frame.split_first_chunk::<4>().ok_or(Reject::ShortBody {
        needed: 4,
        got: frame.len(),
    })?;
    // Drop the reserved upper three bytes; dispatch on the low-byte id.
    let id = u32::from_le_bytes(*verb) & 0xFF;
    match id {
        verb::PANE_STATE => PaneState::decode(body).map(Message::PaneState),
        verb::FILE_CHUNK => FileChunk::decode(body).map(Message::FileChunk),
        verb::FILE_READ => FileRead::decode(body).map(Message::FileRead),
        verb::NET_ADDRESS => NetAddress::decode(body).map(Message::NetAddress),
        verb::FILE_ERROR => FileError::decode(body).map(Message::FileError),
        _ => Err(Reject::UnsupportedVerb { verb: id }),
    }
}

/// Encode one message to a fresh frame `[verb u32][body]`. The verb is written id-in-low-byte with
/// the reserved upper three bytes zero (canonical).
#[must_use]
pub fn encode_message(message: &Message) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&message.verb().to_le_bytes());
    match message {
        Message::PaneState(m) => m.encode(&mut out),
        Message::FileChunk(m) => m.encode(&mut out),
        Message::FileRead(m) => m.encode(&mut out),
        Message::NetAddress(m) => m.encode(&mut out),
        Message::FileError(m) => m.encode(&mut out),
    }
    out
}
