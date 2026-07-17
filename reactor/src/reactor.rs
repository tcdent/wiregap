//! The bare-metal event reactor — the **ratified behavior built fresh**, `no_std`, NOT a port of the
//! std/epoll reactor. It drives the reused `transport` (unchanged) over a **minimal two-method seam**
//! ([`Host`]: `now` + `block`) and a frame source ([`RxSource`]) + sink (`transport::TransmitSink`). The
//! ratified behavior, re-expressed:
//!
//! - **poll-then-block ordering** — [`Reactor::step`] serves one unit of work; the driver drains
//!   (`while step() {}`) and only then [`Reactor::park`]s. Zero parks while there is work.
//! - **deadline composition** — [`arm_from_deadline`]: the *three edge-facts only*, re-typed to the
//!   one-shot timer arm (`Option<Duration>`), NOT the epoll `i32`-ms shape.
//! - **sole-sink-holder** — the reactor uniquely owns the `TransmitSink`; the transport emits only
//!   through it.
//! - **back-pressure as RX flow control** — [`can_pull_rx`]: hold the RX ring (do not pull / advance
//!   RDT) while a delivery is in flight to the consumer (stop-and-wait: one in flight). NOT the old
//!   app-fd `EPOLLONESHOT`.
//!
//! The loop is **pure** (unsafe-free, generic over the seam), so the decision logic is host-tested
//! (`tests/reactor.rs`); the substrate side (LAPIC one-shot timer / `hlt` / MSI-X, the Slice-1..3
//! substrate) is confined entirely in the [`Host`] *impl* (`main.rs`, bare-metal only).

use alloc::vec::Vec;
use core::time::Duration;
use transport::{Delivery, Instant, SessionId, SessionTable, TransmitSink};

/// Why the reactor woke from a park — the **woke-by class** (the clean re-expression of the committed
/// eventfd/token classes; app-fds are gone).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WakeReason {
    /// The NIC's MSI-X interrupt (its RX/TX-completion wakeup; a self-IPI stands in this slice).
    NicMsix,
    /// The LAPIC one-shot timer (a retransmit deadline elapsed).
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

/// RX back-pressure (pure): pull the next inbound frame only when a frame is available **and** no
/// delivery is already in flight to the consumer. While a delivery is held, the RX ring is not advanced
/// (RDT held) — the hardware back-pressures. Stop-and-wait: at most one in flight.
pub const fn can_pull_rx(frame_available: bool, delivery_in_flight: bool) -> bool {
    frame_available && !delivery_in_flight
}

/// The minimal NIC-shaped substrate seam: a monotonic clock + a park-until-wakeup-or-deadline. The
/// bare-metal impl (LAPIC/`hlt`/MSI-X) lives behind this; a fake impl drives the host tests.
pub trait Host {
    /// Monotonic now.
    fn now(&self) -> Instant;
    /// Park until a wakeup (NIC MSI-X) or, when `timeout` is `Some`, the one-shot timer fires — whichever
    /// first. Returns which woke it. Must not lose a wakeup that arrived just before the call.
    fn block(&mut self, timeout: Option<Duration>) -> WakeReason;
}

/// A source of inbound frames (the driver's RX drain in production; a stub in this slice). The returned
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
    /// The one in-flight delivery to the consumer (copied out of the RX buffer — the copy boundary).
    /// While `Some`, RX is back-pressured.
    pending: Option<Vec<u8>>,
    parks: u32,
    malformed: u32,
}

impl<H: Host, R: RxSource, S: TransmitSink> Reactor<H, R, S> {
    pub fn new(transport: SessionTable, host: H, rx: R, sink: S) -> Self {
        Self { transport, host, rx, sink, pending: None, parks: 0, malformed: 0 }
    }

    /// Originate a session (the app-above role): open-by-emission through the sole sink.
    pub fn originate(&mut self, payload: &[u8]) -> SessionId {
        let now = self.host.now();
        self.transport.open(payload, now, &mut self.sink)
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
                Ok(Some(Delivery { payload, .. })) => self.pending = Some(payload.to_vec()),
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
        match arm_from_deadline(self.transport.next_deadline(), now) {
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

    /// Drain all work, then park once. One poll-then-block cycle.
    pub fn pump_once(&mut self) -> Option<WakeReason> {
        while self.step() {}
        self.park()
    }

    /// The consumer takes the in-flight delivery (releasing back-pressure).
    pub fn take_delivery(&mut self) -> Option<Vec<u8>> {
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

    /// Read-only access to the substrate seam (e.g. the `hlt` count) and the sole sink (e.g. the emit
    /// count), for the integration's observed report.
    pub fn host(&self) -> &H {
        &self.host
    }
    pub fn sink(&self) -> &S {
        &self.sink
    }
}
