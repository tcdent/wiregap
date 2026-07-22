//! G1 — host `cargo test` of the bare-metal reactor's **ratified behavior**, deterministically, with no
//! hardware. These exercise the *exact* shipped functions (`arm_from_deadline`, `can_pull_rx`, and the
//! `step`/`park` loop) over a virtual clock + a capture sink + a stub RX source. The substrate `Host`
//! impl (LAPIC/hlt/MSI-X) is proven separately in QEMU (G2); here the behavior is proven in isolation.

use core::cell::Cell;
use core::time::Duration;
use std::collections::VecDeque;
use reactor::{arm_from_deadline, can_pull_rx, Arm, Host, Reactor, RxSource, WakeReason};
use transport::{decode_frame, Instant, Originator, SendError, SessionId, SessionTable, TransmitSink};

// ---- deadline composition (the three edge-facts) ----------------------------------------------

#[test]
fn arm_from_deadline_has_exactly_three_edges() {
    let now = Instant(1_000);
    assert_eq!(arm_from_deadline(None, now), Arm::ParkForever, "no deadline → park forever");
    assert_eq!(arm_from_deadline(Some(Instant(1_000)), now), Arm::DueNow, "deadline == now → due");
    assert_eq!(arm_from_deadline(Some(Instant(500)), now), Arm::DueNow, "deadline in the past → due");
    assert_eq!(
        arm_from_deadline(Some(Instant(1_500)), now),
        Arm::ParkUntil(Duration::from_nanos(500)),
        "future deadline → arm the one-shot for the delta (ns, not i32-ms)"
    );
}

#[test]
fn can_pull_rx_is_back_pressure() {
    assert!(can_pull_rx(true, false), "frame available, nothing in flight → pull");
    assert!(!can_pull_rx(true, true), "frame available but a delivery in flight → HOLD (RDT held)");
    assert!(!can_pull_rx(false, false), "no frame → nothing to pull");
}

// ---- fakes ------------------------------------------------------------------------------------

/// Virtual clock + a scripted sequence of wake reasons; advances time on each block.
struct FakeHost {
    now: Cell<u64>,
    wakes: VecDeque<WakeReason>,
}
impl FakeHost {
    fn new(wakes: impl IntoIterator<Item = WakeReason>) -> Self {
        Self { now: Cell::new(0), wakes: wakes.into_iter().collect() }
    }
}
impl Host for FakeHost {
    fn now(&self) -> Instant {
        Instant(self.now.get())
    }
    fn block(&mut self, timeout: Option<Duration>) -> WakeReason {
        // A timer wake advances the clock to the deadline; a NIC wake does not.
        let reason = self.wakes.pop_front().unwrap_or(WakeReason::NicMsix);
        if let (Some(d), WakeReason::Timer) = (timeout, reason) {
            self.now.set(self.now.get() + d.as_nanos() as u64);
        }
        reason
    }
}

/// A stub RX source over a queue of owned frames.
struct StubRx {
    frames: VecDeque<Vec<u8>>,
    front: Vec<u8>,
}
impl StubRx {
    fn new(frames: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self { frames: frames.into_iter().collect(), front: Vec::new() }
    }
}
impl RxSource for StubRx {
    fn has_frame(&self) -> bool {
        !self.frames.is_empty()
    }
    fn next_frame(&mut self) -> Option<&[u8]> {
        self.front = self.frames.pop_front()?;
        Some(&self.front)
    }
}

/// A capture sink recording every emitted frame.
#[derive(Default)]
struct CaptureSink {
    emitted: Vec<Vec<u8>>,
}
impl TransmitSink for CaptureSink {
    fn emit(&mut self, frame: &[u8]) {
        self.emitted.push(frame.to_vec());
    }
}

/// Build a receiver-role inbound frame for `session` carrying `payload` (matches the transport's L1
/// frame: `[u64 BE session id][payload]`).
fn inbound(session: u64, payload: &[u8]) -> Vec<u8> {
    let mut f = session.to_be_bytes().to_vec();
    f.extend_from_slice(payload);
    f
}

// ---- the loop behavior ------------------------------------------------------------------------

#[test]
fn zero_parks_while_there_is_work() {
    // A burst of inbound frames: step() keeps finding work, so the reactor never parks.
    let frames = (0..5u64).map(|i| inbound(100 + i, b"hello")).collect::<Vec<_>>();
    let mut r = Reactor::new(SessionTable::new(Originator::Low), FakeHost::new([]), StubRx::new(frames), CaptureSink::default());

    let mut processed = 0;
    // Drain the burst, consuming each delivery so back-pressure never latches.
    while r.step() {
        if r.take_delivery().is_some() {
            processed += 1;
        }
    }
    assert_eq!(processed, 5, "all five inbound frames delivered");
    assert_eq!(r.parks(), 0, "ZERO parks while there was work (the fast path never blocks)");
}

