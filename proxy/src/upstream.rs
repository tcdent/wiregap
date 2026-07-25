//! Sprint 11 Slice 1 — the concurrency/ingress foundation. A background worker owns **the** (one,
//! serial) blocking upstream call; it hands response frames to the box-facing loop over a **bounded**
//! channel; the loop drains that channel **every tick** into the Sprint-10 push-queue
//! ([`Dispatcher::enqueue_net_response`]).
//!
//! **Message-passing only** — the only thread boundary is the channel, carrying **owned** `Vec<u8>`
//! frames; the [`Dispatcher`] is owned solely by the loop and never crosses to the worker. No `Arc`, no
//! `Mutex`, no shared mutable state (the `Send` bound on the spawned worker enforces it, and this module
//! deliberately contains no shared-state type). **No new `unsafe`** (MB6): `std::thread` +
//! `std::sync::mpsc`.
//!
//! **Serial, bound = 1** — at most one upstream call is in flight. A second `net-payload` admitted while
//! one is running is denied `net-error(Unavailable)` (the JC-3 seam), never a second worker/channel.
//! Concurrent upstream calls are an explicit Sprint-12+ follow-on.
//!
//! **The evidence boundary (Sprint-8-shaped).** Slice 1 proves the loop-side logic with an INJECTED
//! [`UpstreamSpawn`] (a controllable double); Sprint 18 Slice 2 landed the real `rustls` worker behind
//! the same seam. The one thing these tests do not themselves exercise — that the real reactor runs a
//! tick (and thus drains) when a response arrives during a real quiescent park — was **proven live on
//! hardware in Sprint 14**: the reactor's own bounded wait re-checks unaided, so the wake needs no
//! proxy-side mechanism. Here a test's `Idle` tick after a quiescent arrival stands in for that wake.

use std::ops::ControlFlow;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread::JoinHandle;

use directpath::Event;
use proto::ErrorCode;
use transport::SessionId;

use crate::dispatch::{Dispatcher, Enqueue, Routed};
use crate::nic::Reply;
use crate::vocab::{Route, ToolIdentity};




/// The bounded depth (frames) of the worker→loop channel. A full channel **backpressures** the worker's
/// `send` (it blocks until the loop drains) — never an unbounded buffer, the outbound analog of the
/// push-queue / reassembly caps.
///
/// **Reviewed and confirmed at 64 (2026-07-25),** with its product stated: every item is a
/// `RESPONSE_CHUNK` read, so this channel is bounded at 64 × 16 KiB = **1 MiB**. That is the tightest
/// of the crate's bounded channels and needs no adjustment.
pub const UPSTREAM_CHANNEL_CAP: usize = 64;

/// One item a worker sends to the box-facing loop over the channel (Sprint 11 Slice 2). Owned data —
/// the only thing that crosses the thread boundary.
pub enum UpstreamFrame {
    /// One **verbatim** chunk of the upstream response body → relayed to the box as `net-response`
    /// (MB3: never inspected, neutralized, or transformed — `cloud_relay::relay_response`).
    Chunk(Vec<u8>),
    /// The upstream call **failed at the TRANSPORT level** — timeout, unreachable, TLS — i.e. there is
    /// no response to relay at all → the box is told `net-error(code)` (the proxy's own category; no
    /// upstream body, no credential — no leak). **An HTTP status is NOT a failure**: a 401 or 500 is a
    /// response, and it reaches the box verbatim as [`UpstreamFrame::Chunk`] like any other
    /// (`cloud/approach.md` §5, operator ruling 2026-07-24).
    Failed(ErrorCode),
}

/// One spawned upstream call's loop-side handle. The worker streams [`UpstreamFrame`]s over the channel
/// and drops its `SyncSender` on completion/error/timeout (channel close = end-of-response). `join` is
/// the worker thread's handle for cleanup (`None` only for a threadless test double).
pub struct UpstreamHandle {
    /// The loop's receiving end; the worker holds the matching `SyncSender`.
    pub rx: Receiver<UpstreamFrame>,
    /// The worker thread, joined on end-of-response so no thread is leaked. `None` = a threadless fake.
    pub join: Option<JoinHandle<()>>,
}

/// The upstream-call spawner seam — Slice 1's injection point. Given the request `session` and the
/// recognizer-admitted `body`, spawn the (blocking) call on its own thread and return the loop-side
/// [`UpstreamHandle`]. The real `rustls` worker ([`AnthropicClient`](crate::upstream_dial::AnthropicClient)) implements this; tests inject a
/// controllable double. **`body` is the exact admitted bytes — POSTed verbatim, no re-encode**
/// (bind-the-chain; realized on the real call in Slice 2).
pub trait UpstreamSpawn {
    /// Spawn the upstream call for `session`, dialing **`destination`** — the `Route` U3 actually matched,
    /// threaded from the register rather than held independently by the implementor (Sprint 19).
    ///
    /// Before this took a destination, an implementor dialed a constant of its own and the register's
    /// authority over *where* the request went was illusory: the gate vetted one string while the dial
    /// used another, identical only by coincidence of authorship. Passing it here is what makes "the
    /// register fixes the destination" a property of the code rather than of two files agreeing.
    fn spawn(&self, session: SessionId, destination: Route, body: Vec<u8>) -> UpstreamHandle;
}

