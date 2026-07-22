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
use proxy::open_discipline::pin_root;
use proxy::register::anthropic::VETTED_TOOLS;
use proxy::register::net::ROUTES;
use transport::SessionId;

// A unique temp quarantine root per write test (mirrors fs_read/fs_write's scratch pattern).
static SCRATCH_N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
struct Scratch {
    dir: std::path::PathBuf,
}
impl Scratch {
    fn new() -> Self {
        let n = SCRATCH_N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("proxy-s9d-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::with_quarantine_root(pin_root(self.dir.to_str().unwrap()).unwrap())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn file_write_to(joined: &str) -> Vec<u8> {
    encode_message(&Message::FileWrite(FileWrite(
        scan_safe(joined.replace('/', "\u{1f}").as_bytes()).unwrap(),
    )))
}

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
/// A raw L2 frame `[verb u32 LE][body]` — for verbs dispatch routes opaquely (no body decode).
fn raw_frame(verb: u32, body: &[u8]) -> Vec<u8> {
    let mut f = verb.to_le_bytes().to_vec();
    f.extend_from_slice(body);
    f
}
/// A raw `file-chunk` L2 frame (`[verb 0x21][verbatim bytes]`) — dispatch routes it opaquely to the
/// File handler without decoding the body, so a hand-built frame is exact.
fn file_chunk(bytes: &[u8]) -> Vec<u8> {
    raw_frame(proto::verb::FILE_CHUNK, bytes)
}
/// OR reserved bits (`reserved_upper`, upper three bytes) into a frame's verb `u32` — the un-fakeable
/// input for the JC-1 raw-verb check (the low byte, hence routing, is unchanged).
fn set_reserved(frame: &[u8], reserved_upper: u32) -> Vec<u8> {
    let mut f = frame.to_vec();
    let verb = u32::from_le_bytes(f[0..4].try_into().unwrap()) | reserved_upper;
    f[0..4].copy_from_slice(&verb.to_le_bytes());
    f
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

// ─── Sprint 9 Slice 1 — Task 13 / JC-1: reserved-verb-byte deny ──────────────────────────────

/// JC-1: a message the proxy ACTS ON (one of the four acted-on verbs) whose raw verb u32 carries a
/// nonzero reserved surface (upper three bytes) → `ReservedDeny`, deny-without-binding — proven
/// positively across every acted-on verb × a spread of reserved patterns (the reserved *space*, not
/// one magic value: a low reserved byte, a high reserved byte, and all three reserved bytes).
#[test]
fn jc1_reserved_verb_denies_whole_deny_without_binding() {
    let anthropic = br#"{"model":"m","messages":[]}"#;
    let clean_frames: [(&str, Vec<u8>); 4] = [
        ("net-address", net_address("api.anthropic.com/v1/messages")),
        ("file-write", file_write(3)),
        ("file-chunk", file_chunk(b"bytes")),
        ("net-payload", net_payload(anthropic, true)),
    ];
    for reserved in [0x0000_0100u32, 0x0100_0000, 0xFFFF_FF00] {
        for (name, clean) in &clean_frames {
            let mut d = Dispatcher::new();
            let frame = set_reserved(clean, reserved);
            let raw_verb = u32::from_le_bytes(frame[0..4].try_into().unwrap());
            assert_eq!(
                d.route(SessionId(1), &frame, ROUTES, VETTED_TOOLS),
                Routed::ReservedDeny { verb: raw_verb },
                "{name} + reserved {reserved:#010x} must ReservedDeny (raw verb {raw_verb:#010x})",
            );
            // Deny-WITHOUT-binding, proven positively: NOTHING is created for the session.
            assert_eq!(d.binding_count(), 0, "{name} reserved: no service binding");
            assert_eq!(d.routes_count(), 0, "{name} reserved: no stored route");
            assert_eq!(d.reassembling_count(), 0, "{name} reserved: no reassembler entry");
        }
    }
}

/// The un-fakeable tell: the reserved check reads the RAW verb, not the masked low byte. `0x01000030`
/// has low byte `0x30` (net-address — which ADMITS clean), so a masked-low-byte check would wrongly
/// route+admit it; the raw check must `ReservedDeny`. This test FAILS under a masked implementation.
#[test]
fn jc1_reserved_check_reads_raw_verb_not_masked_low_byte() {
    let clean = net_address("api.anthropic.com/v1/messages"); // low byte 0x30
    // Sanity: the clean frame DOES admit — so a masked check would admit the reserved one too.
    let mut d0 = Dispatcher::new();
    assert!(
        matches!(d0.route(SessionId(9), &clean, ROUTES, VETTED_TOOLS), Routed::NetRouteAdmit(_)),
        "the clean net-address admits — a masked-low-byte check would wrongly admit the reserved variant",
    );
    let frame = set_reserved(&clean, 0x0100_0000); // → verb 0x01000030
    let mut d = Dispatcher::new();
    assert_eq!(
        d.route(SessionId(1), &frame, ROUTES, VETTED_TOOLS),
        Routed::ReservedDeny { verb: 0x0100_0030 },
        "raw check must deny 0x01000030; a masked (& 0xff = 0x30) check would have routed it as net-address",
    );
}

/// JC-1's OTHER half — the relayed-verb EXEMPTION (the checked=0 companion to the acted-on half): a
/// RELAYED verb with reserved bits set still routes to `Relay(..)`, NOT `ReservedDeny`. Proves
/// `acts_on()` correctly excludes relayed verbs and that JC-1 polices nothing on relayed messages
/// (relay is inert; reserved bits pass to the head, whose consumer policy is its own) — the exemption
/// holds through the real dispatch path, across two relay kinds.
#[test]
fn jc1_exempts_relayed_verbs_reserved_bits_still_relay() {
    // display (0x11, Terminal relay) with ALL reserved bytes set → Relay(Terminal), not ReservedDeny.
    let mut d = Dispatcher::new();
    assert_eq!(
        d.route(SessionId(1), &set_reserved(&raw_frame(proto::verb::DISPLAY, b"cells"), 0xFFFF_FF00), ROUTES, VETTED_TOOLS),
        Routed::Relay(RelayKind::Terminal),
        "a relayed display verb with reserved bits must relay (JC-1 exempts relayed messages), not ReservedDeny",
    );
    // request-file (0x26, FileRead relay) with a reserved byte set → Relay(FileRead), not ReservedDeny.
    // (file-read AND list now ACT — Sprint 10 Slice 2 — so `request-file` (head→box, never proxy-acted)
    // is the still-relayed FileRead-kind example.)
    let mut d = Dispatcher::new();
    assert_eq!(
        d.route(SessionId(2), &set_reserved(&raw_frame(proto::verb::REQUEST_FILE, b"src"), 0x0100_0000), ROUTES, VETTED_TOOLS),
        Routed::Relay(RelayKind::FileRead),
        "a relayed request-file verb with reserved bits must relay, not ReservedDeny (proxy never acts on it)",
    );
}

/// The same acted-on verbs with ZERO reserved bytes route by their real logic, unchanged — the
/// reserved check does not disturb the clean path.
#[test]
fn jc1_clean_acted_on_verbs_route_by_real_logic_unchanged() {
    let anthropic = br#"{"model":"m","messages":[]}"#;
    let mut d = Dispatcher::new();
    assert!(matches!(d.route(SessionId(1), &net_address("api.anthropic.com/v1/messages"), ROUTES, VETTED_TOOLS), Routed::NetRouteAdmit(_)));
    let mut d = Dispatcher::new();
    assert_eq!(d.route(SessionId(2), &file_write(3), ROUTES, VETTED_TOOLS), Routed::FileWriteAdmit);
    let mut d = Dispatcher::new();
    assert_eq!(d.route(SessionId(3), &file_chunk(b"x"), ROUTES, VETTED_TOOLS), Routed::ToServiceHandler(Service::File));
    let mut d = Dispatcher::new();
    assert_eq!(d.route(SessionId(4), &net_payload(anthropic, true), ROUTES, VETTED_TOOLS), Routed::NetPayloadAccumulating);
}

/// JC-2 precedence (settled fork): `service_of_verb` on the masked low byte FIRST — an unsupported low
/// byte is `Unsupported`/JC-2 REGARDLESS of reserved bits (never reaches JC-1, as it is never acted on).
#[test]
fn jc2_unsupported_low_byte_precedes_jc1_regardless_of_reserved_bits() {
    // Low byte 0x40 is a reserved block (no service) — Unsupported — even with reserved upper bytes set.
    let mut frame = 0x0100_0040u32.to_le_bytes().to_vec();
    frame.extend_from_slice(b"whatever");
    let mut d = Dispatcher::new();
    assert_eq!(
        d.route(SessionId(1), &frame, ROUTES, VETTED_TOOLS),
        Routed::Unsupported { verb: 0x0100_0040 },
        "an unsupported low byte is JC-2 Unsupported, not JC-1 ReservedDeny — service_of_verb runs first",
    );
    assert_eq!(d.binding_count(), 0, "an unsupported verb never binds");
}

/// JC-2 precedence, mismatch case: a frame whose verb's service does not match the session's binding
/// is `Unsupported` (JC-2) BEFORE the reserved-check even with reserved bits set — a mismatched frame
/// is never *acted on* for this session, so JC-1 does not police it. The standing binding is untouched.
#[test]
fn jc2_service_mismatch_precedes_jc1_even_with_reserved_bits() {
    let mut d = Dispatcher::new();
    let s = SessionId(1);
    let _ = d.route(s, &net_address("api.anthropic.com/v1/messages"), ROUTES, VETTED_TOOLS); // binds Net
    let mismatched = set_reserved(&file_write(3), 0x0100_0000); // file-write (File service) + reserved bits
    let raw = u32::from_le_bytes(mismatched[0..4].try_into().unwrap());
    assert_eq!(
        d.route(s, &mismatched, ROUTES, VETTED_TOOLS),
        Routed::Unsupported { verb: raw },
        "a mismatched frame is Unsupported (JC-2) before the reserved-check — never acted on for this session",
    );
    assert_eq!(d.binding(s), Some(Service::Net), "the binding stands; only the message denies");
}

/// The reserved-SCHEME surface (net-address, scheme != 0) still denies via U2's scheme gate
/// (`vocab::assemble` → `None` → `NetDeny`) — a CLEAN verb, so NOT `ReservedDeny`: the two reserved
/// surfaces are distinct and not double-gated. Confirms Sprint-8 behavior unchanged.
#[test]
fn reserved_scheme_still_denies_via_u2_not_reserveddeny() {
    let mut frame = net_address("api.anthropic.com/v1/messages"); // verb clean; scheme=0 at bytes 4..8
    frame[4..8].copy_from_slice(&5u32.to_le_bytes()); // a reserved scheme value
    let mut d = Dispatcher::new();
    assert_eq!(
        d.route(SessionId(1), &frame, ROUTES, VETTED_TOOLS),
        Routed::NetDeny,
        "a reserved scheme denies via U2 (NetDeny), unchanged — not ReservedDeny, not double-gated",
    );
}

// ─── Sprint 9 Slice 2 — the quarantine-write dispatch integration ────────────────────────────

/// The receive-side upload path end to end: file-write admits + declares the writer; file-chunks
/// append at the monotonic cursor; the bytes land at the confined destination on disk; and
/// **completion = session end** closes the held fd (fd-lifecycle, proven via writes_count()).
#[test]
fn quarantine_write_appends_via_dispatch_and_closes_fd_on_session_end() {
    let sc = Scratch::new();
    let mut d = sc.dispatcher();
    let s = SessionId(1);
    assert_eq!(d.route(s, &file_write_to("up.txt"), ROUTES, VETTED_TOOLS), Routed::FileWriteAdmit);
    assert_eq!(d.writes_count(), 1, "file-write declares the per-session writer");
    assert_eq!(d.route(s, &file_chunk(b"hello "), ROUTES, VETTED_TOOLS), Routed::FileChunkAppended { bytes: 6 });
    assert_eq!(d.route(s, &file_chunk(b"world"), ROUTES, VETTED_TOOLS), Routed::FileChunkAppended { bytes: 5 });
    assert_eq!(std::fs::read(sc.dir.join("up.txt")).unwrap(), b"hello world", "bytes landed verbatim, in order");
    // Completion is the transport session ending (file.md §1): the writer is evicted, the fd closed.
    d.close_session(s);
    assert_eq!(d.writes_count(), 0, "close_session evicts the writer and closes its fd (fd-lifecycle)");
}

/// A file-chunk with NO admitted file-write (no destination declared) is **structural protocol-noise**
/// (JC-3a): `FileChunkNoWriter` — silent, no notification (designer ruling A), nothing created.
#[test]
fn file_chunk_with_no_prior_file_write_is_silent_noise() {
    let sc = Scratch::new();
    let mut d = sc.dispatcher();
    assert_eq!(d.route(SessionId(1), &file_chunk(b"x"), ROUTES, VETTED_TOOLS), Routed::FileChunkNoWriter);
    assert_eq!(d.writes_count(), 0);
    assert_eq!(d.replies_count(), 0, "structural noise is silent — NO JC-3 notification");
}

/// The confined open is the real floor, applied at chunk time — not just the file-write leaf gate: a
/// destination whose name-alphabet is clean (so file-write ADMITS) but whose on-ground resolution
/// escapes via a symlink component is caught by the first file-chunk's confined open (RESOLVE_NO_SYMLINKS)
/// → FileChunkDeny, and the poisoned writer is evicted.
#[test]
fn file_chunk_confined_open_deny_evicts_the_writer() {
    let sc = Scratch::new();
    // Pre-plant a symlink escaping the quarantine root at a clean-named component.
    std::os::unix::fs::symlink("/etc", sc.dir.join("link")).unwrap();
    let mut d = sc.dispatcher();
    let s = SessionId(1);
    assert_eq!(d.route(s, &file_write_to("link/passwd"), ROUTES, VETTED_TOOLS), Routed::FileWriteAdmit, "leaf gate admits a clean-named destination");
    // Confined-open refusal = the write-destination floor firing one message late → FileChunkDeny(Denied)
    // (designer ruling A), which JC-3 notifies as file-error(Denied).
    assert_eq!(
        d.route(s, &file_chunk(b"x"), ROUTES, VETTED_TOOLS),
        Routed::FileChunkDeny { code: proto::ErrorCode::Denied },
        "the confined open denies the symlink escape",
    );
    assert_eq!(d.writes_count(), 0, "the poisoned writer is evicted on the confined-open deny");
}

// ─── MB4 liveness: the reactor loop drives dispatch over a FAKE wire and terminates ──────────
mod reactor_wiring {
    use super::*;
    use std::time::Duration;
    use reactor::{Host, Reactor, RxSource, WakeReason};
    use transport::{Instant, Originator, SessionTable, TransmitSink};

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
        // This endpoint is the proxy (it dispatches box-originated requests), so it allocates from
        // the high half (box → Low, proxy → High; allocator.rs). No real partition stakes here — the
        // fake ring's ids are peer-inbound — but High reads correctly for the role this test drives.
        let mut r = Reactor::new(SessionTable::new(Originator::High), FakeHost, rx, FakeSink);

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

// ─── Sprint 10 Slice 2 — file-read acting + streaming reply ───────────────────────────────────
mod file_read_reply {
    use super::*;
    use proxy::fs_read::READ_CHUNK;
    use proxy::nic::Reply;
    use transport::{IdAllocator, Originator, SendError};

    /// A `file-read` request frame naming a flat source under the readable root.
    fn file_read(name: &str) -> Vec<u8> {
        encode_message(&Message::FileRead(FileRead {
            source: proto::scan_unsafe(name.as_bytes()).unwrap(),
        }))
    }

    /// A controllable [`Reply`] fake: mints reply ids from the proxy's own high half via the real
    /// committed allocator, records every emitted frame **in order** (the `open`, then each `send`), and
    /// WouldBlocks a `send` once its `budget` is exhausted — so pacing is exercised hardware-free.
    struct FakeReply {
        alloc: IdAllocator,
        reply_sid: Option<SessionId>,
        emitted: Vec<Vec<u8>>,
        budget: usize,
    }
    impl FakeReply {
        fn new(budget: usize) -> Self {
            FakeReply { alloc: IdAllocator::new(Originator::High), reply_sid: None, emitted: Vec::new(), budget }
        }
    }
    impl Reply for FakeReply {
        fn open(&mut self, payload: &[u8]) -> SessionId {
            let id = self.alloc.allocate();
            self.reply_sid = Some(id);
            self.emitted.push(payload.to_vec());
            id
        }
        fn send(&mut self, id: SessionId, payload: &[u8]) -> Result<(), SendError> {
            assert_eq!(Some(id), self.reply_sid, "a send must target the opened reply session");
            if self.budget == 0 {
                return Err(SendError::WouldBlock);
            }
            self.budget -= 1;
            self.emitted.push(payload.to_vec());
            Ok(())
        }
    }

    /// Decode one reply frame: the 8-byte big-endian `in_reply_to` (the TASK-16 stub prefix), then the
    /// proto `file-data` message. Returns `(in_reply_to, verbatim bytes)`.
    fn decode_reply_frame(frame: &[u8]) -> (u64, Vec<u8>) {
        let in_reply_to = u64::from_be_bytes(frame[0..8].try_into().unwrap());
        match proto::decode_message(&frame[8..]).expect("a proto file-data message") {
            Message::FileData(fd) => (in_reply_to, fd.0),
            other => panic!("expected file-data, got {other:?}"),
        }
    }

    fn three_chunk_content() -> Vec<u8> {
        (0..(READ_CHUNK * 2 + 1024)).map(|i| (i % 251) as u8).collect()
    }

    /// The happy path: an admitted file-read streams the file back as `file-data`, verbatim and in
    /// order, each frame stamped with the REQUEST session as `in-reply-to` (never the reply's own
    /// proxy-high id — the un-fakeable correlation check), and the completed stream is evicted (fd
    /// closes).
    #[test]
    fn file_read_streams_file_data_stamped_with_the_request_session() {
        let sc = Scratch::new();
        let content = three_chunk_content();
        std::fs::write(sc.dir.join("src.txt"), &content).unwrap();
        let mut d = Dispatcher::with_readable_root(sc.dir.clone());
        let s_req = SessionId(7); // a low-half request session
        assert_eq!(d.route(s_req, &file_read("src.txt"), ROUTES, VETTED_TOOLS), Routed::FileReadStreaming);
        assert_eq!(d.replies_count(), 1, "an admitted file-read registers one outbound stream");

        let mut reply = FakeReply::new(usize::MAX);
        d.pump_replies(&mut reply); // unlimited budget: drains fully in one tick, then evicts
        assert_eq!(d.replies_count(), 0, "the completed stream is evicted (its FileReader fd closes)");

        let reply_sid = reply.reply_sid.expect("a reply session was opened").0;
        assert_ne!(reply_sid & (1 << 63), 0, "the reply session is the proxy's own high half");
        assert!(reply.emitted.len() >= 3, "a >2-chunk file streamed as multiple frames");
        let mut got = Vec::new();
        for frame in &reply.emitted {
            let (irt, body) = decode_reply_frame(frame);
            // THE un-fakeable check: in_reply_to is the REQUEST session, never the reply session.
            assert_eq!(irt, s_req.0, "every frame carries the request session as in-reply-to");
            assert_ne!(irt, reply_sid, "in-reply-to must NOT be the reply's own session id");
            got.extend_from_slice(&body);
        }
        assert_eq!(got, content, "the streamed file-data reassembles to the original, in order");
    }

    /// F2 pacing: a multi-chunk reply paces off `SendError::WouldBlock` — the stream is retained (never
    /// dropped) across backpressure and resumes in order, delivering the whole file with no drop or
    /// reorder, the correlation key on EVERY frame.
    #[test]
    fn file_read_reply_paces_off_wouldblock_no_drop_no_reorder() {
        let sc = Scratch::new();
        let content = three_chunk_content();
        std::fs::write(sc.dir.join("src.txt"), &content).unwrap();
        let mut d = Dispatcher::with_readable_root(sc.dir.clone());
        let s_req = SessionId(9);
        assert_eq!(d.route(s_req, &file_read("src.txt"), ROUTES, VETTED_TOOLS), Routed::FileReadStreaming);

        // Budget 0: the first frame opens (origination cannot WouldBlock); the next send WouldBlocks.
        let mut reply = FakeReply::new(0);
        d.pump_replies(&mut reply);
        assert_eq!(reply.emitted.len(), 1, "only the opened first frame went out; the rest is backpressured");
        assert_eq!(d.replies_count(), 1, "the stream is retained across the WouldBlock, not dropped");

        // Free one send per tick; the stream resumes in order until complete.
        let mut ticks = 0;
        while d.replies_count() > 0 {
            ticks += 1;
            assert!(ticks < 1000, "pacing made no progress");
            reply.budget += 1;
            d.pump_replies(&mut reply);
        }
        let mut got = Vec::new();
        for frame in &reply.emitted {
            let (irt, body) = decode_reply_frame(frame);
            assert_eq!(irt, s_req.0, "the correlation key is on EVERY frame, not just the first");
            got.extend_from_slice(&body);
        }
        assert_eq!(got, content, "WouldBlock-paced stream delivered the whole file in order, no drop");
    }

    /// JC-1 extends to file-read (now acted on): a nonzero reserved surface denies the whole message,
    /// deny-without-binding, and registers no stream.
    #[test]
    fn file_read_reserved_bits_deny_whole_no_stream() {
        let sc = Scratch::new();
        let mut d = Dispatcher::with_readable_root(sc.dir.clone());
        let frame = set_reserved(&file_read("whatever.txt"), 0x0100_0000);
        assert!(matches!(
            d.route(SessionId(1), &frame, ROUTES, VETTED_TOOLS),
            Routed::ReservedDeny { .. }
        ));
        // No DATA stream (the verdict is a deny, not FileReadStreaming); but Slice 4 enqueues the JC-3
        // notification (file-error), verified in the notification tests below.
        assert_eq!(d.replies_count(), 1, "a reserved-denied file-read enqueues a JC-3 notification, not a data stream");
        assert_eq!(d.binding_count(), 0, "deny-without-binding");
    }

    /// A missing source fails safe: `FileReadError` (proxy-owned code, no box name), no stream. The
    /// box-facing `file-error` notification is JC-3's (Slice 4).
    #[test]
    fn file_read_missing_source_fails_safe_no_stream() {
        let sc = Scratch::new();
        let mut d = Dispatcher::with_readable_root(sc.dir.clone());
        match d.route(SessionId(1), &file_read("does-not-exist.txt"), ROUTES, VETTED_TOOLS) {
            Routed::FileReadError { code } => assert_eq!(code, proto::ErrorCode::NotFound),
            other => panic!("expected a fail-safe FileReadError, got {other:?}"),
        }
        // No file-data stream (a failed open reads nothing); Slice 4 enqueues the file-error JC-3
        // notification (content verified in the notification tests).
        assert_eq!(d.replies_count(), 1, "a failed open enqueues a JC-3 file-error notification, no data stream");
    }

    /// fd-lifecycle: `close_session` evicts an in-flight reply stream (its source fd closes), mirroring
    /// the writer eviction.
    #[test]
    fn close_session_evicts_an_in_flight_reply_stream() {
        let sc = Scratch::new();
        std::fs::write(sc.dir.join("src.txt"), vec![0u8; READ_CHUNK * 2]).unwrap();
        let mut d = Dispatcher::with_readable_root(sc.dir.clone());
        let s = SessionId(3);
        assert_eq!(d.route(s, &file_read("src.txt"), ROUTES, VETTED_TOOLS), Routed::FileReadStreaming);
        let mut reply = FakeReply::new(0);
        d.pump_replies(&mut reply); // opens frame 1, backpressured on frame 2 → still in-flight
        assert_eq!(d.replies_count(), 1, "stream in-flight");
        d.close_session(s);
        assert_eq!(d.replies_count(), 0, "close_session evicts the in-flight stream (source fd closes)");
    }

    /// With no readable root configured, a file-read routes as relayed (its pre-Slice-2 disposition),
    /// never a silent accept, and registers no stream.
    #[test]
    fn file_read_without_readable_root_relays() {
        let mut d = Dispatcher::new();
        assert_eq!(
            d.route(SessionId(1), &file_read("x.txt"), ROUTES, VETTED_TOOLS),
            Routed::Relay(RelayKind::FileRead)
        );
        assert_eq!(d.replies_count(), 0);
    }

    // ─── the `list` dir-lister sub-gate (Slice 2, its own named adversarial corpus) ──────────────

    fn list_frame(name: &str) -> Vec<u8> {
        encode_message(&Message::List(proto::List(proto::scan_unsafe(name.as_bytes()).unwrap())))
    }
    fn decode_list_frame(frame: &[u8]) -> (u64, Vec<proto::ListEntry>) {
        let irt = u64::from_be_bytes(frame[0..8].try_into().unwrap());
        match proto::decode_message(&frame[8..]).expect("a proto list-entry message") {
            Message::ListEntries(le) => (irt, le.0),
            other => panic!("expected list-entry, got {other:?}"),
        }
    }
    fn name_of(e: &proto::ListEntry) -> String {
        String::from_utf8_lossy(e.name.as_bytes()).into_owned()
    }

    /// Happy path: a `list` streams the directory's entries as `list-entry` records — ONE entry per
    /// frame (bounded-lazy, never slurped), each stamped with the REQUEST session as `in-reply-to`,
    /// names typed + types correct, and the completed stream evicted.
    #[test]
    fn list_streams_entries_stamped_with_request_session() {
        let sc = Scratch::new();
        std::fs::create_dir(sc.dir.join("d")).unwrap();
        std::fs::write(sc.dir.join("d/alpha.txt"), b"a").unwrap();
        std::fs::write(sc.dir.join("d/beta.txt"), b"bb").unwrap();
        std::fs::create_dir(sc.dir.join("d/sub")).unwrap();
        let mut d = Dispatcher::with_readable_root(sc.dir.clone());
        let s_req = SessionId(11);
        assert_eq!(d.route(s_req, &list_frame("d"), ROUTES, VETTED_TOOLS), Routed::ListStreaming);
        assert_eq!(d.replies_count(), 1);

        let mut reply = FakeReply::new(usize::MAX);
        d.pump_replies(&mut reply);
        assert_eq!(d.replies_count(), 0, "the completed list stream is evicted");
        assert_eq!(reply.emitted.len(), 3, "three entries → three frames (one entry each, bounded-lazy)");

        let mut names = std::collections::HashSet::new();
        for frame in &reply.emitted {
            let (irt, entries) = decode_list_frame(frame);
            assert_eq!(irt, s_req.0, "every list-entry frame carries the request session as in-reply-to");
            assert_eq!(entries.len(), 1, "ONE entry per frame — never slurps the whole directory");
            let e = &entries[0];
            let ft = if name_of(e) == "sub" { proto::FileType::Dir } else { proto::FileType::File };
            assert_eq!(e.file_type, ft, "entry type is correct");
            names.insert(name_of(e));
        }
        let expected: std::collections::HashSet<String> =
            ["alpha.txt", "beta.txt", "sub"].iter().map(|s| s.to_string()).collect();
        assert_eq!(names, expected, "all entries listed, names typed verbatim");
    }

    /// The symlink-follow accepted-harmless case (sub-gate #6): a symlinked source directory is followed
    /// by `read_dir` and lists the TARGET's names — free on a fail-safe read channel (listing a
    /// different directory is harmless), documented as accepted, not papered as a hole.
    #[test]
    fn list_symlink_source_follows_target_harmless() {
        let sc = Scratch::new();
        std::fs::create_dir(sc.dir.join("real")).unwrap();
        std::fs::write(sc.dir.join("real/inside.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(sc.dir.join("real"), sc.dir.join("link")).unwrap();
        let mut d = Dispatcher::with_readable_root(sc.dir.clone());
        assert_eq!(d.route(SessionId(1), &list_frame("link"), ROUTES, VETTED_TOOLS), Routed::ListStreaming);
        let mut reply = FakeReply::new(usize::MAX);
        d.pump_replies(&mut reply);
        let mut names = std::collections::HashSet::new();
        for frame in &reply.emitted {
            let (_, entries) = decode_list_frame(frame);
            for e in &entries {
                names.insert(name_of(e));
            }
        }
        assert!(names.contains("inside.txt"), "the symlinked source lists its target's entries (harmless follow)");
    }

    /// Fail-safe on a non-directory (sub-gate #2): `read_dir` on a regular file errors → `ListError`,
    /// no stream, no box name.
    #[test]
    fn list_non_directory_fails_safe_no_stream() {
        let sc = Scratch::new();
        std::fs::write(sc.dir.join("afile"), b"x").unwrap();
        let mut d = Dispatcher::with_readable_root(sc.dir.clone());
        match d.route(SessionId(1), &list_frame("afile"), ROUTES, VETTED_TOOLS) {
            Routed::ListError { .. } => {}
            other => panic!("expected a fail-safe ListError on a non-directory, got {other:?}"),
        }
        assert_eq!(d.replies_count(), 1, "the ListError enqueues a JC-3 file-error notification, no data stream");
    }

    /// Fail-safe on a missing source: `ListError` with `NotFound`, no stream.
    #[test]
    fn list_missing_source_fails_safe() {
        let sc = Scratch::new();
        let mut d = Dispatcher::with_readable_root(sc.dir.clone());
        match d.route(SessionId(1), &list_frame("nope"), ROUTES, VETTED_TOOLS) {
            Routed::ListError { code } => assert_eq!(code, proto::ErrorCode::NotFound),
            other => panic!("expected ListError NotFound, got {other:?}"),
        }
    }

    /// Traversal/absolute sources are structurally unrepresentable (sub-gate #1): the `list` source is an
    /// `UnsafeSegmentStack` via `scan_unsafe`, which rejects `..`/`.`/`/` — the same codec floor as
    /// file-read/file-write. A traversal source can never reach the dir-lister.
    #[test]
    fn list_traversal_source_unrepresentable_at_the_codec() {
        assert!(proto::scan_unsafe(b"..").is_err(), "`..` is not a representable list source");
        assert!(proto::scan_unsafe(b".").is_err(), "`.` is not a representable list source");
    }

    /// JC-1 extends to list (now acted on): a nonzero reserved surface denies the whole message,
    /// deny-without-binding, no stream.
    #[test]
    fn list_reserved_bits_deny_whole_no_stream() {
        let sc = Scratch::new();
        std::fs::create_dir(sc.dir.join("d")).unwrap();
        let mut d = Dispatcher::with_readable_root(sc.dir.clone());
        let frame = set_reserved(&list_frame("d"), 0x0100_0000);
        assert!(matches!(
            d.route(SessionId(1), &frame, ROUTES, VETTED_TOOLS),
            Routed::ReservedDeny { .. }
        ));
        assert_eq!(d.replies_count(), 1, "a reserved-denied list enqueues a JC-3 notification, not a data stream");
        assert_eq!(d.binding_count(), 0, "deny-without-binding");
    }

    /// With no readable root configured, a list routes as relayed, never a silent accept, no stream.
    #[test]
    fn list_without_readable_root_relays() {
        let mut d = Dispatcher::new();
        assert_eq!(
            d.route(SessionId(1), &list_frame("d"), ROUTES, VETTED_TOOLS),
            Routed::Relay(RelayKind::FileRead)
        );
        assert_eq!(d.replies_count(), 0);
    }

    // ─── Sprint 10 Slice 3 — the relay reply halves: push-fed queue (fake ingress) ───────────────
    //
    // BOUNDARY HONESTY (gate 4): these prove the box-facing SEND + the push-queue pacing against a
    // FAKE ingress that simulates cloud/head data arriving — NOT the real upstream TLS client or head
    // connection (an explicit follow-on). Same receive-only-style honesty as Sprint 8.
    use proxy::dispatch::{Enqueue, REPLY_QUEUE_CAP};

    fn decode_net_response_frame(frame: &[u8]) -> (u64, Vec<u8>) {
        let irt = u64::from_be_bytes(frame[0..8].try_into().unwrap());
        match proto::decode_message(&frame[8..]).expect("a proto net-response message") {
            Message::NetResponse(nr) => (irt, nr.0),
            other => panic!("expected net-response, got {other:?}"),
        }
    }

    /// net-response shares the push-fed shape: chunks arriving from the (fake) cloud ingress stream to
    /// the box as `net-response`, verbatim and in order, each stamped with the BOX's originating
    /// net-request session as `in_reply_to` (never the reply's own session), then closed → evicted.
    #[test]
    fn net_response_streams_stamped_with_the_box_request_session() {
        let mut d = Dispatcher::new(); // no readable root — this is the push (ingress) path
        let s_req = SessionId(21); // the box's net-request session (box-low)
        assert_eq!(d.enqueue_net_response(s_req, b"chunk-A"), Enqueue::Accepted);
        assert_eq!(d.enqueue_net_response(s_req, b"chunk-B"), Enqueue::Accepted);
        assert_eq!(d.enqueue_net_response(s_req, b"chunk-C"), Enqueue::Accepted);
        assert_eq!(d.replies_count(), 1, "one push reply stream for the session");
        d.close_reply(s_req); // upstream EOF

        let mut reply = FakeReply::new(usize::MAX);
        d.pump_replies(&mut reply);
        assert_eq!(d.replies_count(), 0, "closed + drained → evicted");
        let reply_sid = reply.reply_sid.expect("a reply session opened").0;
        let mut chunks = Vec::new();
        for frame in &reply.emitted {
            let (irt, chunk) = decode_net_response_frame(frame);
            assert_eq!(irt, s_req.0, "every net-response frame stamped with the BOX request session");
            assert_ne!(irt, reply_sid, "in-reply-to is NOT the reply's own session");
            chunks.push(chunk);
        }
        assert_eq!(chunks, vec![b"chunk-A".to_vec(), b"chunk-B".to_vec(), b"chunk-C".to_vec()], "verbatim, in order");
    }

    /// GATE 1 — bounded with backpressure, never drop: the fake ingress pushes FASTER than the box
    /// drains; the queue is capped at REPLY_QUEUE_CAP and returns `Full` (the ingress stops reading),
    /// and every pushed chunk is eventually delivered, in order, none dropped, none reordered.
    #[test]
    fn net_response_backpressures_when_pushed_faster_than_drained_never_drops() {
        let mut d = Dispatcher::new();
        let s = SessionId(22);
        let total: u32 = (REPLY_QUEUE_CAP * 3) as u32; // want to push 3× the queue depth
        let mut next_push = 0u32;
        let mut saw_backpressure = false;
        let mut reply = FakeReply::new(0);
        let mut delivered: Vec<u32> = Vec::new();
        let mut guard = 0;
        loop {
            guard += 1;
            assert!(guard < 1_000_000, "no forward progress");
            // Ingress pushes as fast as it can until backpressured — NEVER drops.
            while next_push < total {
                match d.enqueue_net_response(s, &next_push.to_le_bytes()) {
                    Enqueue::Accepted => next_push += 1,
                    Enqueue::Full => {
                        saw_backpressure = true;
                        break;
                    }
                }
            }
            if next_push == total {
                d.close_reply(s);
            }
            // The box drains SLOWLY — one more send freed per tick.
            reply.budget += 1;
            let before = reply.emitted.len();
            d.pump_replies(&mut reply);
            for f in &reply.emitted[before..] {
                delivered.push(u32::from_le_bytes(decode_net_response_frame(f).1.try_into().unwrap()));
            }
            if next_push == total && d.replies_count() == 0 {
                break;
            }
        }
        assert!(saw_backpressure, "pushing faster than draining MUST hit the bounded-queue backpressure");
        assert_eq!(delivered.len() as u32, total, "every pushed chunk delivered — backpressure never dropped one");
        assert!(delivered.iter().copied().eq(0..total), "delivered strictly in order — no reorder");
    }

    /// GATE 2 — EMPTY != DONE: a momentarily-empty (still-open) push queue is NOT evicted; a late chunk
    /// streams on the same live reply session; only close_reply + drain evicts (the tail is never
    /// orphaned).
    #[test]
    fn push_reply_empty_is_not_done_only_closed_and_drained_evicts() {
        let mut d = Dispatcher::new();
        let s = SessionId(23);
        d.enqueue_net_response(s, b"first");
        let mut reply = FakeReply::new(usize::MAX);
        d.pump_replies(&mut reply); // drains "first"; queue now EMPTY but OPEN
        assert_eq!(reply.emitted.len(), 1);
        assert_eq!(d.replies_count(), 1, "empty-but-open stream is NOT evicted (waiting for more)");

        d.enqueue_net_response(s, b"second"); // a late chunk arrives
        d.pump_replies(&mut reply);
        assert_eq!(reply.emitted.len(), 2, "the late chunk streamed on the still-live reply session");
        assert_eq!(d.replies_count(), 1, "still open");

        d.close_reply(s); // upstream EOF → drain (nothing left) → done → evicted
        d.pump_replies(&mut reply);
        assert_eq!(d.replies_count(), 0, "closed + drained → evicted");
    }

    /// Terminal head→box shares the push-fed shape but is relayed VERBATIM — emission (F1: no
    /// correlation key) + zero-modification relay (MB3): the emitted frame is byte-identical to the
    /// input, with NO proxy-added in_reply_to prefix.
    #[test]
    fn terminal_head_to_box_relayed_verbatim_no_correlation_key() {
        let mut d = Dispatcher::new();
        let s = SessionId(24); // the box terminal session
        let input = raw_frame(proto::verb::KEY, b"keystroke-bytes"); // an opaque head→box input frame
        assert_eq!(d.enqueue_terminal_to_box(s, &input), Enqueue::Accepted);
        d.close_reply(s);
        let mut reply = FakeReply::new(usize::MAX);
        d.pump_replies(&mut reply);
        assert_eq!(reply.emitted.len(), 1);
        assert_eq!(reply.emitted[0], input, "terminal input relayed byte-identical — no proxy-added key");
    }

    /// A push reply stream is evicted on close_session too (fd-lifecycle parity with the pull streams).
    #[test]
    fn push_reply_evicted_on_close_session() {
        let mut d = Dispatcher::new();
        let s = SessionId(25);
        d.enqueue_net_response(s, b"x");
        assert_eq!(d.replies_count(), 1);
        d.close_session(s);
        assert_eq!(d.replies_count(), 0, "close_session evicts a push reply stream too");
    }

    // ─── Sprint 10 Slice 4 — JC-3 deny-notification content (the ruled taxonomy) ─────────────────

    #[derive(PartialEq, Eq, Debug)]
    enum Notif {
        File(proto::ErrorCode),
        Net(proto::ErrorCode),
    }
    fn decode_notification(frame: &[u8]) -> (u64, Notif) {
        let irt = u64::from_be_bytes(frame[0..8].try_into().unwrap());
        let n = match proto::decode_message(&frame[8..]).expect("a proto error message") {
            Message::FileError(fe) => Notif::File(fe.0.code),
            Message::NetError(ne) => Notif::Net(ne.0.code),
            other => panic!("expected file-error/net-error, got {other:?}"),
        };
        (irt, n)
    }
    /// Drive the (already-routed) deny's single-frame notification out and decode it.
    fn notification_of(d: &mut Dispatcher) -> (u64, Notif) {
        let mut reply = FakeReply::new(usize::MAX);
        d.pump_replies(&mut reply);
        assert_eq!(reply.emitted.len(), 1, "a JC-3 notification is exactly one frame");
        decode_notification(&reply.emitted[0])
    }
    fn net_payload(content: &[u8]) -> Vec<u8> {
        encode_message(&Message::NetPayload(NetPayload { more: false, content: content.to_vec() }))
    }

    /// A reserved-verb deny (JC-1) notifies with the VERB's service error and code Malformed (ruling B):
    /// a net verb → net-error(Malformed); a file verb → file-error(Malformed). Stamped in_reply_to = the
    /// request session.
    #[test]
    fn jc3_reserved_deny_notifies_service_error_malformed() {
        // net verb (net-address): net-error(Malformed).
        let mut d = Dispatcher::new();
        let s = SessionId(31);
        assert!(matches!(
            d.route(s, &set_reserved(&net_address("api.anthropic.com/v1/messages"), 0x0100_0000), ROUTES, VETTED_TOOLS),
            Routed::ReservedDeny { .. }
        ));
        assert_eq!(notification_of(&mut d), (s.0, Notif::Net(proto::ErrorCode::Malformed)));

        // file verb (file-read): file-error(Malformed).
        let sc = Scratch::new();
        let mut d = Dispatcher::with_readable_root(sc.dir.clone());
        let s = SessionId(32);
        assert!(matches!(
            d.route(s, &set_reserved(&file_read("x.txt"), 0x0100_0000), ROUTES, VETTED_TOOLS),
            Routed::ReservedDeny { .. }
        ));
        assert_eq!(notification_of(&mut d), (s.0, Notif::File(proto::ErrorCode::Malformed)));
    }

    /// A fail-safe file-read (missing source) notifies file-error with the fail-safe code (NotFound).
    #[test]
    fn jc3_file_read_failsafe_notifies_file_error_code() {
        let sc = Scratch::new();
        let mut d = Dispatcher::with_readable_root(sc.dir.clone());
        let s = SessionId(33);
        assert!(matches!(
            d.route(s, &file_read("nope.txt"), ROUTES, VETTED_TOOLS),
            Routed::FileReadError { .. }
        ));
        assert_eq!(notification_of(&mut d), (s.0, Notif::File(proto::ErrorCode::NotFound)));
    }

    /// A net-payload with no admitted route notifies net-error(Denied) (ruling D).
    #[test]
    fn jc3_net_payload_no_route_notifies_net_error_denied() {
        let mut d = Dispatcher::new();
        let s = SessionId(34);
        assert_eq!(
            d.route(s, &net_payload(b"{}"), ROUTES, VETTED_TOOLS),
            Routed::NetPayloadNoRoute
        );
        assert_eq!(notification_of(&mut d), (s.0, Notif::Net(proto::ErrorCode::Denied)));
    }

    /// A file-chunk confined-open deny notifies file-error(Denied) — the write-destination floor firing
    /// one message late (ruling A).
    #[test]
    fn jc3_file_chunk_confined_open_deny_notifies_file_error_denied() {
        let sc = Scratch::new();
        std::os::unix::fs::symlink("/etc", sc.dir.join("link")).unwrap();
        let mut d = sc.dispatcher(); // quarantine-root configured
        let s = SessionId(35);
        assert_eq!(d.route(s, &file_write_to("link/passwd"), ROUTES, VETTED_TOOLS), Routed::FileWriteAdmit);
        assert_eq!(
            d.route(s, &file_chunk(b"x"), ROUTES, VETTED_TOOLS),
            Routed::FileChunkDeny { code: proto::ErrorCode::Denied }
        );
        assert_eq!(notification_of(&mut d), (s.0, Notif::File(proto::ErrorCode::Denied)));
    }

    /// An unsupported verb (JC-2, never bound to a service) is SILENT — no notification (ruling C).
    #[test]
    fn jc3_unsupported_verb_is_silent() {
        let mut d = Dispatcher::new();
        let s = SessionId(36);
        assert!(matches!(
            d.route(s, &raw_frame(0x4F, b""), ROUTES, VETTED_TOOLS), // block 4 → no service
            Routed::Unsupported { .. }
        ));
        assert_eq!(d.replies_count(), 0, "an unsupported verb notifies NOTHING (JC-2/JC-3a)");
    }

    /// A structural decode reject (non-cap) is SILENT — JC-3(a), drop+log, no return.
    #[test]
    fn jc3_structural_decode_is_silent() {
        let mut d = Dispatcher::new();
        let s = SessionId(37);
        // A net-address verb with a truncated body → a proto decode Reject (structural), not a policy deny.
        assert!(matches!(
            d.route(s, &raw_frame(proto::verb::NET_ADDRESS, &[0x00]), ROUTES, VETTED_TOOLS),
            Routed::Decode(_)
        ));
        assert_eq!(d.replies_count(), 0, "a structural decode reject notifies NOTHING (JC-3a)");
    }
}