#[test]
fn one_in_flight_rx_back_pressure() {
    let mut r = Reactor::new(
        SessionTable::new(Originator::Low),
        FakeHost::new([]),
        StubRx::new([inbound(1, b"a"), inbound(2, b"b")]),
        CaptureSink::default(),
    );

    // Process frame 1 → a delivery is held; do NOT consume it.
    assert!(r.step(), "frame 1 processed");
    assert!(r.back_pressured(), "a delivery is in flight → back-pressure engaged");
    // With back-pressure engaged, step() must NOT pull frame 2 (RDT held).
    assert!(!r.step(), "frame 2 is HELD while a delivery is in flight");
    assert!(r.back_pressured());
    // Consume the delivery → back-pressure releases → frame 2 flows.
    assert!(r.take_delivery().is_some());
    assert!(!r.back_pressured());
    assert!(r.step(), "frame 2 flows once back-pressure releases");
}

#[test]
fn deadline_park_then_timer_wake_drives_retransmit() {
    // Originate a session: the sender emits once and arms a retransmit deadline. Idle → park until the
    // timer → the timer wake advances the clock past the deadline → the next drain re-emits (retransmit).
    let mut r = Reactor::new(
        SessionTable::new(Originator::Low),
        FakeHost::new([WakeReason::Timer]),
        StubRx::new([]),
        CaptureSink::default(),
    );
    r.originate(b"payload");

    // Idle (no RX, deadline in the future) → drain nothing, then park; the fake timer wake advances the
    // clock to the deadline.
    assert_eq!(r.pump_all(|_, _| {}), 0, "nothing to drain while idle");
    let reason = r.park();
    assert_eq!(reason, Some(WakeReason::Timer), "woke by the one-shot timer");
    assert_eq!(r.parks(), 1, "it actually parked");

    // Now the deadline is due → the next drain drives the retransmit (a second emission).
    assert!(r.step(), "the due deadline drives a retransmit");
}

#[test]
fn park_forever_when_no_deadline_and_wakes_on_nic_msix() {
    // No sessions, no RX → no deadline → ParkForever; a NIC-MSI-X wake returns the woke-by class.
    let mut r = Reactor::new(
        SessionTable::new(Originator::Low),
        FakeHost::new([WakeReason::NicMsix]),
        StubRx::new([]),
        CaptureSink::default(),
    );
    assert_eq!(r.pump_all(|_, _| {}), 0, "nothing to drain while idle");
    let reason = r.park();
    assert_eq!(reason, Some(WakeReason::NicMsix), "no deadline → parked forever, woken by NIC MSI-X");
    assert_eq!(r.parks(), 1);
}

// ---- the burst-safe drive: pump_all (the reactor's own busy-drain) -----------------------------

#[test]
fn pump_all_busy_drains_a_backlog_with_zero_parks() {
    // The accepted §7 fix, tested where the code now lives: feed a K-frame backlog and drive with
    // `pump_all`; it drains the whole burst handing each delivery to `consume`, with ZERO parks (§3
    // busy-poll while frames are present); the caller parks exactly once, when genuinely idle.
    const K: usize = 32;
    let frames = (0..K).map(|i| inbound(1000 + i as u64, &[i as u8; 8])).collect::<Vec<_>>();
    let mut r =
        Reactor::new(SessionTable::new(Originator::Low), FakeHost::new([]), StubRx::new(frames), CaptureSink::default());

    let mut consumed = 0;
    let work = r.pump_all(|_, _| consumed += 1);

    assert_eq!(consumed, K, "all K frames delivered in ONE busy-drain");
    assert_eq!(work, K, "K work units (one step per delivery)");
    assert_eq!(r.parks(), 0, "ZERO parks across the whole backlog drain (§3: busy-poll while frames present)");

    // Genuinely idle now (ring empty, no due retransmit) → exactly one park.
    r.park();
    assert_eq!(r.parks(), 1, "one park once genuinely idle (block only when idle)");
}

#[test]
fn drive_one_then_park_per_delivery_is_the_defect() {
    // The DEFECT `pump_all` removes, pinned in-crate as a contrast to the ZERO above. The old drive —
    // drain-one-then-park with the take OUTSIDE the drain — parks once PER delivery: under §3.1
    // back-pressure `step()` delivers one (latching `pending`) then returns false until the consumer
    // takes it, so a park falls between every delivery. Reproduced here with the public step/take/park
    // primitives (there is no `pump_once` to misuse — it was removed); K parks for a K-burst, vs zero.
    const K: usize = 8;
    let frames = (0..K).map(|i| inbound(2000 + i as u64, &[i as u8; 8])).collect::<Vec<_>>();
    let mut r =
        Reactor::new(SessionTable::new(Originator::Low), FakeHost::new([]), StubRx::new(frames), CaptureSink::default());

    let mut consumed = 0;
    for _ in 0..K {
        while r.step() {} // delivers one, then back-pressure halts the drain
        if r.take_delivery().is_some() {
            consumed += 1;
        }
        r.park(); // parks after EACH single delivery — the defect
    }
    assert_eq!(consumed, K);
    assert_eq!(r.parks(), K as u32, "the DEFECT: one park PER delivery (contrast pump_all's zero)");
}

