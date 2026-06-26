//! The reactor loop core (Slice 2): **poll-then-block** — the ixy inversion. Busy-poll the RX-drain
//! while frames are present (driving `transport::on_inbound`, which synchronously emits any bare ACK
//! through the held `TransmitSink`), and block on `epoll_wait` **only** when the drain returns empty.
//!
//! The loop body is generic over the **injected** seams (`RxDrain`, `TransmitSink`, and the upward
//! `on_delivery` handler) — there is **no `cfg(test)` in the loop**; tests inject fakes, production
//! injects the real E2/E3 impls, and the audited loop *is* the production loop. The wakeup eventfd and
//! `epoll` are real OS primitives in both (real blocking is what the gate proves); E2 writes the eventfd
//! in production, a test writes it (via a `try_clone`'d notifier) to simulate the ring-non-empty edge.
//!
//! No adaptive interrupt/poll mode-switching, no rate estimator (ixy's `interrupts.c` is not ported).
//! Deadline composition (the `epoll_wait` timeout from `next_deadline`) is Slice 3; app-fd multiplexing
//! and back-pressure are Slice 4. Here the block timeout is supplied by the caller (`-1` in the S2 loop).

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::os::fd::{AsFd, OwnedFd};

use transport::{Clock, Delivery, Instant, SessionId, SessionTable, TransmitSink};

use crate::seam::RxDrain;
use crate::sys::{self, Epoll, Eventfd};

/// The epoll token for the wakeup eventfd. App fds get tokens `>= 1`; the deadline is a timeout, not an fd.
const WAKEUP_TOKEN: u64 = 0;
/// Ready-event buffer capacity (the eventfd + app fds).
const EVENT_CAP: usize = 64;
/// Max bytes read from an app fd per frame — one `read()` is one frame's payload (MTU-ish).
const APP_FRAME_MAX: usize = 1500;

/// Per-app-fd state. `EPOLLONESHOT` gives "read one frame then auto-pause"; `armed` tracks whether it is
/// currently enabled in epoll, so back-pressure (re-arm only when the session is idle) is explicit.
#[derive(Debug)]
struct AppFd {
    fd: OwnedFd,
    /// The session this fd's bytes ride on — `None` until the first frame opens one (open-by-emission).
    session: Option<SessionId>,
    /// Whether the fd is currently armed in epoll (`false` = oneshot fired / paused for back-pressure).
    armed: bool,
}

/// **Pure** composition of the transport's earliest deadline + the current instant into an
/// `epoll_wait` timeout (ms). No syscall, no clock — deterministically testable. The three edges:
///
/// - `None` → `-1` (block indefinitely on the eventfd). **Load-bearing:** a mis-map to `0` would
///   busy-spin forever instead of blocking when there is no armed deadline.
/// - deadline `<= now` → `0` (a deadline already passed — drive `poll` now, do not sleep through a
///   missed retransmit).
/// - future → `ceil(ns → ms)`, rounded **up** (pinned: a round-*down* of a sub-ms wait to `0`
///   busy-spins). Clamped to `i32::MAX` (the `epoll_wait` ceiling).
pub fn compute_timeout(next_deadline: Option<Instant>, now: Instant) -> i32 {
    match next_deadline {
        None => -1,
        Some(d) if d.0 <= now.0 => 0,
        Some(d) => {
            let ns = d.0 - now.0; // strictly > 0 here ⇒ ms >= 1 after ceil
            let ms = ns.div_ceil(1_000_000);
            ms.min(i32::MAX as u64) as i32
        }
    }
}

/// The single-threaded event reactor. Owns the `epoll`, the wakeup eventfd, and the driven `transport`.
/// One thread, one owner of all state — no locks, no `spawn`/`Arc`/`Mutex`.
#[derive(Debug)]
pub struct Reactor {
    epoll: Epoll,
    wakeup: Eventfd,
    transport: SessionTable,
    events: Vec<libc::epoll_event>,
    /// Application source fds, keyed by epoll token (>= 1).
    app_fds: BTreeMap<u64, AppFd>,
    /// Next app-fd token to hand out (the eventfd holds `WAKEUP_TOKEN` = 0).
    next_token: u64,
    /// Count of eventfd latch reads (gate-2 observability: **zero** during a continuous frame stream).
    eventfd_reads: u64,
    /// Count of malformed inbound frames dropped (totality: an adversary-influenced frame is dropped and
    /// the loop continues — never a crash; `grounding/data-path-and-events.md` §1).
    malformed_dropped: u64,
}

