//! Sprint 6 S1 — dispatch certification (MB1-MB6). MB2 correctness (each verb routes to the right
//! disposition; the tier-1 seam invokes the real core with the real register), MB3 tier-fidelity +
//! the session-binding invariant (routing is by verb/envelope, never content), MB5 robustness
//! (random payloads never panic, always a Routed). MB1 = cargo test compiles-then-runs; MB4
//! (bounded resources): the dispatcher is **stateful** — it holds a per-session binding map that
//! grows O(sessions-per-epoch), named and tested as a bounded-per-epoch residual with a hook-ready
//! eviction path (`close_session`), mirroring the transport's own un-reclaimed sessions (R5); the
//! reactor loop's own liveness (drains and terminates over a fake wire) is verified separately.
//! MB6 = pure safe Rust (grep-verified, no unsafe in dispatch).

use proto::{
    Message, NetAddress, NetPayload, Scheme, encode_message, scan_safe,
};
use proto::{FileRead, FileWrite};
use proxy::anthropic::Denial;
use proxy::dispatch::{Dispatcher, RelayKind, Routed, Service, service_of_verb};
use proxy::register::anthropic::VETTED_TOOLS;
use proxy::register::net::ROUTES;
use transport::SessionId;

fn net_address(joined: &str) -> Vec<u8> {
    let addr = NetAddress { scheme: Scheme::Https, address: scan_safe(joined.replace('/', "\u{1f}").as_bytes()).unwrap() };
    encode_message(&Message::NetAddress(addr))
}
fn file_write(n_segments: usize) -> Vec<u8> {
    let joined: Vec<u8> = {
        let mut v = Vec::new();
        for i in 0..n_segments { if i > 0 { v.push(0x1f); } v.push(b'a'); }
        v
    };
    encode_message(&Message::FileWrite(FileWrite(scan_safe(&joined).unwrap())))
}
fn net_payload(content: &[u8], more: bool) -> Vec<u8> {
    encode_message(&Message::NetPayload(NetPayload { more, content: content.to_vec() }))
}

/// MB3 (pure routing, exhaustive over verb blocks): service_of_verb maps by block, no content.
#[test]
fn service_of_verb_maps_by_block() {
    assert_eq!(service_of_verb(0x00), Some(Service::Control));
    assert_eq!(service_of_verb(0x11), Some(Service::Terminal));
    assert_eq!(service_of_verb(0x20), Some(Service::File));
    assert_eq!(service_of_verb(0x30), Some(Service::Net));
    // Reserved blocks (0x4_..0xF_) have no service.
    for hi in 0x4u32..=0xf {
        assert_eq!(service_of_verb(hi << 4), None, "block {hi:#x} has no service");
    }
}

/// MB2 tier-1 seam: a REGISTERED net-address admits carrying the matched route; an unregistered one
/// denies. The real register (ROUTES), never a mock; the real tier-1 core (route::match_route).
#[test]
fn net_address_gates_through_the_real_core_and_register() {
    let mut d = Dispatcher::new();
    let admit = d.route(SessionId(1), &net_address("api.anthropic.com/v1/messages"), ROUTES, VETTED_TOOLS);
    assert!(matches!(admit, Routed::NetRouteAdmit(r) if r.as_str() == "https://api.anthropic.com/v1/messages"));

    let mut d2 = Dispatcher::new();
    let deny = d2.route(SessionId(2), &net_address("evil.example.com/x"), ROUTES, VETTED_TOOLS);
    assert_eq!(deny, Routed::NetDeny);
}

/// MB2 tier-1 seam: file-write runs the PRESCANNED leaf check (RQ2 single-scan path). A valid
/// destination admits; one exceeding MAX_ELEMENTS denies — proving dispatch calls the core.
#[test]
fn file_write_gates_through_prescanned_leaf_check() {
    let mut d = Dispatcher::new();
    assert_eq!(d.route(SessionId(3), &file_write(3), ROUTES, VETTED_TOOLS), Routed::FileWriteAdmit);

    let mut d2 = Dispatcher::new();
    // MAX_ELEMENTS is 64; 65 segments must deny via leaf_validate_prescanned.
    assert_eq!(d2.route(SessionId(4), &file_write(65), ROUTES, VETTED_TOOLS), Routed::FileWriteDeny);
}

