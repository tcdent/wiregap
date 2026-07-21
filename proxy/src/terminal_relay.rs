//! Sprint 6 S4 — the terminal relay (tier 2, `terminal.md` §2–§3; `proxy-head.md` §1;
//! `architecture.md` tier-2 "Proxy terminal relay"). Moves `display`/input frames **box↔head**,
//! decoding **only the verb** (for direction) and relaying the L2 body **opaquely** — the proxy runs
//! no display/input decoder on box bytes (`library.md` §7; the verb-peek dispatch does the routing).
//! A bug fails the *feature* (a misrouted or garbled frame → a misdraw), never containment: the
//! guarantee is the wire's inexpressiveness + the head's no-interpreter invariant (both tier-1
//! design properties), never a proxy check.
//!
//! **MB3 — this relay carries ZERO gating of any kind (the sprint's central discipline at its
//! purest), confirmed by direct operator direction (2026-07-19):**
//! - **No TLS / connection-security logic.** Connection security is *outside the library entirely*
//!   — whatever protects a network hop is a deployment concern, not proxy relay logic. The relay is
//!   transport-agnostic (below) precisely because it neither presents nor enforces transport security.
//! - **No peer-identity / access-control logic.** The relay does not gate, verify, or manage any
//!   peer identity — auth is outside the library, same as TLS. "Should the relay check who it is
//!   talking to?" is exactly the MB3-creep question, and it is settled *no* by operator direction,
//!   not reasoned from the spine.
//!
//! There is no content check, no filter, no neutralization, no peer check — routing is by the
//! verb's inherent direction (`terminal.md` §4: channels split, one-directional, no reflection),
//! never by what the content *is* or who sent it.
//!
//! **Transport-agnostic (the operator's flexible-transport direction).** The relay emits to an
//! [`Endpoint`] seam — a byte sink — never knowing or caring whether it is a local pipe or a network
//! socket. Nothing here hardcodes local-vs-network; the concrete transport is a deployment choice.

use proto::verb;

use crate::dispatch::peek_verb;

/// The relay direction of a terminal frame — fixed by its verb, not by its source (no peer check).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// `display` / `pane-state` — box → head (onto trusted ground; made safe by the wire's
    /// inexpressiveness + the head's no-interpreter invariant, not a proxy check).
    BoxToHead,
    /// `create-pty` / `key` / `resize` / `viewport` / `mouse` / `paste` / `focus` — head → box (the
    /// safe direction, onto untrusted ground, unsanitized — `terminal.md` §4).
    HeadToBox,
}

/// One side of the terminal channel — a **transport-agnostic** byte sink. The relay emits opaque L2
/// frames here without knowing what carries them (local or network). No TLS, no peer identity: those
/// are outside the library (operator direction).
pub trait Endpoint {
    /// Emit one L2 frame's opaque bytes to this endpoint's wire.
    fn emit(&mut self, frame: &[u8]);
}

/// The relay direction of a terminal-service verb (its inherent direction, `terminal.md` §4).
/// `None` for a non-terminal verb (the dispatcher only routes terminal verbs here; this stays total).
/// **Pure routing by verb** — no content, no peer identity, no transport security.
#[must_use]
pub fn direction_of(verb: u32) -> Option<Direction> {
    match verb & 0xff {
        verb::DISPLAY | verb::PANE_STATE => Some(Direction::BoxToHead),
        verb::CREATE_PTY
        | verb::KEY
        | verb::RESIZE
        | verb::VIEWPORT
        | verb::MOUSE
        | verb::PASTE
        | verb::FOCUS => Some(Direction::HeadToBox),
        _ => None,
    }
}

/// Relay one terminal frame **verbatim** to the correct endpoint: peek the verb for direction, then
/// emit the whole opaque `payload` to the box or the head. Returns the direction, or `None` (and
/// emits nothing) for a non-terminal or too-short frame. Decodes **only** the verb — the body rides
/// opaque, never parsed (`proxy-head.md` §1). Never panics.
pub fn relay_frame(
    payload: &[u8],
    to_head: &mut impl Endpoint,
    to_box: &mut impl Endpoint,
) -> Option<Direction> {
    let verb = peek_verb(payload)?;
    match direction_of(verb)? {
        Direction::BoxToHead => {
            to_head.emit(payload);
            Some(Direction::BoxToHead)
        }
        Direction::HeadToBox => {
            to_box.emit(payload);
            Some(Direction::HeadToBox)
        }
    }
}
