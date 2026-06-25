//! The bounded, fail-closed reassembler — the protocol's **one** trusted-side reassembly (net.md §2).
//! The **proxy** accumulates a `net-payload` request whose payload spans transport frames into a
//! complete buffer before the off-wire recognizer rules on it. The transport delivers each frame's L2
//! payload and a `continuation` signal (the L1 framing lives below the crate's copy boundary, model.md
//! §1/§3); the crate parses no frame header. Because the proxy holds the buffer, totality here is **TCB
//! defense**: no payload sequence the box authors may crash, over-read, **unbounded-allocate**, or
//! **hang** it.
//!
//! The defense is one bound — the **capacity cap** ([`crate::verb::REASSEMBLY_CAP`]):
//! - **reject-before-grow:** the cap is checked *before* the append, so the buffer **never allocates
//!   past the cap** — the alloc bound holds at the check (`acc.len() + chunk.len() > cap` → reject),
//!   not after.
//! - **never-ending == cap-exceed:** an endless `continuation`-set stream trips the cap and is denied;
//!   the reassembler never awaits a clear signal it may never receive (no timeout needed at this layer).
//! - **opaque (law 11):** it concatenates the payload bytes without parsing them; the boundary is the
//!   `continuation` signal only, never a length read from content.
//!
//! Single-session: one `Reassembler` is one session's accumulation. The crate holds **no map** — the
//! transport routes each session's payload bytes to a `Reassembler` (no_std-friendly). Sprint 005,
//! seam 5A; the L1 framing excised in 010 (boundary correction).

use alloc::vec::Vec;
use core::mem::take;

use crate::reject::Reject;
use crate::verb;

/// The outcome of feeding one payload chunk to the reassembler.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ReassemblyState {
    /// `continuation` was set — more payload follows; keep feeding the same `Reassembler`.
    Accumulating,
    /// `continuation` was clear — the complete reassembled payload (the reassembler is reset).
    Complete(Vec<u8>),
}

/// One session's bounded reassembly buffer. Construct with [`Reassembler::with_capacity_bound`] (the
/// production bound) or [`Reassembler::new`] (an explicit cap — tests, or a custom-bound consumer).
pub struct Reassembler {
    acc: Vec<u8>,
    cap: usize,
}

impl Reassembler {
    /// The production reassembler, bounded by [`crate::verb::REASSEMBLY_CAP`]. The proxy uses this so
    /// it cannot accidentally pass a wrong bound; the bound is source-anchored at the construction site.
    #[must_use]
    pub fn with_capacity_bound() -> Self {
        Self::new(verb::REASSEMBLY_CAP)
    }

    /// A reassembler with an explicit capacity bound (bytes).
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Reassembler {
            acc: Vec::new(),
            cap,
        }
    }

    /// The capacity bound (bytes).
    #[must_use]
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Feed one payload chunk. `continuation` is the transport's continuation signal (set = more
    /// follows). The chunk is appended **opaquely** (never parsed); `Complete` carries the joined bytes
    /// (and resets `self`).
    ///
    /// Fail-closed: if the accumulated total would exceed the cap (**inclusive**: `≤ cap` is allowed,
    /// `> cap` rejects), returns `Err(ReassemblyOverflow)` **before** any over-cap allocation. The
    /// length addition is `checked_add` (a `usize` overflow on an absurd length also fails closed).
    pub fn push(&mut self, chunk: &[u8], continuation: bool) -> Result<ReassemblyState, Reject> {
        match self.acc.len().checked_add(chunk.len()) {
            Some(total) if total <= self.cap => {
                self.acc.extend_from_slice(chunk);
                if continuation {
                    Ok(ReassemblyState::Accumulating)
                } else {
                    Ok(ReassemblyState::Complete(take(&mut self.acc)))
                }
            }
            // > cap, or the length addition overflowed: fail closed, no over-cap alloc.
            _ => Err(Reject::ReassemblyOverflow { cap: self.cap }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Reassembler, ReassemblyState};
    use crate::reject::Reject;
    use crate::verb;
    use alloc::vec;

    const C: bool = true; // continuation set
    const E: bool = false; // continuation clear (end)

    /// The boundary mechanism, derived from a SMALL reachable cap field (not the 32 MiB const) —
    /// `cap-1 → Ok`, `cap → Ok` (inclusive), `cap+1 → Err`. Re-derives at whatever cap it holds.
    #[test]
    fn cap_boundary_inclusive_small_cap() {
        const CAP: usize = 16;
        // cap-1 in one frame -> Complete
        let mut r = Reassembler::new(CAP);
        assert_eq!(r.push(&vec![0u8; CAP - 1], E).unwrap(), ReassemblyState::Complete(vec![0u8; CAP - 1]));
        // exactly cap -> Complete (inclusive)
        let mut r = Reassembler::new(CAP);
        assert_eq!(r.push(&vec![0u8; CAP], E).unwrap(), ReassemblyState::Complete(vec![0u8; CAP]));
        // cap+1 -> Err(ReassemblyOverflow)
        let mut r = Reassembler::new(CAP);
        assert_eq!(r.push(&vec![0u8; CAP + 1], E), Err(Reject::ReassemblyOverflow { cap: CAP }));
    }

    #[test]
    fn multi_frame_joins_then_completes() {
        let mut r = Reassembler::new(64);
        assert_eq!(r.push(b"AB", C).unwrap(), ReassemblyState::Accumulating);
        assert_eq!(r.push(b"CD", C).unwrap(), ReassemblyState::Accumulating);
        assert_eq!(r.push(b"EF", E).unwrap(), ReassemblyState::Complete(b"ABCDEF".to_vec()));
        // reset after Complete: the next reassembly starts fresh
        assert_eq!(r.push(b"XY", E).unwrap(), ReassemblyState::Complete(b"XY".to_vec()));
    }

    /// Reject-before-grow at the cap boundary across frames: the second frame crosses the cap and is
    /// denied; the cap is enforced at the CHECK (no over-cap accumulation).
    #[test]
    fn reject_before_grow_across_frames() {
        const CAP: usize = 16;
        let mut r = Reassembler::new(CAP);
        assert_eq!(r.push(&vec![0u8; CAP], C).unwrap(), ReassemblyState::Accumulating); // acc == cap
        // one more byte would be cap+1 -> Err, before any over-cap alloc
        assert_eq!(r.push(&[0u8], C), Err(Reject::ReassemblyOverflow { cap: CAP }));
    }

    /// Never-ending (continuation always set) terminates via the SAME cap — no hang.
    #[test]
    fn never_ending_trips_the_cap() {
        const CAP: usize = 16;
        let mut r = Reassembler::new(CAP);
        let mut frames = 0u32;
        loop {
            match r.push(&[0u8; 4], C) {
                Ok(ReassemblyState::Accumulating) => {
                    frames += 1;
                    assert!(frames < 1000, "must trip the cap, not loop forever");
                }
                Err(Reject::ReassemblyOverflow { cap }) => {
                    assert_eq!(cap, CAP);
                    break;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    /// Production-wiring — the production reassembler carries `REASSEMBLY_CAP`. Deterministic, NO
    /// 32 MiB allocation: it reads the cap field, never accumulates to it.
    #[test]
    fn production_reassembler_wired_to_reassembly_cap() {
        assert_eq!(Reassembler::with_capacity_bound().cap(), verb::REASSEMBLY_CAP);
    }

    #[test]
    fn empty_payload_completes_empty() {
        let mut r = Reassembler::new(8);
        assert_eq!(r.push(b"", E).unwrap(), ReassemblyState::Complete(vec![]));
    }
}