/// MB2 routing: relay + handler channels route to the right destination.
#[test]
fn relay_and_handler_channels_route_correctly() {
    let mut d = Dispatcher::new();
    let fr = encode_message(&Message::FileRead(FileRead { source: proto::scan_unsafe(b"real\x1fname.txt").unwrap() }));
    assert_eq!(d.route(SessionId(5), &fr, ROUTES, VETTED_TOOLS), Routed::Relay(RelayKind::FileRead));

    // A net-payload single final chunk (more=false) completes the body and reaches the tier-1
    // recognizer — but only once a Route is established for the session (Sprint 8): a prior admitted
    // net-address stores it. `body-bytes` is not valid JSON, so the seam then denies. The full seam
    // fidelity/transparency corpus lives in tests/net_payload_seam.rs.
    let mut d2 = Dispatcher::new();
    let _ = d2.route(SessionId(6), &net_address("api.anthropic.com/v1/messages"), ROUTES, VETTED_TOOLS);
    let np = encode_message(&Message::NetPayload(NetPayload { more: false, content: b"body-bytes".to_vec() }));
    assert!(matches!(
        d2.route(SessionId(6), &np, ROUTES, VETTED_TOOLS),
        Routed::NetPayloadDeny(_)
    ));
    assert_eq!(d2.binding(SessionId(6)), Some(Service::Net), "net-payload still binds the session to Net");
}

/// MB3 — the session→service binding invariant (box-proxy §1.3): a session binds to a service on
/// its first frame's verb; a later frame of a DIFFERENT service is unsupported (never re-binds).
#[test]
fn session_binds_once_and_rejects_a_mismatched_service() {
    let mut d = Dispatcher::new();
    let s = SessionId(7);
    // First frame: a net-address binds the session to Net.
    let _ = d.route(s, &net_address("api.anthropic.com/v1/messages"), ROUTES, VETTED_TOOLS);
    assert_eq!(d.binding(s), Some(Service::Net));
    // A file-write on the SAME session is a service mismatch → Unsupported, binding unchanged.
    let mismatched = d.route(s, &file_write(3), ROUTES, VETTED_TOOLS);
    assert!(matches!(mismatched, Routed::Unsupported { .. }));
    assert_eq!(d.binding(s), Some(Service::Net), "the binding is never overwritten");
}

/// MB3 — binding is by VERB, not content: two DIFFERENT net-addresses on one session both route to
/// Net (the routing decision does not depend on what the address content is).
#[test]
fn binding_is_by_verb_not_content() {
    let mut d = Dispatcher::new();
    let s = SessionId(8);
    let _ = d.route(s, &net_address("api.anthropic.com/v1/messages"), ROUTES, VETTED_TOOLS); // admit
    let second = d.route(s, &net_address("evil.example.com/x"), ROUTES, VETTED_TOOLS);       // deny, but STILL Net-routed
    assert!(matches!(second, Routed::NetDeny), "content differs, but it still routed to the net gate");
    assert_eq!(d.binding(s), Some(Service::Net));
}

/// MB5 — robustness: random/malformed payloads never panic; every input yields a Routed (a decode
/// reject or an unsupported verb), never a crash. Deterministic xorshift, reproducible.
#[test]
fn fuzz_malformed_payloads_never_panic() {
    struct Rng(u64);
    impl Rng { fn next(&mut self) -> u64 { let mut x=self.0; x^=x<<13; x^=x>>7; x^=x<<17; self.0=x; x } }
    let mut rng = Rng(0xC0FF_EE12_3456_7890);
    let mut d = Dispatcher::new();
    for i in 0..30_000u64 {
        let len = (rng.next() % 128) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| (rng.next() & 0xff) as u8).collect();
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            d.route(SessionId(i), &bytes, ROUTES, VETTED_TOOLS)
        }));
        assert!(out.is_ok(), "dispatch panicked on payload {bytes:?}");
    }
    // The dispatcher stayed bounded and counted its drops.
    assert!(d.dropped() > 0);
}

