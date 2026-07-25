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

use std::io::{self, Read, Write};
use std::ops::ControlFlow;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use directpath::Event;
use proto::ErrorCode;
use transport::SessionId;

use crate::dispatch::{Dispatcher, Enqueue, Routed};
use crate::nic::Reply;
use crate::upstream_tls::TlsPipe;
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
/// [`UpstreamHandle`]. The real `rustls` worker ([`AnthropicClient`]) implements this; tests inject a
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

// ─── Sprint 11 Slice 2 — the real Anthropic upstream client (below the seam) ──────────────────────────
//
// `rustls` is used ONLY here (below the `NicRun`/reply seam), never in the box-reachable core
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
/// Anthropic Messages API over a direct `rustls` pipe — no HTTP client — and streams the response
/// **verbatim** back
/// through the [`UpstreamSpawn`] seam into the Sprint-10 push-queue. The destination is **not** a
/// parameter of this type: it arrives per-call as the matched `Route` (Sprint 19), so the register is the
/// only place a destination is authored.
/// **Holds no destination.** The endpoint it dials arrives per-call from the matched `Route`
/// (Sprint 19); there is deliberately no `endpoint` field, so there is nothing for a dial to diverge
/// from the register *to*.
pub struct AnthropicClient {
    credential: Credential,
}

impl AnthropicClient {
    #[must_use]
    /// Takes ONLY the credential. The endpoint is no longer an input here — it arrives per-call from the
    /// matched `Route` — so a caller cannot supply a destination that differs from the register's.
    pub fn new(credential: Credential) -> Self {
        AnthropicClient { credential }
    }
}

