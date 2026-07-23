//! Sprint 12 Slice 1 — the head-connection transport + framing foundation (isolated, the Sprint-10/11
//! "changed-seam" discipline). A **bidirectional, long-lived** relay between the proxy and the operator's
//! head over a Unix-domain **stream** socket: a reader thread decodes head→box frames and hands them to
//! the box-facing loop over a bounded channel; a writer thread drains box→head frames the loop produces
//! and writes them to the socket.
//!
//! **Framing.** Each frame is `[u32 length][8B SessionId][L2 message]` (length = `8 + payload.len()`,
//! little-endian). The `SessionId` is the box↔proxy session the frame pertains to — the head learns it
//! by observing box→head emissions (`model.md`: the box is the session allocator; the head reads the id
//! off the display/pane-state it receives) and addresses input back to it. A **length-delimited stream**
//! (not a message-preserving datagram) is deliberate: it needs no OS boundary-preservation, so it is
//! MB6-clean (`std::os::unix::net`, no `unsafe`, no `SOCK_SEQPACKET`-via-libc), has no datagram size cap,
//! and is portable to any remote-fronting byte transport the operator provisions — removing the
//! boundary-preservation residual entirely.
//!
//! **Bounded.** The frame length is capped ([`HEAD_FRAME_MAX`], reject-before-alloc — a malicious length
//! can never allocate unbounded); both channels are bounded ([`HEAD_CHANNEL_CAP`]) so a slow loop
//! backpressures the reader (and thus the socket) rather than buffering without limit.
//!
//! **Scope.** Slice 1 is the transport + framing foundation (above). **Slice 2** wires it both
//! directions through [`run_adapter_with_head`]: box→head `display`/`pane-state` relayed to the head,
//! and head→box input routed to the box on a proxy-originated (high-half) session **stamped with the
//! display-session correlation** — the two-session PTY model (a bidirectional session is impossible: an
//! ACK and an empty-data frame are byte-identical). Fail-safe: input for an unknown/stale display
//! session is dropped, never misrouted (the routing lives in [`Dispatcher::route_head_input`]). All
//! proven against a **fake head** over a `UnixStream` socketpair; the live proof against a **real** head
//! is held, same boundary as Sprint 8/11. **No new `unsafe`** (MB6): `std::os::unix::net` +
//! `std::thread` + `std::sync::mpsc`.

use std::io::{self, Read, Write};
use std::ops::ControlFlow;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

use directpath::Event;
use transport::SessionId;

use crate::dispatch::{Dispatcher, RelayKind, Routed, peek_verb};
use crate::nic::Reply;
use crate::terminal_relay::{Direction, direction_of};
use crate::vocab::{Route, ToolIdentity};

/// The bounded depth (frames) of each head↔loop channel. A full channel backpressures the producer (the
/// reader thread, and thus the socket) — never an unbounded buffer. Provisional, marked for review.
pub const HEAD_CHANNEL_CAP: usize = 256;

/// The maximum accepted frame length (`SessionId` + payload), in bytes — **reject-before-alloc**: a
/// decoded length beyond this is refused before any buffer is allocated, so a malicious/garbage length
/// cannot drive an unbounded allocation. A terminal frame (a screen update, a keystroke) is far under
/// this. Provisional (1 MiB), marked for review.
pub const HEAD_FRAME_MAX: usize = 1024 * 1024;

/// One head-connection frame: the box↔proxy `session` it pertains to, and its opaque L2 `payload`
/// (relayed verbatim — the proxy runs no terminal decoder).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HeadFrame {
    pub session: SessionId,
    pub payload: Vec<u8>,
}

/// Encode a frame to the wire form `[u32 len][8B session][payload]` (`len = 8 + payload.len()`, LE).
/// **Bounded (F4)**: an oversized frame (`8 + payload.len() > HEAD_FRAME_MAX`, or a `u32` overflow)
/// returns `None` rather than truncating the length via an unchecked cast — a too-large box display
/// frame is **dropped** (a tolerable misdraw), never written as a desynced/truncated length that would
/// corrupt the stream at the head. Symmetric with the decode side's reject-before-alloc bound.
#[must_use]
pub fn encode_head_frame(frame: &HeadFrame) -> Option<Vec<u8>> {
    let total = 8usize.checked_add(frame.payload.len()).filter(|&t| t <= HEAD_FRAME_MAX)?;
    let mut out = Vec::with_capacity(4 + total);
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&frame.session.0.to_le_bytes());
    out.extend_from_slice(&frame.payload);
    Some(out)
}

