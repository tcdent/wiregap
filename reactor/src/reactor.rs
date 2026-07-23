//! The single-threaded event reactor — the **ratified behavior built fresh**, `no_std`, NOT a port of the
//! std/epoll reactor. It drives the reused `transport` (unchanged) over a **minimal two-method seam**
//! ([`Host`]: `now` + `block`) and a frame source ([`RxSource`]) + sink (`transport::TransmitSink`). The
//! ratified behavior, re-expressed:
//!
//! - **busy-drain-then-block ordering** — [`Reactor::step`] serves one unit of work; the driver busy-drains
//!   all ready work ([`Reactor::pump_all`]) and only then [`Reactor::park`]s. Zero parks while there is work.
//! - **deadline composition** — [`arm_from_deadline`]: the *three edge-facts only*, re-typed to the
//!   one-shot timer arm (`Option<Duration>`), NOT the epoll `i32`-ms shape.
//! - **sole-sink-holder** — the reactor uniquely owns the `TransmitSink`; the transport emits only
//!   through it.
//! - **back-pressure as RX flow control** — [`can_pull_rx`]: hold the RX ring (do not pull / advance
//!   RDT) while a delivery is in flight to the consumer (stop-and-wait: one in flight). NOT the old
//!   app-fd `EPOLLONESHOT`.
//!
//! The loop is **pure** (unsafe-free, generic over the seam), so the decision logic is host-tested
//! (`tests/reactor.rs`); the substrate side (the wait mechanism) is confined entirely in the [`Host`]
//! *impl* — the live one is the pure-direct `DdWaitHost` (in `directpath`): **MONITORX/MWAITX on the RX
//! descriptor `DD` line** (event-driven, no kernel interrupt), NOT a bare-metal LAPIC/`hlt`/MSI-X substrate.

use alloc::vec::Vec;
use core::time::Duration;
use transport::{Delivery, Instant, SendError, SessionId, SessionTable, TransmitSink};

/// Why the reactor woke from a park — the **woke-by class** (the clean re-expression of the committed
/// eventfd/token classes; app-fds are gone).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WakeReason {
    /// A NIC-driven wake — the RX descriptor `DD` store the live `DdWaitHost` monitors (MONITORX/MWAITX);
    /// the woke-by-NIC class (the name is retained from the eventfd/MSI-X era).
    NicMsix,
    /// The deadline timer elapsed (a retransmit deadline; the `DdWaitHost`'s bounded MWAITX/TSC wait).
    Timer,
}

/// The block decision from deadline composition — the three edge-facts, re-typed to the one-shot arm.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arm {
    /// A deadline is already due → do **not** park; loop back and serve it.
    DueNow,
    /// No deadline armed → park until a wakeup (no timer).
    ParkForever,
    /// A future deadline → arm the one-shot timer for this long, then park.
    ParkUntil(Duration),
}

/// Deadline composition (pure). Replaces the committed `compute_timeout`'s `i32`-ms output with the
/// one-shot-timer arm. The three — and only three — edges.
pub fn arm_from_deadline(deadline: Option<Instant>, now: Instant) -> Arm {
    match deadline {
        None => Arm::ParkForever,
        Some(d) if d.0 <= now.0 => Arm::DueNow,
        Some(d) => Arm::ParkUntil(Duration::from_nanos(d.0 - now.0)),
    }
}

