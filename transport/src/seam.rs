//! The three seams that keep the protocol pure software (no syscalls in crate logic), plus the pinned
//! fourth-boundary decision. E3 (data path) and E4 (reactor) implement these later; in test a fake
//! wire + virtual clock stand in (see this module's tests).
//!
//! 1. [`TransmitSink`] — **OUT.** "Emit these bytes onto the wire." E4 writes the TX ring; in test the
//!    fake wire records them. The bytes are one complete L1 frame from [`crate::encode_frame`]; from the
//!    protocol's view it is fire-and-forget (reliability is the sender role's ACK loop, Slice 3).
//! 2. [`Clock`] — **OUT.** A monotonic `now()` ([`Instant`]), advanceable in test. E4 supplies real
//!    time; the protocol *composes* the next retransmit deadline from it (Slice 3) but never sleeps or
//!    waits — the `epoll_wait` that consumes the deadline is E4's (`architecture.md` §3, §5).
//! 3. **The inbound-frame entry — IN.** Owned bytes that arrived (already copied out of the DMA region
//!    by E3 at the copy boundary, `architecture.md` §3.1 — so this crate never touches device-writable
//!    memory). This is the engine's *entry method* (Slice 2), not a trait the crate calls out to: the
//!    reactor calls *in*, the crate calls *out* only via the two traits above.
//!
//! ## Fourth boundary — PINNED (Slice 1 decision): upward delivery is the RETURN VALUE of the inbound
//! entry, NOT a fourth seam.
//!
//! When the inbound entry processes a frame it produces **at most one** upward result (a received
//! payload to deliver up, or nothing for a bare ACK), and upward delivery happens **only** as a
//! consequence of an inbound frame — retransmit (clock-driven) and give-up emit nothing upward
//! (`architecture.md` §7/§8). With no out-of-band upward path and a 0-or-1 fan-out, a **return value**
//! is the minimal correct shape: the reactor (E4) drives the inbound entry and consumes what it
//! returns, and the crate holds **no reference to app/reactor state** — it calls out only to the wire.
//! A held upward *sink* would be a standing capability the crate never needs, and would invert the
//! "reactor owns the loop, transport is pure logic" relation. (The concrete return *type* — how a
//! delivered payload vs an ACK is represented — is role-model-dependent and lands with Slices 2–4;
//! only the boundary *shape* is fixed here.)
//!
//! ## ACK-bound retained buffer (stated here, Slice 3 owns the code): app-owned bytes, not a DMA buffer
//!
//! The sender holds the frame it awaits an ACK for in plain `alloc` memory; the DMA copy lives behind
//! [`TransmitSink`] in E3, so the sink never leaks E3 allocation into this crate.
//!
//! ## Carried external invariants (this crate cannot prove them; tracked deps on E3/E4)
//!
//! (i) the reactor drains the RX ring **before** checking any retransmit deadline (E4 ordering), and
//! (ii) the link does **not** reorder (E3/wire). §6's flag-free, no-stale-ACK correctness rests on
//! both; the Slice-4 harness *imposes* them, it does not prove them.

use core::time::Duration;

/// A monotonic instant from the [`Clock`] seam — opaque ticks (nanoseconds by convention), comparable
/// and ordered.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Instant(pub u64);

impl Instant {
    /// Advance by a [`Duration`]. Saturating: the nanosecond space is `u64` (~584 years), so this never
    /// wraps in practice — a duration past that ceiling clamps rather than panics, keeping deadline math
    /// total. Used by the sender role to arm retransmit / give-up deadlines (Slice 3).
    pub fn saturating_add(self, d: Duration) -> Instant {
        let ns = u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
        Instant(self.0.saturating_add(ns))
    }
}

/// OUT seam: a monotonic clock. The protocol reads `now()`; it never sleeps, blocks, or waits.
pub trait Clock {
    /// The current monotonic instant. Successive calls never go backwards.
    fn now(&self) -> Instant;
}

/// OUT seam: the wire. `emit` is handed one complete L1 frame (`[u64 id BE][payload]`). Fire-and-forget
/// from the protocol's view — the sender role's ACK loop (Slice 3) provides reliability, not the sink.
pub trait TransmitSink {
    /// Emit one complete L1 frame onto the wire.
    fn emit(&mut self, frame: &[u8]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SessionId, encode_frame};
    use alloc::vec;
    use alloc::vec::Vec;
    use core::cell::Cell;

    /// Test stand-in for E4's TX ring: records every emitted frame verbatim so a test can assert the
    /// exact bytes that went on the wire. (This is the "fake wire records emitted frames" of the Slice 1
    /// method; it grows ACK-drop / inject controls in Slice 4.)
    #[derive(Default)]
    struct FakeWire {
        sent: Vec<Vec<u8>>,
    }
    impl TransmitSink for FakeWire {
        fn emit(&mut self, frame: &[u8]) {
            self.sent.push(frame.to_vec());
        }
    }

    /// Test stand-in for E4's clock: a manually advanced virtual clock. Interior mutability so `now()`
    /// takes `&self` (the seam shape) while a test advances it.
    struct VirtualClock {
        now_ns: Cell<u64>,
    }
    impl VirtualClock {
        fn new() -> Self {
            Self {
                now_ns: Cell::new(0),
            }
        }
        fn advance(&self, ns: u64) {
            self.now_ns.set(self.now_ns.get() + ns);
        }
    }
    impl Clock for VirtualClock {
        fn now(&self) -> Instant {
            Instant(self.now_ns.get())
        }
    }

    /// The transmit-sink seam carries a complete L1 frame to the wire, recorded byte-exact.
    #[test]
    fn fake_wire_records_emitted_frame_verbatim() {
        let mut wire = FakeWire::default();
        let mut buf = Vec::new();
        encode_frame(SessionId(0x0102_0304_0506_0708), &[0xAA, 0xBB], &mut buf);
        wire.emit(&buf);

        assert_eq!(wire.sent.len(), 1);
        assert_eq!(
            wire.sent[0],
            vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0xAA, 0xBB]
        );
    }

    /// The clock seam reads `now()` and advances monotonically — the timing surface Slice 3 builds on.
    #[test]
    fn virtual_clock_is_monotonic_and_advanceable() {
        let clk = VirtualClock::new();
        assert_eq!(clk.now(), Instant(0));
        let t0 = clk.now();
        clk.advance(10_000_000); // 10 ms in ns
        let t1 = clk.now();
        assert_eq!(t1, Instant(10_000_000));
        assert!(t1 > t0, "now() must advance, never go backwards");
    }
}
