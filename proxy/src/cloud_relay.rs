//! Sprint 6 S3 — the cloud response relay (tier 3, `approach.md` §5; `architecture.md` tier-3
//! "Cloud response relay"). Upstream response bytes → the box, **verbatim**, frame-by-frame,
//! **never reassembled** by the proxy (`net.md` §3), **never inspected**. A pass-through: a bug
//! fails the *feature* (garbled bytes reach the box — harmless, untrusted ground), never
//! containment. There is **no content decision anywhere** (MB3) — the *same* wrap is produced for
//! any bytes, so nothing branches on what the content *is*. Hardening this direction would
//! re-import the confidentiality worldview the spine discards (`approach.md` §5) — refused.

use alloc::vec::Vec;

use proto::{ErrorBody, Message, NetError, NetResponse, encode_message};

/// Relay one chunk of upstream response bytes to the box as a `net-response` L2 message —
/// **verbatim**. The chunk's bytes are the message body, byte-identical; nothing inspects,
/// neutralizes, or modifies them. (The L1 session framing is added below, at the transport sink.)
#[must_use]
pub fn relay_response(chunk: &[u8]) -> Vec<u8> {
    encode_message(&Message::NetResponse(NetResponse(chunk.to_vec())))
}

/// Emit a `net-error` to the box — a proxy-authored non-response outcome (timeout, unreachable, …).
/// The error body is the proxy's own signal (a category code + optional detail), not box content.
#[must_use]
pub fn relay_error(body: ErrorBody) -> Vec<u8> {
    encode_message(&Message::NetError(NetError(body)))
}
