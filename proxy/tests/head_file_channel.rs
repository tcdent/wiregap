//! Sprint 15 Slice 2 — the head-browses-box file channel: origination-keyed routing + the confused-deputy
//! floor, proven with distinguishing nonces (a leak is un-fakeably detectable) across the two disjoint
//! directions, and the head-origin browse proven structurally UNABLE to reach the proxy's own confined
//! trusted-host reader.

#![cfg(unix)]

use std::io::Write;
use std::ops::ControlFlow;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use directpath::{Event, SessionId};
use proto::{Message, encode_message, scan_unsafe, verb};
use proxy::dispatch::Dispatcher;
use proxy::head::{HeadConnection, HeadFrame, decode_head_frame, encode_head_frame, run_adapter_with_head};
use proxy::nic::Reply;
use proxy::register::anthropic::VETTED_TOOLS;
use proxy::register::net::ROUTES;

// ── scaffold: a unique temp dir standing in for the proxy's confined TRUSTED-HOST readable root ──────
static SCRATCH_N: AtomicUsize = AtomicUsize::new(0);
struct Scratch {
    dir: std::path::PathBuf,
}
impl Scratch {
    fn new() -> Self {
        let n = SCRATCH_N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("proxy-s15-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }
}

fn raw_frame(verb: u32, body: &[u8]) -> Vec<u8> {
    let mut v = verb.to_le_bytes().to_vec();
    v.extend_from_slice(body);
    v
}
/// A box→proxy `list` REQUEST (host-list): the box browsing the trusted host via the proxy's own reader.
fn list_frame(name: &str) -> Vec<u8> {
    encode_message(&Message::List(proto::List(scan_unsafe(name.as_bytes()).unwrap())))
}
fn enc(f: &HeadFrame) -> Vec<u8> {
    encode_head_frame(f).expect("in-bounds")
}

/// A box-facing Reply recorder — every proxy→box origination/send lands here (open = first frame, send =
/// the rest). This is the box side; nothing here should ever carry a head-initiated browse's answer.
struct BoxReply {
    alloc: transport::IdAllocator,
    emitted: Vec<Vec<u8>>,
}
impl BoxReply {
    fn new() -> Self {
        BoxReply { alloc: transport::IdAllocator::new(transport::Originator::High), emitted: Vec::new() }
    }
    fn any_contains(&self, needle: &[u8]) -> bool {
        self.emitted.iter().any(|f| f.windows(needle.len()).any(|w| w == needle))
    }
}
impl Reply for BoxReply {
    fn open(&mut self, payload: &[u8]) -> SessionId {
        self.emitted.push(payload.to_vec());
        self.alloc.allocate()
    }
    fn send(&mut self, _id: SessionId, payload: &[u8]) -> Result<(), transport::SendError> {
        self.emitted.push(payload.to_vec());
        Ok(())
    }
}

/// Assert the head socket carries NO frame within a short window (nothing was `send_to_head`'d).
fn assert_head_silent(sock: &mut UnixStream, what: &str) {
    sock.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    match decode_head_frame(sock) {
        Err(_) => {} // timeout / would-block / EOF — the head received nothing, as required
        Ok(f) => panic!("{what}: head UNEXPECTEDLY received a frame ({} bytes) — a leak", f.payload.len()),
    }
}

/// **DISJOINTNESS (a):** a BOX-AUTHORED `list-entry` answering a head-initiated browse reaches the HEAD
/// (origination-keyed: it arrived box→proxy at `route()` and is verb-classified `FileToHead`), and the
/// box-facing reply path receives NOTHING — the two paths do not cross. Proven with a nonce.
#[test]
fn box_authored_list_entry_answer_reaches_the_head_never_the_box() {
    let (worker_sock, mut fake_head) = UnixStream::pair().unwrap();
    let conn = HeadConnection::spawn(worker_sock).unwrap();
    let mut adapter =
        run_adapter_with_head(Dispatcher::new(), ROUTES, VETTED_TOOLS, conn, |_s, _r| ControlFlow::Continue(()));
    let mut box_reply = BoxReply::new();

    // The box emits a list-entry (answering a head browse) carrying a distinguishing nonce.
    let nonce = b"NONCE-A-box-answer-to-head";
    let list_entry = raw_frame(verb::LIST_ENTRY, nonce);
    let _ = adapter(&mut box_reply, Event::Delivered(SessionId(5), list_entry.clone()));

    // It reached the HEAD, verbatim (opaque relay), nonce intact.
    fake_head.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let to_head = decode_head_frame(&mut fake_head).expect("the box-answer reaches the head");
    assert_eq!(to_head.payload, list_entry, "box list-entry answer relayed VERBATIM to the head");
    // The box-facing reply path received NOTHING — no crossover into the box's own reply queue.
    assert!(!box_reply.any_contains(nonce), "the box-answer nonce never touched the box-facing reply path");
    assert!(box_reply.emitted.is_empty(), "no proxy→box emission for a box→head answer");
}

/// **DISJOINTNESS (b), the sharper one:** a PROXY-AUTHORED host-listing — the box browsing the TRUSTED
/// HOST via the proxy's own confined reader — reaches the BOX and the HEAD **nowhere**. A leak here would
/// expose the confined host filesystem listing the head must never see. Proven with a canary filename.
#[test]
fn proxy_authored_host_listing_reaches_the_box_never_the_head() {
    let sc = Scratch::new();
    std::fs::create_dir(sc.dir.join("d")).unwrap();
    let canary = "CANARY-B-trusted-host-file.secret";
    std::fs::write(sc.dir.join("d").join(canary), b"host bytes").unwrap();

    let (worker_sock, mut fake_head) = UnixStream::pair().unwrap();
    let conn = HeadConnection::spawn(worker_sock).unwrap();
    let mut adapter = run_adapter_with_head(
        Dispatcher::new().with_readable_root(sc.dir.clone()),
        ROUTES,
        VETTED_TOOLS,
        conn,
        |_s, _r| ControlFlow::Continue(()),
    );
    let mut box_reply = BoxReply::new();

    // The BOX browses the trusted host: a box→proxy `list` of "d" → the proxy's confined reader answers.
    let _ = adapter(&mut box_reply, Event::Delivered(SessionId(6), list_frame("d")));
    // Drain the proxy-authored list-entry stream to the box (pump runs each tick).
    let deadline = Instant::now() + Duration::from_secs(10);
    while !box_reply.any_contains(canary.as_bytes()) {
        assert!(Instant::now() < deadline, "the host-listing never reached the box");
        let _ = adapter(&mut box_reply, Event::Idle);
        std::thread::yield_now();
    }
    // The host-listing (the canary) reached the BOX (correct) and the HEAD received NOTHING (the floor).
    assert!(box_reply.any_contains(canary.as_bytes()), "proxy-authored host-listing reaches the box");
    assert_head_silent(&mut fake_head, "proxy-authored host-listing must NEVER reach the head");
}

/// **CONFUSED-DEPUTY, behavioral floor:** a HEAD-initiated `list` naming a path under the proxy's own
/// TRUSTED-HOST readable root is relayed OPAQUELY to the box (the box lists ITS OWN filesystem) and NEVER
/// reaches the proxy's confined reader — the canary (a real trusted-host file) is listed NOWHERE. If a
/// future edit routed a head-origin browse through `route()`/`route_list`, the canary would leak here and
/// this fails. (The structural half — `relay_head_file` has no handle to the reader — makes this
/// unrepresentable in the relay's own code; this test guards the wiring.)
#[test]
fn head_origin_list_of_the_trusted_host_never_reaches_the_confined_reader() {
    let sc = Scratch::new();
    std::fs::create_dir(sc.dir.join("d")).unwrap();
    let canary = "CANARY-C-must-never-leak-to-head.secret";
    std::fs::write(sc.dir.join("d").join(canary), b"host bytes").unwrap();

    let (worker_sock, mut fake_head) = UnixStream::pair().unwrap();
    let conn = HeadConnection::spawn(worker_sock).unwrap();
    let mut adapter = run_adapter_with_head(
        Dispatcher::new().with_readable_root(sc.dir.clone()),
        ROUTES,
        VETTED_TOOLS,
        conn,
        |_s, _r| ControlFlow::Continue(()),
    );
    let mut box_reply = BoxReply::new();

    // The HEAD asks to list "d" — a directory under the proxy's confined trusted-host reader.
    let head_list = raw_frame(verb::LIST, b"d");
    fake_head
        .write_all(&enc(&HeadFrame { session: SessionId(9), payload: head_list.clone() }))
        .unwrap();

    // Drive until the head-list is relayed to the box (reply.open), then keep ticking so ANY (wrongful)
    // confined-reader stream would have drained by now.
    let deadline = Instant::now() + Duration::from_secs(10);
    while box_reply.emitted.is_empty() {
        assert!(Instant::now() < deadline, "head-list never relayed to the box");
        let _ = adapter(&mut box_reply, Event::Idle);
        std::thread::yield_now();
    }
    for _ in 0..500 {
        let _ = adapter(&mut box_reply, Event::Idle);
        std::thread::yield_now();
    }

    // Relayed VERBATIM (the box lists its OWN fs) — the box got the request bytes, not a host listing.
    assert_eq!(box_reply.emitted[0], head_list, "head-list relayed opaquely + verbatim to the box");
    // The trusted-host canary NEVER appears — `route_list` was NOT invoked on a head-origin browse.
    assert!(
        !box_reply.any_contains(canary.as_bytes()),
        "confused-deputy: the trusted-host canary leaked — a head browse reached the proxy's own reader",
    );
    assert_head_silent(&mut fake_head, "no host-listing to the head either");
}