/// Decode one frame from a stream reader (blocking `read_exact` on the length prefix, then the body).
/// **Bounded** (rejects `len > HEAD_FRAME_MAX` before allocating) and total (a short/oversized length is
/// an `InvalidData` error, never a panic). Returns the framed [`HeadFrame`], or an i/o error on EOF /
/// malformed length / read failure.
pub fn decode_head_frame(reader: &mut impl Read) -> io::Result<HeadFrame> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len < 8 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "head frame shorter than a SessionId"));
    }
    if len > HEAD_FRAME_MAX {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "head frame exceeds HEAD_FRAME_MAX"));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    let session = SessionId(u64::from_le_bytes(buf[0..8].try_into().expect("len >= 8 checked")));
    let payload = buf[8..].to_vec();
    Ok(HeadFrame { session, payload })
}

/// The loop-side handle to a live head connection. The box-facing loop drains [`ingress`](Self::ingress)
/// each tick (head→box frames the reader decoded) and sends box→head frames via
/// [`egress`](Self::egress) (the writer thread encodes + writes them). The `Dispatcher` never crosses to
/// either thread — the only boundary is the two channels, carrying owned [`HeadFrame`]s.
pub struct HeadConnection {
    /// Head→box frames, decoded by the reader thread. Bounded; the reader (a separate thread) BLOCKS on
    /// a full channel — fine: that only backpressures the head *read*, never the loop. PRIVATE — drained
    /// **only** via [`try_recv_from_head`](Self::try_recv_from_head) (NON-BLOCKING), so the box-facing
    /// loop can never block on ingress any more than on egress (F3).
    ingress: Receiver<HeadFrame>,
    /// Box→head frames the loop produces; the writer thread encodes + writes them. Bounded, and enqueued
    /// **only** via [`send_to_head`](Self::send_to_head) — NON-BLOCKING, drop-on-full (see there).
    egress: SyncSender<HeadFrame>,
    /// A third socket handle, held solely to `shutdown(Both)` on teardown (F1): a proxy-initiated
    /// cleanup of a live/idle head must unblock the reader (`read_exact` → EOF) and writer (→ error)
    /// even when the head sent no disconnect — `join` alone cannot interrupt a blocked `read_exact`.
    shutdown_sock: UnixStream,
    reader: JoinHandle<()>,
    writer: JoinHandle<()>,
}

/// The outcome of a box→head enqueue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EgressOutcome {
    /// The frame was queued for the head.
    Sent,
    /// The channel was full (the head is slow/stalled — the writer is backed up on the socket) or the
    /// connection is gone; the frame was **DROPPED**, never blocked on.
    Dropped,
}

impl HeadConnection {
    /// Spawn the reader + writer threads over `stream` (bidirectional, long-lived until the socket
    /// closes). The stream is `try_clone`d so the two threads read and write independently.
    pub fn spawn(stream: UnixStream) -> io::Result<Self> {
        let read_sock = stream.try_clone()?;
        let shutdown_sock = stream.try_clone()?; // F1: to shutdown(Both) on teardown
        let write_sock = stream;
        let (ingress_tx, ingress_rx) = sync_channel(HEAD_CHANNEL_CAP);
        let (egress_tx, egress_rx) = sync_channel(HEAD_CHANNEL_CAP);
        let reader = std::thread::spawn(move || read_loop(read_sock, &ingress_tx));
        let writer = std::thread::spawn(move || write_loop(write_sock, &egress_rx));
        Ok(HeadConnection { ingress: ingress_rx, egress: egress_tx, shutdown_sock, reader, writer })
    }