/// One in-flight upstream call (serial bound = 1).
struct InFlight {
    session: SessionId,
    rx: Receiver<UpstreamFrame>,
    join: Option<JoinHandle<()>>,
    /// One response frame **pulled off the channel but not yet accepted** by the box's queue (Sprint 13
    /// Slice 1.5) — the single-slot backpressure memory, mirroring [`ReplyStream::pending`] on the wire
    /// side. While it is `Some`, `drain` does not read the channel, so the mpsc fills and the upstream
    /// worker blocks on `send` (real TCP flow-control) instead of the chunk being silently dropped.
    ///
    /// [`ReplyStream::pending`]: crate::dispatch
    pending: Option<PendingResponse>,
}

/// A response frame pulled from the worker channel awaiting room in the box's reply queue (Sprint 13
/// Slice 1.5). Retained across ticks and retried before any new channel read, so nothing pulled off the
/// channel is ever lost to a momentarily-full box queue.
enum PendingResponse {
    /// A response chunk → re-`enqueue_net_response`.
    Chunk(Vec<u8>),
    /// A terminal failure → re-`enqueue_net_error` (also closes the reply once accepted).
    Failed(ErrorCode),
}

/// The loop-side upstream ingress (Sprint 11 Slice 1). Holds **at most one** in-flight call. On a
/// `NetPayloadAdmit` it spawns the call (or denies a second); each tick it drains the worker's channel
/// into the push-queue and, on channel close (EOF), completes, joins the worker, and evicts.
pub struct UpstreamIngress<S> {
    spawn: S,
    inflight: Option<InFlight>,
}

impl<S: UpstreamSpawn> UpstreamIngress<S> {
    #[must_use]
    pub fn new(spawn: S) -> Self {
        UpstreamIngress { spawn, inflight: None }
    }

    /// A recognizer-admitted request body arrived. If no call is in flight, spawn the upstream worker
    /// (`body` is moved to the worker **verbatim** — bind-the-chain). If one is ALREADY in flight, deny
    /// this one `net-error(Unavailable)` — the serial bound (never a second worker/channel).
    pub(crate) fn on_admit(&mut self, dispatcher: &mut Dispatcher, session: SessionId, body: Vec<u8>) {
        if self.inflight.is_some() {
            dispatcher.enqueue_net_error(session, ErrorCode::Unavailable);
            return;
        }
        // The destination comes from the REGISTER — the `Route` U3 matched and the dispatcher already
        // stores for this session — never from a constant the client happens to hold.
        let Some(destination) = dispatcher.route_for(session) else {
            // Unreachable by construction: `NetPayloadAdmit` is only produced when a route was stored
            // (no stored route ⇒ `NetPayloadNoRoute`, which never reaches here). Fail CLOSED anyway
            // rather than inventing a destination — an admitted request with nowhere vetted to send it
            // is "not an upstream response" (`net.md` §4), so the box is told, not left waiting.
            dispatcher.enqueue_net_error(session, ErrorCode::Unavailable);
            return;
        };
        let handle = self.spawn.spawn(session, destination, body);
        self.inflight = Some(InFlight { session, rx: handle.rx, join: handle.join, pending: None });
    }

