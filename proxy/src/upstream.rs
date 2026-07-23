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
//! [`UpstreamSpawn`] (a controllable double); Slice 2 drops in the real `rustls`/`ureq` worker behind
//! the same seam. The one thing Slice 1 does **not** prove — that the real reactor *wakes* to run a
//! tick (and thus drain) when a response arrives during a real quiescent park — is held pending Task 20
//! (the reactor's bounded-park cap). The drain logic *given a wake* is proven here; a test's driving of
//! an `Idle` tick after a quiescent arrival IS the simulated wake.

use std::io::Read;
use std::ops::ControlFlow;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use directpath::Event;
use proto::ErrorCode;
use transport::SessionId;

use crate::dispatch::{Dispatcher, Routed};
use crate::nic::Reply;
use crate::vocab::{Route, ToolIdentity};

/// The `anthropic-version` header the proxy sends on the upstream call — a proxy-owned constant (the box
/// cannot influence it; the wire conveys no HTTP headers). Provisional, marked for review.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// The bounded global timeout on the upstream call — a hung/slow upstream never leaks a thread or a
/// channel forever (it fails `Timeout` → `net-error`, the worker exits, the channel closes). LLM
/// responses can be long; provisional scalar, marked for review.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(120);

/// The chunk size the response body is streamed in — bounded per-read memory (the outbound analog of
/// `fs_read::READ_CHUNK`). Each read becomes one verbatim `net-response` frame.
const RESPONSE_CHUNK: usize = 16 * 1024;

/// The bounded depth (frames) of the worker→loop channel. A full channel **backpressures** the worker's
/// `send` (it blocks until the loop drains) — never an unbounded buffer, the outbound analog of the
/// push-queue / reassembly caps. Provisional scalar, marked for review.
pub const UPSTREAM_CHANNEL_CAP: usize = 64;

/// One item a worker sends to the box-facing loop over the channel (Sprint 11 Slice 2). Owned data —
/// the only thing that crosses the thread boundary.
pub enum UpstreamFrame {
    /// One **verbatim** chunk of the upstream response body → relayed to the box as `net-response`
    /// (MB3: never inspected, neutralized, or transformed — `cloud_relay::relay_response`).
    Chunk(Vec<u8>),
    /// The upstream call **failed** (timeout / unreachable / TLS / HTTP status) → the box is told
    /// `net-error(code)` (the proxy's own category; no upstream body, no credential — no leak).
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
/// [`UpstreamHandle`]. The real `rustls`/`ureq` worker (Slice 2) implements this; tests inject a
/// controllable double. **`body` is the exact admitted bytes — POSTed verbatim, no re-encode**
/// (bind-the-chain; realized on the real call in Slice 2).
pub trait UpstreamSpawn {
    fn spawn(&self, session: SessionId, body: Vec<u8>) -> UpstreamHandle;
}

/// One in-flight upstream call (serial bound = 1).
struct InFlight {
    session: SessionId,
    rx: Receiver<UpstreamFrame>,
    join: Option<JoinHandle<()>>,
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
    fn on_admit(&mut self, dispatcher: &mut Dispatcher, session: SessionId, body: Vec<u8>) {
        if self.inflight.is_some() {
            dispatcher.enqueue_net_error(session, ErrorCode::Unavailable);
            return;
        }
        let handle = self.spawn.spawn(session, body);
        self.inflight = Some(InFlight { session, rx: handle.rx, join: handle.join });
    }

    /// Drain any response frames that have arrived from the worker into the push-queue (called **every
    /// tick**). On channel close (the worker dropped its `Sender` = end-of-response), close the reply
    /// stream, **join** the worker thread (no leak), and clear the in-flight slot. **Non-blocking**
    /// (`try_recv`): a slow or hung worker simply yields nothing this tick and never stalls the loop.
    fn drain(&mut self, dispatcher: &mut Dispatcher) {
        let Some(session) = self.inflight.as_ref().map(|f| f.session) else {
            return;
        };
        loop {
            // Borrow self.inflight only for the try_recv call (the Result is owned — no held borrow).
            let recv = self.inflight.as_ref().expect("checked above").rx.try_recv();
            match recv {
                // A response chunk → relayed verbatim (net-response, MB3); a failure → net-error(code).
                Ok(UpstreamFrame::Chunk(chunk)) => {
                    dispatcher.enqueue_net_response(session, &chunk);
                }
                Ok(UpstreamFrame::Failed(code)) => {
                    dispatcher.enqueue_net_error(session, code);
                }
                Err(TryRecvError::Empty) => return, // nothing right now — the worker is still running
                Err(TryRecvError::Disconnected) => {
                    // End of response: close the reply stream (drains + evicts), reap the worker thread.
                    dispatcher.close_reply(session);
                    if let Some(done) = self.inflight.take()
                        && let Some(join) = done.join
                    {
                        let _ = join.join(); // no leaked thread
                    }
                    return;
                }
            }
        }
    }

