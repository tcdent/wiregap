//! The L1 frame codec: `[u64 session ID][payload]`, big-endian session-ID prefix.
//!
//! This is the transport's wire frame. `nic` owns L1; proto/L2 rides opaquely in `payload` (proto
//! sprint 010 excised its own envelope, so there is exactly one session-id framing layer — here).
//! There is:
//!
//! - **no length field** — the frame length is supplied by the E3 seam, originating at the RX
//!   descriptor (`architecture.md` §6); it is bounded by the driver, never carried in our framing; and
//! - **no flag byte** (`architecture.md` §6).
//!
//! **data-vs-ACK is not a codec concern.** It is the endpoint's *role/direction* for the session
//! (Slices 3–4, unidirectional model per §6). A **bare ACK is simply a frame with an empty payload**,
//! and it is byte-identical to an empty-payload data frame — the codec neither knows nor can
//! distinguish the two, and that equivalence is *intended* (§6). Both are `encode_frame(id, &[])`
//! → exactly [`SESSION_ID_LEN`] bytes.
//!
//! Decode is zero-copy and **total** (matching proto's `Wire` discipline): any byte slice yields `Ok`
//! with a borrowed payload or a typed [`FrameError`] — never a panic, never an over-read. The
//! descriptor-supplied length behind the inbound seam is adversary-influenced (`architecture.md` §6,
//! `grounding/data-path-and-events.md` §1), so the decoder bounds every read against the slice it was
//! given and trusts no embedded count (there is none to trust).

use alloc::vec::Vec;

/// The big-endian session-ID prefix length: a `u64`, 8 bytes. A bare ACK is exactly this many bytes.
pub const SESSION_ID_LEN: usize = 8;

/// A session identifier — the frame's only key (demux only, zero security weight; `architecture.md`
/// §1.1). Box-allocated, 64-bit monotonic, unique within an epoch. On the wire it is the 8-byte
/// big-endian prefix.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SessionId(pub u64);

/// A decoded frame: a [`SessionId`] and the **borrowed** opaque payload tail (proto's L2 bytes, or
/// empty for a bare ACK). Zero-copy — the payload borrows the input slice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Frame<'a> {
    /// The session this frame is keyed to.
    pub session: SessionId,
    /// The opaque payload (L2). Empty for a bare ACK; the codec does not interpret it.
    pub payload: &'a [u8],
}

/// The one structural fault a frame can have: fewer bytes than the session-ID prefix needs, so the
/// frame cannot even name a session. (There is no length or flag field to be malformed — anything
/// `>= SESSION_ID_LEN` bytes decodes, payload being whatever follows.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameError {
    /// The input was shorter than the 8-byte session-ID prefix.
    ShortFrame { needed: usize, got: usize },
}

/// Append `[session ID big-endian][payload]` to `out`.
///
/// A bare ACK is `encode_frame(id, &[], out)` → exactly [`SESSION_ID_LEN`] bytes, byte-identical to a
/// data frame whose payload happens to be empty (§6). The encoding is the frozen wire contract (E3/E4
/// and the peer depend on it); a change is a re-scope, not a tweak.
pub fn encode_frame(session: SessionId, payload: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&session.0.to_be_bytes());
    out.extend_from_slice(payload);
}

