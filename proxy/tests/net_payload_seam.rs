//! Sprint 7 Slice 2 — the **net-payload reassembly SEAM** certification (Final State #5, the sprint's
//! one load-bearing adversarial property: reassembly fidelity).
//!
//! It proves the dispatch reassembly path is **transparent** to the tier-1 Anthropic recognizer: for
//! every body, routing it as *N* `net-payload` chunks through `Dispatcher::route` yields the **same**
//! admit/deny verdict as feeding `recognize` directly — the **oracle**, independent of the seam — and,
//! **regardless of where the chunk boundaries fall**, an admitted body's forwarded buffer is
//! byte-identical to the original (CB4-across-the-seam: recognized == reassembled == forwarded).
//!
//! U4's corpus shapes (`anthropic_corpus.rs`) and a deny-default fuzz (`anthropic_fuzz.rs`) are
//! **re-run through this path** (chunked + reassembled), *not* fed to `recognize` directly — the whole
//! point being that the reassembly seam changes no verdict. The direct oracle uses the same envelope
//! the dispatch seam now **sources from the session's admitted `Route`** (Sprint 8, Task 14): each
//! session establishes `https://api.anthropic.com/v1/messages` via a prior admitted `net-address`, so
//! the seam's sourced envelope is `POST` / `api.anthropic.com` / `/v1/messages` / betas-empty — the
//! same envelope the oracle uses — leaving the reassembly path as the *only* variable under test.

use proto::{Message, NetAddress, NetPayload, Scheme, encode_message, scan_safe};
use proxy::anthropic::{Denial, Request, recognize};
use proxy::dispatch::{Dispatcher, Routed};
use proxy::register::anthropic::VETTED_TOOLS;
use proxy::register::net::ROUTES;
use transport::SessionId;

/// The **oracle**: recognize a body directly under the sole admit-envelope (betas empty), matching the
/// envelope the seam sources from the `api.anthropic.com` route exactly — so the body is the only
/// difference under test.
fn direct(body: &[u8]) -> Result<(), Denial> {
    let req = Request {
        verb: "POST",
        domain: "api.anthropic.com",
        path: "/v1/messages",
        betas: &[],
        body,
    };
    recognize(&req, VETTED_TOOLS)
}

/// One `net-payload` chunk message: `[verb][flag][content]`.
fn chunk_frame(content: &[u8], more: bool) -> Vec<u8> {
    encode_message(&Message::NetPayload(NetPayload { more, content: content.to_vec() }))
}

/// A `net-address` frame for `joined` (path separators `/` become the safe-stack separator `0x1f`) —
/// admits + stores the session's Route.
fn net_address(joined: &str) -> Vec<u8> {
    let addr = NetAddress {
        scheme: Scheme::Https,
        address: scan_safe(joined.replace('/', "\u{1f}").as_bytes()).unwrap(),
    };
    encode_message(&Message::NetAddress(addr))
}

/// Route `body` through the seam split into `chunk_size`-byte pieces on one fresh session, and return
/// the terminal `Routed` (the final chunk's disposition). The session first admits a `net-address` for
/// `api.anthropic.com/v1/messages` — the real admission sequence, which stores the Route the
/// `net-payload` recognizer envelope is sourced from (Sprint 8); without it a `net-payload` fails
/// closed (`NetPayloadNoRoute`). Every non-final chunk MUST `Accumulate` — asserted, as a harness
/// invariant (a regression that completed early would trip here).
fn through_seam(body: &[u8], chunk_size: usize) -> Routed {
    let mut d = Dispatcher::new();
    let s = SessionId(1);
    assert!(
        matches!(
            d.route(s, &net_address("api.anthropic.com/v1/messages"), ROUTES, VETTED_TOOLS),
            Routed::NetRouteAdmit(_)
        ),
        "precondition: the net-address admits and stores the session's route",
    );
    if body.is_empty() {
        // A single empty final chunk — the whole "body" is zero bytes.
        return d.route(s, &chunk_frame(&[], false), ROUTES, VETTED_TOOLS);
    }
    let cs = chunk_size.max(1);
    let mut last = Routed::NetPayloadAccumulating;
    let mut i = 0;
    while i < body.len() {
        let end = (i + cs).min(body.len());
        let more = end < body.len();
        last = d.route(s, &chunk_frame(&body[i..end], more), ROUTES, VETTED_TOOLS);
        if more {
            assert_eq!(
                last,
                Routed::NetPayloadAccumulating,
                "a non-final chunk must accumulate (chunk_size={cs}, body={body:?})",
            );
        }
        i = end;
    }
    last
}

