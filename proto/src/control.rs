//! The control service (the `0x0_` verb block, model.md §2/§8). The box owns session lifecycle; control
//! verbs are the head→box signals that drive it.
//!
//! Sprint 006, seam 6A: `open-session` (0x00) — an **empty-body** signal. The box is the sole session
//! allocator (§2): the new session id is not a body parameter, it arrives on the **L1 envelope**
//! `session_id` (005). So `open-session` carries nothing — it is the signal, not a carrier.

use alloc::vec::Vec;

use crate::reject::Reject;
use crate::verb;
use crate::wire::Wire;

/// `open-session` (`head → box`): begin a session. An **empty** body (no fields) — the box allocates
/// the id, which rides the envelope, not the message.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OpenSession;

impl Wire for OpenSession {
    const VERB: u32 = verb::OPEN_SESSION;

    fn encode(&self, _out: &mut Vec<u8>) {
        // empty body — the frame is just `[verb 0x00]`.
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        if body.is_empty() {
            Ok(OpenSession)
        } else {
            // A non-empty open-session body is structurally invalid — reuse `TrailingBytes` (no new
            // value-policy variant; the structural-only coherence tell holds).
            Err(Reject::TrailingBytes { extra: body.len() })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OpenSession;
    use crate::reject::Reject;
    use crate::wire::Wire;
    use alloc::vec::Vec;

    #[test]
    fn open_session_roundtrips_empty() {
        let mut out = Vec::new();
        OpenSession.encode(&mut out);
        assert!(out.is_empty(), "open-session body is empty");
        assert_eq!(OpenSession::decode(&[]), Ok(OpenSession));
    }

    #[test]
    fn open_session_rejects_non_empty() {
        assert_eq!(
            OpenSession::decode(&[0xFF]),
            Err(Reject::TrailingBytes { extra: 1 })
        );
    }
}