    /// Drain response frames that have arrived from the worker into the push-queue (called **every
    /// tick**). On channel close (the worker dropped its `Sender` = end-of-response), close the reply
    /// stream, **join** the worker thread (no leak), and clear the in-flight slot. **Non-blocking**
    /// (`try_recv`): a slow or hung worker simply yields nothing this tick and never stalls the loop.
    ///
    /// **Backpressure, never drop (Sprint 13 Slice 1.5).** A frame pulled off the channel that the box's
    /// queue **refuses** ([`Enqueue::Full`]) is **retained** in the one-slot [`InFlight::pending`] and
    /// draining **stops** for this tick — the channel is left unread, so it fills and the worker blocks
    /// on `send` (real flow-control), resuming next tick once the box drains. A pulled chunk is never
    /// handed to a full queue and dropped (the original defect: the frame was already off the channel).
    pub(crate) fn drain(&mut self, dispatcher: &mut Dispatcher) {
        let Some(session) = self.inflight.as_ref().map(|f| f.session) else {
            return;
        };
        // Resume: retry a frame retained from a prior tick's full queue BEFORE reading the channel again,
        // so the channel stays unread (worker keeps backpressuring) until the box makes room.
        if let Some(pending) = self.inflight.as_mut().and_then(|f| f.pending.take())
            && let Some(unaccepted) = try_enqueue(dispatcher, session, pending)
        {
            self.inflight.as_mut().expect("in flight above").pending = Some(unaccepted);
            return; // still full — stop, keep backpressuring
        }
        loop {
            // Borrow self.inflight only for the try_recv call (the Result is owned — no held borrow).
            let recv = self.inflight.as_ref().expect("checked above").rx.try_recv();
            let frame = match recv {
                // A response chunk → relayed verbatim (net-response, MB3); a failure → net-error(code).
                Ok(UpstreamFrame::Chunk(chunk)) => PendingResponse::Chunk(chunk),
                Ok(UpstreamFrame::Failed(code)) => PendingResponse::Failed(code),
                Err(TryRecvError::Empty) => return, // nothing right now — the worker is still running
                Err(TryRecvError::Disconnected) => {
                    // The channel closed. That means the worker RETURNED — but "returned" covers both a
                    // completed response and a PANIC, and the two must not look the same to the box.
                    //
                    // **Reap first, then decide** (Sprint 18, Finding B). This previously did
                    // `close_reply` unconditionally and then discarded the join result with `let _ =`,
                    // so a panicked worker presented to the box as a cleanly-completed response: the
                    // reply stream closed normally and the box saw a truncated body it had no way to
                    // distinguish from a short one. `net.md` §4 is explicit that "any outcome that is not
                    // an upstream response — refusal, unreachable, dropped, timeout, malformed — is one
                    // net-error", and a crashed worker is a dropped exchange, not a response.
                    let panicked = match self.inflight.take().and_then(|done| done.join) {
                        Some(join) => join.join().is_err(), // Err ⇒ the worker panicked
                        None => false,                     // a threadless test double: nothing to reap
                    };
                    if panicked {
                        // Tell the box the exchange was dropped, rather than closing as if it completed.
                        // `enqueue_net_error` closes the reply itself once the frame is accepted.
                        dispatcher.enqueue_net_error(session, ErrorCode::Unavailable);
                    } else {
                        dispatcher.close_reply(session);
                    }
                    return;
                }
            };
            // Enqueue; a full box queue RETAINS this (already-pulled) frame and STOPS the drain — no drop.
            if let Some(unaccepted) = try_enqueue(dispatcher, session, frame) {
                self.inflight.as_mut().expect("checked above").pending = Some(unaccepted);
                return;
            }
        }
    }

    /// **Test observability**: is a call currently in flight? Exercised only by this module's own unit
    /// tests (the serial-bound / worker-lifecycle / backpressure proofs), so it is `#[cfg(test)]`-gated
    /// and private — it was never public API, and a test-only accessor should not read as one
    /// (pre-operator cleanup, Sprint 16). The same-file `mod tests` can see it despite being private.
    #[cfg(test)]
    #[must_use]
    fn is_in_flight(&self) -> bool {
        self.inflight.is_some()
    }
}

/// Enqueue a pulled response frame into the box's reply queue (Sprint 13 Slice 1.5). Returns `None` when
/// accepted, or `Some(frame)` — the un-accepted frame handed back for the caller to retain — when the
/// queue is [`Enqueue::Full`], so a frame already pulled off the channel is never dropped.
fn try_enqueue(
    dispatcher: &mut Dispatcher,
    session: SessionId,
    frame: PendingResponse,
) -> Option<PendingResponse> {
    let outcome = match &frame {
        PendingResponse::Chunk(chunk) => dispatcher.enqueue_net_response(session, chunk),
        PendingResponse::Failed(code) => dispatcher.enqueue_net_error(session, *code),
    };
    match outcome {
        Enqueue::Accepted => None,
        Enqueue::Full => Some(frame),
    }
}

