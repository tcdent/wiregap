//! No-NIC E4 integration (Sprint 3, Slice 4): the **real reactor** (E4) driving the **driver's seams**
//! (E3) over the **sim rings** + the **real transport** (E5) — the full stack wired together with no
//! device. Proves E3 satisfies the E4 seams the reactor was built against: an inbound L1 frame arrives
//! through the driver's `RxDrain`, the reactor demuxes it via the transport, delivers the payload up, and
//! the receiver's bare ACK goes out through the driver's `TransmitSink`.
//!
//! The real-volatile `dma` impls are written + confined (Slice 4) but bench-gated — exercised only on the
//! prototype cards — so this integration uses the software sim rings, which sit behind the *same* seams.

use driver::sim::{SimRxRing, SimTxRing};
use driver::{RXD_STAT_EOP, RxPath, TxPath};
use reactor::{Eventfd, MonotonicClock, Reactor};
use transport::{Delivery, SessionId, SessionTable, encode_frame};

const BUF: usize = 2048;

#[test]
fn full_stack_inbound_frame_delivers_up_and_acks_out() {
    // RX sim ring with one inbound L1 frame: `[u64 session id BE][payload]` for session 5, payload "hi".
    let mut sim_rx = SimRxRing::new(4, BUF);
    let mut l1 = Vec::new();
    encode_frame(SessionId(5), b"hi", &mut l1);
    sim_rx.post(0, l1.len() as u32, RXD_STAT_EOP, l1);

    // The driver's seams: RX-drain (reactor::RxDrain) and TX-write (transport::TransmitSink).
    let mut rx = RxPath::new(sim_rx);
    let mut tx = TxPath::new(SimTxRing::new(4, BUF));

    // The real reactor + transport. Pre-arm the eventfd so the loop's block returns (a receiver session
    // arms no retransmit deadline → timeout is -1).
    let efd = Eventfd::new().unwrap();
    let notifier = efd.try_clone().unwrap();
    let mut reactor = Reactor::new(efd, SessionTable::new()).unwrap();
    notifier.notify(1).unwrap();

    let mut delivered: Vec<(SessionId, Vec<u8>)> = Vec::new();
    {
        let mut on_delivery = |d: Delivery<'_>| delivered.push((d.session, d.payload.to_vec()));
        reactor
            .run_once(&mut rx, &mut tx, &mut on_delivery, &MonotonicClock)
            .unwrap();
    }

    // The inbound frame bubbled to its session and was delivered up.
    assert_eq!(delivered, vec![(SessionId(5), b"hi".to_vec())], "frame delivered up to session 5");

    // The receiver's bare ACK went out through the driver's TX seam: the 8-byte big-endian session id,
    // empty payload — `[00 00 00 00 00 00 00 05]`.
    assert_eq!(
        tx.ring().buffer(0),
        &[0, 0, 0, 0, 0, 0, 0, 5],
        "the bare ACK for session 5 was emitted through the driver TX-write seam"
    );
}
