//! Message geometry and dispatch — the one flat layer (law 1): a message is `[verb u32][body]`, read
//! by reading the verb and dispatching to its decoder.
//!
//! Dispatch keys on the **low byte** of the verb (`verb & 0xFF`); the upper three reserved bytes are
//! dropped regardless of value (Decided Fork #8(ii)). A *known* id with nonzero reserved bytes still
//! dispatches — rejecting it would wrongly deny a structurally valid message (a G2 totality
//! violation). Only an unknown low byte is [`Reject::UnsupportedVerb`] (law 5). Re-encode writes the
//! id in the low byte with the upper three zero.
//!
//! Feature-gating (006 6B): the `Message` shell, the dispatch prologue, and the `_ → UnsupportedVerb`
//! catch-all are **core**; each per-service variant + its `verb()`/`decode`/`encode` arm is gated
//! (variant and arm share a feature). A gated-out verb has no arm → it falls to the catch-all →
//! `UnsupportedVerb`. A build with no service features has an uninhabited `Message` (the matches reduce
//! to the empty match / the catch-all).

use alloc::vec::Vec;

use crate::reject::Reject;
// `verb` (the const namespace) and the `Wire` trait are used only by the per-service arms — gated to
// "any service" so a core-only build does not carry an unused import.
#[cfg(any(
    feature = "control",
    feature = "terminal",
    feature = "file",
    feature = "net"
))]
use crate::verb;
#[cfg(any(
    feature = "control",
    feature = "terminal",
    feature = "file",
    feature = "net"
))]
use crate::wire::Wire;

#[cfg(feature = "control")]
use crate::control::OpenSession;
#[cfg(feature = "file")]
use crate::file::{FileChunk, FileData, FileError, FileRead, FileWrite, List, RequestFile};
#[cfg(feature = "file")]
use crate::list_entry::ListEntries;
#[cfg(feature = "net")]
use crate::net::{NetAddress, NetError, NetPayload, NetResponse};
#[cfg(feature = "terminal")]
use crate::terminal::display::{CreatePty, Display};
#[cfg(feature = "terminal")]
use crate::terminal::input::{Focus, Key, Mouse, Paste, Resize, Viewport};
#[cfg(feature = "terminal")]
use crate::terminal::pane_state::PaneState;

/// A decoded message: one variant per **implemented** verb (see [`verb::IMPLEMENTED`]). Variants are
/// per-service feature-gated; the remaining catalog verbs resolve to [`Reject::UnsupportedVerb`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Message {
    // control (0x0_)
    #[cfg(feature = "control")]
    OpenSession(OpenSession),
    // terminal (0x1_)
    #[cfg(feature = "terminal")]
    Key(Key),
    #[cfg(feature = "terminal")]
    PaneState(PaneState),
    #[cfg(feature = "terminal")]
    CreatePty(CreatePty),
    #[cfg(feature = "terminal")]
    Display(Display),
    #[cfg(feature = "terminal")]
    Resize(Resize),
    #[cfg(feature = "terminal")]
    Viewport(Viewport),
    #[cfg(feature = "terminal")]
    Mouse(Mouse),
    #[cfg(feature = "terminal")]
    Paste(Paste),
    #[cfg(feature = "terminal")]
    Focus(Focus),
    // file (0x2_)
    #[cfg(feature = "file")]
    FileWrite(FileWrite),
    #[cfg(feature = "file")]
    FileChunk(FileChunk),
    #[cfg(feature = "file")]
    FileRead(FileRead),
    #[cfg(feature = "file")]
    FileData(FileData),
    #[cfg(feature = "file")]
    List(List),
    #[cfg(feature = "file")]
    ListEntries(ListEntries),
    #[cfg(feature = "file")]
    RequestFile(RequestFile),
    #[cfg(feature = "file")]
    FileError(FileError),
    // net (0x3_)
    #[cfg(feature = "net")]
    NetAddress(NetAddress),
    #[cfg(feature = "net")]
    NetResponse(NetResponse),
    #[cfg(feature = "net")]
    NetError(NetError),
    #[cfg(feature = "net")]
    NetPayload(NetPayload),
}