/// MB4 (bounded resources — the corrected claim): the binding map is STATEFUL and grows
/// O(sessions-per-epoch) — named and tested as a bounded-per-epoch residual with a hook-ready
/// eviction path, mirroring the transport's own R5 (un-reclaimed receiver sessions). Not a
/// correctness bug: SessionId is monotonic-unique within an epoch, so no stale binding can
/// mismatch; and the growth resets per epoch. `close_session` is the hook, ready to wire to a
/// transport session-close signal once it exists.
#[test]
fn bindings_grow_per_epoch_and_evict_via_the_close_hook() {
    let mut d = Dispatcher::new();
    // N distinct sessions → N bindings (the O(sessions-per-epoch) growth, characterized).
    for i in 0..100u64 {
        let _ = d.route(SessionId(i), &net_address("api.anthropic.com/v1/messages"), ROUTES, VETTED_TOOLS);
    }
    assert_eq!(d.binding_count(), 100, "one binding per live session (O(sessions-per-epoch))");
    // The eviction hook removes a binding (structurally ready for a transport session-close signal).
    assert_eq!(d.close_session(SessionId(50)), Some(Service::Net));
    assert_eq!(d.binding(SessionId(50)), None);
    assert_eq!(d.binding_count(), 99, "eviction shrinks the map — the hook works");
    // After eviction, that session re-binds like a fresh one (the epoch-reset shape).
    let _ = d.route(SessionId(50), &file_write(2), ROUTES, VETTED_TOOLS);
    assert_eq!(d.binding(SessionId(50)), Some(Service::File));
}

/// MB3 conformance (proxy-head.md §1 / library.md §7): the proxy DEEP-PARSES NO terminal L2. A
/// malformed display body — one that `proto::Display::decode` would reject (a truncated cell) —
/// still routes to the terminal relay OPAQUELY (the body is never decoded), instead of being
/// dropped as a decode error. This is the whole point of the verb-peek refactor: the display/input
/// decoders never run on box bytes at the proxy; the head (or box) parses, per §7.
#[test]
fn relay_channels_forward_opaque_without_deep_parsing() {
    let mut d = Dispatcher::new();
    // A display frame (verb 0x11) with a deliberately-malformed body (3 bytes — not a whole 4-byte
    // cell record). decode_message/Display::decode would return PartialRecord; verb-peek forwards it.
    let mut frame = 0x11u32.to_le_bytes().to_vec();
    frame.extend_from_slice(&[0xDE, 0xAD, 0xBE]); // 3 bytes — a partial cell record
    assert_eq!(
        d.route(SessionId(42), &frame, ROUTES, VETTED_TOOLS),
        Routed::Relay(RelayKind::Terminal),
        "a malformed display body must relay OPAQUELY (not decoded, not dropped) — §1 conformance",
    );

    // And a gated channel STILL deep-parses + rejects a malformed body (the proxy acts on
    // net-address). Body = [scheme u32 = 0][safe stack]; a '/' in the STACK is out of the safe
    // alphabet → NetAddress::decode's scan rejects → Routed::Decode.
    let mut bad_addr = 0x30u32.to_le_bytes().to_vec(); // verb
    bad_addr.extend_from_slice(&0u32.to_le_bytes()); // scheme = 0 (https)
    bad_addr.extend_from_slice(b"has/slash"); // stack with '/' → scan rejects
    assert!(
        matches!(d.route(SessionId(43), &bad_addr, ROUTES, VETTED_TOOLS), Routed::Decode(_)),
        "a gated channel still deep-parses and rejects a malformed body",
    );
}