/// The box-facing run-loop adapter **with upstream ingress** (Sprint 11 Slice 1) — [`run_adapter`]'s
/// successor for the net path. Each tick: (1) **drain** the upstream worker's channel into the
/// push-queue (this runs on `Idle` too, so a response arriving while the box is silent still reaches the
/// box — the reactor's own bounded wait guarantees the tick, proven live in Sprint 14); (2) **route**
/// the box event (a
/// `NetPayloadAdmit` spawns the upstream call, or denies a second — serial bound); (3) **pump** the
/// reply streams to the box. The `Dispatcher` and the [`UpstreamIngress`] are owned here; the only
/// thread boundary is the ingress's channel.
///
/// [`run_adapter`]: crate::lifecycle::run_adapter
pub fn run_adapter_with_upstream<S, O>(
    mut dispatcher: Dispatcher,
    register: &'static [Route],
    vetted: &'static [ToolIdentity],
    spawn: S,
    mut on_verdict: O,
) -> impl FnMut(&mut dyn Reply, Event) -> ControlFlow<()>
where
    S: UpstreamSpawn,
    O: FnMut(SessionId, &Routed) -> ControlFlow<()>,
{
    let mut ingress = UpstreamIngress::new(spawn);
    move |reply, event| {
        ingress.drain(&mut dispatcher);
        let cf = match event {
            Event::Delivered(session, payload) => {
                let routed = dispatcher.route(session, &payload, register, vetted);
                if let Routed::NetPayloadAdmit(body) = &routed {
                    // Kick the upstream call (or deny a second). `body` is cloned once — byte-identical
                    // to the admitted bytes (bind-the-chain), moved to the worker.
                    ingress.on_admit(&mut dispatcher, session, body.clone());
                }
                on_verdict(session, &routed)
            }
            Event::Idle => ControlFlow::Continue(()),
        };
        dispatcher.pump_replies(reply);
        cf
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::mpsc::{SyncSender, sync_channel};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    use proto::{Message, NetAddress, NetPayload, Scheme, decode_message, encode_message, scan_safe};

    use crate::dispatch::REPLY_QUEUE_CAP;
    use crate::register::anthropic::VETTED_TOOLS;
    use crate::register::net::ROUTES;

    /// Establish the precondition `on_admit` now requires: an ADMITTED `net-address` for the session, so
    /// the dispatcher holds the matched `Route` the dial is derived from (Sprint 19). This is exactly how
    /// production reaches `on_admit` — the route is stored by `route()` when the address is admitted — so
    /// driving it here keeps these tests on the real path rather than injecting state behind it.
    fn admit_route(d: &mut Dispatcher, session: SessionId) {
        let verdict = d.route(session, &net_address_frame(), ROUTES, VETTED_TOOLS);
        assert!(matches!(verdict, Routed::NetRouteAdmit(_)), "the test's net-address must be admitted");
    }

    // ─── Sprint 16 Slice 2 — the credential descriptor's resolution policy ───────────────────────


    /// The descriptor names a variable and carries no value — and the registered one is the variable
    /// `serve.rs` actually reads. (Its const-validation and the inexpressibility of a literal secret are
    /// proven at COMPILE time by `tests/ui/*credential_source*`.)
    #[test]
    fn registered_credential_source_names_a_variable_never_a_value() {
        use crate::register::anthropic::CREDENTIAL_SOURCE;
        assert_eq!(CREDENTIAL_SOURCE.var_name(), "ANTHROPIC_API_KEY");
    }

    // A body the tier-1 recognizer ADMITS (same as the lifecycle receive-path test).
    pub(crate) const ADMIT_BODY: &[u8] =
        br#"{"model":"claude-x","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#;

    fn net_address_frame() -> Vec<u8> {
        encode_message(&Message::NetAddress(NetAddress {
            scheme: Scheme::Https,
            address: scan_safe("api.anthropic.com/v1/messages".replace('/', "\u{1f}").as_bytes()).unwrap(),
        }))
    }
    fn net_payload_frame(content: &[u8]) -> Vec<u8> {
        encode_message(&Message::NetPayload(NetPayload { more: false, content: content.to_vec() }))
    }

    // A one-frame Reply recorder; decodes net-response / net-error frames (8B in_reply_to + proto msg).
    // `blocked` makes `send` (the wire) WouldBlock, so a backpressuring box can be simulated (open, the
    // origination, never WouldBlocks — the ReplyStream contract).
    struct ReplyRec {
        alloc: transport::IdAllocator,
        reply_sid: Option<SessionId>,
        emitted: Vec<Vec<u8>>,
        blocked: bool,
    }
    impl ReplyRec {
        fn new() -> Self {
            ReplyRec {
                alloc: transport::IdAllocator::new(transport::Originator::High),
                reply_sid: None,
                emitted: Vec::new(),
                blocked: false,
            }
        }
        fn net_responses(&self, irt: u64) -> Vec<Vec<u8>> {
            self.emitted
                .iter()
                .filter_map(|f| {
                    (u64::from_be_bytes(f[0..8].try_into().unwrap()) == irt).then(|| {
                        match decode_message(&f[8..]) {
                            Ok(Message::NetResponse(nr)) => Some(nr.0),
                            _ => None,
                        }
                    })?
                })
                .collect()
        }
        fn net_error_code(&self, irt: u64) -> Option<proto::ErrorCode> {
            self.emitted.iter().find_map(|f| {
                if u64::from_be_bytes(f[0..8].try_into().unwrap()) != irt {
                    return None;
                }
                match decode_message(&f[8..]) {
                    Ok(Message::NetError(ne)) => Some(ne.0.code),
                    _ => None,
                }
            })
        }
    }
    impl Reply for ReplyRec {
        fn open(&mut self, payload: &[u8]) -> SessionId {
            let id = self.alloc.allocate();
            self.reply_sid = Some(id);
            self.emitted.push(payload.to_vec());
            id
        }
        fn send(&mut self, _id: SessionId, payload: &[u8]) -> Result<(), transport::SendError> {
            if self.blocked {
                return Err(transport::SendError::WouldBlock); // the box wire is not draining
            }
            self.emitted.push(payload.to_vec());
            Ok(())
        }
    }

    // A threadless spawner for deterministic LOGIC tests: hands the created `Sender` to the test via a
    // shared cell, counts spawns, and uses no thread (the test IS the worker).
    struct ScriptedSpawner {
        cell: Rc<RefCell<Option<SyncSender<UpstreamFrame>>>>,
        count: Rc<Cell<usize>>,
    }
    impl UpstreamSpawn for ScriptedSpawner {
        fn spawn(&self, _session: SessionId, _destination: Route, _body: Vec<u8>) -> UpstreamHandle {
            self.count.set(self.count.get() + 1);
            let (tx, rx) = sync_channel(UPSTREAM_CHANNEL_CAP);
            *self.cell.borrow_mut() = Some(tx);
            UpstreamHandle { rx, join: None }
        }
    }

    // A real-thread spawner for the isolation/lifecycle/backpressure gates: runs `body(tx)` on a thread.
    struct ThreadSpawner<F: Fn(SyncSender<UpstreamFrame>) + Send + Clone + 'static> {
        body: F,
    }
    impl<F: Fn(SyncSender<UpstreamFrame>) + Send + Clone + 'static> UpstreamSpawn for ThreadSpawner<F> {
        fn spawn(&self, _session: SessionId, _destination: Route, _body: Vec<u8>) -> UpstreamHandle {
            let (tx, rx) = sync_channel(UPSTREAM_CHANNEL_CAP);
            let body = self.body.clone();
            let join = std::thread::spawn(move || body(tx));
            UpstreamHandle { rx, join: Some(join) }
        }
    }

    /// THE QUIESCENT-ARRIVAL GATE: box goes silent, THEN the response arrives, THEN a simulated wake
    /// (an `Idle` tick) drains it to the box. Not merely "drain while the box is still sending".
    ///
    /// The `Idle` tick here stands in for the reactor's own re-check. **There was never a deadlock to
    /// fix**: Sprint 14 established that the reactor's bounded wait already services a quiescent arrival
    /// unaided (~100µs live, inside its ~5ms ceiling). This test pins the DRAIN behaviour given a tick;
    /// that the tick actually happens is the reactor's, proven separately on hardware.
    #[test]
    fn quiescent_arrival_reaches_the_box_on_a_simulated_wake() {
        let cell = Rc::new(RefCell::new(None));
        let count = Rc::new(Cell::new(0));
        let spawner = ScriptedSpawner { cell: cell.clone(), count: count.clone() };
        let mut ingress = UpstreamIngress::new(spawner);
        let mut d = Dispatcher::new();
        let s = SessionId(1);

        // A recognizer-admitted request kicks the upstream call (worker's tx now in the cell).
        admit_route(&mut d, s);
        ingress.on_admit(&mut d, s, ADMIT_BODY.to_vec());
        assert!(ingress.is_in_flight());
        assert_eq!(count.get(), 1);

        // Box goes SILENT and the worker has produced nothing yet: a drain delivers nothing, stays live.
        ingress.drain(&mut d);
        assert!(ingress.is_in_flight(), "empty-but-open: not evicted (waiting for the response)");
        let mut reply = ReplyRec::new();
        d.pump_replies(&mut reply);
        assert!(reply.emitted.is_empty(), "nothing reaches the box while the worker is silent");

        // NOW the upstream response arrives, THEN the simulated wake (drain + pump).
        let tx = cell.borrow_mut().take().expect("worker spawned");
        tx.send(UpstreamFrame::Chunk(b"chunk-1".to_vec())).unwrap();
        tx.send(UpstreamFrame::Chunk(b"chunk-2".to_vec())).unwrap();
        drop(tx); // worker done → channel closes (EOF)
        ingress.drain(&mut d); // the simulated wake drains the arrived response + observes EOF
        assert!(!ingress.is_in_flight(), "EOF completes and evicts the call");
        d.pump_replies(&mut reply);
        assert_eq!(
            reply.net_responses(s.0),
            vec![b"chunk-1".to_vec(), b"chunk-2".to_vec()],
            "the response reached the box, verbatim + in order, stamped with the request session",
        );
    }

    /// SERIAL BOUND = 1: a second admit while one call is in flight is DENIED net-error(Unavailable),
    /// and NO second worker/channel is spawned.
    #[test]
    fn second_concurrent_admit_denied_net_error_no_second_worker() {
        let cell = Rc::new(RefCell::new(None));
        let count = Rc::new(Cell::new(0));
        let mut ingress =
            UpstreamIngress::new(ScriptedSpawner { cell, count: count.clone() });
        let mut d = Dispatcher::new();

        admit_route(&mut d, SessionId(1));
        ingress.on_admit(&mut d, SessionId(1), ADMIT_BODY.to_vec());
        assert_eq!(count.get(), 1, "first admit spawned the worker");

        // Second admit on a different session, while the first is in flight → deny, no second spawn.
        admit_route(&mut d, SessionId(2));
        ingress.on_admit(&mut d, SessionId(2), ADMIT_BODY.to_vec());
        assert_eq!(count.get(), 1, "NO second worker/channel spawned (serial bound = 1)");

        let mut reply = ReplyRec::new();
        d.pump_replies(&mut reply);
        assert_eq!(
            reply.net_error_code(2),
            Some(proto::ErrorCode::Unavailable),
            "the second request is denied net-error(Unavailable), immediately + honestly",
        );
    }

    /// WORKER CLEANUP: a real worker thread that sends then returns (drops its Sender) → the loop
    /// observes EOF, joins the thread (no leak), and clears the in-flight slot.
    #[test]
    fn worker_completion_closes_channel_and_reaps_thread() {
        let spawner = ThreadSpawner {
            body: |tx: SyncSender<UpstreamFrame>| {
                let _ = tx.send(UpstreamFrame::Chunk(b"a".to_vec()));
                let _ = tx.send(UpstreamFrame::Chunk(b"b".to_vec()));
                // returns → tx dropped → channel closes
            },
        };
        let mut ingress = UpstreamIngress::new(spawner);
        let mut d = Dispatcher::new();
        let s = SessionId(1);
        admit_route(&mut d, s);
        ingress.on_admit(&mut d, s, ADMIT_BODY.to_vec());

        // Drive drains until EOF completes the call (the worker runs async; bounded spin).
        let mut ticks = 0;
        while ingress.is_in_flight() {
            ticks += 1;
            assert!(ticks < 100_000, "worker never completed / thread not reaped");
            ingress.drain(&mut d);
        }
        // is_in_flight == false proves drain observed Disconnected AND joined the thread (no leak).
        let mut reply = ReplyRec::new();
        d.pump_replies(&mut reply);
        assert_eq!(reply.net_responses(s.0), vec![b"a".to_vec(), b"b".to_vec()]);
    }

    /// A HUNG worker does NOT block the loop: drain returns promptly (try_recv, non-blocking) and the
    /// call stays in flight; the loop keeps working. Released via a barrier so nothing leaks.
    #[test]
    fn hung_worker_does_not_block_the_loop() {
        let release = Arc::new(Barrier::new(2));
        let worker_release = release.clone();
        let spawner = ThreadSpawner {
            body: move |tx: SyncSender<UpstreamFrame>| {
                worker_release.wait(); // hang until the test releases us — never sends
                drop(tx);
            },
        };
        let mut ingress = UpstreamIngress::new(spawner);
        let mut d = Dispatcher::new();
        admit_route(&mut d, SessionId(1));
        ingress.on_admit(&mut d, SessionId(1), ADMIT_BODY.to_vec());

        // The worker is hung (blocked on the barrier). drain must return without blocking, many times.
        for _ in 0..100 {
            ingress.drain(&mut d); // non-blocking — returns Empty, does not stall
        }
        assert!(ingress.is_in_flight(), "the hung call stays in flight; the loop was never blocked");

        // Release the worker so it exits; then drain observes EOF and reaps it (no leak at test end).
        release.wait();
        let mut ticks = 0;
        while ingress.is_in_flight() {
            ticks += 1;
            assert!(ticks < 100_000);
            ingress.drain(&mut d);
        }
    }

    /// The channel is BOUNDED with backpressure: a worker producing faster than the loop drains cannot
    /// send more than [`UPSTREAM_CHANNEL_CAP`] before its `send` blocks — never an unbounded buffer.
    #[test]
    fn channel_is_bounded_and_backpressures_a_fast_worker() {
        let sent = Arc::new(AtomicUsize::new(0));
        let worker_sent = sent.clone();
        let target = UPSTREAM_CHANNEL_CAP + 8; // wants to send more than the cap
        let spawner = ThreadSpawner {
            body: move |tx: SyncSender<UpstreamFrame>| {
                for _ in 0..target {
                    if tx.send(UpstreamFrame::Chunk(vec![0u8])).is_err() {
                        break;
                    }
                    worker_sent.fetch_add(1, Ordering::SeqCst);
                }
            },
        };
        let mut ingress = UpstreamIngress::new(spawner);
        let mut d = Dispatcher::new();
        admit_route(&mut d, SessionId(1));
        ingress.on_admit(&mut d, SessionId(1), ADMIT_BODY.to_vec());

        // Without any drain, the worker fills the bounded channel and BLOCKS — it can never get past
        // CAP + 1 in flight (CAP buffered + 1 blocked mid-send). Spin briefly to let it fill.
        let mut spins = 0;
        while sent.load(Ordering::SeqCst) < UPSTREAM_CHANNEL_CAP {
            spins += 1;
            assert!(spins < 10_000_000, "worker never filled the bounded channel");
            std::hint::spin_loop();
        }
        // Give the blocked worker a moment; it must NOT exceed CAP+1 (bounded, no unbounded growth).
        for _ in 0..1000 {
            std::hint::spin_loop();
        }
        assert!(
            sent.load(Ordering::SeqCst) <= UPSTREAM_CHANNEL_CAP + 1,
            "a fast worker is backpressured at the cap, never buffering unbounded",
        );

        // Drain AND pump everything so the worker unblocks, finishes, and is reaped (no leak). The pump
        // is required now that `drain` is bounded by the reply queue (Sprint 13 Slice 1.5): without a box
        // draining the replies, `drain` correctly stops at REPLY_QUEUE_CAP and backpressures — it no
        // longer grows the reply queue unbounded (the old, latent-defect behavior this test relied on).
        let mut reply = ReplyRec::new();
        let mut ticks = 0;
        while ingress.is_in_flight() {
            ticks += 1;
            assert!(ticks < 100_000);
            ingress.drain(&mut d);
            d.pump_replies(&mut reply);
        }
        assert_eq!(sent.load(Ordering::SeqCst), target, "once drained + pumped, the worker delivers all frames");
        assert_eq!(reply.net_responses(1).len(), target, "every frame reached the box, none lost");
    }

    /// **SPRINT 19, FINAL STATE #4a — the destination reaches the dial FROM THE REGISTER.**
    ///
    /// Proven below `route()`'s full pipeline on purpose: the originally-planned "register a second route
    /// and watch the dial follow it" test is UNBUILDABLE, because U4's Stage 1 hardcodes one envelope and
    /// denies any second domain before a payload can ever be admitted (that hardcoding is deliberate
    /// defense-in-depth — `anthropic.rs`). So the plumbing is proven directly: give the session an
    /// admitted `Route`, call `on_admit`, and assert the spawner received *that* route as its destination
    /// rather than a value the client held independently.
    ///
    /// This is the wiring half. The structural half — that there is no longer anywhere for a dial to
    /// diverge *to* — is carried by the code's shape, not a test: `AnthropicClient` has no `endpoint`
    /// field and `ANTHROPIC_ENDPOINT` no longer exists, which the compiler enforces.
    #[test]
    fn on_admit_dials_the_route_the_register_matched() {
        use std::cell::RefCell;
        use std::rc::Rc;

        /// Records the destination it was handed, and spawns nothing.
        struct RecordingSpawner {
            seen: Rc<RefCell<Option<Route>>>,
        }
        impl UpstreamSpawn for RecordingSpawner {
            fn spawn(&self, _session: SessionId, destination: Route, _body: Vec<u8>) -> UpstreamHandle {
                *self.seen.borrow_mut() = Some(destination);
                let (_tx, rx) = sync_channel::<UpstreamFrame>(1); // _tx dropped → immediate EOF
                UpstreamHandle { rx, join: None }
            }
        }

        let seen = Rc::new(RefCell::new(None));
        let mut ingress = UpstreamIngress::new(RecordingSpawner { seen: seen.clone() });
        let mut d = Dispatcher::new();
        let s = SessionId(1);
        admit_route(&mut d, s);

        // What the register actually matched for this session — the value the dial must use.
        let matched = d.route_for(s).expect("an admitted address stores its route");
        ingress.on_admit(&mut d, s, ADMIT_BODY.to_vec());

        assert_eq!(
            seen.borrow().map(|r| r.as_str()),
            Some(matched.as_str()),
            "the dial's destination must BE the register's matched route, not a value held elsewhere",
        );
        assert_eq!(
            matched.as_str(),
            "https://api.anthropic.com/v1/messages",
            "sanity: the matched route is the register's entry, so the assertion above is not vacuous",
        );
    }

    /// **FAIL CLOSED WITH NO ROUTE.** Unreachable in production (`NetPayloadNoRoute` blocks it), but the
    /// guard must not invent a destination if it is ever reached: no dial, and the box is told rather
    /// than left waiting (`net.md` §4 — anything that is not an upstream response is a net-error).
    #[test]
    fn on_admit_without_a_matched_route_fails_closed_and_never_dials() {
        use std::cell::Cell;
        use std::rc::Rc;

        struct CountingSpawner {
            calls: Rc<Cell<usize>>,
        }
        impl UpstreamSpawn for CountingSpawner {
            fn spawn(&self, _session: SessionId, _destination: Route, _body: Vec<u8>) -> UpstreamHandle {
                self.calls.set(self.calls.get() + 1);
                let (_tx, rx) = sync_channel::<UpstreamFrame>(1);
                UpstreamHandle { rx, join: None }
            }
        }

        let calls = Rc::new(Cell::new(0));
        let mut ingress = UpstreamIngress::new(CountingSpawner { calls: calls.clone() });
        let mut d = Dispatcher::new(); // deliberately NO admitted address for this session
        let s = SessionId(1);
        ingress.on_admit(&mut d, s, ADMIT_BODY.to_vec());

        assert_eq!(calls.get(), 0, "no route ⇒ NO dial is attempted");
        let mut reply = ReplyRec::new();
        d.pump_replies(&mut reply);
        assert_eq!(
            reply.net_error_code(s.0),
            Some(proto::ErrorCode::Unavailable),
            "the box is told the exchange failed, not left waiting on a dial that never happened",
        );
    }

    /// **SPRINT 18, FINDING B — a PANICKED worker must not present as a completed response.** The
    /// channel closing means the worker returned; it does NOT distinguish "streamed the whole response"
    /// from "crashed partway". Before the fix the join result was discarded and the reply closed
    /// unconditionally, so a panic reached the box as a cleanly-terminated (silently truncated) response
    /// — indistinguishable from a genuinely short one. `net.md` §4: anything that is not an upstream
    /// response is a net-error.
    #[test]
    fn a_panicked_worker_yields_a_net_error_not_a_clean_close() {
        let spawner = ThreadSpawner {
            body: |tx: SyncSender<UpstreamFrame>| {
                // Deliver part of a response, then die. The box has bytes AND an unfinished exchange.
                let _ = tx.send(UpstreamFrame::Chunk(b"partial-".to_vec()));
                panic!("worker crashed mid-response");
            },
        };
        let mut ingress = UpstreamIngress::new(spawner);
        let mut d = Dispatcher::new();
        let s = SessionId(1);
        admit_route(&mut d, s);
        ingress.on_admit(&mut d, s, ADMIT_BODY.to_vec());

        let mut ticks = 0;
        while ingress.is_in_flight() {
            ticks += 1;
            assert!(ticks < 100_000, "the panicked worker was never reaped");
            ingress.drain(&mut d);
        }
        let mut reply = ReplyRec::new();
        d.pump_replies(&mut reply);

        assert_eq!(
            reply.net_error_code(s.0),
            Some(proto::ErrorCode::Unavailable),
            "a crashed worker must be reported to the box as a net-error, never as a clean completion",
        );
    }

    /// **SPRINT 13 SLICE 1.5 — the data-loss regression** (the exact test class that was missing, which
    /// let the drop hide, checked=0): under a BACKPRESSURING box (its wire WouldBlocks), a response
    /// larger than the reply-queue cap must be delivered COMPLETE, IN ORDER, with NONE LOST. Before the
    /// fix, `drain` pulled each chunk off the channel and handed it to a full queue that refused it,
    /// dropping it — here every chunk survives, held across the reply queue, the one-slot pending, the
    /// mpsc, and the blocked worker, then flows once the box drains.
    #[test]
    fn backpressuring_box_never_loses_a_response_chunk() {
        const TOTAL: usize = REPLY_QUEUE_CAP * 3 + 7; // >> both the reply-queue cap and the mpsc cap
        let spawner = ThreadSpawner {
            body: |tx: SyncSender<UpstreamFrame>| {
                for i in 0..TOTAL {
                    // Distinct payloads so an out-of-order or lost chunk is caught, not just a count.
                    if tx.send(UpstreamFrame::Chunk(alloc_chunk(i))).is_err() {
                        return;
                    }
                }
                // returns → tx dropped → EOF
            },
        };
        let mut ingress = UpstreamIngress::new(spawner);
        let mut d = Dispatcher::new();
        let s = SessionId(1);
        admit_route(&mut d, s);
        ingress.on_admit(&mut d, s, ADMIT_BODY.to_vec());
        let mut reply = ReplyRec::new();

        // Phase 1: the box WIRE is BLOCKED. Drive many drain+pump ticks: the reply queue fills, `drain`
        // retains + stops, the mpsc fills, the worker blocks on `send` — nothing is lost, nothing spins.
        reply.blocked = true;
        for _ in 0..10_000 {
            ingress.drain(&mut d);
            d.pump_replies(&mut reply);
        }
        assert!(ingress.is_in_flight(), "the response is HELD under backpressure, not completed or lost");
        assert!(
            reply.net_responses(s.0).len() <= 1,
            "a blocked wire delivered nothing beyond the first frame (open cannot WouldBlock)",
        );

        // Phase 2: RELEASE the wire. Drive to completion — every chunk arrives, verbatim + in order.
        reply.blocked = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while ingress.is_in_flight() {
            assert!(std::time::Instant::now() < deadline, "did not complete after releasing backpressure");
            ingress.drain(&mut d);
            d.pump_replies(&mut reply);
        }
        d.pump_replies(&mut reply); // drain any tail

        let got = reply.net_responses(s.0);
        let expected: Vec<Vec<u8>> = (0..TOTAL).map(alloc_chunk).collect();
        assert_eq!(got.len(), TOTAL, "every response chunk delivered — NONE lost to a full queue");
        assert_eq!(got, expected, "delivered verbatim AND in order across the backpressure");
    }

    // A distinct per-index chunk payload (order + loss are detectable, not just a count).
    fn alloc_chunk(i: usize) -> Vec<u8> {
        format!("chunk-{i}").into_bytes()
    }

    /// The ADAPTER wiring: a real admit sequence (`net-address` then `net-payload`) drives on_admit, and
    /// the quiescent-arrival response reaches the box — end to end through `run_adapter_with_upstream`.
    #[test]
    fn adapter_admit_sequence_spawns_and_delivers_the_response() {
        let cell = Rc::new(RefCell::new(None));
        let count = Rc::new(Cell::new(0));
        let mut adapter = run_adapter_with_upstream(
            Dispatcher::new(),
            ROUTES,
            VETTED_TOOLS,
            ScriptedSpawner { cell: cell.clone(), count: count.clone() },
            |_s, _r| ControlFlow::Continue(()),
        );
        let s = SessionId(1);
        let mut reply = ReplyRec::new();

        let _ = adapter(&mut reply, Event::Delivered(s, net_address_frame()));
        let _ = adapter(&mut reply, Event::Delivered(s, net_payload_frame(ADMIT_BODY)));
        assert_eq!(count.get(), 1, "the recognizer-admitted net-payload spawned the upstream call");

        // Box silent; the response arrives; a simulated wake (Idle tick) delivers it.
        let tx = cell.borrow_mut().take().expect("spawned");
        tx.send(UpstreamFrame::Chunk(b"resp".to_vec())).unwrap();
        drop(tx);
        let _ = adapter(&mut reply, Event::Idle);
        assert_eq!(reply.net_responses(s.0), vec![b"resp".to_vec()]);
    }

    // ─── Sprint 11 Slice 2 — the real Anthropic client ───────────────────────────────────────────


}