/// Compose the retransmit `next` deadline with an optional idle-park **cap** into the effective park
/// deadline (pure). The cap is a park CEILING — `now + cap` — and the effective deadline is the **earlier**
/// of the two: `min(next, now + cap)`. `cap == None` returns `next` unchanged (today's uncapped behavior,
/// byte-identical); `next == None` returns the cap alone. This feeds the existing [`arm_from_deadline`] path
/// — it is NOT a poll: with no NIC wake the park still returns at the cap, one park, via the one-shot timer.
pub fn capped_deadline(next: Option<Instant>, cap: Option<Duration>, now: Instant) -> Option<Instant> {
    let cap_deadline = cap.map(|d| Instant(now.0.saturating_add(d.as_nanos() as u64)));
    match (next, cap_deadline) {
        (Some(a), Some(b)) => Some(Instant(a.0.min(b.0))),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// RX back-pressure (pure): pull the next inbound frame only when a frame is available **and** no
/// delivery is already in flight to the consumer. While a delivery is held, the RX ring is not advanced
/// (RDT held) — the hardware back-pressures. Stop-and-wait: at most one in flight.
pub const fn can_pull_rx(frame_available: bool, delivery_in_flight: bool) -> bool {
    frame_available && !delivery_in_flight
}

/// The minimal NIC-shaped substrate seam: a monotonic clock + a park-until-wakeup-or-deadline. The live
/// pure-direct impl (`DdWaitHost`: MONITORX/MWAITX on the RX descriptor `DD` line) lives behind this; a
/// fake impl drives the host tests.
pub trait Host {
    /// Monotonic now.
    fn now(&self) -> Instant;
    /// Park until a NIC wake (the DD store, [`WakeReason::NicMsix`]) or, when `timeout` is `Some`, the
    /// one-shot timer fires — whichever first. Returns which woke it. Must not lose a wakeup that arrived
    /// just before the call.
    fn block(&mut self, timeout: Option<Duration>) -> WakeReason;
}

/// A source of inbound frames (the driver's RX drain in production; a stub in the host tests). The returned
/// slice is borrowed from the source's own buffer; the reactor copies it out at the boundary.
pub trait RxSource {
    /// Whether a frame is available — a non-consuming check (the real driver reads the ring DD bit), so
    /// back-pressure can gate the pull without dropping a frame.
    fn has_frame(&self) -> bool;
    /// The next available inbound frame, or `None` when the ring is empty.
    fn next_frame(&mut self) -> Option<&[u8]>;
}

/// The reactor: owns the transport, the sole sink, the RX source, and the substrate seam. Generic and
/// unsafe-free.
pub struct Reactor<H, R, S> {
    transport: SessionTable,
    host: H,
    rx: R,
    sink: S,
    /// The one in-flight delivery to the consumer: the transport-computed `SessionId` it is keyed to plus
    /// the payload copied out of the RX buffer (the copy boundary). While `Some`, RX is back-pressured.
    pending: Option<(SessionId, Vec<u8>)>,
    parks: u32,
    malformed: u32,
    /// Optional idle-park CEILING (see [`capped_deadline`]). `None` (the default) = uncapped, today's
    /// behavior byte-identical. `Some(d)` bounds each park to at most `d` so the loop wakes to run its own
    /// Idle logic even with no NIC event — a park ceiling, NOT the retired bounded-poll.
    idle_cap: Option<Duration>,
}

impl<H: Host, R: RxSource, S: TransmitSink> Reactor<H, R, S> {
    pub fn new(transport: SessionTable, host: H, rx: R, sink: S) -> Self {
        Self { transport, host, rx, sink, pending: None, parks: 0, malformed: 0, idle_cap: None }
    }

    /// Set the idle-park cap (a builder; default `None` = uncapped). `Some(d)` bounds each park to at most
    /// `d` — the effective park deadline becomes `min(next_deadline, now + d)` (see [`capped_deadline`]).
    pub fn idle_park_cap(mut self, cap: Option<Duration>) -> Self {
        self.idle_cap = cap;
        self
    }

    /// Originate a session (the app-above role): open-by-emission through the sole sink.
    pub fn originate(&mut self, payload: &[u8]) -> SessionId {
        let now = self.host.now();
        self.transport.open(payload, now, &mut self.sink)
    }

    /// Continue an existing session `id` with another frame (multi-frame streaming) through the sole sink.
    /// Additive wrapper over the transport's existing `send`; `SendError::NotSender` rejects an `id` this
    /// endpoint did not open, so this can only stream on a session it originated — never data on a received
    /// one. (`WouldBlock` if a frame is already in flight on `id`: stop-and-wait, ≤1.)
    pub fn send_on_session(&mut self, id: SessionId, payload: &[u8]) -> Result<(), SendError> {
        let now = self.host.now();
        self.transport.send(id, payload, now, &mut self.sink)
    }

    /// Do one unit of work. Returns `true` if it did work (an inbound frame processed, or a due
    /// retransmit driven), `false` if idle (the caller should [`park`](Self::park)).
    pub fn step(&mut self) -> bool {
        let now = self.host.now();

        // 1. RX, gated by back-pressure.
        if can_pull_rx(self.rx.has_frame(), self.pending.is_some()) {
            // Reborrow the frame and process it. (Distinct fields → distinct borrows.)
            let frame = self.rx.next_frame().expect("rx_available was true");
            match self.transport.on_inbound(frame, &mut self.sink) {
                Ok(Some(Delivery { session, payload })) => self.pending = Some((session, payload.to_vec())),
                Ok(None) => {} // an ACK: released in-flight, nothing up
                Err(_) => self.malformed += 1, // short frame: dropped, loop continues (totality)
            }
            return true;
        }

        // 2. Drive a due retransmit.
        if let Some(deadline) = self.transport.next_deadline()
            && deadline.0 <= now.0
        {
            self.transport.poll(now, &mut self.sink);
            return true;
        }

        false
    }

    /// If idle, park per the deadline composition. Returns the wake reason, or `None` when a deadline is
    /// already due (no park — the caller re-drains). Only an actual park increments the park counter.
    pub fn park(&mut self) -> Option<WakeReason> {
        let now = self.host.now();
        let deadline = capped_deadline(self.transport.next_deadline(), self.idle_cap, now);
        match arm_from_deadline(deadline, now) {
            Arm::DueNow => None,
            Arm::ParkForever => {
                self.parks += 1;
                Some(self.host.block(None))
            }
            Arm::ParkUntil(d) => {
                self.parks += 1;
                Some(self.host.block(Some(d)))
            }
        }
    }

    /// Busy-drain: run [`step`](Self::step) until no work remains, handing each delivery to `consume`
    /// **inside** the drain (so back-pressure never latches mid-burst), and return the number of work
    /// units drained. This method **never parks** — the caller blocks (via [`park`](Self::park)) only
    /// when this returns `0` (genuinely idle: RX ring empty, no due retransmit).
    ///
    /// This is the burst-safe drive (`architecture.md` §3, "busy-poll drains the ring while frames are
    /// present, then blocks"): draining a K-frame backlog costs **zero** parks. It is the correct
    /// alternative to a drive-one-then-park loop, which under the §3.1 stop-and-wait back-pressure parks
    /// once *per delivery* — pacing a K-burst against the §7 retransmit window (the reactor's
    /// `drive_one_then_park_per_delivery_is_the_defect` test pins that defect as a contrast to this
    /// method's zero).
    pub fn pump_all(&mut self, mut consume: impl FnMut(SessionId, Vec<u8>)) -> usize {
        let mut work = 0;
        while self.step() {
            work += 1;
            if let Some((session, payload)) = self.take_delivery() {
                consume(session, payload);
            }
        }
        work
    }

    /// The consumer takes the in-flight delivery — the `(SessionId, payload)` pair — releasing back-pressure.
    pub fn take_delivery(&mut self) -> Option<(SessionId, Vec<u8>)> {
        self.pending.take()
    }

    // --- observability (the gate re-expressions) ---------------------------------------------

    /// Actual parks so far — the clean analog of the committed `eventfd_reads`: **zero while there is
    /// work** (the fast path never blocks).
    pub fn parks(&self) -> u32 {
        self.parks
    }
    /// Malformed inbound frames dropped (totality).
    pub fn malformed(&self) -> u32 {
        self.malformed
    }
    /// Whether a delivery is currently held (back-pressure engaged).
    pub fn back_pressured(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether the reactor is quiescent: no delivery held **and** no sender has a frame in flight (no armed
    /// retransmit deadline). After a burst of originations, this becomes true once every one has been ACKed
    /// — the signal a driver uses on an idle tick to pace the next burst.
    pub fn quiescent(&self) -> bool {
        self.pending.is_none() && self.transport.next_deadline().is_none()
    }

    /// Read-only access to the substrate seam (the `Host` impl, e.g. the `DdWaitHost`) and the sole sink
    /// (e.g. the emit count), for the integration's observed report.
    pub fn host(&self) -> &H {
        &self.host
    }
    pub fn sink(&self) -> &S {
        &self.sink
    }
}