impl Reactor {
    /// Build a reactor over an injected wakeup eventfd and a transport instance; registers the eventfd
    /// on a fresh `epoll`. (E2 supplies the real eventfd in production; a test supplies one it can also
    /// notify via a `try_clone`'d handle.)
    pub fn new(wakeup: Eventfd, transport: SessionTable) -> io::Result<Self> {
        let epoll = Epoll::new()?;
        epoll.add(wakeup.as_fd(), libc::EPOLLIN as u32, WAKEUP_TOKEN)?;
        Ok(Self {
            epoll,
            wakeup,
            transport,
            events: vec![libc::epoll_event { events: 0, u64: 0 }; EVENT_CAP],
            app_fds: BTreeMap::new(),
            next_token: 1, // 0 is the wakeup eventfd
            eventfd_reads: 0,
            malformed_dropped: 0,
        })
    }

    /// Register an application source fd on the same `epoll` (armed `EPOLLIN | EPOLLONESHOT`), returning
    /// its token. ONESHOT gives one frame per readiness (read-one-then-pause); the reactor re-arms it
    /// only when its session is idle (back-pressure). The reactor owns the fd (closed on drop / EOF).
    pub fn add_app_fd(&mut self, fd: OwnedFd) -> io::Result<u64> {
        let token = self.next_token;
        self.next_token += 1;
        self.epoll
            .add(fd.as_fd(), (libc::EPOLLIN | libc::EPOLLONESHOT) as u32, token)?;
        self.app_fds.insert(
            token,
            AppFd {
                fd,
                session: None,
                armed: true,
            },
        );
        Ok(token)
    }

    /// Test/observability: whether an app fd is currently armed (`false` = paused for back-pressure).
    pub fn app_fd_armed(&self, token: u64) -> Option<bool> {
        self.app_fds.get(&token).map(|a| a.armed)
    }

    /// Test/observability: the session an app fd's bytes ride on (`None` until its first frame opens one).
    pub fn app_fd_session(&self, token: u64) -> Option<SessionId> {
        self.app_fds.get(&token).and_then(|a| a.session)
    }

    /// Total eventfd latch reads so far (gate-2 observable).
    pub fn eventfd_reads(&self) -> u64 {
        self.eventfd_reads
    }

    /// Total malformed inbound frames dropped so far (totality observable).
    pub fn malformed_dropped(&self) -> u64 {
        self.malformed_dropped
    }

