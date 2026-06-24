//! Message geometry and dispatch — the one flat layer (law 1): a message is `[verb u32][body]`, read
//! by reading the verb and dispatching to its decoder.
//!
//! Dispatch keys on the **low byte** of the verb (`verb & 0xFF`); the upper three reserved bytes are
//! dropped regardless of value (Decided Fork #8(ii)). A *known* id with nonzero reserved bytes still
//! dispatches — rejecting it would wrongly deny a structurally valid message (a G2 totality
//! violation). Only an unknown low byte is [`Reject::UnsupportedVerb`] (law 5). Re-encode writes the
//! id in the low byte with the upper three zero.

use alloc::vec::Vec;

use crate::file::{FileChunk, FileData, FileError, FileRead, FileWrite, List, RequestFile};
use crate::list_entry::ListEntries;
use crate::net::{NetAddress, NetError, NetPayload, NetResponse};
use crate::reject::Reject;
use crate::terminal::display::{CreatePty, Display};
use crate::terminal::input::{Focus, Mouse, Paste, Resize, Viewport};
use crate::terminal::pane_state::PaneState;
use crate::verb;
use crate::wire::Wire;

/// A decoded message: one variant per **implemented** verb (see [`verb::IMPLEMENTED`]). The remaining
/// catalog verbs are declared in [`verb`] but have no decode arm yet — they resolve to
/// [`Reject::UnsupportedVerb`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Message {
    PaneState(PaneState),
    FileChunk(FileChunk),
    FileRead(FileRead),
    NetAddress(NetAddress),
    FileError(FileError),
    CreatePty(CreatePty),
    Display(Display),
    FileWrite(FileWrite),
    FileData(FileData),
    List(List),
    ListEntries(ListEntries),
    RequestFile(RequestFile),
    Resize(Resize),
    Viewport(Viewport),
    Mouse(Mouse),
    Paste(Paste),
    Focus(Focus),
    NetResponse(NetResponse),
    NetError(NetError),
    NetPayload(NetPayload),
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
            Message::CreatePty(_) => CreatePty::VERB,
            Message::Display(_) => Display::VERB,
            Message::FileWrite(_) => FileWrite::VERB,
            Message::FileData(_) => FileData::VERB,
            Message::List(_) => List::VERB,
            Message::ListEntries(_) => ListEntries::VERB,
            Message::RequestFile(_) => RequestFile::VERB,
            Message::Resize(_) => Resize::VERB,
            Message::Viewport(_) => Viewport::VERB,
            Message::Mouse(_) => Mouse::VERB,
            Message::Paste(_) => Paste::VERB,
            Message::Focus(_) => Focus::VERB,
            Message::NetResponse(_) => NetResponse::VERB,
            Message::NetError(_) => NetError::VERB,
            Message::NetPayload(_) => NetPayload::VERB,
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
        verb::CREATE_PTY => CreatePty::decode(body).map(Message::CreatePty),
        // The production head path: 0x11 routes straight through `Display::decode` (which rides the
        // 2A `decode_record_tail`) — no inline cell loop here.
        verb::DISPLAY => Display::decode(body).map(Message::Display),
        verb::FILE_WRITE => FileWrite::decode(body).map(Message::FileWrite),
        verb::FILE_DATA => FileData::decode(body).map(Message::FileData),
        verb::LIST => List::decode(body).map(Message::List),
        // 0x25 routes through `ListEntries::decode` → the 3A `decode_list_entries` (terminator-bounded);
        // an `UnterminatedRecord` from here is the observable proof of the route-through.
        verb::LIST_ENTRY => ListEntries::decode(body).map(Message::ListEntries),
        verb::REQUEST_FILE => RequestFile::decode(body).map(Message::RequestFile),
        verb::RESIZE => Resize::decode(body).map(Message::Resize),
        verb::VIEWPORT => Viewport::decode(body).map(Message::Viewport),
        // 0x15 routes through `Mouse::decode` — the observable mouse `TrailingBytes` from here proves
        // the route-through.
        verb::MOUSE => Mouse::decode(body).map(Message::Mouse),
        verb::PASTE => Paste::decode(body).map(Message::Paste),
        verb::FOCUS => Focus::decode(body).map(Message::Focus),
        verb::NET_RESPONSE => NetResponse::decode(body).map(Message::NetResponse),
        // 0x32 routes through `ErrorBody::decode_body` (reuse, not re-impl) — an `ErrorBody` reject
        // surfacing here proves the route-through.
        verb::NET_ERROR => NetError::decode(body).map(Message::NetError),
        // 0x33 decodes the COMPLETE reassembled buffer once (verb 0x33 in frame 1; the reassembler
        // joined the raw continuation payloads opaquely upstream) — a verbatim tail over the payload.
        verb::NET_PAYLOAD => NetPayload::decode(body).map(Message::NetPayload),
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
        Message::CreatePty(m) => m.encode(&mut out),
        Message::Display(m) => m.encode(&mut out),
        Message::FileWrite(m) => m.encode(&mut out),
        Message::FileData(m) => m.encode(&mut out),
        Message::List(m) => m.encode(&mut out),
        Message::ListEntries(m) => m.encode(&mut out),
        Message::RequestFile(m) => m.encode(&mut out),
        Message::Resize(m) => m.encode(&mut out),
        Message::Viewport(m) => m.encode(&mut out),
        Message::Mouse(m) => m.encode(&mut out),
        Message::Paste(m) => m.encode(&mut out),
        Message::Focus(m) => m.encode(&mut out),
        Message::NetResponse(m) => m.encode(&mut out),
        Message::NetError(m) => m.encode(&mut out),
        Message::NetPayload(m) => m.encode(&mut out),
    }
    out
}
