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
use proxy::dispatch::{Dispatcher, RelayKind, Routed, Service, service_of_verb};
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
    let admit = d.route(SessionId(1), &net_address("api.anthropic.com/v1/messages"), ROUTES);
    assert!(matches!(admit, Routed::NetRouteAdmit(r) if r.as_str() == "https://api.anthropic.com/v1/messages"));

    let mut d2 = Dispatcher::new();
    let deny = d2.route(SessionId(2), &net_address("evil.example.com/x"), ROUTES);
    assert_eq!(deny, Routed::NetDeny);
}

/// MB2 tier-1 seam: file-write runs the PRESCANNED leaf check (RQ2 single-scan path). A valid
/// destination admits; one exceeding MAX_ELEMENTS denies — proving dispatch calls the core.
#[test]
fn file_write_gates_through_prescanned_leaf_check() {
    let mut d = Dispatcher::new();
    assert_eq!(d.route(SessionId(3), &file_write(3), ROUTES), Routed::FileWriteAdmit);

    let mut d2 = Dispatcher::new();
    // MAX_ELEMENTS is 64; 65 segments must deny via leaf_validate_prescanned.
    assert_eq!(d2.route(SessionId(4), &file_write(65), ROUTES), Routed::FileWriteDeny);
}

/// MB2 routing: relay + handler channels route to the right destination.
#[test]
fn relay_and_handler_channels_route_correctly() {
    let mut d = Dispatcher::new();
    let fr = encode_message(&Message::FileRead(FileRead { source: proto::scan_unsafe(b"real\x1fname.txt").unwrap() }));
    assert_eq!(d.route(SessionId(5), &fr, ROUTES), Routed::Relay(RelayKind::FileRead));

    let mut d2 = Dispatcher::new();
    let np = encode_message(&Message::NetPayload(NetPayload(b"body-bytes".to_vec())));
    assert_eq!(d2.route(SessionId(6), &np, ROUTES), Routed::ToServiceHandler(Service::Net));
}

/// MB3 — the session→service binding invariant (box-proxy §1.3): a session binds to a service on
/// its first frame's verb; a later frame of a DIFFERENT service is unsupported (never re-binds).
#[test]
fn session_binds_once_and_rejects_a_mismatched_service() {
    let mut d = Dispatcher::new();
    let s = SessionId(7);
    // First frame: a net-address binds the session to Net.
    let _ = d.route(s, &net_address("api.anthropic.com/v1/messages"), ROUTES);
    assert_eq!(d.binding(s), Some(Service::Net));
    // A file-write on the SAME session is a service mismatch → Unsupported, binding unchanged.
    let mismatched = d.route(s, &file_write(3), ROUTES);
    assert!(matches!(mismatched, Routed::Unsupported { .. }));
    assert_eq!(d.binding(s), Some(Service::Net), "the binding is never overwritten");
}

/// MB3 — binding is by VERB, not content: two DIFFERENT net-addresses on one session both route to
/// Net (the routing decision does not depend on what the address content is).
#[test]
fn binding_is_by_verb_not_content() {
    let mut d = Dispatcher::new();
    let s = SessionId(8);
    let _ = d.route(s, &net_address("api.anthropic.com/v1/messages"), ROUTES); // admit
    let second = d.route(s, &net_address("evil.example.com/x"), ROUTES);       // deny, but STILL Net-routed
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
            d.route(SessionId(i), &bytes, ROUTES)
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
        let _ = d.route(SessionId(i), &net_address("api.anthropic.com/v1/messages"), ROUTES);
    }
    assert_eq!(d.binding_count(), 100, "one binding per live session (O(sessions-per-epoch))");
    // The eviction hook removes a binding (structurally ready for a transport session-close signal).
    assert_eq!(d.close_session(SessionId(50)), Some(Service::Net));
    assert_eq!(d.binding(SessionId(50)), None);
    assert_eq!(d.binding_count(), 99, "eviction shrinks the map — the hook works");
    // After eviction, that session re-binds like a fresh one (the epoch-reset shape).
    let _ = d.route(SessionId(50), &file_write(2), ROUTES);
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
        d.route(SessionId(42), &frame, ROUTES),
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
        matches!(d.route(SessionId(43), &bad_addr, ROUTES), Routed::Decode(_)),
        "a gated channel still deep-parses and rejects a malformed body",
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
            let routed = d.route(session, &payload, ROUTES);
            seen.push((session.0, routed));
        });

        // Both frames were dispatched to the right gate, and their sessions bound by verb.
        assert!(seen.iter().any(|(s, r)| *s == 100 && matches!(r, Routed::NetRouteAdmit(_))));
        assert!(seen.iter().any(|(s, r)| *s == 200 && matches!(r, Routed::FileWriteAdmit)));
        assert_eq!(d.binding(SessionId(100)), Some(Service::Net));
        assert_eq!(d.binding(SessionId(200)), Some(Service::File));
    }
}
