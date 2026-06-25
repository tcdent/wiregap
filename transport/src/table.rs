//! The session table + open-by-emission + inbound dispatch + the sender role's table integration
//! (`architecture.md` §4, §6, §7, §8).
//!
//! An ID→session map. The inbound entry decodes a frame (Slice 1 codec) and dispatches it:
//!
//! - **Sender session** (this endpoint opened it): an arriving frame is an **ACK** (§6, unidirectional data) — release the in-flight buffer, deliver nothing up (`Ok(None)`).
//! - **Receiver session, known**: route + deliver the payload up.
//! - **Unknown id**: **open-by-emission** — instantiate a receiver session + deliver up.
//!
//! The sender/receiver split is a **role branch** (§6 decides data-vs-ACK by role) — *not* a
//! protocol-type branch; gate 3's "zero protocol-type branching at dispatch" is preserved (the only
//! branch is role, never "what protocol/application is spoken"). "Opened" is not announced; it is
//! **noticed** by the consumer (§8) — open and route return an identical upward shape.
//!
//! **No retired-ID memory.** The table holds only live sessions; a new epoch is a fresh table. A
//! give-up removes its session (positive reclaim); a reused id is then a new session by emission (§8).
//!
//! The **sender role** itself ([`crate::sender::Sender`]) is the per-session stop-and-wait,
//! retransmit, and timeout-only give-up. This module wires it to the table:
//! [`SessionTable::open`]/[`send`](SessionTable::send) drive it, [`poll`](SessionTable::poll) advances
//! retransmits and **removes** gave-up sessions, and [`next_deadline`](SessionTable::next_deadline)
//! exposes the earliest armed retransmit for the reactor. The **receiver role** (delivering up and
//! emitting bare ACKs) and the ready-set / closed-loop harness are Slice 4.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::time::Duration;

use crate::allocator::IdAllocator;
use crate::frame::{Frame, FrameError, SessionId, decode_frame, encode_frame};
use crate::seam::{Instant, TransmitSink};
use crate::sender::{DEFAULT_GIVE_UP_AFTER, DEFAULT_RETRANSMIT_INTERVAL, SendError, Sender, Tick};

/// Per-session state. `sender == Some` ⟺ this endpoint is the **sender** (it opened the session);
/// `None` ⟺ a **receiver** session (open-by-emission). The role decides data-vs-ACK on inbound (§6).
/// Slice 4 fills the receiver half (emitting bare ACKs); here a receiver session is just its existence.
#[derive(Debug, Default)]
pub struct Session {
    sender: Option<Sender>,
}

/// One upward delivery — the result of processing an inbound frame (the **fourth boundary**: a *return
/// value*, not a seam). Carries the session and the opaque payload (proto's L2), **borrowed** from the
/// inbound buffer (zero-copy; the reactor owns that buffer per the copy boundary, §3.1). No "opened"
/// flag — open-by-emission and route-to-existing are indistinguishable; the consumer notices openness.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Delivery<'a> {
    /// The session this delivery is keyed to.
    pub session: SessionId,
    /// The opaque payload (L2), borrowed from the inbound buffer. Empty for a bare-ACK-shaped frame.
    pub payload: &'a [u8],
}

/// The ID→session table, the opener's allocator, and the sender-role timing. Holds **only live
/// sessions** — there is no retired-ID set (no cross-epoch state).
#[derive(Debug)]
pub struct SessionTable {
    sessions: BTreeMap<SessionId, Session>,
    allocator: IdAllocator,
    retransmit_interval: Duration,
    give_up_after: Duration,
}

impl SessionTable {
    /// A fresh table for a new epoch, with the provisional production timing (§7, dialable).
    pub fn new() -> Self {
        Self::with_timing(DEFAULT_RETRANSMIT_INTERVAL, DEFAULT_GIVE_UP_AFTER)
    }

