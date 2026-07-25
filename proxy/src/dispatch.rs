//! Sprint 6 S1 — the codec + dispatch layer (tier 2, `architecture.md` "wire-protocol parser"):
//! the demux that sits between the reactor's frame delivery and the services. It **decodes** proto's
//! L2 message, **binds** each session to a service by its first frame's verb, **routes** every
//! subsequent frame of that session to the bound service, and at the **tier-1 seam** hands gated
//! channels into the certified security core — never re-implementing or duplicating what the core
//! certifies.
//!
//! This module is a **sibling to the tier-1 core, never inside it** (`proxy::{leaf, address, route,
//! anthropic, register, vocab, open_discipline}` are the certified core; this consumes them). Its
//! own bar is memory-safety + correctness + liveness, **not** security logic — routing is by
//! *envelope* (session id, verb), **never** by content (`contract/box-proxy.md` §1.3, §1.4; the one
//! security-relevant wiring invariant, MB3).


use std::os::fd::{AsFd, OwnedFd};
use std::path::PathBuf;

use proto::{
    ErrorBody, ErrorCode, FileData, FileError, FileRead, FileWrite, List, ListEntries, Message,
    NetAddress, NetError, NetPayload, Reassembler, ReassemblyState, Reject, Wire, encode_message,
    verb,
};
use transport::{SendError, SessionId};

use crate::anthropic::{Denial, Request, recognize};
use crate::fs_read::{DirReader, DirStep, FileReader, ReadStep};
use crate::fs_write::{QuarantineWriter, WriteStep};
use crate::nic::Reply;
use crate::vocab::{Route, ToolIdentity};

/// Peek the L2 verb — the first `u32` (LE) of the payload — **without decoding the body**. This is
/// the whole of the deep-parsing the proxy does for a *relay* channel (`proxy-head.md` §1,
/// `library.md` §7: "decodes L1/session routing and the binding verb only; deep-parses no
/// terminal-service L2 in either direction"). `None` if the payload is shorter than the verb.
#[must_use]
pub fn peek_verb(payload: &[u8]) -> Option<u32> {
    payload
        .split_first_chunk::<4>()
        .map(|(v, _)| u32::from_le_bytes(*v))
}

/// The four services a session can bind to — one per verb block (`model.md` §8 verb-number blocks:
/// `0x0_` control, `0x1_` terminal, `0x2_` file, `0x3_` net).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Service {
    Control,
    Terminal,
    File,
    Net,
}

/// The service a verb belongs to — its block (the high nibble of the low byte). A verb outside the
/// four defined blocks has no service and is unsupported. **Pure routing by verb; no content.**
#[must_use]
pub fn service_of_verb(verb: u32) -> Option<Service> {
    match (verb & 0xff) >> 4 {
        0x0 => Some(Service::Control),
        0x1 => Some(Service::Terminal),
        0x2 => Some(Service::File),
        0x3 => Some(Service::Net),
        _ => None,
    }
}

/// The verbs the proxy **acts on** (decodes + gates) today — the JC-1 reserved-check set: the verbs
/// `dispatch` itself decodes (`box-proxy.md` §3.1's gated set). `file-read` and `list` join here in
/// Sprint 10 Slice 2 now that they are genuinely acted on (decoded + streamed, not relayed) — Sprint
/// 9's own named JC-1 follow-up. A relayed or control verb is **not** acted on, so JC-1 does not police
/// it (relay is inert; §2). Matches on the low-byte id; the reserved-bit check is applied by the caller
/// on the raw u32.
fn acts_on(verb: u32) -> bool {
    matches!(
        verb & 0xff,
        verb::NET_ADDRESS
            | verb::FILE_WRITE
            | verb::FILE_CHUNK
            | verb::NET_PAYLOAD
            | verb::FILE_READ
            | verb::LIST
    )
}

/// The disposition of one delivered frame after decode + binding + routing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Routed {
    /// A single-frame **gated** decision made at the tier-1 seam: the net route matched (admit),
    /// carrying the matched [`Route`].
    NetRouteAdmit(Route),
    /// A single-frame gated decision: the net address was denied by the tier-1 core (reserved
    /// scheme or no matching route).
    NetDeny,
    /// A single-frame gated decision: the file-write destination passed the tier-1 leaf check
    /// (via `leaf_validate_prescanned` — the single-scan-site path, RQ2). The validated destination is
    /// stored as a per-session [`QuarantineWriter`] for the ensuing `file-chunk`s to append to.
    FileWriteAdmit,
    /// A single-frame gated decision: the file-write destination was denied by the tier-1 core.
    FileWriteDeny,
    /// A `file-chunk` appended `bytes` to the session's quarantine file (Sprint 9 Slice 2) — the
    /// confined write succeeded at the monotonic cursor.
    FileChunkAppended { bytes: usize },
    /// A `file-chunk` was **denied** by the write floor / a write error, and the poisoned writer
    /// evicted (Sprint 9 S2; Sprint 10 S4 JC-3 sub-cause split, designer ruling A 2026-07-22): the
    /// confined open refused the destination (traversal/symlink/exists — the write-destination floor
    /// firing one message later than `file-write`, so `code = Denied`), the per-file cap tripped
    /// (`code = Malformed`), or the write errored (`code = Unavailable`). Each is a **meaningful
    /// outcome the box must know** → notifies `file-error(code)`. Fail-closed; nothing further written.
    FileChunkDeny { code: ErrorCode },
    /// A `file-chunk` arrived with **no admitted `file-write`** (no destination) — **structural
    /// protocol-noise** (JC-3a), not a policy deny: dropped, **silent**, no notification (designer
    /// ruling A — "notify on a meaningful outcome, silent on structural noise"; the broad bucket).
    FileChunkNoWriter,
    /// A `file-read` was **admitted** (Sprint 10 Slice 2): its source opened fail-safe under the
    /// readable root, and a per-session outbound [`ReplyStream`] is registered to stream `file-data`
    /// back to the box. The bytes flow on later `pump_replies` ticks (bounded lazy stream, WouldBlock
    /// -paced), each frame stamped with the request's own session id as `in-reply-to`.
    FileReadStreaming,
    /// A `file-read` failed **fail-safe** at open (not-found / denied / non-regular / i-o): the
    /// read path is harmless, so a failed open reads nothing — no stream is registered. Carries the
    /// proxy-owned [`ErrorCode`] (never a box name). The box-facing `file-error` *notification* is
    /// JC-3's job (Slice 4); this slice registers no reply.
    FileReadError { code: ErrorCode },
    /// A `list` was **admitted** (Sprint 10 Slice 2): its directory opened fail-safe under the readable
    /// root via the confined [`DirReader`], and a per-session outbound [`ReplyStream`] streams
    /// `list-entry` records back (one per frame, bounded-lazy, WouldBlock-paced), each stamped with the
    /// request's own session id as `in-reply-to`.
    ListStreaming,
    /// A `list` failed **fail-safe** at open (non-directory / not-found / i-o): no stream. Carries the
    /// proxy-owned [`ErrorCode`] (no box name). The box-facing notification is JC-3's (Slice 4).
    ListError { code: ErrorCode },
    /// A `net-payload` chunk accumulated into the session's [`Reassembler`]; its header flag was
    /// `more`, so more chunks are expected — no decision yet (the body is incomplete).
    NetPayloadAccumulating,
    /// The final `net-payload` chunk completed the request body, which the tier-1 Anthropic
    /// recognizer ([`recognize`]) **ADMITTED**. Carries the reassembled, verbatim body — the exact
    /// bytes the recognizer vetted and the exact bytes to forward upstream (CB4-across-the-seam:
    /// recognized == reassembled == forwarded, no split, no rewrite).
    NetPayloadAdmit(alloc::vec::Vec<u8>),
    /// The completed request body was **DENIED** by the tier-1 recognizer — carries the legible
    /// [`Denial`]. Fail-closed; the body is not forwarded.
    NetPayloadDeny(Denial),
    /// A `net-payload` completed on a session with **no admitted `Route`** (no prior `net-address`,
    /// or one that was denied) — so no recognizer envelope can be sourced. Fail-closed: the body is
    /// **not** recognized and **not** forwarded (Sprint 8's no-prior-Route-denies rung). This is
    /// distinct from a recognizer [`Denial`]: recognition never runs, because there is nothing to
    /// vet the request *as*.
    NetPayloadNoRoute,
    /// A follow-on frame routed to its service's stateful handler (file-chunk writing;
    /// open-session control). The full lifecycle is the service's own. Carries the bound service.
    ToServiceHandler(Service),
    /// A relay channel (file-read, net-response, display/input) routed to its relay (S2–S4).
    Relay(RelayKind),
    /// A verb no proxy service supports, or a frame whose verb's service does not match the
    /// session's binding (`box-proxy.md` JC-2) — dropped, logged.
    Unsupported { verb: u32 },
    /// **JC-1 (`box-proxy.md` §2): reserved disposition.** A message the proxy *acts on* (one of the
    /// four acted-on verbs — `net-address`/`file-write`/`file-chunk`/`net-payload`) whose **raw** verb
    /// `u32` carries a nonzero reserved surface (the upper three bytes) **denies the whole message,
    /// deny-without-binding**. The check reads the raw [`peek_verb`] u32, never the masked low byte (a
    /// masked check would pass vacuously). Silent, like every deny — the JC-3b error-verb notification
    /// stays R-c-deferred. Carries the raw verb (numeric, safe to log). Relayed verbs are exempt (JC-1
    /// polices nothing on relayed messages; relay is inert). Forward-compatibility floor: whatever ids
    /// ratify later, today's proxy denies a reserved surface fail-closed.
    ReservedDeny { verb: u32 },
    /// The L2 payload failed to decode (a structural `proto::Reject`) — dropped, logged.
    Decode(Reject),
}