    /// **Test/observability**: is a call currently in flight?
    #[must_use]
    pub fn is_in_flight(&self) -> bool {
        self.inflight.is_some()
    }
}

/// The box-facing run-loop adapter **with upstream ingress** (Sprint 11 Slice 1) — [`run_adapter`]'s
/// successor for the net path. Each tick: (1) **drain** the upstream worker's channel into the
/// push-queue (this runs on `Idle` too, so a response arriving while the box is silent still reaches the
/// box — *given* the reactor wakes to run the tick, which is Task 20); (2) **route** the box event (a
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

// ─── Sprint 11 Slice 2 — the real Anthropic upstream client (below the seam) ──────────────────────────
//
// `rustls` + `ureq` are used ONLY here (below the `NicRun`/reply seam), never in the box-reachable core
// (tier-3, off the L10 review budget). No proxy-owned `unsafe` (MB6). The box's request never carries or
// influences the credential; the credential never reaches a box-facing frame or a log.

/// The upstream credential (the Anthropic API key) — the concrete form of Sprint 8's
/// [`Authority::Credential`](crate::lifecycle::Authority::Credential). Trusted-side only. Its `Debug` is
/// **redacted** so it can never reach a log or diagnostic; nothing echoes it to the box.
pub struct Credential(String);

impl Credential {
    #[must_use]
    pub fn new(key: String) -> Self {
        Credential(key)
    }
}

impl core::fmt::Debug for Credential {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
}

/// The real upstream client (Sprint 11 Slice 2): POSTs the recognizer-admitted body **verbatim** to the
/// Anthropic Messages API over `rustls` TLS via `ureq`, and streams the response **verbatim** back
/// through the [`UpstreamSpawn`] seam into the Sprint-10 push-queue. `endpoint` is a composition-root
/// parameter (production `https://api.anthropic.com/v1/messages`; a test points it at a local server).
pub struct AnthropicClient {
    credential: Credential,
    endpoint: String,
}

impl AnthropicClient {
    #[must_use]
    pub fn new(credential: Credential, endpoint: String) -> Self {
        AnthropicClient { credential, endpoint }
    }
}

impl UpstreamSpawn for AnthropicClient {
    fn spawn(&self, _session: SessionId, body: Vec<u8>) -> UpstreamHandle {
        let (tx, rx) = sync_channel(UPSTREAM_CHANNEL_CAP);
        let endpoint = self.endpoint.clone();
        let key = self.credential.0.clone();
        let join = std::thread::spawn(move || {
            match upstream_call(&endpoint, &key, &body) {
                Ok(reader) => stream_response(reader, &tx),
                Err(code) => {
                    let _ = tx.send(UpstreamFrame::Failed(code));
                }
            }
            // `tx` dropped on return → channel closes → the loop observes end-of-response.
        });
        UpstreamHandle { rx, join: Some(join) }
    }
}

/// The blocking upstream call — **the network leg** (the live round-trip is the held item, pending
/// Task 20 for the reactor wake + a real credential). POSTs `body` **byte-verbatim** (`send(body)` — no
/// re-encode between the tier-1 vet and the wire; bind-the-chain) with the credential + version headers,
/// and returns a streaming reader over the response body, or a mapped [`ErrorCode`].
fn upstream_call(endpoint: &str, key: &str, body: &[u8]) -> Result<impl Read, ErrorCode> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(UPSTREAM_TIMEOUT))
        .build()
        .new_agent();
    match agent
        .post(endpoint)
        .header("x-api-key", key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .send(body)
    {
        Ok(response) => Ok(response.into_body().into_reader()),
        Err(e) => Err(map_ureq_error(&e)),
    }
}

/// Stream a response body **verbatim** into the channel as [`UpstreamFrame::Chunk`]s (MB3 — never
/// inspected, neutralized, or transformed). Bounded per-read memory ([`RESPONSE_CHUNK`]). A read error
/// mid-stream → `Failed(Unavailable)`. Stops early if the loop hung up (the box session closed).
fn stream_response(mut reader: impl Read, tx: &SyncSender<UpstreamFrame>) {
    let mut buf = [0u8; RESPONSE_CHUNK];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return, // EOF — the caller drops `tx` (channel close = end-of-response)
            Ok(n) => {
                if tx.send(UpstreamFrame::Chunk(buf[..n].to_vec())).is_err() {
                    return; // the receiver (loop) is gone — stop
                }
            }
            Err(_) => {
                let _ = tx.send(UpstreamFrame::Failed(ErrorCode::Unavailable));
                return;
            }
        }
    }
}