impl Message {
    /// The verb id this message encodes under (low byte; upper three bytes zero).
    #[must_use]
    pub fn verb(&self) -> u32 {
        match self {
            #[cfg(feature = "control")]
            Message::OpenSession(_) => OpenSession::VERB,
            #[cfg(feature = "terminal")]
            Message::Key(_) => Key::VERB,
            #[cfg(feature = "terminal")]
            Message::PaneState(_) => PaneState::VERB,
            #[cfg(feature = "terminal")]
            Message::CreatePty(_) => CreatePty::VERB,
            #[cfg(feature = "terminal")]
            Message::Display(_) => Display::VERB,
            #[cfg(feature = "terminal")]
            Message::Resize(_) => Resize::VERB,
            #[cfg(feature = "terminal")]
            Message::Viewport(_) => Viewport::VERB,
            #[cfg(feature = "terminal")]
            Message::Mouse(_) => Mouse::VERB,
            #[cfg(feature = "terminal")]
            Message::Paste(_) => Paste::VERB,
            #[cfg(feature = "terminal")]
            Message::Focus(_) => Focus::VERB,
            #[cfg(feature = "file")]
            Message::FileWrite(_) => FileWrite::VERB,
            #[cfg(feature = "file")]
            Message::FileChunk(_) => FileChunk::VERB,
            #[cfg(feature = "file")]
            Message::FileRead(_) => FileRead::VERB,
            #[cfg(feature = "file")]
            Message::FileData(_) => FileData::VERB,
            #[cfg(feature = "file")]
            Message::List(_) => List::VERB,
            #[cfg(feature = "file")]
            Message::ListEntries(_) => ListEntries::VERB,
            #[cfg(feature = "file")]
            Message::RequestFile(_) => RequestFile::VERB,
            #[cfg(feature = "file")]
            Message::FileError(_) => FileError::VERB,
            #[cfg(feature = "net")]
            Message::NetAddress(_) => NetAddress::VERB,
            #[cfg(feature = "net")]
            Message::NetResponse(_) => NetResponse::VERB,
            #[cfg(feature = "net")]
            Message::NetError(_) => NetError::VERB,
            #[cfg(feature = "net")]
            Message::NetPayload(_) => NetPayload::VERB,
            // No-service build: `Message` is uninhabited, but rustc treats `&Message` as inhabited, so
            // a wildcard is required. `match *self {}` proves the place is uninhabited (unreachable, no
            // value can exist) — no panic path.
            #[cfg(not(any(
                feature = "control",
                feature = "terminal",
                feature = "file",
                feature = "net"
            )))]
            _ => match *self {},
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
    // Core-only build: no decode arm consumes `body`; discard it so the no-service build is warning-clean.
    #[cfg(not(any(
        feature = "control",
        feature = "terminal",
        feature = "file",
        feature = "net"
    )))]
    let _ = body;
    match id {
        #[cfg(feature = "control")]
        verb::OPEN_SESSION => OpenSession::decode(body).map(Message::OpenSession),
        #[cfg(feature = "terminal")]
        verb::KEY => Key::decode(body).map(Message::Key),
        #[cfg(feature = "terminal")]
        verb::PANE_STATE => PaneState::decode(body).map(Message::PaneState),
        #[cfg(feature = "terminal")]
        verb::CREATE_PTY => CreatePty::decode(body).map(Message::CreatePty),
        // The production head path: 0x11 routes straight through `Display::decode` (which rides the
        // 2A `decode_record_tail`) — no inline cell loop here.
        #[cfg(feature = "terminal")]
        verb::DISPLAY => Display::decode(body).map(Message::Display),
        #[cfg(feature = "terminal")]
        verb::RESIZE => Resize::decode(body).map(Message::Resize),
        #[cfg(feature = "terminal")]
        verb::VIEWPORT => Viewport::decode(body).map(Message::Viewport),
        // 0x15 routes through `Mouse::decode` — the observable mouse `TrailingBytes` from here proves
        // the route-through.
        #[cfg(feature = "terminal")]
        verb::MOUSE => Mouse::decode(body).map(Message::Mouse),
        #[cfg(feature = "terminal")]
        verb::PASTE => Paste::decode(body).map(Message::Paste),
        #[cfg(feature = "terminal")]
        verb::FOCUS => Focus::decode(body).map(Message::Focus),
        #[cfg(feature = "file")]
        verb::FILE_WRITE => FileWrite::decode(body).map(Message::FileWrite),
        #[cfg(feature = "file")]
        verb::FILE_CHUNK => FileChunk::decode(body).map(Message::FileChunk),
        #[cfg(feature = "file")]
        verb::FILE_READ => FileRead::decode(body).map(Message::FileRead),
        #[cfg(feature = "file")]
        verb::FILE_DATA => FileData::decode(body).map(Message::FileData),
        #[cfg(feature = "file")]
        verb::LIST => List::decode(body).map(Message::List),
        // 0x25 routes through `ListEntries::decode` → the 3A `decode_list_entries` (terminator-bounded);
        // an `UnterminatedRecord` from here is the observable proof of the route-through.
        #[cfg(feature = "file")]
        verb::LIST_ENTRY => ListEntries::decode(body).map(Message::ListEntries),
        #[cfg(feature = "file")]
        verb::REQUEST_FILE => RequestFile::decode(body).map(Message::RequestFile),
        #[cfg(feature = "file")]
        verb::FILE_ERROR => FileError::decode(body).map(Message::FileError),
        #[cfg(feature = "net")]
        verb::NET_ADDRESS => NetAddress::decode(body).map(Message::NetAddress),
        #[cfg(feature = "net")]
        verb::NET_RESPONSE => NetResponse::decode(body).map(Message::NetResponse),
        // 0x32 routes through `ErrorBody::decode_body` (reuse, not re-impl) — an `ErrorBody` reject
        // surfacing here proves the route-through.
        #[cfg(feature = "net")]
        verb::NET_ERROR => NetError::decode(body).map(Message::NetError),
        // 0x33 decodes the COMPLETE reassembled buffer once (the reassembler joined the transport's
        // continued payload chunks opaquely upstream) — a verbatim tail over the payload.
        #[cfg(feature = "net")]
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
        #[cfg(feature = "control")]
        Message::OpenSession(m) => m.encode(&mut out),
        #[cfg(feature = "terminal")]
        Message::Key(m) => m.encode(&mut out),
        #[cfg(feature = "terminal")]
        Message::PaneState(m) => m.encode(&mut out),
        #[cfg(feature = "terminal")]
        Message::CreatePty(m) => m.encode(&mut out),
        #[cfg(feature = "terminal")]
        Message::Display(m) => m.encode(&mut out),
        #[cfg(feature = "terminal")]
        Message::Resize(m) => m.encode(&mut out),
        #[cfg(feature = "terminal")]
        Message::Viewport(m) => m.encode(&mut out),
        #[cfg(feature = "terminal")]
        Message::Mouse(m) => m.encode(&mut out),
        #[cfg(feature = "terminal")]
        Message::Paste(m) => m.encode(&mut out),
        #[cfg(feature = "terminal")]
        Message::Focus(m) => m.encode(&mut out),
        #[cfg(feature = "file")]
        Message::FileWrite(m) => m.encode(&mut out),
        #[cfg(feature = "file")]
        Message::FileChunk(m) => m.encode(&mut out),
        #[cfg(feature = "file")]
        Message::FileRead(m) => m.encode(&mut out),
        #[cfg(feature = "file")]
        Message::FileData(m) => m.encode(&mut out),
        #[cfg(feature = "file")]
        Message::List(m) => m.encode(&mut out),
        #[cfg(feature = "file")]
        Message::ListEntries(m) => m.encode(&mut out),
        #[cfg(feature = "file")]
        Message::RequestFile(m) => m.encode(&mut out),
        #[cfg(feature = "file")]
        Message::FileError(m) => m.encode(&mut out),
        #[cfg(feature = "net")]
        Message::NetAddress(m) => m.encode(&mut out),
        #[cfg(feature = "net")]
        Message::NetResponse(m) => m.encode(&mut out),
        #[cfg(feature = "net")]
        Message::NetError(m) => m.encode(&mut out),
        #[cfg(feature = "net")]
        Message::NetPayload(m) => m.encode(&mut out),
        // No-service build: `Message` is uninhabited (unreachable); a non-diverging `()` arm keeps the
        // trailing `out` reachable.
        #[cfg(not(any(
            feature = "control",
            feature = "terminal",
            feature = "file",
            feature = "net"
        )))]
        _ => {}
    }
    out
}

/// G0 per-slice coherence (006 6B) — a **lib-unit** test so it runs on every feature slice
/// (`cargo test --lib --no-default-features --features <slice>`), not just the all-features integration
/// build. References ONLY core symbols (`verb::CATALOG`/`IMPLEMENTED`, `decode_message`,
/// `Reject::UnsupportedVerb`) — never a gated type — so it compiles on any slice. It catches an
/// element-cfg(`IMPLEMENTED`) vs arm-cfg(dispatch) mismatch that a clean build alone would miss.
#[cfg(test)]
mod coherence {
    use super::decode_message;
    use crate::reject::Reject;
    use crate::verb;

    #[test]
    fn implemented_matches_dispatch_per_slice() {
        for &v in verb::CATALOG.iter() {
            let unsupported = matches!(
                decode_message(&v.to_le_bytes()),
                Err(Reject::UnsupportedVerb { .. })
            );
            // implemented ⇔ has a dispatch arm (Ok or a *structural* reject, not UnsupportedVerb).
            assert_eq!(
                verb::IMPLEMENTED.contains(&v),
                !unsupported,
                "verb {v:#x}: IMPLEMENTED membership disagrees with dispatch (cfg mismatch)"
            );
        }
    }
}