    /// A fresh table with explicit sender timing (for dialing the §7 knobs, and for deterministic tests).
    pub fn with_timing(retransmit_interval: Duration, give_up_after: Duration) -> Self {
        Self {
            sessions: BTreeMap::new(),
            allocator: IdAllocator::new(),
            retransmit_interval,
            give_up_after,
        }
    }

    /// Open a session by emission: allocate a fresh id, create its sender, and emit the first frame
    /// (arming the retransmit). Returns the new id.
    pub fn open<S: TransmitSink>(&mut self, payload: &[u8], now: Instant, sink: &mut S) -> SessionId {
        let id = self.allocator.allocate();
        let mut sender = Sender::new(id, self.retransmit_interval, self.give_up_after);
        sender.arm_and_emit(payload, now, sink);
        self.sessions.insert(id, Session { sender: Some(sender) });
        id
    }

    /// Send a subsequent frame on a sender session. Stop-and-wait: if a frame is already in flight, this
    /// **back-pressures** ([`SendError::WouldBlock`]) and emits nothing.
    pub fn send<S: TransmitSink>(
        &mut self,
        id: SessionId,
        payload: &[u8],
        now: Instant,
        sink: &mut S,
    ) -> Result<(), SendError> {
        let sess = self.sessions.get_mut(&id).ok_or(SendError::NoSuchSession)?;
        let sender = sess.sender.as_mut().ok_or(SendError::NotSender)?;
        sender.try_send(payload, now, sink)
    }

    /// The **inbound-frame entry**. Decodes one frame and dispatches by role (the only branch — a §6
    /// role branch, not a protocol-type branch). Returns `Err` only on a structurally short frame
    /// (totality from the codec), and then touches no session.
    ///
    /// - **Sender session** → the frame is an **ACK**: release the in-flight buffer, deliver nothing up,
    ///   emit nothing.
    /// - **Receiver session** (known, or unknown ⇒ open-by-emission) → emit a **bare ACK** back via the
    ///   sink (the receiver role, §6) and deliver the payload up.
    pub fn on_inbound<'a, S: TransmitSink>(
        &mut self,
        frame: &'a [u8],
        sink: &mut S,
    ) -> Result<Option<Delivery<'a>>, FrameError> {
        let Frame { session, payload } = decode_frame(frame)?;
        // Sender session ⇒ the arriving frame is an ACK (§6): release the buffer, nothing up, no emit.
        if let Some(sender) = self.sessions.get_mut(&session).and_then(|s| s.sender.as_mut()) {
            sender.on_ack();
            return Ok(None);
        }
        // Receiver path (known receiver, or unknown ⇒ open-by-emission): ensure the session exists,
        // emit a bare ACK (the receiver role), and deliver the payload up. The role split is a §6 role
        // branch, never a protocol-type branch — gate 3 preserved.
        self.sessions.entry(session).or_default();
        emit_bare_ack(session, sink);
        Ok(Some(Delivery { session, payload }))
    }

    /// Reactor-driven timing. Advance every sender to `now`: re-emit due retransmits, and **remove** any
    /// session that gave up (timeout-only reclaim — buffer already freed by the sender, session dropped
    /// here; the give-up emits nothing on the wire). A dormant sender is left untouched.
    pub fn poll<S: TransmitSink>(&mut self, now: Instant, sink: &mut S) {
        let mut reclaim: Vec<SessionId> = Vec::new();
        for (id, sess) in self.sessions.iter_mut() {
            if let Some(sender) = sess.sender.as_mut()
                && sender.tick(now, sink) == Tick::GaveUp
            {
                reclaim.push(*id);
            }
        }
        for id in reclaim {
            self.sessions.remove(&id); // positive give-up observable: session REMOVED from the table
        }
    }

    /// The earliest armed retransmit deadline across all sender sessions (the reactor seam composes this
    /// into `epoll_wait`). `None` when no sender has a frame in flight.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.sessions
            .values()
            .filter_map(|s| s.sender.as_ref().and_then(Sender::next_deadline))
            .min()
    }

    /// The **ready-set**: the sessions that currently have a frame to send (one in flight, stop-and-wait).
    /// The reactor drains it **trivially** — this is a plain filter over the map, with **no scheduler,
    /// no priority/QoS/fairness** (the gate-5 structural property, re-derivable here and at
    /// [`poll`](Self::poll), which is likewise a plain iteration).
    pub fn ready_set(&self) -> impl Iterator<Item = SessionId> + '_ {
        self.sessions
            .iter()
            .filter(|(_, s)| s.sender.as_ref().is_some_and(Sender::has_in_flight))
            .map(|(id, _)| *id)
    }

    /// Live-session count (inspection / test).
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Whether the table holds no live sessions.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Whether `id` names a live session.
    pub fn contains(&self, id: SessionId) -> bool {
        self.sessions.contains_key(&id)
    }

    /// Test-only: a weak handle to a sender session's in-flight buffer, for lifetime observation.
    #[cfg(test)]
    fn in_flight_weak(&self, id: SessionId) -> Option<alloc::rc::Weak<[u8]>> {
        self.sessions.get(&id)?.sender.as_ref()?.in_flight_weak()
    }
}