/// Map a `ureq` transport error to the proxy's own [`ErrorCode`] category (no upstream detail, no
/// credential — the box learns only *that* it failed and roughly why). Timeout → `Timeout`;
/// host-not-found / connection-refused → `Unreachable`; everything else (HTTP status, TLS, i/o,
/// protocol) → `Unavailable`.
fn map_ureq_error(e: &ureq::Error) -> ErrorCode {
    use std::io::ErrorKind;
    match e {
        ureq::Error::Timeout(_) => ErrorCode::Timeout,
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => ErrorCode::Unreachable,
        // A refused / reset / aborted connection is "couldn't reach the upstream" → Unreachable.
        ureq::Error::Io(io)
            if matches!(
                io.kind(),
                ErrorKind::ConnectionRefused
                    | ErrorKind::ConnectionReset
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::NotConnected
            ) =>
        {
            ErrorCode::Unreachable
        }
        // Everything else (HTTP status, TLS, other i/o, protocol) → a generic Unavailable.
        _ => ErrorCode::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::mpsc::{SyncSender, sync_channel};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    use proto::{Message, NetAddress, NetPayload, Scheme, decode_message, encode_message, scan_safe};

    use crate::register::anthropic::VETTED_TOOLS;
    use crate::register::net::ROUTES;

    // A body the tier-1 recognizer ADMITS (same as the lifecycle receive-path test).
    const ADMIT_BODY: &[u8] =
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
    struct ReplyRec {
        alloc: transport::IdAllocator,
        reply_sid: Option<SessionId>,
        emitted: Vec<Vec<u8>>,
    }
    impl ReplyRec {
        fn new() -> Self {
            ReplyRec {
                alloc: transport::IdAllocator::new(transport::Originator::High),
                reply_sid: None,
                emitted: Vec::new(),
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
        fn spawn(&self, _session: SessionId, _body: Vec<u8>) -> UpstreamHandle {
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
        fn spawn(&self, _session: SessionId, _body: Vec<u8>) -> UpstreamHandle {
            let (tx, rx) = sync_channel(UPSTREAM_CHANNEL_CAP);
            let body = self.body.clone();
            let join = std::thread::spawn(move || body(tx));
            UpstreamHandle { rx, join: Some(join) }
        }
    }

    /// THE QUIESCENT-ARRIVAL GATE: box goes silent, THEN the response arrives, THEN a simulated wake
    /// (an `Idle` tick) drains it to the box — the deadlock case Task 20 exists to fix, with the wake
    /// simulated (the real wake is the held item). Not merely "drain while the box is still sending."
    #[test]
    fn quiescent_arrival_reaches_the_box_on_a_simulated_wake() {
        let cell = Rc::new(RefCell::new(None));
        let count = Rc::new(Cell::new(0));
        let spawner = ScriptedSpawner { cell: cell.clone(), count: count.clone() };
        let mut ingress = UpstreamIngress::new(spawner);
        let mut d = Dispatcher::new();
        let s = SessionId(1);

        // A recognizer-admitted request kicks the upstream call (worker's tx now in the cell).
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

        ingress.on_admit(&mut d, SessionId(1), ADMIT_BODY.to_vec());
        assert_eq!(count.get(), 1, "first admit spawned the worker");

        // Second admit on a different session, while the first is in flight → deny, no second spawn.
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

        // Drain everything so the worker unblocks, finishes, and is reaped (no leak).
        let mut ticks = 0;
        while ingress.is_in_flight() {
            ticks += 1;
            assert!(ticks < 100_000);
            ingress.drain(&mut d);
        }
        assert_eq!(sent.load(Ordering::SeqCst), target, "once drained, the worker delivers all frames");
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

    use std::io::{BufRead, BufReader, Cursor, Write};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    /// MB3 (unit): `stream_response` relays the response body VERBATIM — the concatenated `Chunk`s equal
    /// the input bytes exactly, never inspected or transformed. Driven over an in-memory `Cursor` (no
    /// network).
    #[test]
    fn stream_response_relays_verbatim() {
        let input: Vec<u8> = (0..(RESPONSE_CHUNK * 2 + 500)).map(|i| (i % 251) as u8).collect();
        let (tx, rx) = sync_channel(UPSTREAM_CHANNEL_CAP * 4);
        stream_response(Cursor::new(input.clone()), &tx);
        drop(tx);
        let mut got = Vec::new();
        while let Ok(frame) = rx.recv() {
            match frame {
                UpstreamFrame::Chunk(c) => got.extend_from_slice(&c),
                UpstreamFrame::Failed(_) => panic!("no failure expected on a clean read"),
            }
        }
        assert_eq!(got, input, "the response is streamed byte-verbatim (MB3)");
    }

    /// Error mapping (real ureq): a connection to a closed port maps to `net-error(Unreachable)` over the
    /// seam — the real ureq transport-error path, no mock. (The live round-trip to Anthropic itself stays
    /// held pending Task 20 + a real credential.)
    #[test]
    fn upstream_connection_refused_maps_to_net_error_unreachable() {
        // 127.0.0.1:1 — nothing listens; connect is refused immediately.
        let client = AnthropicClient::new(Credential::new("sk".into()), "http://127.0.0.1:1/v1/messages".into());
        let handle = client.spawn(SessionId(1), ADMIT_BODY.to_vec());
        let mut saw = None;
        while let Ok(frame) = handle.rx.recv_timeout(Duration::from_secs(10)) {
            if let UpstreamFrame::Failed(code) = frame {
                saw = Some(code);
            }
        }
        assert_eq!(saw, Some(proto::ErrorCode::Unreachable), "a refused connection → net-error(Unreachable)");
        handle.join.unwrap().join().unwrap();
    }

    /// A minimal one-shot HTTP/1.1 server (handles `Expect: 100-continue`). Returns its URL and a channel
    /// delivering the captured `(request_body, raw_headers)`.
    fn mock_server(response_body: &'static [u8]) -> (String, std::sync::mpsc::Receiver<(Vec<u8>, String)>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/v1/messages");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let mut headers = String::new();
            let mut content_length = 0usize;
            let mut expect_continue = false;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                let low = line.to_ascii_lowercase();
                if let Some(v) = low.strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
                if low.starts_with("expect:") && low.contains("100-continue") {
                    expect_continue = true;
                }
                headers.push_str(&line);
            }
            if expect_continue {
                writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").unwrap();
                writer.flush().unwrap();
            }
            let mut body = vec![0u8; content_length];
            std::io::Read::read_exact(&mut reader, &mut body).unwrap();
            let _ = tx.send((body, headers));
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", response_body.len());
            writer.write_all(resp.as_bytes()).unwrap();
            writer.write_all(response_body).unwrap();
            writer.flush().unwrap();
        });
        (url, rx)
    }

    /// BIND-THE-CHAIN + MB3 + NO-LEAK against a real ureq POST to a local server: the POSTed body is
    /// byte-identical to the admitted body (no re-encode); the credential is sent upstream as `x-api-key`
    /// (correct) but NEVER appears in the relayed response; the response streams back verbatim.
    #[test]
    fn real_post_is_verbatim_credential_sent_upstream_not_leaked_to_box() {
        let (url, captured) = mock_server(b"the-upstream-response-body");
        let client = AnthropicClient::new(Credential::new("sk-secret-key".into()), url);
        let handle = client.spawn(SessionId(1), ADMIT_BODY.to_vec());

        let (req_body, headers) = captured.recv_timeout(Duration::from_secs(10)).unwrap();
        // BIND-THE-CHAIN: the POST body IS the admitted bytes, verbatim (no re-encode between vet + send).
        assert_eq!(req_body, ADMIT_BODY, "the POST body is byte-identical to the admitted body");
        // The credential is sent UPSTREAM (correct) as x-api-key + the version header is present.
        let low = headers.to_ascii_lowercase();
        assert!(low.contains("x-api-key: sk-secret-key"), "credential sent upstream as x-api-key");
        assert!(low.contains("anthropic-version:"), "the version header is sent");

        // The worker streams the response back VERBATIM (MB3).
        let mut got = Vec::new();
        while let Ok(frame) = handle.rx.recv_timeout(Duration::from_secs(10)) {
            match frame {
                UpstreamFrame::Chunk(c) => got.extend_from_slice(&c),
                UpstreamFrame::Failed(code) => panic!("unexpected failure: {code:?}"),
            }
        }
        assert_eq!(got, b"the-upstream-response-body", "response relayed to the box verbatim");
        // NO-LEAK: the credential never appears in the bytes relayed to the box.
        assert!(
            !got.windows(b"sk-secret-key".len()).any(|w| w == b"sk-secret-key"),
            "the credential never leaks into the box-facing response",
        );
        handle.join.unwrap().join().unwrap();
    }

    /// The credential's `Debug` is redacted — it can never reach a log or diagnostic.
    #[test]
    fn credential_debug_is_redacted() {
        let cred = Credential::new("sk-super-secret".into());
        assert_eq!(format!("{cred:?}"), "Credential(<redacted>)");
        assert!(!format!("{cred:?}").contains("secret"), "the key never appears in Debug output");
    }
}
