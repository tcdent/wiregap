//! The L1 frame envelope — the frame header the box encodes and the **proxy** decodes (model.md
//! §1/§2). It wraps the L2 message ([`crate::decode_message`]) with the session id and the
//! continuation flag the reassembler needs:
//!
//! `[session_id u32 LE][flags u32 LE][L2 [verb u32][body]]` — 8 bytes, keeping the L2 verb 4-byte
//! aligned (model.md §5). The crate **parses** the envelope (single-source/anti-drift: the box
//! encoder and the proxy decoder agree on one layout); the **consumer routes by `session_id`** —
//! the crate never acts on identity or multiplexing.
//!
//! The continuation bit is read from a **fixed position** (`flags` bit 0), never from content —
//! adversarial bytes cannot steer reassembly (law 11). Reserved flag bits 1–31 are **dropped** (the
//! 002/004 reserved-bit-drop). Sprint 005, seam 5A.

use alloc::vec::Vec;

use crate::reject::Reject;
use crate::verb;
use crate::wire::take_prefix;

/// A decoded L1 frame: the session id, the continuation flag, and the **borrowed** L2 bytes (located
/// in the caller's owned frame copy — never the NIC ring). The consumer routes by `session_id`; the
/// reassembler reads `continuation`; `l2` is handed to [`crate::decode_message`] (after reassembly,
/// for `net-payload`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Frame<'a> {
    pub session_id: u32,
    pub continuation: bool,
    pub l2: &'a [u8],
}

/// Decode an L1 frame. Total: `< ENVELOPE_LEN` bytes → `ShortBody`; reserved flag bits 1–31 dropped.
pub fn decode_frame(frame: &[u8]) -> Result<Frame<'_>, Reject> {
    // Take the fixed `ENVELOPE_LEN` header (`< ENVELOPE_LEN` → ShortBody{ENVELOPE_LEN, frame.len()});
    // the remainder is the borrowed L2 tail. Read session_id + flags from the destructured header.
    let (header, l2) = take_prefix::<{ verb::ENVELOPE_LEN }>(frame)?;
    let [s0, s1, s2, s3, f0, f1, f2, f3] = *header;
    let flags = u32::from_le_bytes([f0, f1, f2, f3]);
    Ok(Frame {
        session_id: u32::from_le_bytes([s0, s1, s2, s3]),
        // continuation from the fixed bit only; bits 1–31 dropped (never read).
        continuation: flags & (1u32 << verb::FLAG_CONTINUATION_BIT) != 0,
        l2,
    })
}

/// Encode an L1 frame (box-side, single-source). Reserved flag bits are written zero (canonical).
pub fn encode_frame(session_id: u32, continuation: bool, l2: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&session_id.to_le_bytes());
    let flags: u32 = if continuation {
        1u32 << verb::FLAG_CONTINUATION_BIT
    } else {
        0
    };
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(l2);
}

#[cfg(test)]
mod tests {
    use super::{Frame, decode_frame, encode_frame};
    use crate::reject::Reject;
    use crate::verb;
    use alloc::vec::Vec;

    fn enc(session_id: u32, continuation: bool, l2: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        encode_frame(session_id, continuation, l2, &mut out);
        out
    }

    #[test]
    fn frame_roundtrips() {
        for &(sid, cont) in &[(0u32, false), (1, true), (u32::MAX, true), (42, false)] {
            let l2: &[u8] = &[0x33, 0x00, 0x00, 0x00, 0xAB, 0xCD];
            let wire = enc(sid, cont, l2);
            assert_eq!(wire.len(), verb::ENVELOPE_LEN + l2.len());
            let f = decode_frame(&wire).unwrap();
            assert_eq!(
                f,
                Frame { session_id: sid, continuation: cont, l2 }
            );
            // canonical re-encode
            assert_eq!(enc(f.session_id, f.continuation, f.l2), wire);
        }
    }

    #[test]
    fn decode_frame_total_short() {
        for n in 0..verb::ENVELOPE_LEN {
            let buf = alloc::vec![0u8; n];
            assert_eq!(
                decode_frame(&buf),
                Err(Reject::ShortBody { needed: verb::ENVELOPE_LEN, got: n })
            );
        }
        // exactly ENVELOPE_LEN with an empty L2 is valid (empty l2).
        let f = decode_frame(&[0u8; 8]).unwrap();
        assert_eq!(f.l2.len(), 0);
        assert!(!f.continuation);
    }

    #[test]
    fn reserved_flag_bits_dropped() {
        // flags = 0xFFFFFFFF — every bit set; only bit 0 (continuation) is read, 1–31 dropped.
        #[rustfmt::skip]
        let wire: &[u8] = &[
            0x07, 0x00, 0x00, 0x00, // session_id = 7
            0xFF, 0xFF, 0xFF, 0xFF, // flags all-set
            0x99,                   // l2
        ];
        let f = decode_frame(wire).unwrap();
        assert!(f.continuation); // bit 0 set
        assert_eq!(f.session_id, 7);
        // re-encode drops the reserved bits -> flags == 1 (only continuation)
        let canonical = enc(7, true, &[0x99]);
        assert_eq!(&canonical[4..8], &1u32.to_le_bytes());
    }
}