impl UpstreamSpawn for AnthropicClient {
    fn spawn(&self, _session: SessionId, destination: Route, body: Vec<u8>) -> UpstreamHandle {
        let (tx, rx) = sync_channel(UPSTREAM_CHANNEL_CAP);
        // ONE decomposition of the matched route, here, feeding peer + SNI + Host alike.
        let dest = Destination::of(destination);
        let key = self.credential.0.clone();
        let join = std::thread::spawn(move || {
            match upstream_call(dest, &key, &body) {
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

/// The dial target: **one value, derived once**, that the TLS peer, the SNI, and the HTTP `Host` header
/// are all read out of.
///
/// This exists to make the peer-binding a single property rather than three that happen to agree.
/// [`Route::split`] is the crate's one decomposition and [`Destination::of`] is the only constructor
/// reached from production, so a matched route cannot end up connecting to one host, presenting a second
/// name for verification, and addressing a third in the request. Sprint 19 removed exactly this shape one
/// level down — a dial that agreed with the register by authorship rather than by construction.
#[derive(Clone, Copy)]
struct Destination {
    /// The TCP peer, the name certificate verification runs against, AND the `Host` header. One field.
    host: &'static str,
    /// The request-line target.
    path: &'static str,
}

impl Destination {
    /// Decompose the **matched** `Route` — the vetted destination the register authored and U3/U4 proved
    /// the box's address equals. Nothing else constructs a production `Destination`.
    fn of(route: Route) -> Self {
        let (host, path) = route.split();
        Destination { host, path }
    }
}

/// The port every upstream call dials. Fixed, not derived: `Route`'s canonical form is `https://` +
/// host + path and carries no port, so there is no box-influenced value here at all.
const HTTPS_PORT: u16 = 443;

/// The blocking upstream call — **the network leg**, now a direct TLS pipe with no HTTP client in it
/// (Sprint 18). Connects to `dest`, writes the request, and hands back the live pipe so the caller
/// streams the response **exactly as it arrives**.
///
/// What comes back is the WHOLE upstream response — status line, headers, body — unparsed (designer
/// ruling, 2026-07-24). This function frames what the proxy *emits* and reads nothing of what it
/// *receives*: there is no response parser here to be wrong, and therefore no status, no encoding, and no
/// redirect for the proxy to have an opinion about. `cloud/approach.md` §5's "never inspects, neutralizes,
/// modifies, or sanitizes" is satisfied structurally rather than by a configuration flag.
fn upstream_call(dest: Destination, key: &str, body: &[u8]) -> Result<TlsPipe, ErrorCode> {
    call_over(dest, key, body, || TlsPipe::connect(dest.host, HTTPS_PORT, UPSTREAM_TIMEOUT))
}

/// [`upstream_call`]'s body with the socket-opening step injected — **the only** thing that varies. The
/// tests supply an opener aimed at the local test server (a name production cannot reach, on a port
/// `Route` cannot express); everything from the request bytes onward is the shipped path, so what they
/// prove is what production does. Slice 1's `connect_with_config` precedent: move the test below the
/// seam rather than widen a production type to accommodate it.
fn call_over(
    dest: Destination,
    key: &str,
    body: &[u8],
    open: impl FnOnce() -> io::Result<TlsPipe>,
) -> Result<TlsPipe, ErrorCode> {
    let mut pipe = open().map_err(|e| map_transport_error(&e))?;
    write_request(&mut pipe, dest, key, body).map_err(|e| map_transport_error(&e))?;
    Ok(pipe)
}

/// Write the request: the **fixed** header set, then the recognizer-admitted body byte-verbatim.
///
/// **The complete list of what goes on the wire, and why each line has to be there** — this is the
/// request-side half of "a dumb, fixed, verbatim pipe", so the set is closed and auditable by reading it:
///
/// | line | why it cannot be dropped |
/// |---|---|
/// | `POST <path> HTTP/1.1` | the request line; `path` is the register's, via [`Destination`] |
/// | `host` | HTTP/1.1 requires it — and it is the SAME field as the TLS peer and SNI |
/// | `x-api-key` | the credential. The one thing the proxy adds that the box never sees |
/// | `anthropic-version` | the API requires it; a fixed compiled-in constant |
/// | `content-type` | the API requires it; fixed |
/// | `content-length` | frames the body so the upstream knows where it ends |
/// | `connection: close` | makes the response close-delimited, so the relay ends on a transport event |
///
/// Nothing else. No `user-agent`, no `accept`, no `accept-encoding`, no cookies, no proxy headers — an
/// HTTP client adds those on your behalf, which is precisely why there is no longer an HTTP client here.
/// The absence of `accept-encoding` is also why nothing arrives compressed to be tempted to decompress.
///
/// **The box contributes `body` and nothing else, and it cannot escape the body.** `content-length` is
/// computed here from the actual byte count and the body is written after the blank line, so bytes the
/// box authored are read by the upstream as body regardless of what they contain — a `\r\n` in the
/// payload is payload. `key`, `host`, and `path` are all compiled-in or operator-authored, never
/// box-influenced, so no header line takes a box-supplied value at all.
fn write_request(w: &mut impl Write, dest: Destination, key: &str, body: &[u8]) -> io::Result<()> {
    let mut req = Vec::with_capacity(HEADROOM + body.len());
    req.extend_from_slice(
        format!(
            "POST {} HTTP/1.1\r\n\
             host: {}\r\n\
             x-api-key: {}\r\n\
             anthropic-version: {}\r\n\
             content-type: application/json\r\n\
             content-length: {}\r\n\
             connection: close\r\n\
             \r\n",
            dest.path,
            dest.host,
            key,
            ANTHROPIC_VERSION,
            body.len(),
        )
        .as_bytes(),
    );
    req.extend_from_slice(body);
    // One write for the whole request: the credential never sits in a partially-flushed buffer across a
    // failure, and the upstream sees headers and body together.
    w.write_all(&req)?;
    w.flush()
}

/// Slack for the fixed header block so the request buffer is sized once. Not a limit on anything.
const HEADROOM: usize = 256;

/// Stream a response **verbatim** into the channel as [`UpstreamFrame::Chunk`]s (MB3 — never inspected,
/// neutralized, or transformed). Bounded per-read memory ([`RESPONSE_CHUNK`]). A read error mid-stream →
/// `Failed(Unavailable)`. Stops early if the loop hung up (the box session closed).
///
/// Every byte the upstream sent is relayed, starting at the status line: the box receives the response
/// the cloud actually gave, and decides for itself what a 401 or a 302 means. An HTTP status is a
/// RESPONSE, not an error (operator ruling, 2026-07-24) — and with no parser here, there is now no code
/// path that *could* turn one into a proxy-invented `net-error`.
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

/// Map a transport failure to the proxy's own [`ErrorCode`] category (no upstream detail, no credential —
/// the box learns only *that* it failed and roughly why). Timeout → `Timeout`; unresolvable or
/// unreachable peer → `Unreachable`; everything else, TLS verification failures included → `Unavailable`.
///
/// **Reached only when there is no response to relay.** A refused connection, a name that does not
/// resolve, a certificate that fails verification, a hung upstream: in each case nothing came back, so
/// `net-error` is the only thing the box can be told (`proto/net.md` §4 — net-error is for what is NOT an
/// upstream response). An HTTP status never arrives here; it is a response, and responses go down the
/// relay untouched.
///
/// A read timeout on a socket carrying `SO_RCVTIMEO` surfaces as `WouldBlock`, not `TimedOut`, so both
/// map to `Timeout` — the distinction is the platform's, not the proxy's.
fn map_transport_error(e: &io::Error) -> ErrorCode {
    use std::io::ErrorKind;
    match e.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => ErrorCode::Timeout,
        ErrorKind::ConnectionRefused
        | ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::NotConnected
        | ErrorKind::HostUnreachable
        | ErrorKind::NetworkUnreachable
        | ErrorKind::AddrNotAvailable
        | ErrorKind::NotFound => ErrorCode::Unreachable,
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

    /// Establish the precondition `on_admit` now requires: an ADMITTED `net-address` for the session, so
    /// the dispatcher holds the matched `Route` the dial is derived from (Sprint 19). This is exactly how
    /// production reaches `on_admit` — the route is stored by `route()` when the address is admitted — so
    /// driving it here keeps these tests on the real path rather than injecting state behind it.
    fn admit_route(d: &mut Dispatcher, session: SessionId) {
        let verdict = d.route(session, &net_address_frame(), ROUTES, VETTED_TOOLS);
        assert!(matches!(verdict, Routed::NetRouteAdmit(_)), "the test's net-address must be admitted");
    }

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

    use std::io::Cursor;
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

    // ---------------------------------------------------------------------------------------------
    // Sprint 18 Slice 2 — the pipe's own evidence.
    //
    // Every test below runs against the shared local TLS server (`upstream_tls::testserver`), through
    // `call_over` — the shipped `upstream_call` body with only the socket target swapped. What varies is
    // the thing that MUST vary for an offline test (a name production cannot reach, on a port `Route`
    // cannot express); the request bytes, the relay, and the error mapping are all the production path.
    // ---------------------------------------------------------------------------------------------

    use crate::upstream_tls::testserver::{IO_TIMEOUT, Fixture, fixture, now_offset, serve_scripted, trusting};

    /// A fixture whose leaf is valid, in date, and named `localhost` — the host every arm below dials.
    fn local_fixture() -> Fixture {
        fixture("localhost", now_offset(-3600), now_offset(3600))
    }

    /// Run the real call against a local server on `port`, dialling it as `host`.
    fn call_local(fx: &Fixture, host: &'static str, port: u16, key: &str) -> Result<TlsPipe, ErrorCode> {
        let dest = Destination { host, path: "/v1/messages" };
        call_over(dest, key, ADMIT_BODY, || {
            TlsPipe::connect_with_config(host, port, IO_TIMEOUT, trusting(fx))
        })
    }

    /// Drain a pipe through the SAME `stream_response` production uses, returning the relayed bytes.
    fn relayed(pipe: TlsPipe) -> Vec<u8> {
        let (tx, rx) = sync_channel(UPSTREAM_CHANNEL_CAP);
        let worker = std::thread::spawn(move || stream_response(pipe, &tx));
        let mut got = Vec::new();
        while let Ok(frame) = rx.recv_timeout(Duration::from_secs(10)) {
            match frame {
                UpstreamFrame::Chunk(c) => got.extend_from_slice(&c),
                UpstreamFrame::Failed(code) => panic!("unexpected transport failure: {code:?}"),
            }
        }
        worker.join().expect("relay worker");
        got
    }

    /// **FINAL STATE #5 — the request side, byte-exact.** The wire request is the admitted body plus the
    /// credential header and the fixed framing, and *nothing else*.
    ///
    /// Asserted as ONE full-buffer equality rather than a set of `contains` checks, because `contains`
    /// can only find what you thought to look for: it passes just as happily with a `user-agent`, an
    /// `accept-encoding`, or a cookie the HTTP client added on your behalf. Equality is the only form
    /// that proves the *absence* of everything unlisted — which is the actual claim being made about a
    /// dumb, fixed pipe.
    #[test]
    fn the_wire_request_is_exactly_the_admitted_body_plus_the_credential_header() {
        let fx = local_fixture();
        let (port, requests) = serve_scripted(&fx, b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}");
        let pipe = call_local(&fx, "localhost", port, "sk-secret-key").expect("the local call connects");
        let req = requests.recv_timeout(Duration::from_secs(10)).expect("the server captured a request");
        drop(pipe);

        let mut want: Vec<u8> = format!(
            "POST /v1/messages HTTP/1.1\r\nhost: localhost\r\nx-api-key: sk-secret-key\r\n\
             anthropic-version: 2023-06-01\r\ncontent-type: application/json\r\n\
             content-length: {}\r\nconnection: close\r\n\r\n",
            ADMIT_BODY.len()
        )
        .into_bytes();
        want.extend_from_slice(ADMIT_BODY);

        assert_eq!(
            String::from_utf8_lossy(&req),
            String::from_utf8_lossy(&want),
            "the wire request must be EXACTLY the fixed header set plus the admitted body — no extra \
             header, no re-encode",
        );
        // BIND-THE-CHAIN, stated separately so a future edit to `want` cannot quietly lose it: the body
        // on the wire is the admitted bytes themselves.
        assert!(req.ends_with(ADMIT_BODY), "the body is the admitted bytes, byte-identical");
    }

    /// **FINAL STATE #4 — the response side, verbatim, including the status line.** The box receives the
    /// upstream's own bytes from the first octet: a 401 is relayed as a 401, not replaced by a
    /// proxy-invented `net-error` (`cloud/approach.md` §5; operator ruling 2026-07-24). This is the case
    /// Sprint 17's capture will exercise live against a deliberately-invalid credential.
    #[test]
    fn the_whole_response_is_relayed_verbatim_status_line_and_headers_included() {
        const RESPONSE: &[u8] = b"HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\n\
                                  content-length: 62\r\n\r\n\
                                  {\"type\":\"error\",\"error\":{\"type\":\"authentication_error\"}}";
        let fx = local_fixture();
        let (port, _requests) = serve_scripted(&fx, RESPONSE);
        let pipe = call_local(&fx, "localhost", port, "sk-invalid").expect("the local call connects");
        assert_eq!(
            String::from_utf8_lossy(&relayed(pipe)),
            String::from_utf8_lossy(RESPONSE),
            "the ENTIRE upstream response — status line, headers, body — reaches the box unmodified",
        );
    }

    /// **FINAL STATE #8, defect 1 — a redirect is RELAYED, never followed.**
    ///
    /// The ureq defect had two halves and this proves both: the client followed 3xx automatically, and it
    /// did not strip `x-api-key` when doing so — so a redirect chosen by whatever answered the connection
    /// could walk the credential to a host the register never vetted.
    ///
    /// The `Location` here points at a SECOND live local server. The proof is not only that the 302 came
    /// back, but that the second server received *nothing at all*: no follow-up request, and therefore no
    /// credential delivered anywhere the route did not name.
    #[test]
    fn a_redirect_is_relayed_not_followed_and_the_credential_goes_nowhere_else() {
        let fx = local_fixture();
        let (elsewhere_port, elsewhere) = serve_scripted(&fx, b"HTTP/1.1 200 OK\r\n\r\nFOLLOWED");
        let location = format!("https://localhost:{elsewhere_port}/stolen");
        let redirect: &'static [u8] = Box::leak(
            format!("HTTP/1.1 302 Found\r\nlocation: {location}\r\ncontent-length: 0\r\n\r\n").into_bytes()
                .into_boxed_slice(),
        );
        let (port, _requests) = serve_scripted(&fx, redirect);

        let pipe = call_local(&fx, "localhost", port, "sk-secret-key").expect("the local call connects");
        let got = relayed(pipe);

        assert_eq!(
            String::from_utf8_lossy(&got),
            String::from_utf8_lossy(redirect),
            "the 302 is handed to the box verbatim — the proxy has no opinion about a status code",
        );
        assert!(
            elsewhere.recv_timeout(Duration::from_secs(2)).is_err(),
            "the redirect target was CONTACTED — the redirect was followed and the credential travelled \
             to a host the register never vetted",
        );
    }

    /// **FINAL STATE #8, defect 2 — an ambient proxy environment variable is ignored.**
    ///
    /// Proven in a CHILD PROCESS, because `std::env::set_var` is `unsafe` under edition 2024 and this
    /// crate is `deny(unsafe_code)` outside its two audited modules (MB6) — so the environment is set the
    /// only way a test here legitimately can: on a child. The child re-runs the ignored arm below, which
    /// performs a real round trip to a local server. If any part of the upstream path consulted
    /// `HTTPS_PROXY`/`HTTP_PROXY`/`ALL_PROXY`, the connection would go to the black-holed address instead
    /// and the child would fail.
    ///
    /// The black hole is `127.0.0.1:1` — nothing listens, so a consulted proxy setting cannot accidentally
    /// succeed.
    #[test]
    fn an_ambient_proxy_environment_variable_is_ignored() {
        let exe = std::env::current_exe().expect("test binary path");
        let out = std::process::Command::new(exe)
            .args(["--exact", "--ignored", "--nocapture", "upstream::tests::proxy_env_child_arm"])
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "socks5://127.0.0.1:1")
            .env("https_proxy", "http://127.0.0.1:1")
            .env("http_proxy", "http://127.0.0.1:1")
            .output()
            .expect("re-exec the test binary");
        assert!(
            out.status.success(),
            "with proxy variables set, the upstream call did not reach its destination — something in \
             the path consults the ambient environment:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("1 passed"),
            "the child ran no test — the arm name or the harness flags drifted, so this proved nothing:\n{}",
            String::from_utf8_lossy(&out.stdout),
        );
    }

    /// The child arm for the test above. `#[ignore]`d so it runs ONLY under the parent's re-exec, with the
    /// proxy variables set; on its own it would prove nothing (a plain run has no proxy in the
    /// environment, so it would pass vacuously).
    #[test]
    #[ignore = "driven by an_ambient_proxy_environment_variable_is_ignored, which sets the environment"]
    fn proxy_env_child_arm() {
        assert!(
            std::env::var("HTTPS_PROXY").is_ok(),
            "the child must inherit the proxy environment, or it tests nothing",
        );
        let fx = local_fixture();
        let (port, requests) = serve_scripted(&fx, b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}");
        let pipe = call_local(&fx, "localhost", port, "sk-secret-key")
            .expect("the call must reach the destination, not the proxy named in the environment");
        let req = requests.recv_timeout(Duration::from_secs(10)).expect("the DESTINATION got the request");
        assert!(req.starts_with(b"POST /v1/messages HTTP/1.1\r\n"), "an origin-form request line");
        assert!(
            !String::from_utf8_lossy(&req).contains("http://"),
            "an absolute-form request line means the request was addressed to a proxy",
        );
        assert_eq!(String::from_utf8_lossy(&relayed(pipe)).len(), 40);
    }

    /// **FINAL STATE #8, defect 3 — a compressed body is relayed COMPRESSED.**
    ///
    /// The response below is real gzip, labelled `content-encoding: gzip`. An HTTP client would hand the
    /// caller the decompressed plaintext and quietly drop the header, which is a modified response: the
    /// box would be told the upstream said something it did not literally say, and the bytes it received
    /// would no longer be the bytes on the wire. Both the compressed octets and the header must survive.
    #[test]
    fn a_gzip_labelled_body_is_relayed_compressed_with_its_header_intact() {
        /// Valid gzip of `{"id":"msg_gzip_probe"}`.
        const GZIP_BODY: [u8; 43] = [
            0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0xab, 0x56, 0xca, 0x4c, 0x51,
            0xb2, 0x52, 0xca, 0x2d, 0x4e, 0x8f, 0x4f, 0xaf, 0xca, 0x2c, 0x88, 0x2f, 0x28, 0xca, 0x4f,
            0x4a, 0x55, 0xaa, 0x05, 0x00, 0xce, 0x47, 0x4c, 0xc3, 0x17, 0x00, 0x00, 0x00,
        ];
        let mut response =
            b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\ncontent-length: 43\r\n\r\n".to_vec();
        response.extend_from_slice(&GZIP_BODY);
        let response: &'static [u8] = Box::leak(response.into_boxed_slice());

        let fx = local_fixture();
        let (port, _requests) = serve_scripted(&fx, response);
        let got = relayed(call_local(&fx, "localhost", port, "sk-secret-key").expect("connects"));

        assert_eq!(got, response, "the compressed body and its header are relayed untouched");
        assert!(
            !got.windows(b"msg_gzip_probe".len()).any(|w| w == b"msg_gzip_probe"),
            "the plaintext appeared — the body was DECOMPRESSED somewhere in the path",
        );
    }

    /// **FINAL STATE #10a — the destination IS the register's, for every route it holds.**
    #[test]
    fn a_destination_is_exactly_the_matched_routes_host_and_path() {
        for route in ROUTES {
            let dest = Destination::of(*route);
            let rest = route.as_str().strip_prefix("https://").expect("a canonical route is https://");
            let tail = rest.strip_prefix(dest.host).expect("the host is the route's leading segment");
            assert!(
                tail == dest.path || (tail.is_empty() && dest.path == "/"),
                "{route:?}: host + path must recompose the route (a bare domain gains the root path)",
            );
            assert!(!dest.host.contains('/'), "the host carries no path");
            assert!(dest.path.starts_with('/'), "the path is origin-form");
        }
    }

    /// **FINAL STATE #10b — peer, SNI, and `Host` cannot be steered apart.**
    ///
    /// Both arms dial the SAME listener, whose certificate names `localhost` and nothing else. Only the
    /// single [`Destination::host`] field differs:
    ///
    /// - `localhost` — the handshake succeeds (so the SNI and the verified peer name are `localhost`) AND
    ///   the captured request says `host: localhost`. All three agree.
    /// - `127.0.0.1` — reaches the *identical* listener, so nothing about reachability changed, and the
    ///   handshake is REFUSED because the name is not in the certificate.
    ///
    /// The second arm is the load-bearing one: it shows the `Host` header cannot be set to one name while
    /// the connection is verified against another, because there is only one field to set. Three separate
    /// derivations that happened to agree would let the arms diverge; one value cannot.
    #[test]
    fn peer_sni_and_host_header_cannot_be_steered_apart() {
        let fx = local_fixture();

        let (port, requests) = serve_scripted(&fx, b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}");
        let pipe = call_local(&fx, "localhost", port, "sk").expect("the SAN-matching name is accepted");
        let req = requests.recv_timeout(Duration::from_secs(10)).expect("captured");
        drop(pipe);
        assert!(
            String::from_utf8_lossy(&req).contains("\r\nhost: localhost\r\n"),
            "the Host header carries the same name the certificate was verified against",
        );

        // Arm 2. A certificate that is valid, in date, and issued by a CA the client trusts — but which
        // names a DIFFERENT host. It is served on a `localhost` listener, so the dialled name reaches the
        // peer exactly as in arm 1; the ONLY thing that changed is whether the certificate carries the
        // name being dialled.
        //
        // (This arm was first written dialling `127.0.0.1` at the arm-1 listener. It "passed" as a
        // failure and proved nothing: that listener binds via `localhost`, which resolves to `::1` here,
        // so the connection was REFUSED before any TLS work — the vacuous-pass shape Slice 1's review
        // caught in the refusal arms. The `Unreachable` code, not `Unavailable`, is what exposed it.)
        let elsewhere = fixture("not-the-destination.example", now_offset(-3600), now_offset(3600));
        let (port2, requests2) = serve_scripted(&elsewhere, b"HTTP/1.1 200 OK\r\n\r\n");

        let err = TlsPipe::connect_with_config("localhost", port2, IO_TIMEOUT, trusting(&elsewhere))
            .err()
            .expect("a certificate that does not name the dialled host must be refused");
        let err = err.to_string();
        assert!(
            err.contains("tls handshake failed:"),
            "the arm failed before the handshake, so it exercised nothing: {err}",
        );
        assert!(err.contains("not valid for name"), "refused, but not on the NAME: {err}");

        // The join, stated as the consequence that matters: no request was written at all, so there is no
        // way to put `host: localhost` on a wire whose peer was not verified as `localhost`. One field
        // drives the verified name and the header alike; separate derivations could have diverged here.
        let captured = requests2.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
        assert!(
            captured.is_empty(),
            "request bytes reached a peer that failed verification: {:?}",
            String::from_utf8_lossy(&captured),
        );

        // And the box-facing code for that failure is `Unavailable`, not `Unreachable`: the peer was
        // reached, so this is not "could not get there" — it is "nothing trustworthy came back".
        let refused = call_local(&elsewhere, "localhost", serve_scripted(&elsewhere, b"x").0, "sk")
            .err()
            .expect("refused");
        assert_eq!(refused, ErrorCode::Unavailable, "a verification failure has no response to relay");
    }

    /// Transport-error mapping, on the real path: a refused connection becomes `net-error(Unreachable)`
    /// over the seam. Driven at `call_over` because a `Route` cannot name a local target (no `http://`,
    /// no port — the canonical form forbids both, deliberately).
    #[test]
    fn a_refused_connection_maps_to_net_error_unreachable() {
        let dest = Destination { host: "localhost", path: "/v1/messages" };
        // Port 1: nothing listens, so the connect is refused before any TLS work begins.
        let err = call_over(dest, "sk", ADMIT_BODY, || TlsPipe::connect(dest.host, 1, IO_TIMEOUT))
            .err()
            .expect("a refused connection must fail");
        assert_eq!(err, ErrorCode::Unreachable, "a refused connection → net-error(Unreachable)");
    }

    /// The credential's `Debug` is redacted — it can never reach a log or diagnostic.
    #[test]
    fn credential_debug_is_redacted() {
        let cred = Credential::new("sk-super-secret".into());
        assert_eq!(format!("{cred:?}"), "Credential(<redacted>)");
        assert!(!format!("{cred:?}").contains("secret"), "the key never appears in Debug output");
    }
}
