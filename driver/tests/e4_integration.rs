//! No-NIC E4 integration (Sprint 3, Slice 4; re-wired for the Sprint-6 reactor move): the **fresh
//! reactor** (E4) driving the **driver's seams** (E3) over the **sim rings** + the **real transport**
//! (E5) — the full stack wired together with no device. Proves E3 satisfies the E4 seams the reactor was
//! built against: an inbound L1 frame arrives through the driver's `RxDrain`, is adapted to the reactor's
//! `RxSource` by [`RxDrainAsSource`], the reactor demuxes it via the transport, delivers the payload up,
//! and the receiver's bare ACK goes out through the driver's `TransmitSink`.
//!
//! This is also the adapter's **real-driver** integration proof: the driver's actual owned-frame
//! `RxDrain` (not a stub) flows through `RxDrainAsSource` into the fresh reactor.
//!
//! The real-volatile `dma` impls are written + confined (Slice 4) but bench-gated — exercised only on the
//! prototype cards — so this integration uses the software sim rings, which sit behind the *same* seams.

use core::time::Duration;
use driver::sim::{SimRxRing, SimTxRing};
use driver::{RXD_STAT_EOP, RxPath, TxPath};
use reactor::{Host, Reactor, RxDrainAsSource, WakeReason};
use transport::{Instant, SessionId, SessionTable, encode_frame};

const BUF: usize = 2048;

/// A trivial substrate seam for the no-NIC test: monotonic zero, and a `block` that never has to wait
/// (all work here is a single synchronous RX step).
struct ImmediateHost;
impl Host for ImmediateHost {
    fn now(&self) -> Instant {
        Instant(0)
    }
    fn block(&mut self, _timeout: Option<Duration>) -> WakeReason {
        WakeReason::NicMsix
    }
}

#[test]
fn full_stack_inbound_frame_delivers_up_and_acks_out() {
    // RX sim ring with one inbound L1 frame: `[u64 session id BE][payload]` for session 5, payload "hi".
    let mut sim_rx = SimRxRing::new(4, BUF);
    let mut l1 = Vec::new();
    encode_frame(SessionId(5), b"hi", &mut l1);
    sim_rx.post(0, l1.len() as u32, RXD_STAT_EOP, l1);

    // The driver's seams: RX-drain (reactor::RxDrain) adapted to the reactor's RxSource, and TX-write
    // (transport::TransmitSink). The reactor owns all three plus the transport.
    let rx = RxDrainAsSource::new(RxPath::new(sim_rx));
    let tx = TxPath::new(SimTxRing::new(4, BUF));
    let mut reactor = Reactor::new(SessionTable::new(), ImmediateHost, rx, tx);

    // One unit of work: the inbound frame is pulled through the adapter, demuxed by the transport,
    // delivered up (held in the reactor's pending slot), and the bare ACK emitted through the TX seam.
    assert!(reactor.step(), "the inbound frame was processed");

    // The inbound frame bubbled to its session and was delivered up (the fresh reactor holds the payload;
    // the consumer takes it, releasing back-pressure).
    assert_eq!(reactor.take_delivery().map(|(_, p)| p).as_deref(), Some(b"hi".as_slice()), "payload delivered up");

    // The receiver's bare ACK went out through the driver's TX seam: the 8-byte big-endian session id,
    // empty payload — `[00 00 00 00 00 00 00 05]`.
    assert_eq!(
        reactor.sink().ring().buffer(0),
        &[0, 0, 0, 0, 0, 0, 0, 5],
        "the bare ACK for session 5 was emitted through the driver TX-write seam"
    );
}