    /// Drain ONE head→box frame — **NON-BLOCKING** (`try_recv`); `None` if none is available (or the
    /// connection is gone). The box-facing loop MUST use this, never a blocking `recv`: a blocking
    /// ingress drain would let a quiet head stall the loop exactly as an unbounded egress send would
    /// (the symmetric counterpart of [`send_to_head`](Self::send_to_head)). F3.
    #[must_use]
    pub fn try_recv_from_head(&self) -> Option<HeadFrame> {
        self.ingress.try_recv().ok()
    }

    /// Enqueue a box→head frame — **NON-BLOCKING, drop-on-full**. This is the box-facing loop's egress
    /// path, and it MUST never block: a blocking egress would let one slow/stalled head (even a
    /// non-adversarial remote head with latency) stall the ENTIRE loop — it could no longer drain
    /// ingress, pump the net/file reply streams, or serve any other session. So a full channel (the head
    /// is behind; the writer is backed up on the socket) DROPS the frame ([`EgressOutcome::Dropped`]).
    ///
    /// A dropped display frame is a **tolerable misdraw** (`terminal.md` §4 — legibility, not security;
    /// the head re-syncs on the next update). This is the mirror of Sprint 10's distinction:
    /// display-is-droppable, `file-data`-is-reliable — now applied to head egress.
    #[must_use]
    pub fn send_to_head(&self, frame: HeadFrame) -> EgressOutcome {
        match self.egress.try_send(frame) {
            Ok(()) => EgressOutcome::Sent,
            Err(_) => EgressOutcome::Dropped, // full or disconnected — never block the loop
        }
    }

    /// Join both worker threads (cleanup). **Drops the channel endpoints first** so the threads can
    /// terminate: dropping `egress` closes the writer's channel (its `recv` returns `Err` → it exits),
    /// and dropping `ingress` unblocks the reader if it is mid-`send`. The reader also exits on its own
    /// when the socket closes (EOF); the writer additionally exits on a write error. Returns once both
    /// are reaped — no leaked thread.
    pub fn join(self) {
        let HeadConnection { ingress, egress, shutdown_sock, reader, writer } = self;
        // F1: shut the socket down FIRST so a live/idle reader (blocked in read_exact) hits EOF and a
        // blocked writer errors — cleanup completes regardless of who initiated teardown (head- OR
        // proxy-initiated), the Sprint-11 stalled-peer-must-not-strand-cleanup precedent.
        let _ = shutdown_sock.shutdown(std::net::Shutdown::Both);
        drop(egress); // close the writer's channel
        drop(ingress); // unblock the reader if it is parked on a full channel
        let _ = reader.join();
        let _ = writer.join();
    }
}

/// The reader thread: decode frames from the socket → the ingress channel, until EOF / error / the loop
/// hangs up.
fn read_loop(mut sock: UnixStream, tx: &SyncSender<HeadFrame>) {
    loop {
        match decode_head_frame(&mut sock) {
            Ok(frame) => {
                if tx.send(frame).is_err() {
                    return; // the loop (Receiver) is gone
                }
            }
            Err(_) => return, // EOF / malformed / read error — the connection is done
        }
    }
}

/// The writer thread: drain box→head frames from the egress channel → encode → write to the socket,
/// until the channel closes or a write fails.
fn write_loop(mut sock: UnixStream, rx: &Receiver<HeadFrame>) {
    while let Ok(frame) = rx.recv() {
        match encode_head_frame(&frame) {
            // An oversized frame is dropped (F4) rather than written truncated — no stream desync.
            None => continue,
            Some(bytes) => {
                if sock.write_all(&bytes).is_err() {
                    return; // the socket closed / errored
                }
            }
        }
    }
}