    /// **Busy-poll**: drain the RX seam fully (while frames are present). Each frame goes to
    /// `transport::on_inbound` with the held `sink` — which **synchronously emits** a bare ACK for a
    /// received data frame and releases the buffer for an ACK. A returned `Delivery` is handed up via
    /// `on_delivery`. A **malformed** frame (the codec rejects it) is **dropped** and the drain
    /// continues (totality). Returns the number of frames seen (delivered + ACK-consumed + dropped).
    pub fn drain<R, S, D>(&mut self, rx: &mut R, sink: &mut S, on_delivery: &mut D) -> usize
    where
        R: RxDrain,
        S: TransmitSink,
        D: FnMut(Delivery<'_>),
    {
        let mut seen = 0;
        while let Some(frame) = rx.next_frame() {
            seen += 1;
            match self.transport.on_inbound(&frame, sink) {
                Ok(Some(delivery)) => on_delivery(delivery),
                Ok(None) => {} // an ACK on our sender session — consumed, nothing upward
                Err(_) => self.malformed_dropped += 1, // adversary-influenced frame: drop, continue
            }
        }
        seen
    }

    /// **Block** on `epoll_wait` until a registered source is ready (`timeout_ms`; `-1` = block
    /// indefinitely). If the wakeup eventfd is among the ready sources, **read-clear** its latch (so a
    /// blocked loop is not re-woken by a stale latch). **Token-aware**: an app-fd-only readiness does not
    /// touch the eventfd. Returns the ready-event count (the caller dispatches app-fd tokens).
    ///
    /// This is the eventfd-wake + latch-clear path the gate-2 tests prove **and** the exact path
    /// [`Reactor::run_once`] runs — one code, so the proof cannot drift from the live loop.
    pub fn block(&mut self, timeout_ms: i32) -> io::Result<usize> {
        let n = self.epoll.wait(&mut self.events, timeout_ms)?;
        for i in 0..n {
            if sys::event_token(&self.events[i]) == WAKEUP_TOKEN {
                self.wakeup.read_clear()?;
                self.eventfd_reads += 1;
            }
        }
        Ok(n)
    }

    /// One reactor iteration — **the audited production loop body**, `loop { run_once(..) }`:
    ///
    /// 1. **poll**: busy-poll-drain the RX seam fully (processing inbound, incl. ACKs that clear sessions);
    /// 2. **re-arm**: re-enable any paused app fd whose session is now idle (back-pressure release);
    /// 3. **block** via [`Reactor::block`]: `epoll_wait` with the timeout composed from the transport's
    ///    earliest deadline; the eventfd-wake + latch-clear is `block`'s code (the gate-2-proven path);
    /// 4. **dispatch**: a **deadline wake** (timed out, no fd) → `transport::poll(now)` (retransmit /
    ///    give-up); else for each ready app-fd token → read **one** frame and open/send it on its
    ///    session (then it is oneshot-paused). The eventfd is already cleared by `block`.
    ///
    /// **Single producer:** `sink` is the one `&mut S` threaded into *every* transport call (the drain's
    /// `on_inbound`, the app-fd `open`/`send`, and the deadline `poll`) — one holder, one thread, no
    /// `spawn`/`Arc`/`Mutex`. Emission is synchronous (no ready-set drain).
    pub fn run_once<R, S, D, C>(
        &mut self,
        rx: &mut R,
        sink: &mut S,
        on_delivery: &mut D,
        clock: &C,
    ) -> io::Result<()>
    where
        R: RxDrain,
        S: TransmitSink,
        D: FnMut(Delivery<'_>),
        C: Clock,
    {
        // 1. poll (busy-poll-drain) — may clear sender sessions via ACKs.
        self.drain(rx, sink, on_delivery);
        // 2. re-arm paused app fds whose session is now idle (back-pressure release).
        self.rearm_idle_app_fds()?;
        // 3. block — wait + (token-aware) eventfd latch-clear. This delegates the eventfd-wake to the
        //    gate-2-proven block(), so the live loop runs exactly the proven code (no duplicated handling).
        let timeout = compute_timeout(self.transport.next_deadline(), clock.now());
        let n = self.block(timeout)?;
        // 4. dispatch.
        if n == 0 {
            // Deadline wake (timed out, no fd ready) → drive retransmit/give-up through the held sink.
            self.transport.poll(clock.now(), sink);
            return Ok(());
        }
        // The eventfd (if it fired) was already cleared by block(); dispatch the app-fd tokens.
        for i in 0..n {
            let token = sys::event_token(&self.events[i]);
            if token != WAKEUP_TOKEN {
                self.handle_app_fd(token, clock.now(), sink)?;
            }
        }
        Ok(())
    }

    /// Re-arm any app fd that fired (paused) and whose session is now idle — the back-pressure release.
    /// A fd whose session still has a frame in flight stays paused (no read, no buffer) until its ACK.
    fn rearm_idle_app_fds(&mut self) -> io::Result<()> {
        let in_flight: BTreeSet<SessionId> = self.transport.ready_set().collect();
        let to_rearm: Vec<u64> = self
            .app_fds
            .iter()
            .filter(|(_, af)| {
                !af.armed && af.session.is_none_or(|id| !in_flight.contains(&id))
            })
            .map(|(&token, _)| token)
            .collect();
        for token in to_rearm {
            if let Some(af) = self.app_fds.get(&token) {
                self.epoll
                    .modify(af.fd.as_fd(), (libc::EPOLLIN | libc::EPOLLONESHOT) as u32, token)?;
            }
            if let Some(af) = self.app_fds.get_mut(&token) {
                af.armed = true;
            }
        }
        Ok(())
    }

    /// Handle a readable app fd: read **one** frame and open (first data) or send it on its session.
    /// The fd is now oneshot-paused; it re-arms only when its session goes idle (`rearm_idle_app_fds`).
    fn handle_app_fd<S: TransmitSink>(&mut self, token: u64, now: Instant, sink: &mut S) -> io::Result<()> {
        let mut buf = [0u8; APP_FRAME_MAX];
        let (nread, session) = {
            let af = match self.app_fds.get_mut(&token) {
                Some(a) => a,
                None => return Ok(()),
            };
            af.armed = false; // the oneshot fired → paused until re-armed
            let nread = sys::read(af.fd.as_fd(), &mut buf)?;
            (nread, af.session)
        };
        if nread == 0 {
            // EOF: the source closed. Deregister + drop the fd. (The session lingers — no close signal, §8.)
            if let Some(af) = self.app_fds.remove(&token) {
                self.epoll.delete(af.fd.as_fd())?;
            }
            return Ok(());
        }
        let frame = &buf[..nread];
        match session {
            None => {
                // First data on this fd → open a session by emission; map the fd to it.
                let id = self.transport.open(frame, now, sink);
                if let Some(af) = self.app_fds.get_mut(&token) {
                    af.session = Some(id);
                }
            }
            Some(id) => {
                // Re-armed only when idle, so this send succeeds. A `WouldBlock` is unreachable by the
                // back-pressure discipline (the fd is armed only when the session is idle) — the
                // debug_assert documents that invariant. Either way nothing is buffered: the fd stays
                // paused and re-arms on the next ACK. (`NoSuchSession` can occur only if the session was
                // reclaimed by give-up while paused; the frame is dropped — no buffer.)
                let result = self.transport.send(id, frame, now, sink);
                debug_assert!(
                    result != Err(transport::SendError::WouldBlock),
                    "app fd was armed while its session had a frame in flight — back-pressure invariant violated"
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::time::Duration;
    use transport::{SessionId, encode_frame};

    /// Injected fake clock: `now` is settable, so all timing logic is exercised deterministically with
    /// no wall clock.
    struct FakeClock(Cell<u64>);
    impl FakeClock {
        fn at(ns: u64) -> Self {
            Self(Cell::new(ns))
        }
    }
    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            Instant(self.0.get())
        }
    }

    /// Injected fake RX-drain: hands out queued OWNED frames, then `None`.
    struct FakeRx {
        frames: VecDeque<Vec<u8>>,
    }
    impl FakeRx {
        fn new<const N: usize>(frames: [Vec<u8>; N]) -> Self {
            Self {
                frames: frames.into_iter().collect(),
            }
        }
        fn push(&mut self, frame: Vec<u8>) {
            self.frames.push_back(frame);
        }
    }
    impl RxDrain for FakeRx {
        fn next_frame(&mut self) -> Option<Vec<u8>> {
            self.frames.pop_front()
        }
    }

    /// Injected capture sink: records every frame the transport emits (data ACKs here).
    #[derive(Default)]
    struct CaptureSink {
        sent: Vec<Vec<u8>>,
    }
    impl TransmitSink for CaptureSink {
        fn emit(&mut self, frame: &[u8]) {
            self.sent.push(frame.to_vec());
        }
    }

    fn frame(id: u64, payload: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        encode_frame(SessionId(id), payload, &mut b);
        b
    }

    /// Gate 2: the RX-drain is fully emptied (all delivered + a bare ACK per data frame through the held
    /// sink) and zero eventfd reads happen during the drain.
    #[test]
    fn drain_empties_fully_and_emits_acks_with_zero_eventfd_reads() {
        let efd = Eventfd::new().unwrap();
        let mut r = Reactor::new(efd, SessionTable::new()).unwrap();
        let mut rx = FakeRx::new([frame(1, b"a"), frame(2, b"b"), frame(3, b"c")]);
        let mut sink = CaptureSink::default();
        let mut delivered: Vec<(SessionId, Vec<u8>)> = Vec::new();
        let mut on_d = |d: Delivery<'_>| delivered.push((d.session, d.payload.to_vec()));

        let seen = r.drain(&mut rx, &mut sink, &mut on_d);

        assert_eq!(seen, 3, "all frames drained");
        assert_eq!(delivered.len(), 3, "all delivered up");
        assert_eq!(sink.sent.len(), 3, "a bare ACK emitted per data frame (sink wired in the drain)");
        assert_eq!(r.eventfd_reads(), 0, "draining reads the eventfd zero times");
    }

    /// Gate 2: a continuous stream (frames always present across drains) incurs zero eventfd reads — the
    /// loop never blocks while there is work.
    #[test]
    fn continuous_stream_takes_zero_eventfd_reads() {
        let efd = Eventfd::new().unwrap();
        let mut r = Reactor::new(efd, SessionTable::new()).unwrap();
        let mut sink = CaptureSink::default();
        let mut sunk = 0usize;
        let mut on_d = |_d: Delivery<'_>| sunk += 1;

        for round in 0..5 {
            let mut rx = FakeRx::new([frame(round, b"x"), frame(round + 100, b"y")]);
            r.drain(&mut rx, &mut sink, &mut on_d);
        }
        assert_eq!(sunk, 10);
        assert_eq!(r.eventfd_reads(), 0, "a continuous stream blocks zero times → zero eventfd reads");
    }

    /// Gate 2: malformed inbound frames are dropped (totality), the drain continues, good frames deliver.
    #[test]
    fn malformed_frame_is_dropped_and_drain_continues() {
        let efd = Eventfd::new().unwrap();
        let mut r = Reactor::new(efd, SessionTable::new()).unwrap();
        let mut rx = FakeRx::new([frame(1, b"ok"), vec![0u8; 7], frame(2, b"ok2")]); // 7 bytes = short
        let mut sink = CaptureSink::default();
        let mut delivered = 0usize;
        let mut on_d = |_d: Delivery<'_>| delivered += 1;

        let seen = r.drain(&mut rx, &mut sink, &mut on_d);

        assert_eq!(seen, 3, "all three frames seen");
        assert_eq!(delivered, 2, "the two good frames delivered");
        assert_eq!(r.malformed_dropped(), 1, "the short frame dropped, drain continued");
    }

    /// Gate 2 (positive block proof): with nothing to drain, the loop actually blocks and wakes ONLY on
    /// the eventfd write; the wake read-clears the latch; a subsequent block with no new write times out
    /// empty (latch truly cleared, no busy-spin).
    #[test]
    fn block_actually_blocks_wakes_on_eventfd_and_latch_clears() {
        let efd = Eventfd::new().unwrap();
        let notifier = efd.try_clone().unwrap();
        let mut r = Reactor::new(efd, SessionTable::new()).unwrap();

        std::thread::scope(|s| {
            s.spawn(|| {
                // Simulate E2/the kernel firing during the block.
                std::thread::sleep(Duration::from_millis(20));
                notifier.notify(1).unwrap();
            });
            // Block with a generous ceiling: it must WAKE on the eventfd (a broken wake fails cleanly by
            // timing out empty after 5 s, rather than hanging). Outcome asserted, not latency.
            let n = r.block(5000).unwrap();
            assert!(n >= 1, "blocked, then woke on the eventfd write");
        });

        assert_eq!(r.eventfd_reads(), 1, "the wake read-cleared the latch exactly once");
        assert_eq!(r.block(50).unwrap(), 0, "latch cleared → a fresh block times out empty (no busy-spin)");
    }

    /// Gate 2 (lost-wakeup): an eventfd write in the window between drain-empty and the block is NOT
    /// lost — the eventfd latches, so the block returns immediately and the loop re-drains the late frame.
    #[test]
    fn lost_wakeup_is_prevented_by_the_latch() {
        let efd = Eventfd::new().unwrap();
        let notifier = efd.try_clone().unwrap();
        let mut r = Reactor::new(efd, SessionTable::new()).unwrap();
        let mut sink = CaptureSink::default();
        let mut delivered: Vec<(SessionId, Vec<u8>)> = Vec::new();
        let mut on_d = |d: Delivery<'_>| delivered.push((d.session, d.payload.to_vec()));

        // Drain finds the ring empty.
        let mut rx = FakeRx::new([]);
        assert_eq!(r.drain(&mut rx, &mut sink, &mut on_d), 0);

        // WINDOW (between drain-empty and the block): the interrupt fires AND a frame lands.
        notifier.notify(1).unwrap();
        rx.push(frame(7, b"late"));

        // The latch is already set → the block returns immediately; the wake is not lost.
        let n = r.block(5000).unwrap();
        assert!(n >= 1, "latched eventfd → block returns immediately, wake not lost");

        // Re-drain: the late frame is found and delivered. No frame missed.
        assert_eq!(r.drain(&mut rx, &mut sink, &mut on_d), 1);
        assert!(delivered.iter().any(|(s, p)| *s == SessionId(7) && p == b"late"));
    }

    /// The composed iteration drains FULLY before it blocks: with frames queued and the latch pre-set
    /// (so the block returns at once), all frames deliver and exactly one block (eventfd read) follows.
    #[test]
    fn poll_then_block_drains_fully_before_blocking() {
        let efd = Eventfd::new().unwrap();
        let notifier = efd.try_clone().unwrap();
        let mut r = Reactor::new(efd, SessionTable::new()).unwrap();
        let mut rx = FakeRx::new([frame(1, b"a"), frame(2, b"b")]);
        let mut sink = CaptureSink::default();
        let mut delivered = 0usize;
        let mut on_d = |_d: Delivery<'_>| delivered += 1;

        notifier.notify(1).unwrap(); // pre-set the latch so the block returns immediately (no hang)
        // No armed deadline ⇒ compute_timeout → -1; the pre-set latch makes block(-1) return at once.
        let clock = FakeClock::at(0);
        r.run_once(&mut rx, &mut sink, &mut on_d, &clock).unwrap();

        assert_eq!(delivered, 2, "drained fully (both delivered) before the block");
        assert_eq!(r.eventfd_reads(), 1, "then blocked once (latch read-cleared)");
    }

    // ---- Slice 3: compute_timeout (pure, all edges) -------------------------------------------

    #[test]
    fn compute_timeout_none_blocks_indefinitely() {
        // Load-bearing: None must be -1 (block on the eventfd), never 0 (which would busy-spin).
        assert_eq!(compute_timeout(None, Instant(123)), -1);
    }

    #[test]
    fn compute_timeout_past_or_now_is_zero() {
        assert_eq!(compute_timeout(Some(Instant(50)), Instant(50)), 0, "deadline == now → 0");
        assert_eq!(compute_timeout(Some(Instant(40)), Instant(50)), 0, "deadline < now → 0");
    }

    #[test]
    fn compute_timeout_future_rounds_up() {
        assert_eq!(compute_timeout(Some(Instant(1)), Instant(0)), 1, "1 ns → 1 ms (ceil, never 0)");
        assert_eq!(compute_timeout(Some(Instant(1_000_000)), Instant(0)), 1, "exactly 1 ms → 1");
        assert_eq!(compute_timeout(Some(Instant(1_000_001)), Instant(0)), 2, "1 ms + 1 ns → 2 (round up)");
        assert_eq!(compute_timeout(Some(Instant(10_000_000)), Instant(0)), 10, "10 ms → 10");
    }

    #[test]
    fn compute_timeout_clamps_to_i32_max() {
        assert_eq!(compute_timeout(Some(Instant(u64::MAX)), Instant(0)), i32::MAX);
    }

    // ---- Slice 3: a deadline wake drives transport::poll → retransmit emits through the held sink ----

    #[test]
    fn deadline_wake_drives_poll_and_retransmit_emits_through_held_sink() {
        // Open a sender session on the table BEFORE moving it into the reactor (no reactor open() API
        // yet — that is S4; this keeps the reactor the sole sink holder for the drive path under test).
        let interval = Duration::from_nanos(10);
        let give_up = Duration::from_secs(100); // far away → the wake is a retransmit, not a give-up
        let mut table = SessionTable::with_timing(interval, give_up);
        let mut setup = CaptureSink::default();
        let _id = table.open(b"data", Instant(0), &mut setup); // first emit + arm retransmit at Instant(10)
        assert_eq!(setup.sent.len(), 1, "open emitted the first frame");
        assert_eq!(table.next_deadline(), Some(Instant(10)), "retransmit armed at interval");

        let efd = Eventfd::new().unwrap();
        let mut r = Reactor::new(efd, table).unwrap();
        let mut rx = FakeRx::new([]); // nothing inbound
        let mut sink = CaptureSink::default(); // the reactor's held sink (distinct from the pre-reactor setup)
        let mut on_d = |_d: Delivery<'_>| {};
        let clock = FakeClock::at(10); // now == the retransmit deadline ⇒ compute_timeout → 0 ⇒ block→0

        r.run_once(&mut rx, &mut sink, &mut on_d, &clock).unwrap();

        assert_eq!(
            sink.sent.len(),
            1,
            "the deadline wake (block timed out) drove transport::poll → a retransmit emitted via the held sink"
        );
        assert_eq!(sink.sent[0], setup.sent[0], "the retransmit re-emits the same frame bytes");
    }

    /// Single producer: the ONE held sink receives BOTH an inbound-driven bare ACK (drain) and a
    /// deadline-driven retransmit (poll) — one sink for all emission (structural: run_once threads a
    /// single `&mut S`).
    #[test]
    fn one_held_sink_receives_both_ack_and_retransmit() {
        let interval = Duration::from_nanos(10);
        let give_up = Duration::from_secs(100);
        let mut table = SessionTable::with_timing(interval, give_up);
        let mut setup = CaptureSink::default();
        let _id = table.open(b"data", Instant(0), &mut setup); // armed at Instant(10)

        let efd = Eventfd::new().unwrap();
        let mut r = Reactor::new(efd, table).unwrap();
        let mut sink = CaptureSink::default();
        let mut rx = FakeRx::new([frame(99, b"inbound")]); // an inbound data frame (a new receiver session)
        let mut on_d = |_d: Delivery<'_>| {};
        let clock = FakeClock::at(10); // deadline reached → poll drives a retransmit too

        r.run_once(&mut rx, &mut sink, &mut on_d, &clock).unwrap();

        // The single held sink got the inbound frame's bare ACK AND the sender's retransmit.
        assert_eq!(sink.sent.len(), 2, "one sink received both the ACK and the retransmit");
    }

    // ---- Slice 4: app-fd multiplexing + token dispatch + back-pressure + closed heartbeat ----------

    /// Token dispatch: a readable app fd (token >= 1, distinct from the eventfd's token 0) opens a
    /// session and emits — proving readiness is routed by token, not "any readiness is the eventfd".
    #[test]
    fn app_fd_readiness_dispatched_by_token_opens_a_session() {
        use std::io::Write;

        let (reader, mut writer) = std::io::pipe().unwrap();
        let efd = Eventfd::new().unwrap();
        let mut r = Reactor::new(efd, SessionTable::with_timing(Duration::from_secs(3600), Duration::from_secs(7200))).unwrap();
        let token = r.add_app_fd(OwnedFd::from(reader)).unwrap();
        assert_ne!(token, 0, "app fd token is distinct from the eventfd's WAKEUP_TOKEN");

        let mut sink = CaptureSink::default();
        let mut rx = FakeRx::new([]);
        let mut on_d = |_d: Delivery<'_>| {};
        let clock = FakeClock::at(0);

        writer.write_all(b"request").unwrap();
        r.run_once(&mut rx, &mut sink, &mut on_d, &clock).unwrap();

        assert_eq!(sink.sent.len(), 1, "the app-fd read opened a session and emitted one frame");
        assert!(r.app_fd_session(token).is_some(), "the fd is now mapped to a session");
        assert_eq!(r.app_fd_armed(token), Some(false), "oneshot fired → paused (in flight)");
        assert_eq!(r.eventfd_reads(), 0, "no eventfd read — this was an app-fd readiness, not the eventfd");
    }

    /// Back-pressure: while the session is in flight the app fd is PAUSED — a second write waits in the
    /// pipe (not read, nothing buffered in the reactor); the ACK re-arms the fd and the next frame goes.
    #[test]
    fn app_fd_back_pressure_pauses_then_resumes_on_ack() {
        use std::io::Write;

        let (reader, mut writer) = std::io::pipe().unwrap();
        let efd = Eventfd::new().unwrap();
        // Long timers so no retransmit/give-up interferes with the back-pressure observation.
        let mut r = Reactor::new(efd, SessionTable::with_timing(Duration::from_secs(3600), Duration::from_secs(7200))).unwrap();
        let token = r.add_app_fd(OwnedFd::from(reader)).unwrap();
        let mut sink = CaptureSink::default();
        let mut on_d = |_d: Delivery<'_>| {};
        let clock = FakeClock::at(0);

        // Frame 1: opens the session, emits, fd auto-pauses (in flight).
        writer.write_all(b"AAA").unwrap();
        r.run_once(&mut FakeRx::new([]), &mut sink, &mut on_d, &clock).unwrap();
        assert_eq!(sink.sent.len(), 1, "frame 1 emitted");
        assert_eq!(r.app_fd_armed(token), Some(false), "paused while in flight");
        let id = r.app_fd_session(token).expect("session opened");

        // Frame 2's data lands in the PIPE while the fd is paused. It must NOT be read or buffered.
        writer.write_all(b"BBB").unwrap();
        assert_eq!(r.app_fd_armed(token), Some(false), "still paused — back-pressure holds");
        assert_eq!(sink.sent.len(), 1, "frame 2 NOT emitted while in flight (waits in the pipe, nothing buffered)");

        // The ACK arrives → drain clears the session → re-arm → the paused fd fires → frame 2 goes.
        let ack = frame(id.0, &[]);
        r.run_once(&mut FakeRx::new([ack]), &mut sink, &mut on_d, &clock).unwrap();
        assert_eq!(sink.sent.len(), 2, "ACK re-armed the fd → frame 2 emitted");
        assert_eq!(r.app_fd_armed(token), Some(false), "frame 2 in flight → paused again");
    }

    /// Closed heartbeat: one single-threaded loop interleaves all three readiness kinds — an inbound
    /// frame bubbles to its session (delivered up + ACKed), a readable app fd goes out as a session
    /// send, and a deadline fires a retransmit — all through the one held sink, no locks.
    #[test]
    fn closed_heartbeat_interleaves_inbound_appfd_and_deadline() {
        use std::io::Write;

        let (reader, mut writer) = std::io::pipe().unwrap();
        let efd = Eventfd::new().unwrap();
        let interval = Duration::from_nanos(10);
        let give_up = Duration::from_secs(100);
        let mut r = Reactor::new(efd, SessionTable::with_timing(interval, give_up)).unwrap();
        let token = r.add_app_fd(OwnedFd::from(reader)).unwrap();
        let mut sink = CaptureSink::default();
        let mut delivered: Vec<(SessionId, Vec<u8>)> = Vec::new();

        // Iteration 1: an inbound data frame AND a readable app fd, together on one loop.
        writer.write_all(b"egress").unwrap();
        {
            let mut on_d = |d: Delivery<'_>| delivered.push((d.session, d.payload.to_vec()));
            let mut rx1 = FakeRx::new([frame(500, b"ingress")]);
            r.run_once(&mut rx1, &mut sink, &mut on_d, &FakeClock::at(0)).unwrap();
        }
        assert!(
            delivered.iter().any(|(s, p)| *s == SessionId(500) && p == b"ingress"),
            "inbound frame bubbled to its session and was delivered up"
        );
        assert!(r.app_fd_session(token).is_some(), "app fd opened its egress session");
        // sink got: the bare ACK for session 500 (inbound) + the egress open frame (app fd).
        assert_eq!(sink.sent.len(), 2, "one loop emitted both the inbound ACK and the egress frame");

        // Iteration 2: nothing inbound, app fd paused (egress in flight) → the deadline fires a retransmit.
        let before = sink.sent.len();
        let mut noop = |_d: Delivery<'_>| {};
        r.run_once(&mut FakeRx::new([]), &mut sink, &mut noop, &FakeClock::at(10)).unwrap();
        assert_eq!(sink.sent.len(), before + 1, "a deadline wake retransmitted the egress frame");
        // it re-emitted the egress session's frame (same session still in flight, paused for back-pressure)
        assert_eq!(r.app_fd_armed(token), Some(false), "egress fd still paused while in flight");
    }
}