/// Which relay a routed frame goes to (the actual relay logic is Slices 2–4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RelayKind {
    /// `file-read` → the filesystem read service (S2).
    FileRead,
    /// `net-response`/`net-error` → the cloud response relay (S3).
    CloudResponse,
    /// `display`/input → the terminal relay (S4).
    Terminal,
    /// A **box-authored** `list-entry`/`file-error` answering a **head-initiated** file-browse (Sprint 15
    /// Slice 2) → relayed **to the head** (`send_to_head`). Origination-keyed: this verdict is reached
    /// only at the NIC-ingress `route()` (a box→proxy frame), so the direction is box→head **by where the
    /// frame came from**, never by decoding the body. Distinct from a **proxy-authored** `list-entry`
    /// answering a **box-initiated** host-list, which flows to the box via the reply queue and never
    /// touches this verdict (the two disjoint paths that must never cross — the confused-deputy floor).
    FileToHead,
}

/// The source of an outbound reply stream — a confined fail-safe reader (Sprint 10 Slice 2). Each
/// variant yields the next reply **message body** (proto-encoded) on demand; the shared streaming /
/// pacing / eviction machinery in [`ReplyStream`] is identical for both. Its fd closes when dropped.
/// The bounded depth of a **push** reply queue (net-response / terminal head→box), in frames. When a
/// push stream holds this many un-drained frames, [`Dispatcher::enqueue_net_response`] returns
/// [`Enqueue::Full`] —
/// **backpressure to the ingress** (it must stop reading upstream/head and retry after the box drains),
/// never a drop, never unbounded growth. Provisional scalar (marked for review), the outbound analog of
/// the transport ring depth / `REASSEMBLY_CAP`.
pub const REPLY_QUEUE_CAP: usize = 64;

/// The result of enqueuing a push-reply frame — the flow-control signal to the ingress.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Enqueue {
    /// The frame is queued for the box.
    Accepted,
    /// The queue is at [`REPLY_QUEUE_CAP`] — **backpressure**: the ingress must stop reading its source
    /// and retry after the box drains. The frame was **not** taken (never dropped, never grows the queue).
    Full,
}

/// The outcome of routing a head→box terminal input (Sprint 12 Slice 2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerminalInput {
    /// Enqueued to the box, stamped with the display-session correlation.
    Routed,
    /// The addressed display session is not active (unknown or **stale** after its pty closed) — the
    /// input was **dropped, never misrouted** (fail-safe). A *correct* drop, not a loss.
    Dropped,
    /// The display's box queue is **full** (Sprint 13 Slice 1.5): the input was **not taken** — the
    /// caller MUST retain it and stop draining the head channel so the channel fills and the reader
    /// backpressures the socket, resuming next tick once the box drains. Never a dropped keystroke.
    Backpressure,
}

/// The outcome of pulling the next frame from a [`ReplySource`].
enum Pull {
    /// The next final (framed) body to emit.
    Body(alloc::vec::Vec<u8>),
    /// Nothing available **right now**, but the source is **not** finished — a push queue awaiting more
    /// from its (async) ingress. The stream stays live; the next tick retries. Pull sources never yield
    /// this (a local reader always has a next chunk or EOF).
    Pending,
    /// The source is finished — end-of-file/dir (pull), or an empty **and closed** queue (push).
    Done,
}

/// The source of an outbound reply stream (Sprint 10 Slices 2–3). Two shapes share ONE pacing core
/// ([`ReplyStream`]): **pull** sources (a confined local reader — `file-read`/`list`) that always have a
/// next chunk or EOF, and a **push** [`Queue`](ReplySource::Queue) (`net-response` / terminal head→box)
/// fed asynchronously by an ingress, where "empty" means "wait for more," **not** "done." Each variant
/// yields the FINAL frame bytes (pull sources `in_reply_to`-stamp here; the push queue holds
/// already-prepared frames), so the pacing core is source-agnostic.
enum ReplySource {
    /// A `file-read` source — yields `file-data` frames, stamped with the request session. Never `Pending`.
    File { reader: FileReader, in_reply_to: SessionId },
    /// A `list` source — yields `list-entry` frames (one record each), stamped. Never `Pending`.
    Dir { reader: DirReader, in_reply_to: SessionId },
    /// A bounded push queue of ready-to-emit frames (net-response: `in_reply_to`-stamped by the enqueuer;
    /// terminal head→box: verbatim, no key — emission per F1) plus a `closed` flag. Empty+open →
    /// `Pending` (wait); empty+closed → `Done` (drained). Depth bounded by [`REPLY_QUEUE_CAP`] at enqueue.
    Queue { frames: alloc::collections::VecDeque<alloc::vec::Vec<u8>>, closed: bool },
}

impl ReplySource {
    /// The next FINAL frame to emit, or `Pending` (push queue awaiting more) / `Done` (finished).
    fn next_frame(&mut self) -> Pull {
        match self {
            ReplySource::File { reader, in_reply_to } => match reader.next_chunk() {
                ReadStep::Data(chunk) => {
                    let body = encode_message(&Message::FileData(FileData(chunk)));
                    Pull::Body(task16_stub_reply_frame(*in_reply_to, &body))
                }
                // EOF or a fail-safe read error: done (the box-facing error notification is JC-3, S4).
                ReadStep::Eof | ReadStep::Error(_) => Pull::Done,
            },
            ReplySource::Dir { reader, in_reply_to } => match reader.next_entry() {
                DirStep::Entry(e) => {
                    let body = encode_message(&Message::ListEntries(ListEntries(alloc::vec![e])));
                    Pull::Body(task16_stub_reply_frame(*in_reply_to, &body))
                }
                DirStep::Eof | DirStep::Error(_) => Pull::Done,
            },
            // EMPTY != DONE: an open queue that is momentarily empty is `Pending` (never evicted); only
            // an empty AND closed queue is `Done` (the drained tail is never orphaned).
            ReplySource::Queue { frames, closed } => match frames.pop_front() {
                Some(frame) => Pull::Body(frame),
                None if *closed => Pull::Done,
                None => Pull::Pending,
            },
        }
    }
}

/// One in-flight outbound reply stream — the **source-agnostic pacing core** (Sprint 10 Slices 2–3):
/// WouldBlock-paced emission of successive framed bodies from a [`ReplySource`] (pull or push) to the
/// box. The `in_reply_to` correlation is baked into each frame by the source (pull streams stamp;
/// net-response is stamped by its enqueuer; terminal is verbatim), so this core never inspects content.
struct ReplyStream {
    /// The proxy→box reply session, opened lazily on the first emitted frame (`None` until then). From
    /// the proxy's own high half.
    reply_sid: Option<SessionId>,
    /// The reply source (a confined local reader whose fd closes on drop, or a bounded push queue).
    source: ReplySource,
    /// One prepared-not-yet-sent frame, retained across ticks on `WouldBlock` — the single unit of
    /// buffered pacing memory (the source's own bound caps the rest).
    pending: Option<alloc::vec::Vec<u8>>,
    /// Finished (source `Done`) — evicted after the current pump pass. **Never** set on a momentary
    /// empty push queue (empty != done).
    done: bool,
}