/// The box-facing run-loop adapter **with the head connection** (Sprint 12 Slice 2) — wires
/// `terminal_relay` BOTH directions through the head connection. Each tick: (1) **drain** head→box input
/// (NON-BLOCKING, F3) and route each frame via [`Dispatcher::route_head_input`] (fail-safe: input for an
/// unknown/stale display session is dropped, never misrouted); (2) **route** the box event — a box→head
/// `display`/`pane-state` is sent to the head via [`HeadConnection::send_to_head`] (NON-BLOCKING,
/// drop-on-full — a slow head cannot stall the loop); (3) **pump** the reply streams (the routed input
/// reaches the box, stamped with its display-session correlation, plus the net/file replies).
///
/// The `Dispatcher` and the [`HeadConnection`] are owned here; the only thread boundary is the head
/// connection's two channels. (Composing this with the upstream ingress into one production loop is a
/// later integration — each seam is proven in isolation first.)
pub fn run_adapter_with_head<O>(
    mut dispatcher: Dispatcher,
    register: &'static [Route],
    vetted: &'static [ToolIdentity],
    head: HeadConnection,
    mut on_verdict: O,
) -> impl FnMut(&mut dyn Reply, Event) -> ControlFlow<()>
where
    O: FnMut(SessionId, &Routed) -> ControlFlow<()>,
{
    move |reply, event| {
        // 1. Drain head→box input, NON-BLOCKING — route each (fail-safe on unknown/stale display S).
        while let Some(HeadFrame { session, payload }) = head.try_recv_from_head() {
            let _ = dispatcher.route_head_input(session, &payload);
        }
        // 2. Handle the box event.
        let cf = match event {
            Event::Delivered(session, payload) => {
                let routed = dispatcher.route(session, &payload, register, vetted);
                // A box→head display/pane-state → send to the head (non-blocking, drop-on-full).
                if matches!(routed, Routed::Relay(RelayKind::Terminal))
                    && let Some(verb) = peek_verb(&payload)
                    && direction_of(verb) == Some(Direction::BoxToHead)
                {
                    let _ = head.send_to_head(HeadFrame { session, payload: payload.to_vec() });
                }
                on_verdict(session, &routed)
            }
            Event::Idle => ControlFlow::Continue(()),
        };
        // 3. Pump reply streams (routed input → box, net/file replies).
        dispatcher.pump_replies(reply);
        cf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(session: u64, payload: &[u8]) -> HeadFrame {
        HeadFrame { session: SessionId(session), payload: payload.to_vec() }
    }

    /// Encode a (small, in-bounds) test frame — unwraps the F4 bound check.
    fn enc(f: &HeadFrame) -> Vec<u8> {
        encode_head_frame(f).expect("test frames are in-bounds")
    }

    /// Non-blocking spin-drain of one ingress frame with a bound (ingress is private / non-blocking now).
    fn recv_spin(conn: &HeadConnection) -> HeadFrame {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(f) = conn.try_recv_from_head() {
                return f;
            }
            assert!(std::time::Instant::now() < deadline, "no ingress frame within 10s");
            std::thread::yield_now();
        }
    }

    /// Codec round-trip: encode then decode yields the original frame.
    #[test]
    fn codec_round_trips() {
        let f = frame(0x1234_5678_9abc_def0, b"display-payload-bytes");
        let bytes = enc(&f);
        let mut cursor = std::io::Cursor::new(bytes);
        assert_eq!(decode_head_frame(&mut cursor).unwrap(), f);
    }

    /// Length-framing integrity: several frames written back-to-back into ONE buffer are split back into
    /// exactly those frames — the delimiter is the length prefix, no boundary confusion (the whole point
    /// of a length-delimited stream vs message boundaries).
    #[test]
    fn back_to_back_frames_split_correctly() {
        let fs = [frame(1, b"a"), frame(2, b""), frame(3, b"three-bytes-xyz"), frame(4, &[0u8; 5000])];
        let mut buf = Vec::new();
        for f in &fs {
            buf.extend_from_slice(&enc(f));
        }
        let mut cursor = std::io::Cursor::new(buf);
        for f in &fs {
            assert_eq!(&decode_head_frame(&mut cursor).unwrap(), f);
        }
        assert!(decode_head_frame(&mut cursor).is_err(), "the stream is exhausted");
    }

    /// Bounded: a garbage/oversized length is refused BEFORE any allocation (reject-before-alloc), and a
    /// short length (< a SessionId) is a clean error, never a panic.
    #[test]
    fn frame_length_is_bounded_and_short_is_rejected() {
        // len = HEAD_FRAME_MAX + 1 → refused before allocating.
        let mut over = ((HEAD_FRAME_MAX + 1) as u32).to_le_bytes().to_vec();
        over.extend_from_slice(&[0u8; 8]); // a little body; we should never read it
        assert!(decode_head_frame(&mut std::io::Cursor::new(over)).is_err());
        // len = 4 (< 8) → error.
        let short = [4u32.to_le_bytes().to_vec(), vec![0u8; 4]].concat();
        assert!(decode_head_frame(&mut std::io::Cursor::new(short)).is_err());
    }

    /// The bidirectional round-trip over a real `UnixStream` socketpair (the fake head): a frame the fake
    /// head writes arrives on `ingress` decoded; a frame the loop puts on `egress` arrives at the fake
    /// head encoded. Both directions, through the real reader/writer threads.
    #[test]
    fn bidirectional_round_trip_over_socketpair() {
        let (worker_sock, mut fake_head) = UnixStream::pair().unwrap();
        let conn = HeadConnection::spawn(worker_sock).unwrap();

        // head → box: the fake head writes a framed message; it arrives decoded on ingress.
        fake_head.write_all(&enc(&frame(7, b"keystroke"))).unwrap();
        assert_eq!(recv_spin(&conn), frame(7, b"keystroke"));

        // box → head: the loop enqueues a frame (non-blocking); it arrives at the fake head.
        assert_eq!(conn.send_to_head(frame(9, b"display-update")), EgressOutcome::Sent);
        let got = decode_head_frame(&mut fake_head).unwrap();
        assert_eq!(got, frame(9, b"display-update"));

        // Close the fake head → the reader hits EOF and exits; dropping egress ends the writer. Joined.
        drop(fake_head);
        conn.join(); // returns → both threads reaped, no leak
    }

    /// A slow loop backpressures the reader via the bounded ingress channel (never an unbounded buffer):
    /// the fake head sends more than the channel holds; the reader blocks on `send` once it is full, so
    /// no more than the cap is buffered until the loop drains.
    #[test]
    fn full_ingress_channel_backpressures_the_reader() {
        let (worker_sock, mut fake_head) = UnixStream::pair().unwrap();
        let conn = HeadConnection::spawn(worker_sock).unwrap();

        // Send well past the channel cap without draining.
        let total = HEAD_CHANNEL_CAP + 50;
        for i in 0..total {
            fake_head.write_all(&enc(&frame(i as u64, b"x"))).unwrap();
        }
        // Now drain everything, in order — the reader backpressured (blocked on a full channel) rather
        // than buffering unbounded, and every frame is delivered once the loop catches up.
        for i in 0..total {
            let f = recv_spin(&conn);
            assert_eq!(f.session.0, i as u64, "frames delivered in order, none lost");
        }
        drop(fake_head);
        conn.join();
    }

    /// LIVENESS: a slow/stalled head must NOT stall the box-facing loop. With the head not reading
    /// (egress backs up: the writer blocks on a full socket, the egress channel fills), the loop's
    /// `send_to_head` DROPS-ON-FULL and NEVER blocks — so the loop keeps running (proven: 10 000 egress
    /// attempts all return without blocking, and ≥1 is `Dropped`), and the ingress direction keeps
    /// flowing (the head's input still arrives). This is the reviewer's asymmetric-mechanism gate:
    /// egress is drop-on-full (display is droppable), ingress backpressures only the head read.
    #[test]
    fn a_stalled_head_never_stalls_the_loop() {
        let (worker_sock, mut fake_head) = UnixStream::pair().unwrap();
        let conn = HeadConnection::spawn(worker_sock).unwrap();

        // The fake head never reads. Enqueue many large box→head frames — every `send_to_head` returns
        // WITHOUT blocking; once the channel + socket fill, frames are DROPPED (not blocked on).
        let mut dropped = 0;
        for i in 0..10_000u64 {
            if conn.send_to_head(frame(i, &[0u8; 8192])) == EgressOutcome::Dropped {
                dropped += 1;
            }
        }
        assert!(dropped > 0, "a stalled head DROPS egress frames — the loop never blocks on it");

        // Ingress still flows despite the fully-stalled egress (independent reader thread).
        fake_head.write_all(&enc(&frame(42, b"input-still-flows"))).unwrap();
        assert_eq!(
            recv_spin(&conn),
            frame(42, b"input-still-flows"),
            "ingress flows while egress is stalled — one slow head cannot wedge the loop or the other direction",
        );
        drop(fake_head); // the writer's blocked write now errors → it exits
        conn.join();
    }

    /// PARTIAL-READ reassembly: a stream that returns ONE byte per `read` reassembles the frame
    /// identically to one delivered whole (a frame can split across `recv` calls; `read_exact`
    /// accumulates the whole frame before decoding, into a buffer bounded by the length cap).
    #[test]
    fn frame_reassembles_across_partial_reads() {
        struct OneByteAtATime<'a> {
            data: &'a [u8],
            pos: usize,
        }
        impl Read for OneByteAtATime<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.pos >= self.data.len() || buf.is_empty() {
                    return Ok(0);
                }
                buf[0] = self.data[self.pos];
                self.pos += 1;
                Ok(1)
            }
        }
        let f = frame(0xABCD, b"a-frame-split-byte-by-byte");
        let bytes = enc(&f);
        let mut r = OneByteAtATime { data: &bytes, pos: 0 };
        assert_eq!(decode_head_frame(&mut r).unwrap(), f, "one-byte reads reassemble identically");
    }

    /// The fuzz-shaped corpus (malformed length, truncated frame, garbage after a valid frame, oversized
    /// claim, arbitrary bytes) — the codec is TOTAL: every input is `Ok` or `Err`, never a panic or an
    /// over-read. This is both the length-framing correctness gate and the head-side-injectable
    /// memory-safety bar (one surface, one proof).
    #[test]
    fn malformed_frames_never_panic() {
        // Truncated: claims len=100 but only 10 bytes follow → Err (UnexpectedEof), no panic.
        let mut trunc = 100u32.to_le_bytes().to_vec();
        trunc.extend_from_slice(&[0u8; 10]);
        assert!(decode_head_frame(&mut std::io::Cursor::new(trunc)).is_err());

        // A valid frame followed by trailing garbage: the valid one decodes; the trailing bytes error.
        let mut stream = enc(&frame(1, b"ok"));
        stream.extend_from_slice(&[0xFF, 0xFF]); // a partial length → Err on the next decode
        let mut c = std::io::Cursor::new(stream);
        assert_eq!(decode_head_frame(&mut c).unwrap(), frame(1, b"ok"));
        assert!(decode_head_frame(&mut c).is_err(), "trailing garbage errors, never panics");

        // Oversized claim: len = HEAD_FRAME_MAX + 1 → refused before allocating.
        assert!(
            decode_head_frame(&mut std::io::Cursor::new(((HEAD_FRAME_MAX + 1) as u32).to_le_bytes().to_vec())).is_err()
        );

        // Arbitrary short/random byte streams → always Ok or Err, never a panic.
        for seed in 0..500u32 {
            let n = (seed % 40) as usize;
            let bytes: Vec<u8> = (0..n).map(|i| (i.wrapping_mul(31).wrapping_add(seed as usize)) as u8).collect();
            let _ = decode_head_frame(&mut std::io::Cursor::new(bytes)); // must not panic
        }
    }

    /// F1: a PROXY-INITIATED teardown of an IDLE head (connected but silent — the reader is blocked in
    /// read_exact) must NOT hang. `join()` shuts the socket down first, so the blocked reader hits EOF
    /// and the writer errors, regardless of who initiated teardown.
    #[test]
    fn proxy_initiated_teardown_of_idle_head_does_not_hang() {
        let (worker_sock, _fake_head) = UnixStream::pair().unwrap();
        let conn = HeadConnection::spawn(worker_sock).unwrap();
        // The head is idle (sends nothing) and stays connected; the reader is parked in read_exact.
        conn.join(); // must return — shutdown(Both) unblocks the reader/writer (would hang without F1)
    }

    /// F4: an oversized egress frame is DROPPED by encode (returns `None`), never written with a
    /// truncated length that would desync the stream; an in-bounds frame still encodes.
    #[test]
    fn oversized_egress_frame_is_dropped_by_encode() {
        let big = HeadFrame { session: SessionId(1), payload: vec![0u8; HEAD_FRAME_MAX] }; // 8+MAX > MAX
        assert!(encode_head_frame(&big).is_none(), "oversized egress frame dropped, not truncated");
        let ok = HeadFrame { session: SessionId(1), payload: vec![0u8; HEAD_FRAME_MAX - 8] }; // == MAX
        assert!(encode_head_frame(&ok).is_some());
    }

    // ── The Slice-2 run-loop adapter, both directions end-to-end (over a fake head) ──────────────

    use crate::register::anthropic::VETTED_TOOLS;
    use crate::register::net::ROUTES;

    fn raw_frame(verb: u32, body: &[u8]) -> Vec<u8> {
        let mut v = verb.to_le_bytes().to_vec();
        v.extend_from_slice(body);
        v
    }

    /// A one-frame Reply recorder (the box-facing side) — the proxy→box terminal input lands here.
    struct BoxRec {
        alloc: transport::IdAllocator,
        reply_sid: Option<SessionId>,
        emitted: Vec<Vec<u8>>,
    }
    impl BoxRec {
        fn new() -> Self {
            BoxRec {
                alloc: transport::IdAllocator::new(transport::Originator::High),
                reply_sid: None,
                emitted: Vec::new(),
            }
        }
    }
    impl Reply for BoxRec {
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

    /// End to end through `run_adapter_with_head`: a box→head `display` on session S is relayed to the
    /// head (framed), which marks S active; the head's input for S is drained (non-blocking), routed to
    /// the box **stamped with S** (the pty-routing correlation), and reaches the box — while an input for
    /// an UNKNOWN session is dropped, never misrouted.
    #[test]
    fn adapter_relays_display_to_head_and_routes_head_input_to_the_box() {
        let (worker_sock, mut fake_head) = UnixStream::pair().unwrap();
        let conn = HeadConnection::spawn(worker_sock).unwrap();
        let mut adapter =
            run_adapter_with_head(Dispatcher::new(), ROUTES, VETTED_TOOLS, conn, |_s, _r| ControlFlow::Continue(()));
        let s = SessionId(24);
        let mut box_rec = BoxRec::new();

        // Box emits display on S → the adapter relays it to the head (and marks S an active display).
        let display = raw_frame(proto::verb::DISPLAY, b"screen-bytes");
        let _ = adapter(&mut box_rec, Event::Delivered(s, display.clone()));
        let to_head = recv_head_spin(&mut fake_head);
        assert_eq!(to_head, HeadFrame { session: s, payload: display }, "display relayed to the head, framed");

        // The head sends input for S; drive Idle ticks until it is drained + routed to the box.
        let input = raw_frame(proto::verb::KEY, b"keystroke");
        fake_head.write_all(&enc(&HeadFrame { session: s, payload: input.clone() })).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while box_rec.emitted.is_empty() {
            assert!(std::time::Instant::now() < deadline, "input never reached the box");
            let _ = adapter(&mut box_rec, Event::Idle);
            std::thread::yield_now();
        }
        // The box got the input stamped with S (the display correlation), input riding verbatim after.
        let correlation = u64::from_be_bytes(box_rec.emitted[0][0..8].try_into().unwrap());
        assert_eq!(correlation, s.0, "input stamped with the display session S as the pty-routing key");
        assert_eq!(&box_rec.emitted[0][8..], input.as_slice());

        // Input for an UNKNOWN display session is dropped, never misrouted.
        let before = box_rec.emitted.len();
        fake_head.write_all(&enc(&HeadFrame { session: SessionId(999), payload: raw_frame(proto::verb::KEY, b"x") })).unwrap();
        for _ in 0..200 {
            let _ = adapter(&mut box_rec, Event::Idle);
            std::thread::yield_now();
        }
        assert_eq!(box_rec.emitted.len(), before, "input for an unknown display session reaches the box nowhere");
    }

    fn recv_head_spin(sock: &mut UnixStream) -> HeadFrame {
        decode_head_frame(sock).expect("a framed head frame")
    }
}
