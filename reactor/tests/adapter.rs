//! Positive tests for the NEW glue: [`RxDrainAsSource`] (Sprint 6 Slice 3). This adapter is *not* frozen
//! and *not* covered by the reactor re-run tests, so it carries its own coverage: a fake [`RxDrain`]
//! yielding OWNED frames, driven through the adapter and (last test) through a real [`Reactor`]. The
//! load-bearing properties (reviewer's pre-build concerns): exact one-frame lookahead — **no dropped
//! frame, no double-pull, correct empty→None** — and **liveness** (a frame arriving after an empty poll
//! is still delivered, because `has_frame` re-pulls).

use reactor::{Reactor, RxDrain, RxDrainAsSource, RxSource, WakeReason};
use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;
use transport::{Instant, Originator, SessionTable, TransmitSink};

// ---- a fake RxDrain (owned frames) that counts its pulls -------------------------------------

/// A consume-only drain over a queue of owned frames, counting every `next_frame` call (via a shared
/// counter the test retains) so tests can assert the adapter never double-pulls. A queued `None` models
/// "empty right now" WITHOUT ending the stream — frames after it model later arrivals (liveness).
struct StubDrain {
    /// Each item is one `next_frame` result, in order (`Some(frame)` or a transient `None`).
    script: VecDeque<Option<Vec<u8>>>,
    pulls: Rc<Cell<u32>>,
}
impl StubDrain {
    fn new(script: impl IntoIterator<Item = Option<Vec<u8>>>) -> (Self, Rc<Cell<u32>>) {
        let pulls = Rc::new(Cell::new(0));
        (Self { script: script.into_iter().collect(), pulls: pulls.clone() }, pulls)
    }
}
impl RxDrain for StubDrain {
    fn next_frame(&mut self) -> Option<Vec<u8>> {
        self.pulls.set(self.pulls.get() + 1);
        // Once the script is exhausted, the drain is permanently empty.
        self.script.pop_front().flatten()
    }
}

// A minimal capture sink + inbound-frame builder (mirrors tests/reactor.rs).
#[derive(Default)]
struct CaptureSink {
    emitted: Vec<Vec<u8>>,
}
impl TransmitSink for CaptureSink {
    fn emit(&mut self, frame: &[u8]) {
        self.emitted.push(frame.to_vec());
    }
}
fn inbound(session: u64, payload: &[u8]) -> Vec<u8> {
    let mut f = session.to_be_bytes().to_vec();
    f.extend_from_slice(payload);
    f
}

// A Host that never needs to block in these tests (all work is synchronous RX).
struct NoWakeHost;
impl reactor::Host for NoWakeHost {
    fn now(&self) -> Instant {
        Instant(0)
    }
    fn block(&mut self, _timeout: Option<core::time::Duration>) -> WakeReason {
        WakeReason::NicMsix
    }
}

// ---- the adapter's contract ------------------------------------------------------------------

#[test]
fn lends_frames_in_order_then_empty_to_none() {
    let (drain, _pulls) = StubDrain::new([
        Some(b"one".to_vec()),
        Some(b"two".to_vec()),
        Some(b"three".to_vec()),
    ]);
    let mut src = RxDrainAsSource::new(drain);

    for expected in [b"one".as_slice(), b"two", b"three"] {
        assert!(src.has_frame(), "a frame is available");
        assert_eq!(src.next_frame(), Some(expected), "frames lent in order, none dropped");
    }
    assert!(!src.has_frame(), "drained → empty");
    assert_eq!(src.next_frame(), None, "empty → None");
}

#[test]
fn has_frame_is_idempotent_no_double_pull() {
    let (drain, pulls) = StubDrain::new([Some(b"a".to_vec()), Some(b"b".to_vec())]);
    let mut src = RxDrainAsSource::new(drain);

    // Repeated has_frame (as the reactor does under back-pressure) pulls the lookahead exactly ONCE —
    // the buffered frame is neither dropped nor re-pulled.
    assert!(src.has_frame());
    assert!(src.has_frame());
    assert!(src.has_frame());
    assert_eq!(pulls.get(), 1, "three peeks, ONE pull (no double-pull)");
    assert_eq!(src.next_frame(), Some(b"a".as_slice()), "the single buffered frame, intact");

    // The next lookahead pull happens only now (on the next peek), and again just once.
    assert!(src.has_frame());
    assert!(src.has_frame());
    assert_eq!(pulls.get(), 2, "second frame pulled exactly once");
    assert_eq!(src.next_frame(), Some(b"b".as_slice()));
}

#[test]
fn recovers_after_empty_poll_liveness() {
    // A yields, then the drain is momentarily empty (None), then B arrives later. The adapter MUST
    // deliver B — has_frame re-pulls after an empty poll rather than latching empty forever.
    let (drain, _pulls) = StubDrain::new([
        Some(b"A".to_vec()),
        None, // empty poll — nothing on the ring yet (the peek)
        None, // still empty on the immediately-following pull
        Some(b"B".to_vec()), // later arrival
    ]);
    let mut src = RxDrainAsSource::new(drain);

    assert!(src.has_frame());
    assert_eq!(src.next_frame(), Some(b"A".as_slice()));

    // The ring is momentarily empty → has_frame reports false (the reactor would park here) and a pull
    // in that empty window also yields None; no frame is invented.
    assert!(!src.has_frame(), "empty poll → no frame");
    assert_eq!(src.next_frame(), None);

    // B has now arrived; the reactor wakes and re-checks — the adapter re-pulls and delivers it.
    assert!(src.has_frame(), "liveness: a frame that arrived after the empty poll is seen");
    assert_eq!(src.next_frame(), Some(b"B".as_slice()));
}

#[test]
fn real_reactor_consumes_and_back_pressures_through_the_adapter() {
    // The end-to-end shape: a real Reactor driven by the adapter over owned driver frames. Two inbound
    // frames; the first delivery latches back-pressure (RDT held) so the second is NOT pulled until the
    // delivery is consumed — proving the adapter's has_frame/next_frame drive the reactor's flow control
    // correctly (no drop, no double-pull) end-to-end.
    let (drain, _pulls) = StubDrain::new([Some(inbound(1, b"a")), Some(inbound(2, b"b"))]);
    let src = RxDrainAsSource::new(drain);
    let mut r = Reactor::new(SessionTable::new(Originator::Low), NoWakeHost, src, CaptureSink::default());

    assert!(r.step(), "frame 1 processed");
    assert!(r.back_pressured(), "a delivery is in flight → back-pressure engaged");
    assert!(!r.step(), "frame 2 is HELD in the lookahead while a delivery is in flight");
    assert_eq!(r.take_delivery().map(|(_, p)| p).as_deref(), Some(b"a".as_slice()), "frame 1's payload delivered");
    assert!(!r.back_pressured());
    assert!(r.step(), "frame 2 flows once back-pressure releases");
    assert_eq!(r.take_delivery().map(|(_, p)| p).as_deref(), Some(b"b".as_slice()), "frame 2's payload delivered, in order");
}