impl ReplyStream {
    fn new(source: ReplySource) -> Self {
        ReplyStream { reply_sid: None, source, pending: None, done: false }
    }

    /// Drain this stream as far as the wire accepts this tick (see [`Dispatcher::pump_replies`]).
    fn pump(&mut self, reply: &mut dyn Reply) {
        loop {
            if self.done {
                return;
            }
            // Pull the next frame only once the previous is fully sent (backpressure to the source).
            if self.pending.is_none() {
                match self.source.next_frame() {
                    Pull::Body(frame) => self.pending = Some(frame),
                    // Push queue momentarily empty (not closed): nothing to do this tick; stay live.
                    Pull::Pending => return,
                    // Source finished: mark done (evicted after the pass) and stop.
                    Pull::Done => {
                        self.done = true;
                        return;
                    }
                }
            }
            let frame = self.pending.take().expect("pulled just above");
            match self.reply_sid {
                // First frame: open a new proxy→box reply session (origination cannot WouldBlock).
                None => self.reply_sid = Some(reply.open(&frame)),
                // Subsequent frames: send on the reply session, stop-and-wait paced.
                Some(sid) => match reply.send(sid, &frame) {
                    Ok(()) => {}
                    Err(SendError::WouldBlock) => {
                        // Retain the framed chunk and retry next tick — no drop (reliable-ordered).
                        self.pending = Some(frame);
                        return;
                    }
                    // A reply session we opened is never NotSender/NoSuchSession; treat any other error
                    // as terminal and stop this stream (fail-safe).
                    Err(_) => {
                        self.done = true;
                        return;
                    }
                },
            }
        }
    }
}

/// **TASK-16 STUB** — the reply-frame correlation-key wire layout is proto's Task 16 (queued, not
/// written). A LOUD, deliberately non-canonical placeholder: prepend the 8-byte `in_reply_to` session
/// id (big-endian, fixed-width) ahead of the proto-encoded reply message, so the *mechanism* — the key
/// carried faithfully on EVERY frame (the request's own session, never the reply's) — is built and
/// testable now. When proto Task 16 lands the canonical layout, this single choke point swaps to it and
/// nothing else in the streaming/pacing/eviction path changes. **Not** a finalized wire format
/// (Slice-2 acceptance forbids finalizing ahead of the fill-in).
fn task16_stub_reply_frame(in_reply_to: SessionId, encoded_msg: &[u8]) -> alloc::vec::Vec<u8> {
    let mut frame = alloc::vec::Vec::with_capacity(8 + encoded_msg.len());
    frame.extend_from_slice(&in_reply_to.0.to_be_bytes()); // the REQUEST session id, fixed-width u64
    frame.extend_from_slice(encoded_msg);
    frame
}

/// **TASK-16 STUB** (Sprint 12 Slice 2) — the terminal input's **pty-routing** correlation. A LOUD,
/// deliberately non-canonical placeholder: prepend the 8-byte `s_display` session id (the DISPLAY
/// session the head addressed, big-endian) ahead of the verbatim input payload, so the box binds the
/// input to the pty that opened that display session. **Distinct in meaning** from
/// [`task16_stub_reply_frame`]'s `in_reply_to` (a *reply* correlation) — this is a *routing* correlation
/// the two-session PTY model requires; same byte shape, different semantic. Swaps to proto Task 16's
/// canonical layout when it lands; **not** a finalized wire format.
fn task16_terminal_correlation_frame(s_display: SessionId, input_payload: &[u8]) -> alloc::vec::Vec<u8> {
    let mut frame = alloc::vec::Vec::with_capacity(8 + input_payload.len());
    frame.extend_from_slice(&s_display.0.to_be_bytes()); // the DISPLAY session id, the pty-routing key
    frame.extend_from_slice(input_payload);
    frame
}

/// Encode a proxy-authored `file-error` body for a JC-3(b) notification — a category `code` only, with
/// an **empty detail**: no box-supplied name ever enters the body (`file.md` §0; unsafe names are
/// display-only and physically cannot reach a proxy-authored error).
fn file_error(code: ErrorCode) -> alloc::vec::Vec<u8> {
    encode_message(&Message::FileError(FileError(ErrorBody { code, detail: alloc::vec::Vec::new() })))
}

/// Encode a proxy-authored `net-error` body — a category `code` only, no box content (empty detail).
fn net_error(code: ErrorCode) -> alloc::vec::Vec<u8> {
    encode_message(&Message::NetError(NetError(ErrorBody { code, detail: alloc::vec::Vec::new() })))
}