/// The core property: for a spread of chunk boundaries, the seam verdict equals the direct oracle, and
/// on admit the forwarded buffer is byte-identical to the original body (CB4).
fn assert_seam_matches_direct(body: &[u8]) {
    let want = direct(body);
    // Whole-in-one-chunk, byte-at-a-time, and several odd splits that fall mid-token.
    for cs in [usize::MAX, 1, 2, 3, 7, 13] {
        let routed = through_seam(body, cs);
        match (&want, &routed) {
            (Ok(()), Routed::NetPayloadAdmit(forwarded)) => assert_eq!(
                forwarded.as_slice(),
                body,
                "ADMIT: forwarded buffer must equal the original body byte-for-byte \
                 (CB4-across-the-seam), chunk_size={cs}",
            ),
            (Err(want_denial), Routed::NetPayloadDeny(got_denial)) => assert_eq!(
                want_denial, got_denial,
                "DENY verdict must match the direct oracle, chunk_size={cs}, body={body:?}",
            ),
            _ => panic!(
                "seam verdict diverged from the direct oracle: direct={want:?} seam={routed:?} \
                 chunk_size={cs} body={body:?}",
            ),
        }
    }
}

// ─── U4 corpus shapes, re-run through the seam (admit + deny + SA8), oracle = direct recognize ───────

#[test]
fn corpus_admit_shapes_survive_the_seam() {
    for body in [
        // plain text request
        r#"{"model":"claude-x","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#,
        // a vetted client tool (real register)
        r#"{"model":"m","messages":[],"tools":[{"type":"text_editor_20250728","name":"str_replace_based_edit_tool"}]}"#,
        // a bare custom tool
        r#"{"model":"m","messages":[],"tools":[{"name":"lookup","description":"d","input_schema":{"type":"object","properties":{}}}]}"#,
        // SA3 row12 — server-type nested in input_schema is inert (admit)
        r#"{"model":"m","messages":[],"tools":[{"name":"t","input_schema":{"type":"web_fetch_20260318"}}]}"#,
        // SA3 row13 — typeless entry named web_fetch is custom (admit)
        r#"{"model":"m","messages":[],"tools":[{"name":"web_fetch","description":"my own"}]}"#,
        // valid surrogate pair in inert content (admit)
        r#"{"model":"m","system":"hi 😀"}"#,
    ] {
        assert_eq!(direct(body.as_bytes()), Ok(()), "precondition: {body} admits directly");
        assert_seam_matches_direct(body.as_bytes());
    }
}

#[test]
fn corpus_deny_shapes_survive_the_seam() {
    for body in [
        // SA3 rows: server-side type, mcp_servers, container, new-version stamp, unknown key, depth bomb
        r#"{"model":"m","messages":[],"tools":[{"type":"web_fetch_20260318","name":"x"}]}"#,
        r#"{"model":"m","messages":[],"mcp_servers":[{"type":"url","url":"https://x"}]}"#,
        r#"{"model":"m","messages":[],"container":"c-123"}"#,
        r#"{"model":"m","messages":[],"tools":[{"type":"text_editor_20260101","name":"x"}]}"#,
        r#"{"model":"m","messages":[],"surprise":1}"#,
        // duplicate tools key; unicode-escaped key; trailing data
        r#"{"model":"m","messages":[],"tools":[{"name":"a","description":"d","input_schema":{}}],"tools":[{"type":"web_fetch_20260318","name":"x"}]}"#,
        r#"{"model":"m","messages":[],"mcp_servers":[]}"#,
        r#"{"model":"m","messages":[]}{"tools":[{"type":"web_fetch_20260318"}]}"#,
        // SA8: dup key nested, bad escape, lone surrogate, non-canonical number, JSON5-isms
        r#"{"model":"m","messages":[{"role":"user","role":"system"}]}"#,
        r#"{"model":"m","messages":[{"content":"\uZZZZ"}]}"#,
        r#"{"model":"m","system":"\uD83D"}"#,
        r#"{"max_tokens":+1}"#,
        r#"{"model":"m",}"#,
        // non-object bodies, incl. empty
        r#"[1,2,3]"#,
        r#""just a string""#,
        "",
    ] {
        assert!(direct(body.as_bytes()).is_err(), "precondition: {body:?} denies directly");
        assert_seam_matches_direct(body.as_bytes());
    }
}