/// MB4 (the Slice-2 reassembler residual): a per-session reassembly buffer exists ONLY while a body
/// is in flight — created on the first `more` chunk, removed the instant the body completes — and the
/// `close_session` hook evicts an in-flight buffer alongside the binding. This is a *tighter* residual
/// than the binding map (which lives for the whole session), and shares the same eviction hook.
#[test]
fn reassemblers_are_in_flight_only_and_evict_via_close() {
    let mut d = Dispatcher::new();
    let s = SessionId(70);
    // A non-final chunk starts an in-flight reassembly (and binds the session to Net by verb).
    assert_eq!(
        d.route(s, &net_payload(b"partial", true), ROUTES, VETTED_TOOLS),
        Routed::NetPayloadAccumulating,
    );
    assert_eq!(d.reassembling_count(), 1, "an in-flight body holds exactly one reassembler");
    assert_eq!(d.binding(s), Some(Service::Net));
    // A second non-final chunk keeps accumulating on the SAME buffer (still one entry).
    assert_eq!(
        d.route(s, &net_payload(b"-more", true), ROUTES, VETTED_TOOLS),
        Routed::NetPayloadAccumulating,
    );
    assert_eq!(d.reassembling_count(), 1);
    // The close hook evicts the in-flight buffer alongside the binding.
    assert_eq!(d.close_session(s), Some(Service::Net));
    assert_eq!(d.reassembling_count(), 0, "close_session drops the in-flight reassembler");
    assert_eq!(d.binding(s), None);

    // A COMPLETING chunk removes the reassembler immediately — no residual after completion (a valid
    // body admits and leaves nothing in the map; the session binding persists as normal). The session
    // first admits a net-address so the recognizer envelope has a real Route to source (Sprint 8).
    let mut d2 = Dispatcher::new();
    let s2 = SessionId(71);
    let _ = d2.route(s2, &net_address("api.anthropic.com/v1/messages"), ROUTES, VETTED_TOOLS);
    let done = d2.route(
        s2,
        &net_payload(br#"{"model":"claude-x","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#, false),
        ROUTES,
        VETTED_TOOLS,
    );
    assert!(matches!(done, Routed::NetPayloadAdmit(_)), "a complete valid body admits");
    assert_eq!(d2.reassembling_count(), 0, "a completed body leaves no reassembler behind");
    assert_eq!(d2.binding(s2), Some(Service::Net), "the session binding persists after completion");
}

/// Sprint 8 / Task 14 — the no-prior-Route rung: a `net-payload` that completes on a session with NO
/// admitted `net-address` (no stored Route) fails closed with `NetPayloadNoRoute` — recognition never
/// runs (there is no envelope to vet the request *as*), and the body is not forwarded. The verb-block
/// service binding still happens (net-payload maps to Net); only the recognition step now needs a Route.
#[test]
fn net_payload_with_no_admitted_route_denies() {
    let mut d = Dispatcher::new();
    let s = SessionId(80);
    // A complete, otherwise-valid Anthropic body — but no prior net-address, so no stored Route.
    let body = br#"{"model":"claude-x","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#;
    assert_eq!(
        d.route(s, &net_payload(body, false), ROUTES, VETTED_TOOLS),
        Routed::NetPayloadNoRoute,
        "a net-payload with no admitted Route must fail closed, not self-bind and recognize with a stub",
    );
    // The verb-block service binding still happened (unchanged) — only recognition required the Route.
    assert_eq!(d.binding(s), Some(Service::Net));
}

/// Sprint 8 / Task 14 — the recognizer envelope is sourced from the session's REAL Route, not a shared
/// stub. Two different admitted routes recognize under two different envelopes: the SAME request body
/// admits under `api.anthropic.com/v1/messages` but DENIES under `pypi.org/simple` (Stage-1
/// `BadEnvelope`, domain/path mismatch). A shared hardcoded envelope (the old stub) would wrongly admit
/// BOTH — so this discriminates the real route-binding from the stub-in-disguise.
#[test]
fn net_payload_envelope_is_route_sourced_not_a_shared_stub() {
    let body = br#"{"model":"claude-x","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#;

    // Session A: the Anthropic route → envelope POST api.anthropic.com /v1/messages → admits.
    let mut d = Dispatcher::new();
    let sa = SessionId(90);
    let _ = d.route(sa, &net_address("api.anthropic.com/v1/messages"), ROUTES, VETTED_TOOLS);
    assert!(
        matches!(
            d.route(sa, &net_payload(body, false), ROUTES, VETTED_TOOLS),
            Routed::NetPayloadAdmit(_)
        ),
        "under the api.anthropic.com route, a valid Anthropic body admits",
    );

    // Session B: the pypi route → envelope POST pypi.org /simple → the Anthropic recognizer denies at
    // Stage 1 (domain/path are not api.anthropic.com//v1/messages). The SAME body as session A.
    let sb = SessionId(91);
    let _ = d.route(sb, &net_address("pypi.org/simple"), ROUTES, VETTED_TOOLS);
    assert_eq!(
        d.route(sb, &net_payload(body, false), ROUTES, VETTED_TOOLS),
        Routed::NetPayloadDeny(Denial::BadEnvelope),
        "under the pypi route, the same body denies at Stage 1 — the envelope is route-sourced, not shared",
    );
}

// ─── MB4 liveness: the reactor loop drives dispatch over a FAKE wire and terminates ──────────
mod reactor_wiring {
    use super::*;
    use std::time::Duration;
    use reactor::{Host, Reactor, RxSource, WakeReason};
    use transport::{Instant, SessionTable, TransmitSink};

    /// A fake RX ring: yields a fixed list of pre-encoded L1 frames, then empties.
    struct FakeRx { frames: Vec<Vec<u8>>, pos: usize }
    impl RxSource for FakeRx {
        fn has_frame(&self) -> bool { self.pos < self.frames.len() }
        fn next_frame(&mut self) -> Option<&[u8]> {
            let i = self.pos;
            self.pos += 1;
            self.frames.get(i).map(|v| v.as_slice())
        }
    }
    struct FakeHost;
    impl Host for FakeHost {
        fn now(&self) -> Instant { Instant(0) }
        fn block(&mut self, _t: Option<Duration>) -> WakeReason { WakeReason::Timer } // unreached in pump_all
    }
    struct FakeSink;
    impl TransmitSink for FakeSink { fn emit(&mut self, _frame: &[u8]) {} }

    /// The reactor busy-drains a fake wire of encoded L1 frames, handing each delivery to the
    /// Dispatcher; the loop makes progress and TERMINATES (returns), and the dispatcher processed
    /// every frame (bound the sessions). This is the concrete reactor↔dispatch wiring + MB4 liveness.
    #[test]
    fn reactor_drives_dispatch_and_terminates() {
        // Two L1 frames on two sessions: a net-address and a file-write.
        let f1 = {
            let mut b = Vec::new();
            transport::encode_frame(SessionId(100), &net_address("api.anthropic.com/v1/messages"), &mut b);
            b
        };
        let f2 = {
            let mut b = Vec::new();
            transport::encode_frame(SessionId(200), &file_write(2), &mut b);
            b
        };
        let rx = FakeRx { frames: vec![f1, f2], pos: 0 };
        let mut r = Reactor::new(SessionTable::new(), FakeHost, rx, FakeSink);

        let mut d = Dispatcher::new();
        let mut seen: Vec<(u64, Routed)> = Vec::new();
        // The wiring: pump_all hands each (session, L2 payload) to the dispatcher. It TERMINATES
        // (the fake ring empties → step() returns false → the loop exits).
        r.pump_all(|session, payload| {
            let routed = d.route(session, &payload, ROUTES, VETTED_TOOLS);
            seen.push((session.0, routed));
        });

        // Both frames were dispatched to the right gate, and their sessions bound by verb.
        assert!(seen.iter().any(|(s, r)| *s == 100 && matches!(r, Routed::NetRouteAdmit(_))));
        assert!(seen.iter().any(|(s, r)| *s == 200 && matches!(r, Routed::FileWriteAdmit)));
        assert_eq!(d.binding(SessionId(100)), Some(Service::Net));
        assert_eq!(d.binding(SessionId(200)), Some(Service::File));
    }
}
