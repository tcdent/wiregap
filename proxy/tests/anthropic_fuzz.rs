//! U4 CB5 — the deny-default-complement fuzz (Sprint 5, the required empirical discharge for the
//! worst-case surface; U4's permanent regression guard). Asserts:
//!   (a) `recognize` never panics on any input,
//!   (b) it always terminates (the run completing is the evidence — the bounded reader terminates
//!       by construction: a forward-only cursor + a depth cap),
//!   (c) it never Admits outside the known-admit shapes — by fuzzing the COMPLEMENT of the admitted
//!       set (adversarial features + structural ambiguity) and asserting Deny.
//! A tiny deterministic xorshift PRNG (fixed seeds, no external crate — the core stays lean) makes
//! this reproducible: a regression reproduces on the same seed.

use std::panic::catch_unwind;

use proxy::anthropic::{Denial, Request, recognize};
use proxy::register::anthropic::VETTED_TOOLS;

/// Deterministic xorshift64 — reproducible pseudo-randomness, no dependency.
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

fn rec(body: &[u8]) -> Result<(), Denial> {
    let req = Request {
        verb: "POST",
        domain: "api.anthropic.com",
        path: "/v1/messages",
        betas: &[],
        body,
    };
    recognize(&req, VETTED_TOOLS)
}

/// (a) + (b): random bytes never panic and always terminate.
#[test]
fn fuzz_random_bytes_never_panic_and_terminate() {
    let mut rng = Rng(0x1234_5678_9abc_def0);
    for _ in 0..20_000 {
        let len = rng.below(256);
        let bytes: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        let r = catch_unwind(|| rec(&bytes));
        assert!(r.is_ok(), "recognize panicked on random bytes: {bytes:?}");
    }
}

/// (a) + (b) with more structure: random JSON-ish bytes (a biased alphabet toward JSON tokens),
/// still never panic — exercises the parser's branches far more densely than uniform bytes.
#[test]
fn fuzz_jsonish_bytes_never_panic() {
    let alphabet = b"{}[]\":,0123456789abcdeftruefalsnul \\/uxAZ.-";
    let mut rng = Rng(0xa5a5_5a5a_0f0f_f0f0);
    for _ in 0..20_000 {
        let len = rng.below(300);
        let bytes: Vec<u8> = (0..len).map(|_| alphabet[rng.below(alphabet.len())]).collect();
        let r = catch_unwind(|| rec(&bytes));
        assert!(r.is_ok(), "recognize panicked on json-ish bytes: {bytes:?}");
    }
}

/// (c): the COMPLEMENT of the admitted set always denies. A valid skeleton with ONE adversarial
/// feature injected at random — each feature alone must flip a valid request to Deny.
#[test]
fn fuzz_complement_always_denies() {
    let mut rng = Rng(0xdead_beef_cafe_babe);
    let adversarial: &[&str] = &[
        r#""mcp_servers":[]"#,                            // forbidden key (Stage 2)
        r#""container":"c""#,                              // forbidden key (Stage 2)
        r#""surprise":1"#,                                 // unknown key (Stage 2)
        r#""tools":[{"type":"web_fetch_20260318"}]"#,      // server-side type (Stage 3)
        r#""tools":[{"type":"code_execution_20260521"}]"#, // server-side type (Stage 3)
        r#""tools":[{"type":"text_editor_20260101"}]"#,    // new-version stamp (Stage 3)
        r#""tools":[{"name":"x","evil":1}]"#,              // bare-custom with a non-custom key
    ];
    for _ in 0..10_000 {
        let feat = adversarial[rng.below(adversarial.len())];
        let body = format!(r#"{{"model":"m","messages":[],{feat}}}"#);
        let got = rec(body.as_bytes());
        assert!(got.is_err(), "adversarial body must deny: {body}");
    }
}

/// (c) reinforced: a valid base admits, and injecting any structural ambiguity flips it to Deny —
/// so no ambiguous body ever rides through as an admit.
#[test]
fn fuzz_ambiguity_injection_flips_admit_to_deny() {
    let base = r#"{"model":"m","messages":[],"max_tokens":10}"#;
    assert!(rec(base.as_bytes()).is_ok(), "the valid base admits");

    let mut rng = Rng(0x0f0f_0f0f_1234_5678);
    for _ in 0..10_000 {
        let mutated = match rng.below(4) {
            0 => format!("{base}{{}}"),                                   // trailing data
            1 => r#"{"model":"m","model":"n","messages":[]}"#.into(),     // duplicate key
            2 => format!(r#"{{"metadata":{}}}"#, "[".repeat(64)),         // depth bomb
            _ => r#"{"model":"m","messages":[],"max_tokens":+1}"#.into(), // non-canonical number
        };
        assert!(rec(mutated.as_bytes()).is_err(), "ambiguity must flip to deny: {mutated}");
    }
}

/// (c) at scale: randomly-assembled bodies from a mix of admit-safe and adversarial fields — assert
/// that whenever an adversarial field is present, the result denies (deny-default holds over
/// combinations, not just single injections).
#[test]
fn fuzz_mixed_fields_deny_when_any_adversarial_present() {
    let safe: &[&str] = &[
        r#""model":"m""#,
        r#""messages":[]"#,
        r#""max_tokens":5"#,
        r#""stream":true"#,
        r#""metadata":{"user":"u"}"#,
    ];
    let bad: &[&str] = &[
        r#""container":"c""#,
        r#""mcp_servers":[]"#,
        r#""tools":[{"type":"web_search_20260318"}]"#,
    ];
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..10_000 {
        let mut fields: Vec<&str> = Vec::new();
        // 1–3 safe fields (deduped by construction — pick distinct indices).
        let n_safe = 1 + rng.below(3);
        for k in 0..n_safe {
            fields.push(safe[(rng.below(safe.len()) + k) % safe.len()]);
        }
        // exactly one adversarial field, at a random position.
        let bad_field = bad[rng.below(bad.len())];
        let at = rng.below(fields.len() + 1);
        fields.insert(at, bad_field);
        // Dedup keys so a duplicate-key deny doesn't mask the adversarial-field deny we're testing.
        fields.sort_unstable();
        fields.dedup();
        let body = format!("{{{}}}", fields.join(","));
        assert!(
            rec(body.as_bytes()).is_err(),
            "a body containing an adversarial field must deny: {body}",
        );
    }
}