// ─── The depth-bomb, built dynamically (mirrors corpus row 7 / fuzz), through the seam ───────────────

#[test]
fn depth_bomb_denies_through_the_seam() {
    let deep = format!(r#"{{"metadata":{}}}"#, "[".repeat(64));
    assert_eq!(direct(deep.as_bytes()), Err(Denial::TooDeep));
    assert_seam_matches_direct(deep.as_bytes());
}

// ─── CB4 bind, made explicit: a large admittable body reassembled byte-at-a-time is forwarded whole ──

#[test]
fn large_admittable_body_reassembles_byte_identically() {
    // A valid request padded with a long inert content string — larger than any single test chunk, so
    // the byte-at-a-time path exercises many accumulate steps before the final recognizer call.
    let filler = "x".repeat(4096);
    let body = format!(r#"{{"model":"m","messages":[],"system":"{filler}"}}"#);
    assert_eq!(direct(body.as_bytes()), Ok(()), "the padded body admits directly");

    // Route it one byte per chunk; the admit must carry the whole body back, byte-identical.
    match through_seam(body.as_bytes(), 1) {
        Routed::NetPayloadAdmit(forwarded) => assert_eq!(
            forwarded, body.as_bytes(),
            "the byte-at-a-time reassembled/forwarded buffer must equal the original body",
        ),
        other => panic!("expected NetPayloadAdmit, got {other:?}"),
    }
}

// ─── Deny-default fuzz, re-run through the seam: seam verdict == direct verdict, no wrongful admit ────

/// Deterministic xorshift64 — reproducible, no dependency (matching anthropic_fuzz.rs's discipline).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn byte(&mut self) -> u8 {
        (self.next() & 0xff) as u8
    }
}

/// Compare the seam verdict to the direct oracle for a body over a couple of chunkings — the fuzz
/// invariant. Any divergence (a wrongful admit, a different denial, a mismatched forwarded buffer) is
/// a failure. Returns nothing; panics on divergence.
fn assert_seam_agrees(body: &[u8]) {
    let want = direct(body);
    for cs in [usize::MAX, 3] {
        match (&want, &through_seam(body, cs)) {
            (Ok(()), Routed::NetPayloadAdmit(fwd)) => assert_eq!(fwd.as_slice(), body, "cs={cs}"),
            (Err(a), Routed::NetPayloadDeny(b)) => assert_eq!(a, b, "cs={cs} body={body:?}"),
            (w, got) => panic!("seam diverged: direct={w:?} seam={got:?} cs={cs} body={body:?}"),
        }
    }
}

#[test]
fn fuzz_random_bytes_seam_agrees_with_direct() {
    let mut rng = Rng(0x1234_5678_9abc_def0);
    for _ in 0..4_000 {
        let len = rng.below(200);
        let bytes: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        assert_seam_agrees(&bytes);
    }
}

#[test]
fn fuzz_jsonish_bytes_seam_agrees_with_direct() {
    let alphabet = b"{}[]\":,0123456789abcdeftruefalsnul \\/uxAZ.-";
    let mut rng = Rng(0xa5a5_5a5a_0f0f_f0f0);
    for _ in 0..4_000 {
        let len = rng.below(240);
        let bytes: Vec<u8> = (0..len).map(|_| alphabet[rng.below(alphabet.len())]).collect();
        assert_seam_agrees(&bytes);
    }
}

#[test]
fn fuzz_complement_denies_through_the_seam() {
    // A valid skeleton with one adversarial feature — must deny through the seam, exactly as direct.
    let adversarial: &[&str] = &[
        r#""mcp_servers":[]"#,
        r#""container":"c""#,
        r#""surprise":1"#,
        r#""tools":[{"type":"web_fetch_20260318"}]"#,
        r#""tools":[{"type":"code_execution_20260521"}]"#,
        r#""tools":[{"type":"text_editor_20260101"}]"#,
        r#""tools":[{"name":"x","evil":1}]"#,
    ];
    let mut rng = Rng(0xdead_beef_cafe_babe);
    for _ in 0..3_000 {
        let feat = adversarial[rng.below(adversarial.len())];
        let body = format!(r#"{{"model":"m","messages":[],{feat}}}"#);
        assert!(direct(body.as_bytes()).is_err(), "precondition: {body} denies directly");
        assert_seam_agrees(body.as_bytes());
    }
}