/// The dispatch state: the per-session service binding (`box-proxy.md` §1.3). A session is bound
/// once (its first frame's verb) and never re-bound.
///
/// **Resource residual (MB4), named and hook-ready:** the binding map has one entry per session and
/// grows **O(sessions-per-epoch)** — a session is inserted but only removed via [`close_session`],
/// which awaits a transport session-close/give-up signal that **does not exist in the committed
/// transport yet** (a *separate* cross-crate gap from net-payload's reassembly, which needs a
/// proto-side length/end-marker instead — different owners, different fixes). So the map is bounded
/// *per epoch* (the box's session-id space resets on reboot — `nic/architecture.md` §8's
/// no-cross-epoch-state) but not reclaimed within one, exactly mirroring the transport's own
/// un-reclaimed receiver sessions (`transport` table R5). This is a *named, tested, hook-ready*
/// residual — not a correctness bug (SessionId is monotonic-unique within an epoch, so no stale
/// binding can mismatch) — resolved fully once the transport surfaces a session-close signal, at
/// which point [`close_session`] is wired to it.
///
/// The per-session `reassemblers` map (Sprint 7) inherits the **same** bounded-per-epoch shape and
/// the **same** [`close_session`] eviction hook. It is in fact a *tighter* residual: an entry exists
/// only while a session's net-payload body is mid-reassembly — a completed body or a cap-overflow
/// removes it immediately (see [`Dispatcher::route`]), so only in-flight sessions hold one. Each
/// buffer is itself capped by `REASSEMBLY_CAP` (reject-before-grow, `proto::Reassembler`), so the
/// per-entry size is bounded too, not just the entry count.
///
/// The per-session `routes` map (Sprint 8, Task 14) holds the matched [`Route`] from a session's
/// `net-address` admission, so a later `net-payload` on that session sources its recognizer envelope
/// (domain/path) from the **real** route rather than a hardcoded stub. It shares the binding map's
/// bounded-per-epoch shape and [`close_session`] eviction. A `net-payload` that completes with **no**
/// entry here (no prior admitted `net-address`) fails closed ([`Routed::NetPayloadNoRoute`]).
pub struct Dispatcher {
    bindings: alloc::collections::BTreeMap<SessionId, Service>,
    /// One in-flight net-payload reassembly buffer per session, created on the session's first
    /// `net-payload` chunk and removed the instant the body completes or overflows the cap. Shares
    /// the binding map's bounded-per-epoch shape and [`close_session`] eviction (see the type doc).
    reassemblers: alloc::collections::BTreeMap<SessionId, Reassembler>,
    /// The matched [`Route`] per session, stored on `net-address` admission — the real source of the
    /// `net-payload` recognizer envelope (Task 14). Shares the binding map's eviction shape.
    routes: alloc::collections::BTreeMap<SessionId, Route>,
    /// The pinned quarantine root fd (Sprint 9 Slice 2), a config hook set by
    /// [`Dispatcher::with_quarantine_root`]. `None` when the write path is not configured (e.g. a
    /// hardware-free routing test that sends no chunks) — a `file-chunk` then routes to the File
    /// handler unchanged. All confined opens resolve BENEATH this fd (`file.md` §1).
    quarantine_root: Option<OwnedFd>,
    /// One in-flight quarantine write per session (Sprint 9 Slice 2), created on `file-write` admission
    /// and removed on session-end or a chunk-deny. Shares the eviction shape; a *tighter* residual (an
    /// entry only while a file is mid-upload) whose per-file size is bounded by `QUARANTINE_WRITE_CAP`.
    /// The held fd closes on eviction (fd-lifecycle — scarcer than memory).
    writers: alloc::collections::BTreeMap<SessionId, QuarantineWriter>,
    /// The readable-root path (Sprint 10 Slice 2), a config hook set by
    /// [`Dispatcher::with_readable_root`] — the fail-safe FS **read** root (`fs_read`'s
    /// `readable_root`, the read-path analog of `quarantine_root`). `None` when the read path is not
    /// configured (a routing-only test), in which case a `file-read` routes as relayed (its pre-Slice-2
    /// disposition). A serving proxy always sets it (the ladder validates FS roots first).
    readable_root: Option<PathBuf>,
    /// One in-flight outbound reply stream per admitted `file-read`/`list` session (Sprint 10 Slice 2):
    /// `file-data` for a read, `list-entry` for a listing. Registered on admission, drained across
    /// `pump_replies` ticks (bounded lazy stream, WouldBlock-paced), evicted on reply-completion and
    /// `close_session` — its held source fd (`FileReader`/`DirReader`) closes on eviction. Shares the
    /// reassembler/writers bounded-per-epoch MB4 shape; a *tighter* residual (an entry only while a
    /// reply is mid-stream), and per-entry memory is one pulled-not-yet-sent frame, never the whole
    /// source.
    replies: alloc::collections::BTreeMap<SessionId, ReplyStream>,
    /// The set of **active display sessions** (Sprint 12 Slice 2) — a box-originated terminal session
    /// on which the box has emitted a `display`/`pane-state` (a live pty). Head input addressed to a
    /// display session `S` is routed to the box only if `S ∈ terminal_displays`; input for an unknown or
    /// **stale** `S` (its pty already closed → evicted here) is **dropped, never misrouted**. This is
    /// the new per-session resource the two-session PTY model introduces (the bidirectional model would
    /// have needed none): **bounded per-epoch, evicted on [`close_session`]**, the same shape as the
    /// binding/reassembler/writers/replies maps — no leak of the correlation state. (The proxy-originated
    /// input *stream* itself lives in `replies`, keyed by `S`, evicted by the same hook.)
    terminal_displays: alloc::collections::BTreeSet<SessionId>,
    /// The per-session reassembly capacity bound. In production this is always
    /// `proto::verb::REASSEMBLY_CAP` (set by [`Dispatcher::new`]); a `#[cfg(test)]` constructor may
    /// inject a small cap to exercise the evict-on-overflow path cheaply — the field is otherwise
    /// invisible to callers.
    reassembly_cap: usize,
    /// Count of frames dropped as unsupported/undecodable — a liveness/observability counter, not a
    /// gate. (No box content ever enters it — a verb id and a `Reject` class are safe by
    /// construction.)
    dropped: u64,
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Dispatcher {
    #[must_use]
    pub fn new() -> Self {
        Dispatcher {
            bindings: alloc::collections::BTreeMap::new(),
            reassemblers: alloc::collections::BTreeMap::new(),
            routes: alloc::collections::BTreeMap::new(),
            quarantine_root: None,
            writers: alloc::collections::BTreeMap::new(),
            readable_root: None,
            replies: alloc::collections::BTreeMap::new(),
            terminal_displays: alloc::collections::BTreeSet::new(),
            reassembly_cap: verb::REASSEMBLY_CAP,
            dropped: 0,
        }
    }

    /// Configure the fail-safe FS **read** path (Sprint 10 Slice 2): `root` is the readable root every
    /// `file-read` source resolves under (`fs_read`'s `readable_root` — the read-path analog of
    /// [`with_quarantine_root`](Self::with_quarantine_root)). Without it, a `file-read` routes as
    /// relayed (its pre-Slice-2 disposition); a serving proxy always sets it (the ladder validates FS
    /// roots first).
    /// **A builder method, taking `self`** — deliberately, and this is a bug fix, not a style change
    /// (Sprint 16). Both root setters used to be *associated constructors* that filled their own field
    /// and defaulted every other one (`..Dispatcher::new()`), so calling both left only the second:
    /// **no dispatcher could hold both roots at once**, and a servable proxy needs both (readable for
    /// `file-read`/`list`, quarantine for `file-chunk` writes). Taking and returning `self` composes —
    /// `Dispatcher::new().with_readable_root(r).with_quarantine_root(q)` — and removes the hazard
    /// structurally: a method that consumes `self` has nothing to reset.
    #[must_use]
    pub fn with_readable_root(mut self, root: PathBuf) -> Self {
        self.readable_root = Some(root);
        self
    }

    /// Configure the quarantine write path (Sprint 9 Slice 2): `root` is the pinned quarantine root fd
    /// (opened once by the composition root via [`crate::open_discipline::pin_root`] — a config hook,
    /// the same shape as `fs_read`'s `readable_root`). Every `file-chunk`'s confined open resolves
    /// BENEATH this fd. Without it, the write path is inert (a `file-chunk` routes to the handler
    /// unchanged); a serving proxy always sets it (the lifecycle ladder validates the FS roots first).
    /// A builder method taking `self` — see [`Dispatcher::with_readable_root`] for why both setters
    /// compose rather than each defaulting the other away.
    #[must_use]
    pub fn with_quarantine_root(mut self, root: OwnedFd) -> Self {
        self.quarantine_root = Some(root);
        self
    }

    /// **Test-only** ([`cfg(test)`]): a dispatcher whose net-payload reassembly is bounded by an
    /// explicit small `cap`, so the dispatch-level evict-on-overflow behavior is reachable cheaply
    /// (the production cap is 32 MiB). This does **not** exist in the production build — the
    /// `#[cfg(test)]` gate keeps it out of the public API entirely. The proto-level reject-before-grow
    /// mechanism itself is cited, not re-proven here (it is exhaustively certified in `proto/tests`
    /// with a small cap); this only confirms dispatch *propagates* the reject and *evicts* the entry.
    #[cfg(test)]
    fn with_reassembly_cap_for_test(cap: usize) -> Self {
        Dispatcher {
            reassembly_cap: cap,
            ..Dispatcher::new()
        }
    }

    /// Route one reactor-delivered frame: decode its L2 payload, bind or check the session, dispatch by
    /// verb, and — on a JC-3(b) policy deny — enqueue the box-facing `file-error`/`net-error`
    /// notification. `register` is the tier-1 route register (U5's compiled-in `register::net::ROUTES`)
    /// and `vetted` the tier-1 vetted-tool register (`register::anthropic::VETTED_TOOLS`) — the two seams
    /// that reach the certified core. **Total**: every input maps to exactly one [`Routed`], no panic.
    pub fn route(
        &mut self,
        session: SessionId,
        payload: &[u8],
        register: &[Route],
        vetted: &[ToolIdentity],
    ) -> Routed {
        let routed = self.route_inner(session, payload, register, vetted);
        // JC-3(b) (`box-proxy.md` §2, Sprint 10 Slice 4): a policy deny on an *acted* service notifies
        // the box with the service's error verb, over the SAME push reply seam as net-response. Reused,
        // never a re-implemented send path. Structural rejects (JC-3a), relayed/unsupported verbs, and
        // admits do NOT notify — see [`notify_deny`](Self::notify_deny).
        self.notify_deny(session, &routed);
        routed
    }

    /// The routing + binding + dispatch decision, without the JC-3(b) notification side effect (that is
    /// [`route`](Self::route)'s wrapper). Returns the [`Routed`] verdict.
    fn route_inner(
        &mut self,
        session: SessionId,
        payload: &[u8],
        register: &[Route],
        vetted: &[ToolIdentity],
    ) -> Routed {
        // Peek the verb ONLY — no body decode (proxy-head.md §1: routing + binding verb only).
        let Some(verb) = peek_verb(payload) else {
            self.dropped += 1;
            return Routed::Decode(Reject::ShortBody {
                needed: 4,
                got: payload.len(),
            });
        };
        // JC-2 precedence (box-proxy.md §2, the settled fork): `service_of_verb` runs on the MASKED low
        // byte FIRST. An unsupported low byte is never *acted on*, so it denies as `Unsupported`
        // regardless of any reserved bits — JC-1 applies only to messages the proxy acts on, so the
        // reserved-check must not run for a verb the proxy would never act on.
        let Some(service) = service_of_verb(verb) else {
            self.dropped += 1;
            return Routed::Unsupported { verb };
        };

        // JC-2 (later frame, wrong service): a mismatched verb is likewise never acted on for this
        // session — `Unsupported`, *before* the JC-1 reserved-check, and without disturbing the standing
        // binding (the session stands; only this message denies).
        if let Some(&bound) = self.bindings.get(&session)
            && bound != service
        {
            self.dropped += 1;
            return Routed::Unsupported { verb };
        }

        // JC-1 (box-proxy.md §2): a message the proxy *acts on* whose **raw** verb `u32` carries a
        // nonzero reserved surface (upper three bytes) denies the whole message, **deny-without-binding**
        // — returned before the binding insert, so a reserved first frame never leaves a session
        // half-bound. Checked on the raw peek_verb u32 (a masked-low-byte check would pass vacuously);
        // reached only for a recognized, matched, acted-on verb (the JC-2 checks above ran first).
        if acts_on(verb) && (verb & 0xFFFF_FF00) != 0 {
            return Routed::ReservedDeny { verb };
        }

        // Bind on the first frame (both JC-2 and JC-1 passed); a matched later frame is a no-op.
        self.bindings.entry(session).or_insert(service);

        self.dispatch(session, verb, payload, register, vetted)
    }

    /// Route a bound frame by verb. The **gated** channels (net-address, file-write) — and *only*
    /// those — decode their specific body and invoke the certified tier-1 core (`library.md` §7's
    /// "deep-parse only what it acts on"). Every **relay** channel returns a [`RelayKind`] with the
    /// body **un-decoded** — the caller forwards the opaque payload; the receiver's own parser
    /// handles it. So the proxy runs *no* terminal/display/input decoder on box bytes.
    fn dispatch(
        &mut self,
        session: SessionId,
        verb: u32,
        payload: &[u8],
        register: &[Route],
        vetted: &[ToolIdentity],
    ) -> Routed {
        let body = &payload[4..]; // safe: peek_verb confirmed payload.len() >= 4
        match verb & 0xff {
            // ── Gated: deep-parse the body, invoke the tier-1 core (the only decodes on box bytes) ──
            verb::NET_ADDRESS => match NetAddress::decode(body) {
                Ok(addr) => match crate::route::match_route(&addr, register) {
                    Ok(route) => {
                        // Store the matched Route so this session's later net-payload sources its
                        // recognizer envelope from the real route (Task 14), not a hardcoded stub.
                        self.routes.insert(session, route);
                        Routed::NetRouteAdmit(route)
                    }
                    Err(_) => Routed::NetDeny,
                },
                Err(reject) => Routed::Decode(reject),
            },
            verb::FILE_WRITE => match FileWrite::decode(body) {
                // The single-scan-site path (RQ2): FileWrite::decode already scan_safe-validated the
                // stack, so the core does count-only via leaf_validate_prescanned.
                Ok(fw) => match crate::leaf::leaf_validate_prescanned(&fw.0) {
                    Ok(()) => {
                        // Admit: declare the per-session quarantine write for the ensuing file-chunks
                        // (Slice 2). The fd is opened lazily on the first chunk (no empty file on a
                        // file-write-then-abandon). The destination was just validated; store it.
                        self.writers.insert(session, QuarantineWriter::declare(fw.0));
                        Routed::FileWriteAdmit
                    }
                    Err(_) => Routed::FileWriteDeny,
                },
                Err(reject) => Routed::Decode(reject),
            },

            // ── Gated stateful channel: reassemble the body, then invoke the tier-1 recognizer ─────
            verb::NET_PAYLOAD => self.route_net_payload(session, body, vetted),

            // ── Gated stateful channel: append the chunk to the session's confined quarantine file ──
            verb::FILE_CHUNK => self.route_file_chunk(session, body),
            verb::OPEN_SESSION => Routed::ToServiceHandler(Service::Control),

            // ── Gated fail-safe read channels: open the confined source, stream the reply back (S10 S2) ──
            verb::FILE_READ => self.route_file_read(session, body),
            verb::LIST => self.route_list(session, body),

            // ── Relay channels: forward the OPAQUE payload; NO body decode (proxy-head.md §1) ──────
            verb::NET_RESPONSE | verb::NET_ERROR => Routed::Relay(RelayKind::CloudResponse),
            // ── Origination-keyed split of the box→proxy (NIC-ingress) file verbs (Sprint 15 Slice 2) ──
            // A `list-entry`/`file-error` arriving here is BOX-AUTHORED — the box answering a
            // head-initiated file-browse — so it is relayed to the HEAD (by WHERE it came from, not by
            // decoding the body). A proxy-authored `list-entry`/`file-error` answering a *box*-initiated
            // op never reaches `route()` (it is emitted straight to the box via the reply queue) — the two
            // disjoint paths of the confused-deputy floor.
            verb::LIST_ENTRY | verb::FILE_ERROR => Routed::Relay(RelayKind::FileToHead),
            // `file-data` and `request-file` arriving box→proxy are ANOMALOUS: `file-data` is
            // proxy→box-only (the box never emits it), `request-file` is head→box-only. A box emitting
            // either is protocol noise the proxy acts on in NO direction → fail safe (dropped, logged,
            // never relayed, never a filesystem touch), exactly like a JC-2 unsupported verb.
            verb::FILE_DATA | verb::REQUEST_FILE => Routed::Unsupported { verb },
            verb::DISPLAY | verb::PANE_STATE | verb::KEY | verb::MOUSE | verb::PASTE | verb::FOCUS
            | verb::CREATE_PTY | verb::RESIZE | verb::VIEWPORT => {
                // A box→head `display`/`pane-state` marks `session` an ACTIVE display session (Slice 2):
                // a live pty the head can now address input to. Only the box→head direction marks it —
                // head→box input arrives via the head connection, never the box wire.
                if crate::terminal_relay::direction_of(verb) == Some(crate::terminal_relay::Direction::BoxToHead) {
                    self.terminal_displays.insert(session);
                }
                Routed::Relay(RelayKind::Terminal)
            }

            // An unknown id within a known block — no service supports it.
            _ => {
                self.dropped += 1;
                Routed::Unsupported { verb }
            }
        }
    }

    /// The `net-payload` handler (Sprint 7): decode this chunk's `[flag][content]`, push its
    /// **content** into the session's [`Reassembler`] with the header's `more` flag, and — only when
    /// the body is complete — hand the whole reassembled buffer to the tier-1 Anthropic recognizer.
    ///
    /// The reassembler concatenates *content* (the 1-byte flag is stripped at decode and never enters
    /// the buffer — the collision-freedom property, `proto::net` / net.md §2), so the completed body
    /// is byte-identical to the box's request body. On `Complete`, the same buffer is what
    /// [`recognize`] vets and what [`Routed::NetPayloadAdmit`] carries forward to be forwarded — one
    /// buffer, no split (CB4-across-the-seam). The recognizer envelope's domain/path are sourced from
    /// the session's admitted [`Route`] (Task 14); a completion with no stored Route fails closed
    /// ([`Routed::NetPayloadNoRoute`]). Fail-closed on cap-overflow (the poisoned reassembler is
    /// evicted) and on a malformed chunk header (`ShortBody`).
    fn route_net_payload(&mut self, session: SessionId, body: &[u8], vetted: &[ToolIdentity]) -> Routed {
        // Decode this chunk: strip the fixed 1-byte continuation-flag header, keep the content tail.
        let chunk = match NetPayload::decode(body) {
            Ok(chunk) => chunk,
            Err(reject) => {
                self.dropped += 1;
                return Routed::Decode(reject);
            }
        };
        let cap = self.reassembly_cap; // read before the mutable entry borrow (no self-alias)
        let reassembler = self
            .reassemblers
            .entry(session)
            .or_insert_with(|| Reassembler::new(cap));
        match reassembler.push(&chunk.content, chunk.more) {
            Ok(ReassemblyState::Accumulating) => Routed::NetPayloadAccumulating,
            Ok(ReassemblyState::Complete(request_body)) => {
                // The body is whole; the reassembler already reset, but drop the map entry so the
                // next request on this session starts a fresh reassembly (and the residual shrinks).
                self.reassemblers.remove(&session);
                // Source the recognizer envelope from the session's admitted Route (Task 14). No
                // stored Route (no prior admitted net-address) → fail closed: nothing to vet the
                // request *as*, so it is neither recognized nor forwarded.
                let Some(route) = self.routes.get(&session).copied() else {
                    return Routed::NetPayloadNoRoute;
                };
                let (domain, path) = split_route(route);
                // verb/betas are justified constants for the net service: the wire (`NetAddress` +
                // `net-payload`) conveys neither an HTTP method nor beta headers, and the recognizer
                // reads betas only from this envelope field, never from the body — so `betas: &[]` is
                // the literal truth of what the box can express, not a dropped field.
                let request = Request {
                    verb: "POST",
                    domain,
                    path,
                    betas: &[],
                    body: &request_body,
                };
                match recognize(&request, vetted) {
                    Ok(()) => Routed::NetPayloadAdmit(request_body),
                    Err(denial) => Routed::NetPayloadDeny(denial),
                }
            }
            Err(reject) => {
                // Cap-exceeded (or an absurd-length overflow): fail closed and evict the poisoned
                // reassembler so a subsequent chunk cannot resume a rejected, over-cap accumulation.
                self.reassemblers.remove(&session);
                self.dropped += 1;
                Routed::Decode(reject)
            }
        }
    }

    /// The `file-chunk` handler (Sprint 9 Slice 2): append the chunk to the session's confined
    /// quarantine file via the reused `open_discipline` confined-create (full write-floor mask, opened
    /// lazily on the first chunk). **Fail-closed** — a confined-open deny (traversal/symlink/exists),
    /// a per-file cap trip, a write error, or a chunk with no admitted `file-write` evicts the poisoned
    /// writer and denies. Without a configured quarantine root the write path is inert (the pre-Slice-2
    /// handler disposition), which a serving proxy never hits (the ladder validates FS roots first).
    fn route_file_chunk(&mut self, session: SessionId, body: &[u8]) -> Routed {
        let Some(root) = self.quarantine_root.as_ref() else {
            return Routed::ToServiceHandler(Service::File); // write path not configured (routing-only)
        };
        let root_fd = root.as_fd();
        let Some(writer) = self.writers.get_mut(&session) else {
            // A chunk with no admitted file-write — structural protocol-noise (JC-3a): silent, no notify.
            return Routed::FileChunkNoWriter;
        };
        match writer.write_chunk(root_fd, body) {
            WriteStep::Appended(n) => Routed::FileChunkAppended { bytes: n },
            // A meaningful outcome the box must know → evict the poisoned writer and deny with the ruled
            // code (designer ruling A): confined-open refusal = the write-destination floor firing late
            // → Denied; per-file cap trip → Malformed; write i/o error → Unavailable.
            WriteStep::Denied(_) => {
                self.writers.remove(&session);
                Routed::FileChunkDeny { code: ErrorCode::Denied }
            }
            WriteStep::Capped => {
                self.writers.remove(&session);
                Routed::FileChunkDeny { code: ErrorCode::Malformed }
            }
            WriteStep::WriteFailed(_) => {
                self.writers.remove(&session);
                Routed::FileChunkDeny { code: ErrorCode::Unavailable }
            }
        }
    }

    /// Admit a `file-read` (Sprint 10 Slice 2): decode the source, open it **fail-safe** under the
    /// readable root via the reused [`FileReader`] (`O_NOFOLLOW`-final + regular-file check, unchanged),
    /// and register a per-session outbound [`ReplyStream`] that later `pump_replies` ticks drain as
    /// `file-data` frames. The stream's `in_reply_to` is **this request's own session id** (`session`) —
    /// the box correlates the reply to its request by that echoed id, never by the reply's own
    /// (proxy-high) session. A failed open is fail-safe ([`Routed::FileReadError`], no stream, no box
    /// name); the box-facing `file-error` *notification* is JC-3's job (Slice 4). No readable root
    /// configured ⇒ relay (the pre-Slice-2 disposition), never a silent accept.
    fn route_file_read(&mut self, session: SessionId, body: &[u8]) -> Routed {
        let Some(root) = self.readable_root.as_ref() else {
            return Routed::Relay(RelayKind::FileRead); // read path not configured (routing-only)
        };
        match FileRead::decode(body) {
            Ok(fr) => match FileReader::open(root, &fr.source) {
                Ok(reader) => {
                    // in_reply_to = the ORIGINATING request's session (`session`), NOT the reply session
                    // `pump_replies` opens — the box correlates on the request id it already holds.
                    self.replies
                        .insert(session, ReplyStream::new(ReplySource::File { reader, in_reply_to: session }));
                    Routed::FileReadStreaming
                }
                Err(err) => Routed::FileReadError { code: err.code },
            },
            Err(reject) => Routed::Decode(reject),
        }
    }

    /// Admit a `list` (Sprint 10 Slice 2): decode the source directory, open it **fail-safe** under the
    /// readable root via the confined [`DirReader`] (path-based `read_dir`, same unsayable-traversal
    /// confinement as [`FileReader`], no new `unsafe`), and register a per-session outbound
    /// [`ReplyStream`] that streams `list-entry` records back (one per frame, bounded-lazy). Like
    /// `route_file_read`: `in_reply_to` is **this request's own session** (`session`); a failed open is
    /// fail-safe ([`Routed::ListError`], no stream, no box name); no readable root ⇒ relay.
    fn route_list(&mut self, session: SessionId, body: &[u8]) -> Routed {
        let Some(root) = self.readable_root.as_ref() else {
            return Routed::Relay(RelayKind::FileRead); // read path not configured (routing-only)
        };
        match List::decode(body) {
            Ok(list) => match DirReader::open(root, &list.0) {
                Ok(reader) => {
                    self.replies
                        .insert(session, ReplyStream::new(ReplySource::Dir { reader, in_reply_to: session }));
                    Routed::ListStreaming
                }
                Err(err) => Routed::ListError { code: err.code },
            },
            Err(reject) => Routed::Decode(reject),
        }
    }

    /// Drive every in-flight outbound reply stream one **pump tick** (Sprint 10 Slice 2): called from
    /// the run-loop adapter on each `Event::Delivered`/`Event::Idle`, with the [`Reply`] handle live for
    /// this callback invocation only (never held across calls). Bounded lazy stream, **WouldBlock-paced**
    /// (F2): each stream drains as far as the wire accepts — one body pulled from the source (a
    /// `file-data` chunk or a `list-entry` record), stamped with `in_reply_to` on EVERY frame, emitted
    /// (first frame `open`s the reply session, the rest `send` on it); on [`SendError::WouldBlock`] the
    /// pulled frame is **retained**
    /// and retried next tick (no drop — reliable-ordered), so backpressure reaches the source and the
    /// whole file is never buffered. Completed/errored streams are evicted after the pass (their
    /// `FileReader` fd closes on drop — fd-lifecycle). Never panics.
    pub fn pump_replies(&mut self, reply: &mut dyn Reply) {
        // NAMED RESIDUAL — fairness, non-blocking (reviewer-ruled Slice 2, 2026-07-22: track-don't-fix):
        // streams drain in BTreeMap id-order, so on the stop-and-wait wire a long low-id reply can
        // deterministically starve higher-id replies until it completes. LIVENESS HOLDS (every reply is
        // finite; no deadlock) and this matches the transport's own no-scheduler posture. The cheap fix,
        // if concurrent-reply latency ever matters, is to rotate the drain-start index (round-robin) —
        // deliberately NOT built now (not worth the scope for a fairness nicety).
        for stream in self.replies.values_mut() {
            stream.pump(reply);
        }
        // Evict finished streams — dropping each ReplyStream closes its source fd (fd-lifecycle).
        self.replies.retain(|_, s| !s.done);
    }

    /// Enqueue one upstream cloud-response chunk for the box (Sprint 10 Slice 3) — `net-response`,
    /// stamped with `box_session` (the box's ORIGINATING net-request session) as `in_reply_to`. Fed by
    /// the cloud ingress (a **fake** in tests; the real one is `upstream::AnthropicClient`, a TLS client
    /// to the upstream — the box-facing side of this queue carries plaintext and no TLS whatsoever),
    /// drained to the box on `pump_replies` ticks. Bounded: returns [`Enqueue::Full`] as **backpressure**
    /// at [`REPLY_QUEUE_CAP`] — the ingress must then stop reading upstream and retry (never drop, never
    /// grow). Creates the per-session push stream on the first accepted chunk.
    pub fn enqueue_net_response(&mut self, box_session: SessionId, chunk: &[u8]) -> Enqueue {
        let body = crate::cloud_relay::relay_response(chunk);
        let frame = task16_stub_reply_frame(box_session, &body);
        self.push_reply_frame(box_session, frame)
    }

    /// Route a head→box terminal INPUT for display-session `s_display` (Sprint 12 Slice 2) — the head
    /// addressed its input to the pty it observed on `s_display`. **Fail-safe, never misroute**: if
    /// `s_display` is not an ACTIVE display session (unknown, or **stale** after its pty closed — evicted
    /// from `terminal_displays` by `close_session`), the input is **DROPPED** ([`TerminalInput::Dropped`]),
    /// never routed to whatever session now holds that id. Otherwise it is enqueued to the box on a
    /// proxy-originated (high-half) input session, **stamped with `s_display`** as the pty-routing
    /// correlation, and the push stream is **keyed by `s_display`** (not the input session's own id), so
    /// the box routes it to the pty that opened `s_display`.
    ///
    /// **This SUPERSEDES Sprint 10's verbatim `enqueue_terminal_to_box`** (no correlation key): that was
    /// correct for the *reply* correlation (F1 — input is not a reply), but the two-session PTY ruling
    /// (2026-07-23: a bidirectional session is impossible — ACK and empty-data frames are byte-identical)
    /// requires a *pty-routing* correlation, a genuinely different thing. A forced consequence of the
    /// ruling, not new policy.
    pub fn route_head_input(&mut self, s_display: SessionId, input_payload: &[u8]) -> TerminalInput {
        if !self.terminal_displays.contains(&s_display) {
            // No live pty for this display session (unknown or stale-after-close) → fail safe.
            return TerminalInput::Dropped;
        }
        let frame = task16_terminal_correlation_frame(s_display, input_payload);
        match self.push_reply_frame(s_display, frame) {
            // Keyed by s_display; the push is bounded (backpressure, never a drop — Sprint 13 Slice 1.5).
            Enqueue::Accepted => TerminalInput::Routed,
            // Full: NOT taken — the caller retains this input and stops draining so the socket backpressures.
            Enqueue::Full => TerminalInput::Backpressure,
        }
    }

    /// The `Route` U3 matched for `session`, if one was admitted — the **register's** destination for
    /// this exchange (Sprint 19).
    ///
    /// Narrow on purpose: it exposes the already-stored route and nothing else. The route was always kept
    /// here (U4 sources its envelope from it) and then discarded, so the dial fell back to a constant held
    /// elsewhere; this is the one accessor that lets the destination reach the dial from the register
    /// instead. `None` means no admitted `net-address` for this session, which the fail-closed
    /// `NetPayloadNoRoute` rung already prevents from reaching a dial.
    #[must_use]
    pub fn route_for(&self, session: SessionId) -> Option<Route> {
        self.routes.get(&session).copied()
    }

    /// **Test/observability**: the number of active display sessions (the Slice-2 MB4 resource).
    #[must_use]
    pub fn terminal_display_count(&self) -> usize {
        self.terminal_displays.len()
    }

    /// Enqueue a single-frame `net-error(code)` notification to the box (Sprint 11) — the JC-3 net-error
    /// seam reused for a proxy-side net deny that is **not** a route verdict. Its one use this sprint:
    /// the serial upstream bound (`bound = 1`) — a second `net-payload` admitted while one upstream call
    /// is already in flight is denied `net-error(Unavailable)`, honest to the box immediately rather
    /// than silently held. Stamped `in_reply_to = session`, single-frame (enqueue then close → drains →
    /// evicts).
    pub fn enqueue_net_error(&mut self, session: SessionId, code: ErrorCode) -> Enqueue {
        let frame = task16_stub_reply_frame(session, &net_error(code));
        match self.push_reply_frame(session, frame) {
            // Enqueued: close (single-frame notification → drains → evicts).
            Enqueue::Accepted => {
                self.close_reply(session);
                Enqueue::Accepted
            }
            // Full: NOT enqueued and NOT closed (Sprint 13 Slice 1.5) — the caller retains + retries, so
            // the error notification is never lost to a full queue (the same backpressure as a chunk).
            Enqueue::Full => Enqueue::Full,
        }
    }

    /// Enqueue a prepared frame into `box_session`'s push reply queue, bounded by [`REPLY_QUEUE_CAP`]:
    /// **backpressure** ([`Enqueue::Full`]) when full — never drop, never grow. Creates the queue stream
    /// on first use.
    fn push_reply_frame(&mut self, box_session: SessionId, frame: alloc::vec::Vec<u8>) -> Enqueue {
        let stream = self.replies.entry(box_session).or_insert_with(|| {
            ReplyStream::new(ReplySource::Queue {
                frames: alloc::collections::VecDeque::new(),
                closed: false,
            })
        });
        match &mut stream.source {
            ReplySource::Queue { frames, .. } => {
                if frames.len() >= REPLY_QUEUE_CAP {
                    Enqueue::Full // backpressure: not taken, not dropped, not grown
                } else {
                    frames.push_back(frame);
                    Enqueue::Accepted
                }
            }
            // The session already carries a pull stream (file-read/list) — a net/terminal frame for it is
            // a mis-key (services are session-bound, so this cannot arise); ignore defensively.
            _ => Enqueue::Accepted,
        }
    }

    /// Signal end-of-stream for a push reply (upstream/head EOF): the queue drains, then the stream
    /// completes and is evicted on a later `pump_replies` pass (EMPTY != DONE holds until this). A no-op
    /// for an unknown / non-push session.
    pub fn close_reply(&mut self, box_session: SessionId) {
        if let Some(stream) = self.replies.get_mut(&box_session)
            && let ReplySource::Queue { closed, .. } = &mut stream.source
        {
            *closed = true;
        }
    }

    /// JC-3(b) deny notification (Sprint 10 Slice 4): map a [`Routed`] policy-deny verdict to the
    /// box-facing `file-error`/`net-error` and enqueue a single frame on `session`'s reply queue,
    /// stamped `in_reply_to = session` (a deny is a reply to the box's request), then close it (single
    /// frame → drains → evicted). Reuses the push seam; **no** box name in the error body (`file.md` §0).
    ///
    /// The taxonomy (`box-proxy.md` §2 JC-3 + §3.1; lead-ruled 2026-07-22):
    /// - `file-write` deny → `file-error(Denied)`; `file-read`/`list` fail-safe → `file-error(<code>)`.
    /// - `net-address` deny, recognizer deny, no-admitted-route → `net-error(Denied)`.
    /// - net-payload reassembly cap-trip → `net-error(Malformed)` — the ONE structural reject the spec
    ///   elevates to a notify; every OTHER `Decode` reject is JC-3(a) structural (drop+log, no return).
    /// - reserved-verb deny (JC-1) → the verb's service error, `Malformed` (a formal defect in how the
    ///   box built the message, like the cap-trip — lead ruling B).
    /// - `file-chunk` deny (designer ruling A — sub-cause split): confined-open refusal → `file-error
    ///   (Denied)` (the write-destination floor firing one message late), cap-trip → `file-error
    ///   (Malformed)`, i/o error → `file-error(Unavailable)`; a chunk with **no admitted file-write** is
    ///   silent structural noise ([`Routed::FileChunkNoWriter`]) → no notification.
    /// - **No notification**: unsupported/mismatched verbs (JC-2, never bound to a service — ruling C),
    ///   other structural rejects (JC-3a), `file-chunk`-no-writer noise, and every non-deny verdict.
    fn notify_deny(&mut self, session: SessionId, routed: &Routed) {
        let body = match routed {
            Routed::FileWriteDeny => Some(file_error(ErrorCode::Denied)),
            Routed::FileReadError { code }
            | Routed::ListError { code }
            | Routed::FileChunkDeny { code } => Some(file_error(*code)),
            Routed::NetDeny | Routed::NetPayloadDeny(_) | Routed::NetPayloadNoRoute => {
                Some(net_error(ErrorCode::Denied))
            }
            Routed::Decode(Reject::ReassemblyOverflow { .. }) => Some(net_error(ErrorCode::Malformed)),
            Routed::ReservedDeny { verb } => Some(match service_of_verb(*verb) {
                Some(Service::Net) => net_error(ErrorCode::Malformed),
                _ => file_error(ErrorCode::Malformed),
            }),
            _ => None,
        };
        if let Some(body) = body {
            let frame = task16_stub_reply_frame(session, &body);
            self.push_reply_frame(session, frame);
            self.close_reply(session); // single-frame notification: enqueue then close → drains → evicts
        }
    }

    /// Evict a session's binding **and any in-flight reassembly buffer** — the **eviction hook** for
    /// the MB4 bounded-resource residual. It is structurally ready to wire to a transport
    /// session-close/give-up signal; that signal is not in the committed transport yet (the same
    /// cross-crate gap the residual names), so the hook is present-and-tested but not yet *driven*.
    /// Returns the evicted binding (the reassembler and stored route, if any, are dropped silently —
    /// they hold only in-flight box content / a matched route, nothing observable).
    pub fn close_session(&mut self, session: SessionId) -> Option<Service> {
        self.reassemblers.remove(&session);
        self.routes.remove(&session);
        // Drop the session's quarantine writer — its held fd (an owned File) closes here (fd-lifecycle:
        // an fd leak is more acute than the map residuals). This IS the file's completion (file.md §1:
        // completion is the transport session ending — no wire close signal).
        self.writers.remove(&session);
        // Drop any in-flight outbound reply stream — its source fd (a FileReader) closes here too. A
        // reply cut short by session-end simply stops mid-stream (fail-safe: the box got a prefix).
        self.replies.remove(&session);
        // Evict the display-session-active mark (Sprint 12 Slice 2): a closed pty's `S` is no longer a
        // valid input target — later head input for it fails safe (dropped, never misrouted). The
        // input stream keyed by `S` in `replies` was already evicted just above.
        self.terminal_displays.remove(&session);
        self.bindings.remove(&session)
    }

    /// The number of in-flight outbound reply streams (test/observability) — the tighter MB4 residual
    /// (an entry only while a `file-read` is mid-stream; evicted on completion and `close_session`).
    #[must_use]
    pub fn replies_count(&self) -> usize {
        self.replies.len()
    }

    /// The number of live session bindings — the O(sessions-per-epoch) resource the residual names.
    #[must_use]
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    /// Frames dropped as unsupported/undecodable (liveness/observability, not a gate).
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The service a session is bound to, if any (test/observability).
    #[must_use]
    pub fn binding(&self, session: SessionId) -> Option<Service> {
        self.bindings.get(&session).copied()
    }

    /// The number of sessions with an in-flight net-payload reassembly — the tighter MB4 residual
    /// (an entry lives only between a session's first chunk and its completing/overflowing chunk).
    #[must_use]
    pub fn reassembling_count(&self) -> usize {
        self.reassemblers.len()
    }

    /// The number of sessions holding a stored matched `Route` (test/observability) — used to prove
    /// deny-without-binding leaves no route trace, alongside `binding_count`/`reassembling_count`.
    #[must_use]
    pub fn routes_count(&self) -> usize {
        self.routes.len()
    }

    /// The number of sessions holding an in-flight quarantine writer (test/observability) — used to
    /// prove the held fd is closed on session-end / chunk-deny (the fd-lifecycle gate).
    #[must_use]
    pub fn writes_count(&self) -> usize {
        self.writers.len()
    }
}

/// Split a canonical [`Route`] (`https://<domain>[/<path…>]`) into `(domain, path)` for the
/// recognizer envelope (Task 14). The `Route` is validated-canonical by construction
/// ([`crate::vocab::Route::new`]), so the `https://` prefix is guaranteed and the split is total:
/// the first `/` after the scheme separates the domain from the path (which keeps its leading `/`);
/// a path-less route (a bare domain) yields the root path `/`. The returned slices are `&'static`
/// (the `Route` owns a `&'static str`), so they outlive the `&self` borrow and can back a `Request`.
fn split_route(route: Route) -> (&'static str, &'static str) {
    // Delegates to the ONE decomposition on `Route` itself (Sprint 18 Slice 2). U4's envelope and the
    // upstream dial both derive host/path from a matched route; two implementations that happened to
    // agree would let U4 check one destination while the dial used another — the same
    // agreement-by-authorship gap Sprint 19 removed between the register and the dial.
    route.split()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// A raw `net-payload` L2 frame: `[verb][1-byte flag][content]` (flag `0x00` = more, nonzero =
    /// final) — the bytes `route` receives. Built by hand to keep this unit test dependency-light.
    fn net_payload_frame(content: &[u8], more: bool) -> Vec<u8> {
        let mut v = verb::NET_PAYLOAD.to_le_bytes().to_vec();
        v.push(if more { 0 } else { 1 });
        v.extend_from_slice(content);
        v
    }

    /// A one-frame [`Reply`] recorder — captures the single JC-3 notification frame (`open`ed first).
    struct CaptureReply(Option<Vec<u8>>);
    impl Reply for CaptureReply {
        fn open(&mut self, payload: &[u8]) -> SessionId {
            self.0 = Some(payload.to_vec());
            SessionId(1 << 63) // proxy-high; cosmetic for this recorder
        }
        fn send(&mut self, _id: SessionId, _payload: &[u8]) -> Result<(), SendError> {
            Ok(())
        }
    }

    /// Dispatch-level **evict-on-overflow** (the one genuinely new bit of logic at this layer): a
    /// per-session reassembly that exceeds the cap fails closed with `Decode(ReassemblyOverflow)`
    /// **and** evicts the poisoned buffer, so a later chunk cannot resume an over-cap accumulation.
    /// Uses the `#[cfg(test)]` small-cap injector so the overflow is reachable cheaply (production cap
    /// is 32 MiB). The reject-before-grow mechanism itself is cited, not re-proven — it is exhaustively
    /// certified in `proto/tests` (`reassembler.rs`, small-cap boundary); this confirms only that
    /// dispatch *propagates* the reject and *evicts* the entry. `register`/`vetted` are unused on the
    /// overflow path (the recognizer is never reached), so empty slices suffice.
    #[test]
    fn net_payload_overflow_fails_closed_and_evicts() {
        let mut d = Dispatcher::with_reassembly_cap_for_test(8);
        let s = SessionId(1);
        // First chunk (6 bytes, more): 6 ≤ 8 → accumulates, one in-flight buffer.
        assert_eq!(
            d.route(s, &net_payload_frame(&[0u8; 6], true), &[], &[]),
            Routed::NetPayloadAccumulating,
        );
        assert_eq!(d.reassembling_count(), 1);
        let dropped_before = d.dropped();
        // Second chunk (6 bytes): 6 + 6 = 12 > 8 → overflow, fail closed, evict the buffer.
        assert_eq!(
            d.route(s, &net_payload_frame(&[0u8; 6], true), &[], &[]),
            Routed::Decode(Reject::ReassemblyOverflow { cap: 8 }),
        );
        assert_eq!(d.reassembling_count(), 0, "the poisoned reassembler is evicted on overflow");
        assert_eq!(d.dropped(), dropped_before + 1, "the overflow frame is counted as dropped");

        // JC-3 (Slice 4): the cap-trip is the ONE structural reject the spec elevates to a notify —
        // net-error(Malformed), stamped in_reply_to = the request session, over the push reply seam.
        assert_eq!(d.replies_count(), 1, "the cap-trip enqueues a JC-3 net-error notification");
        let mut cap = CaptureReply(None);
        d.pump_replies(&mut cap);
        let frame = cap.0.expect("one notification frame emitted");
        assert_eq!(u64::from_be_bytes(frame[0..8].try_into().unwrap()), s.0, "stamped with the request session");
        match proto::decode_message(&frame[8..]).expect("a net-error message") {
            Message::NetError(ne) => assert_eq!(ne.0.code, ErrorCode::Malformed),
            other => panic!("expected net-error(Malformed), got {other:?}"),
        }

        // A fresh chunk on the same session starts a NEW reassembly (the evicted buffer did not
        // silently resume the over-cap accumulation).
        assert_eq!(
            d.route(s, &net_payload_frame(&[0u8; 4], true), &[], &[]),
            Routed::NetPayloadAccumulating,
        );
        assert_eq!(d.reassembling_count(), 1, "a post-overflow chunk begins a fresh, empty buffer");
    }
}