impl Default for SessionTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Emit a **bare ACK** — the session id with an empty payload, exactly [`crate::SESSION_ID_LEN`] bytes
/// (§6) — via the transmit sink. Byte-identical to an empty-payload data frame; role/direction
/// disambiguates at the peer (the sender, for whom an arriving frame is an ACK).
fn emit_bare_ack<S: TransmitSink>(session: SessionId, sink: &mut S) {
    let mut ack = Vec::new();
    encode_frame(session, &[], &mut ack);
    sink.emit(&ack);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::encode_frame;
    use alloc::rc::Rc;
    use alloc::vec::Vec;

    // ---- test wire (records emitted frames verbatim) -------------------------------------------

    #[derive(Default)]
    struct FakeWire {
        sent: Vec<Vec<u8>>,
    }
    impl TransmitSink for FakeWire {
        fn emit(&mut self, frame: &[u8]) {
            self.sent.push(frame.to_vec());
        }
    }

    /// Build an L1 frame for `(id, payload)` via the Slice 1 codec (e.g. a bare ACK = `frame(id, &[])`).
    fn frame(id: u64, payload: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        encode_frame(SessionId(id), payload, &mut b);
        b
    }

    /// Tiny exact timing for deterministic virtual-clock assertions: interval 10 ns, give-up 100 ns.
    fn table() -> SessionTable {
        SessionTable::with_timing(Duration::from_nanos(10), Duration::from_nanos(100))
    }

    // ---- S2 behaviors (unchanged; receiver path) ----------------------------------------------

    #[test]
    fn open_by_emission_instantiates_and_delivers() {
        let mut t = table();
        let mut w = FakeWire::default();
        assert!(t.is_empty());
        let f = frame(5, &[1, 2, 3]);
        let d = t.on_inbound(&f, &mut w).unwrap().unwrap();
        assert_eq!(d.session, SessionId(5));
        assert_eq!(d.payload, &[1, 2, 3]);
        assert!(t.contains(SessionId(5)));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn known_id_routes_without_duplicating() {
        let mut t = table();
        let mut w = FakeWire::default();
        let f1 = frame(5, &[0xAA]);
        let f2 = frame(5, &[0xBB, 0xCC]);
        let d1 = t.on_inbound(&f1, &mut w).unwrap().unwrap();
        let d2 = t.on_inbound(&f2, &mut w).unwrap().unwrap();
        assert_eq!(d1.payload, &[0xAA]);
        assert_eq!(d2.payload, &[0xBB, 0xCC]);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn open_and_route_have_identical_upward_shape() {
        let mut t = table();
        let mut w = FakeWire::default();
        let f = frame(9, &[7, 7]);
        let open = t.on_inbound(&f, &mut w).unwrap().unwrap();
        let route = t.on_inbound(&f, &mut w).unwrap().unwrap();
        assert_eq!(open, route);
    }

    #[test]
    fn epoch_reuse_is_a_new_session() {
        let id = 7;
        let mut w = FakeWire::default();
        let mut epoch_a = table();
        let fa = frame(id, &[0x01]);
        epoch_a.on_inbound(&fa, &mut w).unwrap();
        assert!(epoch_a.contains(SessionId(id)));

        let mut epoch_b = table();
        assert!(!epoch_b.contains(SessionId(id)));
        let fb = frame(id, &[0x02]);
        let d = epoch_b.on_inbound(&fb, &mut w).unwrap().unwrap();
        assert_eq!(d.session, SessionId(id));
        assert_eq!(d.payload, &[0x02]);
        assert_eq!(epoch_b.len(), 1);
    }

    #[test]
    fn short_frame_rejects_and_instantiates_nothing() {
        let mut t = table();
        let mut w = FakeWire::default();
        let short = [0u8; 7];
        assert_eq!(
            t.on_inbound(&short, &mut w),
            Err(FrameError::ShortFrame { needed: 8, got: 7 })
        );
        assert!(t.is_empty());
    }

    // ---- S3 sender role (gate 4) --------------------------------------------------------------

    /// Back-pressure: a second payload offered while one is in flight is refused and emits NO 2nd frame.
    #[test]
    fn back_pressure_emits_no_second_frame() {
        let mut t = table();
        let mut w = FakeWire::default();
        let id = t.open(&[0xAA], Instant(0), &mut w);
        assert_eq!(w.sent.len(), 1, "open emits the first frame");

        let r = t.send(id, &[0xBB], Instant(1), &mut w);
        assert_eq!(r, Err(SendError::WouldBlock), "stop-and-wait back-pressures");
        assert_eq!(w.sent.len(), 1, "no second wire frame while one is in flight");
    }

    /// ACK releases the buffer and advances: after the ACK a fresh send is allowed and emits.
    #[test]
    fn ack_releases_and_allows_next_send() {
        let mut t = table();
        let mut w = FakeWire::default();
        let id = t.open(&[0xAA], Instant(0), &mut w);

        // ACK = a frame arriving on our sender session (role-disambiguated, §6).
        let ack = frame(id.0, &[]);
        assert_eq!(t.on_inbound(&ack, &mut w).unwrap(), None, "ACK delivers nothing up");

        let r = t.send(id, &[0xBB], Instant(1), &mut w);
        assert_eq!(r, Ok(()), "after ACK, stop-and-wait may send again");
        assert_eq!(w.sent.len(), 2);
    }

    /// Buffer held across a retransmit: the SAME allocation is re-emitted (not freed+realloc).
    #[test]
    fn buffer_held_across_retransmit_same_allocation() {
        let mut t = table();
        let mut w = FakeWire::default();
        let id = t.open(&[1, 2, 3], Instant(0), &mut w);

        let weak = t.in_flight_weak(id).unwrap();
        let ptr_before = Rc::as_ptr(&weak.upgrade().unwrap());

        // Retransmit fires at the 10 ns interval.
        t.poll(Instant(10), &mut w);
        assert_eq!(w.sent.len(), 2, "retransmit re-emits");

        let ptr_after = Rc::as_ptr(&t.in_flight_weak(id).unwrap().upgrade().unwrap());
        assert_eq!(ptr_before, ptr_after, "same buffer re-emitted, not freed+realloc");
        assert!(weak.upgrade().is_some(), "buffer still held across retransmit");
    }

    /// Not freed on an unrelated event: an inbound frame on a DIFFERENT session leaves this one's buffer.
    #[test]
    fn buffer_not_freed_on_unrelated_event() {
        let mut t = table();
        let mut w = FakeWire::default();
        let a = t.open(&[0xAA], Instant(0), &mut w);
        let weak_a = t.in_flight_weak(a).unwrap();

        // An inbound frame on some other (peer-opened) session — open-by-emission, unrelated to `a`.
        let other = frame(a.0.wrapping_add(1_000_000), &[0x01]);
        t.on_inbound(&other, &mut w).unwrap();

        assert!(weak_a.upgrade().is_some(), "unrelated inbound must not free A's buffer");
    }

    /// Freed on ACK.
    #[test]
    fn buffer_freed_on_ack() {
        let mut t = table();
        let mut w = FakeWire::default();
        let id = t.open(&[0xAA], Instant(0), &mut w);
        let weak = t.in_flight_weak(id).unwrap();
        assert!(weak.upgrade().is_some());

        t.on_inbound(&frame(id.0, &[]), &mut w).unwrap(); // ACK
        assert!(weak.upgrade().is_none(), "ACK frees the retained buffer");
    }

    /// Freed on give-up.
    #[test]
    fn buffer_freed_on_give_up() {
        let mut t = table();
        let mut w = FakeWire::default();
        let id = t.open(&[0xAA], Instant(0), &mut w);
        let weak = t.in_flight_weak(id).unwrap();

        t.poll(Instant(100), &mut w); // give-up bound = 100 ns
        assert!(weak.upgrade().is_none(), "give-up frees the retained buffer");
    }

    /// Retransmit fires AT the configured interval — exact virtual-clock boundaries.
    #[test]
    fn retransmit_fires_at_interval_exactly() {
        let mut t = table();
        let mut w = FakeWire::default();
        let _id = t.open(&[0xAA], Instant(0), &mut w);
        assert_eq!(w.sent.len(), 1);

        t.poll(Instant(9), &mut w); // one ns before the deadline
        assert_eq!(w.sent.len(), 1, "no retransmit before the interval");

        t.poll(Instant(10), &mut w); // exactly at the deadline
        assert_eq!(w.sent.len(), 2, "retransmit at the interval");

        t.poll(Instant(20), &mut w); // next interval
        assert_eq!(w.sent.len(), 3, "retransmit again one interval later");
    }

    /// next-deadline is the earliest armed retransmit; `None` once dormant.
    #[test]
    fn next_deadline_is_earliest_armed_retransmit() {
        let mut t = table();
        let mut w = FakeWire::default();
        let id = t.open(&[0xAA], Instant(0), &mut w);
        assert_eq!(t.next_deadline(), Some(Instant(10)), "armed at open + interval");

        t.on_inbound(&frame(id.0, &[]), &mut w).unwrap(); // ACK -> dormant
        assert_eq!(t.next_deadline(), None, "dormant: nothing armed");
    }

    /// Give-up = timeout-only positive reclaim: buffer freed AND session removed from the table, AND the
    /// give-up emits NOTHING (silence is necessary, not sufficient — the removal is the positive proof).
    #[test]
    fn give_up_reclaims_positively_and_emits_nothing() {
        let mut t = table();
        let mut w = FakeWire::default();
        let id = t.open(&[0xAA], Instant(0), &mut w);
        assert!(t.contains(id));
        let emitted_before = w.sent.len(); // 1 (the open frame)
        let weak = t.in_flight_weak(id).unwrap();

        // Poll straight at the give-up bound: tick checks give-up FIRST, so it emits nothing.
        t.poll(Instant(100), &mut w);

        assert!(!t.contains(id), "give-up REMOVES the session from the table (positive)");
        assert_eq!(t.len(), 0);
        assert!(weak.upgrade().is_none(), "give-up frees the buffer (positive)");
        assert_eq!(w.sent.len(), emitted_before, "give-up emits nothing on the wire");
    }

    /// Dormant ≠ dead: a session with nothing in flight is NEVER reclaimed, even far past the give-up
    /// bound (give-up is timeout-only on an outstanding frame; no consumer-driven completion — R5).
    #[test]
    fn dormant_session_survives_past_give_up_bound() {
        let mut t = table();
        let mut w = FakeWire::default();
        let id = t.open(&[0xAA], Instant(0), &mut w);
        t.on_inbound(&frame(id.0, &[]), &mut w).unwrap(); // ACK -> dormant (nothing in flight)

        t.poll(Instant(100_000), &mut w); // far past the 100 ns give-up bound
        assert!(t.contains(id), "a dormant session is never reclaimed");
    }
}
