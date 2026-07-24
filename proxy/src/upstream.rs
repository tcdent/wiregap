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
//! the same seam. The one thing these tests do not themselves exercise — that the real reactor runs a
//! tick (and thus drains) when a response arrives during a real quiescent park — was **proven live on
//! hardware in Sprint 14**: the reactor's own bounded wait re-checks unaided, so the wake needs no
//! proxy-side mechanism. Here a test's `Idle` tick after a quiescent arrival stands in for that wake.

use std::io::Read;
use std::ops::ControlFlow;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use directpath::Event;
use proto::ErrorCode;
use transport::SessionId;

use crate::dispatch::{Dispatcher, Enqueue, Routed};
use crate::nic::Reply;
use crate::vocab::{CredentialSource, Route, ToolIdentity};

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
        let handle = self.spawn.spawn(session, body);
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
                    // End of response: close the reply stream (drains + evicts), reap the worker thread.
                    dispatcher.close_reply(session);
                    if let Some(done) = self.inflight.take()
                        && let Some(join) = done.join
                    {
                        let _ = join.join(); // no leaked thread
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

    /// Resolve the credential from its compiled [`CredentialSource`] descriptor (Sprint 16) — read the
    /// named environment variable at startup. The descriptor fixes *where* to look; this reads it.
    /// **Empty is absent** (see [`Credential::from_env_value`]).
    #[must_use]
    pub fn from_source(source: &CredentialSource) -> Option<Credential> {
        Self::from_env_value(std::env::var(source.var_name()))
    }

    /// The resolution **policy**, pure and total over every `env::var` outcome, so it is directly
    /// testable without mutating the process environment (which under edition 2024 would require
    /// `unsafe` — forbidden by the crate's `deny(unsafe_code)`).
    ///
    /// **Set-but-empty is treated exactly as unset**: an empty string is not a credential, and letting
    /// one satisfy `Authority::Credential`'s presence check would be a silently-open authority rung —
    /// the opposite of the fail-closed guarantee the rung exists to make. Non-UTF-8 is likewise absent:
    /// a value the proxy cannot even read is not a credential it can present.
    fn from_env_value(value: Result<String, std::env::VarError>) -> Option<Credential> {
        match value {
            Ok(v) if !v.is_empty() => Some(Credential(v)),
            // Unset, set-but-empty, or non-UTF-8 → absent. Fail closed, identically in all three cases.
            _ => None,
        }
    }

    /// Whether this credential is the empty string — the one property of the secret that is safe to
    /// expose (it reveals no content). Lets the authority rung fail closed on an empty credential
    /// however it was constructed, not only on one resolved through [`Credential::from_env_value`].
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
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
/// a real credential). POSTs `body` **byte-verbatim** (`send(body)` — no
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

    use crate::dispatch::REPLY_QUEUE_CAP;
    use crate::register::anthropic::VETTED_TOOLS;
    use crate::register::net::ROUTES;

    // ─── Sprint 16 Slice 2 — the credential descriptor's resolution policy ───────────────────────

    /// **THE EMPTY-ENV-VAR GATE**: a set-but-empty variable resolves to *absent*, **identically** to
    /// unset — and the `Authority::Credential` rung then fails closed on it. Driven through the pure
    /// policy so it is deterministic and needs no process-environment mutation (which under edition 2024
    /// would require `unsafe`, forbidden by the crate's `deny(unsafe_code)`).
    #[test]
    fn set_but_empty_env_var_is_absent_exactly_like_unset() {
        use crate::lifecycle::{Authority, validate_authority};
        use std::env::VarError;

        // The three absent cases — set-but-empty, unset, and unreadable — are one outcome: None.
        let empty = Credential::from_env_value(Ok(String::new()));
        let unset = Credential::from_env_value(Err(VarError::NotPresent));
        let non_utf8 = Credential::from_env_value(Err(VarError::NotUnicode("\u{fffd}".into())));
        assert!(empty.is_none(), "a set-but-empty variable is NOT a credential");
        assert!(unset.is_none());
        assert!(non_utf8.is_none(), "a value the proxy cannot read is not a credential");

        // A real value resolves — so the gate is not vacuously-closed (the success arm).
        let present = Credential::from_env_value(Ok("sk-test-value".into()));
        assert!(present.is_some(), "a non-empty value DOES resolve");

        // Chained to the authority rung: empty fails closed EXACTLY as unset does, and only a real
        // credential validates. This is the property the rung exists to guarantee.
        assert!(
            !validate_authority(Authority::Credential, ROUTES, empty.as_ref(), None),
            "an empty credential must FAIL the authority rung, not satisfy it by mere presence",
        );
        assert!(!validate_authority(Authority::Credential, ROUTES, unset.as_ref(), None));
        assert!(validate_authority(Authority::Credential, ROUTES, present.as_ref(), None));

        // The rung's own second floor: an empty credential constructed DIRECTLY (not through the
        // resolution policy) also fails — the guarantee does not depend on how it was built.
        let hand_built_empty = Credential::new(String::new());
        assert!(
            !validate_authority(Authority::Credential, ROUTES, Some(&hand_built_empty), None),
            "empty fails closed however constructed",
        );
    }

    /// The descriptor names a variable and carries no value — and the registered one is the variable
    /// `serve.rs` actually reads. (Its const-validation and the inexpressibility of a literal secret are
    /// proven at COMPILE time by `tests/ui/*credential_source*`.)
    #[test]
    fn registered_credential_source_names_a_variable_never_a_value() {
        use crate::register::anthropic::CREDENTIAL_SOURCE;
        assert_eq!(CREDENTIAL_SOURCE.var_name(), "ANTHROPIC_API_KEY");
    }

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
    /// pending a real credential.)
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
