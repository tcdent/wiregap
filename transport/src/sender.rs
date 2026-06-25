//! The sender role: strict **stop-and-wait** + **retransmit-on-silence** + **timeout-only give-up**
//! (`architecture.md` §6, §7; reclaim ruling R5 — timeout-only, no consumer-driven completion).
//!
//! A [`Sender`] is the per-session state for a session **this endpoint opened** (the box is the sole
//! opener/allocator, §1.1). It holds **at most one** frame in flight — `in_flight: Option<InFlight>`,
//! a single slot, **not a queue**, so `≤1` is re-derivable from the *type*, not just a test (gate 4).
//!
//! - **Send / back-pressure.** [`Sender::try_send`] emits and arms a retransmit only if the slot is
//!   empty; offered a second payload while one is in flight it returns [`SendError::WouldBlock`] and
//!   emits **no** second wire frame (no unbounded internal queue).
//! - **Retained buffer = app-owned bytes.** The in-flight frame is held as `Rc<[u8]>` (plain `alloc`
//!   heap, **not** a DMA buffer — the DMA copy lives behind the [`TransmitSink`] in E3). The refcount is
//!   the lifetime *observable* gate 4 asks for: held while the sender owns it, freed (last `Rc` dropped)
//!   exactly on ACK or give-up, and the **same allocation** re-emitted across a retransmit.
//! - **ACK.** [`Sender::on_ack`] releases the slot (drops the `Rc`) and advances — stop-and-wait can
//!   send again.
//! - **Tick (reactor-driven).** [`Sender::tick`] checks **give-up first** (timeout-only): past the
//!   give-up bound with a frame still in flight it frees the buffer and reports [`Tick::GaveUp`] so the
//!   table removes the session — emitting **nothing**. Otherwise, if the retransmit deadline has passed,
//!   it re-emits the **same** buffer and re-arms. A **dormant** sender (nothing in flight) is
//!   [`Tick::Idle`] — **never** reclaimed (give-up fires only on an outstanding frame).

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::time::Duration;

use crate::frame::{SessionId, encode_frame};
use crate::seam::{Instant, TransmitSink};

/// Provisional retransmit interval (~10 ms, `architecture.md` §7) — **dialable**; the good band is wide
/// (hundreds of µs to tens of ms). Marked provisional pending a bench measurement (§7 "Bench-gated").
pub const DEFAULT_RETRANSMIT_INTERVAL: Duration = Duration::from_millis(10);

/// Provisional give-up bound (~1–5 s, `architecture.md` §7) — **dialable**; reclaiming is destructive,
/// so it is generous (a µs-RTT peer silent for seconds is genuinely gone). Provisional pending review.
pub const DEFAULT_GIVE_UP_AFTER: Duration = Duration::from_secs(2);

/// Why a [`Sender::try_send`] / table `send` did not emit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SendError {
    /// Back-pressure: a frame is already in flight on this session (stop-and-wait, ≤1). **No** wire
    /// frame was emitted; the caller holds the payload (no unbounded internal queue).
    WouldBlock,
    /// No live session with that id.
    NoSuchSession,
    /// The session exists but this endpoint is not its sender (it is a receiver session).
    NotSender,
}

/// The single in-flight slot's contents.
#[derive(Debug)]
struct InFlight {
    /// The retained, app-owned encoded frame (`[id BE][payload]`), re-emitted verbatim on retransmit.
    frame: Rc<[u8]>,
    /// When the frame first went out — the anchor for the give-up bound (§7: "no ACK for the window").
    armed_at: Instant,
    /// The next retransmit deadline (advanced by one interval each retransmit).
    next_retransmit: Instant,
}

/// Per-session sender state. `in_flight` is a single slot (**not** a queue) — `≤1` is structural.
#[derive(Debug)]
pub struct Sender {
    session: SessionId,
    interval: Duration,
    give_up_after: Duration,
    in_flight: Option<InFlight>,
}