// ---- the quiescence observability accessor (ADDED, Sprint-9 Slice-2 facade transition) ----------

#[test]
fn quiescent_reflects_pending_and_in_flight() {
    // The one-directional-burst pacing signal `quiescent()` — no held delivery AND no sender in flight.
    // Expected values are derived from the reactor's real pending/next_deadline behavior, not hand-computed.
    let mut r = Reactor::new(
        SessionTable::new(Originator::Low),
        FakeHost::new([]),
        StubRx::new([inbound(7, b"data")]),
        CaptureSink::default(),
    );
    assert!(r.quiescent(), "fresh: no session, no held delivery → quiescent");
    assert!(r.step(), "process the inbound receiver frame → a delivery is held");
    assert!(!r.quiescent(), "a held delivery → NOT quiescent (back-pressure engaged)");
    assert!(r.take_delivery().is_some());
    assert!(r.quiescent(), "delivery consumed, no sender in flight → quiescent again");

    // An originated (unacked) sender arms a retransmit deadline → NOT quiescent until it is ACKed.
    r.originate(b"payload");
    assert!(!r.quiescent(), "an in-flight unacked sender (armed deadline) → NOT quiescent");
}

// ---- per-delivery session demux (ADDED, Sprint-9 Slice-2 Delivered(SessionId, payload) ruling) ---------

#[test]
fn delivers_each_session_id_with_its_payload() {
    // Two concurrent inbound sessions must each be delivered keyed to its OWN transport-computed SessionId
    // (the multiplexed-demux the earlier `Delivery { payload, .. }` drop could not express). Expected values
    // are read from the real (session, payload) pairs the reactor produces, not hand-computed.
    let mut r = Reactor::new(
        SessionTable::new(Originator::Low),
        FakeHost::new([]),
        StubRx::new([inbound(10, b"alpha"), inbound(20, b"bravo")]),
        CaptureSink::default(),
    );
    let mut got: Vec<(SessionId, Vec<u8>)> = Vec::new();
    let work = r.pump_all(|session, payload| got.push((session, payload)));

    assert_eq!(work, 2, "both inbound frames drained");
    assert_eq!(got.len(), 2, "both sessions delivered");
    assert_eq!(got[0], (SessionId(10), b"alpha".to_vec()), "first delivery keyed to session 10");
    assert_eq!(got[1], (SessionId(20), b"bravo".to_vec()), "second delivery keyed to session 20");
}

// ---- multi-frame session continuation (ADDED, Sprint-9 open/send split) -------------------------

#[test]
fn multi_frame_stream_continues_on_one_session() {
    // open() (originate) emits frame 1; a continuation while frame 1 is in flight is WouldBlock
    // (stop-and-wait, ≤1); after the ACK, send_on_session(id, ..) streams frame 2 on the SAME session.
    // Expected values are read from the real emitted frames (both decode to the one session id + payloads),
    // not hand-computed. (Same-session continuation is collision-safe under the shared allocator — one
    // originator; the both-originate model needs the Task-15 originator-partition, not built.)
    let mut r = Reactor::new(
        SessionTable::new(Originator::Low),
        FakeHost::new([]),
        StubRx::new([inbound(0, b"")]), // the bare ACK for session 0 (the first-opened session)
        CaptureSink::default(),
    );
    let id = r.originate(b"frame-1");
    assert_eq!(id, SessionId(0), "first opened session id");
    assert_eq!(
        r.send_on_session(id, b"frame-2"),
        Err(SendError::WouldBlock),
        "a continuation while frame 1 is in flight is refused (stop-and-wait)"
    );
    assert!(r.step(), "the ACK for session 0 is processed → frame 1 released");
    assert_eq!(r.send_on_session(id, b"frame-2"), Ok(()), "with frame 1 ACKed, the continuation streams");

    let emitted = &r.sink().emitted;
    assert_eq!(emitted.len(), 2, "two data frames emitted: frame 1 + the continuation");
    let f1 = decode_frame(&emitted[0]).expect("frame 1 decodes");
    let f2 = decode_frame(&emitted[1]).expect("frame 2 decodes");
    assert_eq!((f1.session, f1.payload), (SessionId(0), b"frame-1".as_slice()), "frame 1 on session 0");
    assert_eq!((f2.session, f2.payload), (SessionId(0), b"frame-2".as_slice()), "frame 2 on the SAME session 0");
}
