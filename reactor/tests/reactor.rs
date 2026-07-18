//! G1 — host `cargo test` of the bare-metal reactor's **ratified behavior**, deterministically, with no
//! hardware. These exercise the *exact* shipped functions (`arm_from_deadline`, `can_pull_rx`, and the
//! `step`/`park` loop) over a virtual clock + a capture sink + a stub RX source. The substrate `Host`
//! impl (LAPIC/hlt/MSI-X) is proven separately in QEMU (G2); here the behavior is proven in isolation.

use core::cell::Cell;
use core::time::Duration;
use std::collections::VecDeque;
use reactor::{arm_from_deadline, can_pull_rx, Arm, Host, Reactor, RxSource, WakeReason};
use transport::{Instant, SessionTable, TransmitSink};

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
    let mut r = Reactor::new(SessionTable::new(), FakeHost::new([]), StubRx::new(frames), CaptureSink::default());

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
        SessionTable::new(),
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
        SessionTable::new(),
        FakeHost::new([WakeReason::Timer]),
        StubRx::new([]),
        CaptureSink::default(),
    );
    r.originate(b"payload");

    // Idle (no RX, deadline in the future) → drain nothing, then park; the fake timer wake advances the
    // clock to the deadline.
    assert_eq!(r.pump_all(|_| {}), 0, "nothing to drain while idle");
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
        SessionTable::new(),
        FakeHost::new([WakeReason::NicMsix]),
        StubRx::new([]),
        CaptureSink::default(),
    );
    assert_eq!(r.pump_all(|_| {}), 0, "nothing to drain while idle");
    let reason = r.park();
    assert_eq!(reason, Some(WakeReason::NicMsix), "no deadline → parked forever, woken by NIC MSI-X");
    assert_eq!(r.parks(), 1);
}

// ---- the burst-safe drive: pump_all (salvaged from directpath::drive::drain) -------------------

#[test]
fn pump_all_busy_drains_a_backlog_with_zero_parks() {
    // The accepted §7 fix, tested where the code now lives: feed a K-frame backlog and drive with
    // `pump_all`; it drains the whole burst handing each delivery to `consume`, with ZERO parks (§3
    // busy-poll while frames are present); the caller parks exactly once, when genuinely idle.
    const K: usize = 32;
    let frames = (0..K).map(|i| inbound(1000 + i as u64, &[i as u8; 8])).collect::<Vec<_>>();
    let mut r =
        Reactor::new(SessionTable::new(), FakeHost::new([]), StubRx::new(frames), CaptureSink::default());

    let mut consumed = 0;
    let work = r.pump_all(|_| consumed += 1);

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
        Reactor::new(SessionTable::new(), FakeHost::new([]), StubRx::new(frames), CaptureSink::default());

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