/// The result of a [`Sender::tick`], telling the table what to do with the session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tick {
    /// Dormant — nothing in flight. Never reclaimed (give-up is timeout-only on an outstanding frame).
    Idle,
    /// In flight; a retransmit was emitted if due. Keep the session.
    Armed,
    /// Past the give-up bound with a frame outstanding — buffer freed; the table must remove the session.
    GaveUp,
}

impl Sender {
    pub(crate) fn new(session: SessionId, interval: Duration, give_up_after: Duration) -> Self {
        Self {
            session,
            interval,
            give_up_after,
            in_flight: None,
        }
    }

    /// Encode, emit, retain, and arm — the first frame (open-by-emission) or a fresh send. Caller
    /// guarantees the slot is empty (the table's `open` on a new sender, or [`Sender::try_send`] after
    /// its emptiness check).
    pub(crate) fn arm_and_emit<S: TransmitSink>(
        &mut self,
        payload: &[u8],
        now: Instant,
        sink: &mut S,
    ) {
        let mut buf = Vec::new();
        encode_frame(self.session, payload, &mut buf);
        let frame: Rc<[u8]> = Rc::from(buf);
        sink.emit(&frame);
        self.in_flight = Some(InFlight {
            frame,
            armed_at: now,
            next_retransmit: now.saturating_add(self.interval),
        });
    }

    /// Send a payload if the slot is empty; otherwise **back-pressure** ([`SendError::WouldBlock`]) and
    /// emit nothing.
    pub(crate) fn try_send<S: TransmitSink>(
        &mut self,
        payload: &[u8],
        now: Instant,
        sink: &mut S,
    ) -> Result<(), SendError> {
        if self.in_flight.is_some() {
            return Err(SendError::WouldBlock);
        }
        self.arm_and_emit(payload, now, sink);
        Ok(())
    }

    /// ACK arrived: release the retained buffer (drops the `Rc`) and advance. Idempotent if nothing is
    /// in flight (a stray/duplicate ACK on an idle sender is harmless — no flag, role-disambiguated §6).
    pub(crate) fn on_ack(&mut self) {
        self.in_flight = None;
    }

    /// Reactor tick at `now`. **Give-up checked first** (timeout-only): if a frame is in flight past the
    /// give-up bound, free it and report [`Tick::GaveUp`], emitting nothing. Else, if the retransmit
    /// deadline has passed, re-emit the **same** buffer and re-arm. Dormant → [`Tick::Idle`].
    pub(crate) fn tick<S: TransmitSink>(&mut self, now: Instant, sink: &mut S) -> Tick {
        let Some(inf) = self.in_flight.as_mut() else {
            return Tick::Idle; // dormant: never give up — no outstanding frame
        };
        if now >= inf.armed_at.saturating_add(self.give_up_after) {
            self.in_flight = None; // free buffer; emit NOTHING (silent reclaim, §7/§8)
            return Tick::GaveUp;
        }
        if now >= inf.next_retransmit {
            sink.emit(&inf.frame); // SAME buffer re-emitted (not freed+realloc)
            inf.next_retransmit = inf.next_retransmit.saturating_add(self.interval);
        }
        Tick::Armed
    }

    /// The earliest armed deadline for this sender (its next retransmit), or `None` when dormant — the
    /// reactor seam composes the table-wide minimum into `epoll_wait`.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.in_flight.as_ref().map(|inf| inf.next_retransmit)
    }

    /// Whether a frame is currently in flight — i.e. the session is in the ready-set / awaiting an ACK.
    pub(crate) fn has_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Test-only: a weak handle to the in-flight buffer, for lifetime observation (held/freed via
    /// `upgrade`, same-allocation via `Rc::as_ptr`). Never compiled into the audited library.
    #[cfg(test)]
    pub(crate) fn in_flight_weak(&self) -> Option<alloc::rc::Weak<[u8]>> {
        self.in_flight.as_ref().map(|inf| Rc::downgrade(&inf.frame))
    }
}