/// Decode `[session ID big-endian][payload]`.
///
/// Total: returns [`FrameError::ShortFrame`] if fewer than [`SESSION_ID_LEN`] bytes are present, else
/// `Ok(Frame)` with `payload` borrowing the (possibly empty) tail. Never panics, never reads past
/// `bytes`. `split_first_chunk` is bounds-checked safe Rust — no indexing, no `unwrap`.
pub fn decode_frame(bytes: &[u8]) -> Result<Frame<'_>, FrameError> {
    let (id_bytes, payload) =
        bytes
            .split_first_chunk::<SESSION_ID_LEN>()
            .ok_or(FrameError::ShortFrame {
                needed: SESSION_ID_LEN,
                got: bytes.len(),
            })?;
    Ok(Frame {
        session: SessionId(u64::from_be_bytes(*id_bytes)),
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    /// GOLDEN BYTE-PIN (gate 2): the 8-byte prefix is big-endian (most-significant byte first), and the
    /// payload follows verbatim. These exact bytes are the frozen contract.
    #[test]
    fn golden_big_endian_prefix() {
        // A recognizable id that exposes byte order: 0x0102030405060708 -> 01 02 03 04 05 06 07 08.
        let mut buf = Vec::new();
        encode_frame(SessionId(0x0102_0304_0506_0708), &[0xAA, 0xBB], &mut buf);
        assert_eq!(
            buf,
            vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0xAA, 0xBB]
        );

        // id = 1 -> seven zero bytes then 0x01 (big-endian, not little).
        let mut one = Vec::new();
        encode_frame(SessionId(1), &[], &mut one);
        assert_eq!(one, vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]);
    }

    /// GOLDEN EQUIVALENCE (gate 2): a bare ACK is exactly 8 bytes, and an empty-payload data frame and a
    /// bare ACK are byte-identical — the codec cannot tell them apart (role/direction does, §6).
    #[test]
    fn bare_ack_equals_empty_data_frame_exactly_eight_bytes() {
        let id = SessionId(0xDEAD_BEEF_0000_0001);

        // What a *receiver* emits as a bare ACK on this session.
        let mut ack = Vec::new();
        encode_frame(id, &[], &mut ack);

        // What a *sender* emits as a data frame whose payload is empty.
        let mut empty_data = Vec::new();
        encode_frame(id, &[], &mut empty_data);

        assert_eq!(ack.len(), SESSION_ID_LEN, "a bare ACK is exactly 8 bytes");
        assert_eq!(ack, empty_data, "bare ACK == empty-payload data frame, byte for byte");

        // And on decode they are indistinguishable: same session, same (empty) payload.
        assert_eq!(decode_frame(&ack), decode_frame(&empty_data));
        let f = decode_frame(&ack).unwrap();
        assert_eq!(f.session, id);
        assert!(f.payload.is_empty());
    }

    /// Round-trip: encode then decode recovers the session and payload exactly, for empty and non-empty
    /// payloads (the payload is carried verbatim — opaque L2).
    #[test]
    fn round_trip_preserves_session_and_payload() {
        for payload in [
            &[][..],
            &[0x00][..],
            &[0xFF, 0x00, 0x7F, 0x80][..],
            &[0xAB; 1500][..], // a full-MTU-ish opaque payload
        ] {
            let id = SessionId(0x1234_5678_9ABC_DEF0);
            let mut buf = Vec::new();
            encode_frame(id, payload, &mut buf);
            let f = decode_frame(&buf).expect("a frame we just encoded must decode");
            assert_eq!(f.session, id);
            assert_eq!(f.payload, payload);
        }
    }

    /// A frame of exactly the prefix length decodes to an empty payload (the minimal valid frame).
    #[test]
    fn minimal_frame_is_empty_payload() {
        let buf = [0u8; SESSION_ID_LEN];
        let f = decode_frame(&buf).unwrap();
        assert_eq!(f.session, SessionId(0));
        assert!(f.payload.is_empty());
    }

    /// Totality, negative side: every length below the prefix is a typed `ShortFrame`, never a panic.
    #[test]
    fn short_frames_reject_typed() {
        for got in 0..SESSION_ID_LEN {
            let buf = vec![0u8; got];
            assert_eq!(
                decode_frame(&buf),
                Err(FrameError::ShortFrame {
                    needed: SESSION_ID_LEN,
                    got,
                }),
                "len {got} must reject as ShortFrame"
            );
        }
    }

    /// Totality (gate 2 "arbitrary bytes never panic/over-read"): sweep many lengths and content
    /// patterns; each decode yields a typed outcome and, on Ok, the payload is exactly the tail past the
    /// 8-byte prefix. The test running to completion is the no-panic / no-over-read proof.
    #[test]
    fn arbitrary_bytes_never_panic() {
        for len in 0..64usize {
            for seed in [0u8, 0x55, 0xAA, 0xFF] {
                let buf: Vec<u8> = (0..len).map(|i| seed ^ (i as u8)).collect();
                match decode_frame(&buf) {
                    Ok(f) => {
                        assert!(len >= SESSION_ID_LEN);
                        assert_eq!(f.payload, &buf[SESSION_ID_LEN..]);
                        assert_eq!(f.session.0, u64::from_be_bytes(buf[..SESSION_ID_LEN].try_into().unwrap()));
                    }
                    Err(FrameError::ShortFrame { needed, got }) => {
                        assert_eq!(needed, SESSION_ID_LEN);
                        assert_eq!(got, len);
                        assert!(len < SESSION_ID_LEN);
                    }
                }
            }
        }
    }
}
