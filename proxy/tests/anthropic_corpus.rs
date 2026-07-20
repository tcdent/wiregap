//! U4 Anthropic-recognizer corpus (Sprint 5, the strictest CB1–CB6 + Sprint-4's discharged
//! build-time obligation). Every SA3 closure row (13) and SA8 row (duplicate-key, bad-escape,
//! trailing-data, bad-number, JSON5) is a case here, **independently re-derived from the spec**
//! (`spec/anthropic-recognizer.md` §5/§6) — not from the recognizer's own behavior — and run
//! against the REAL recognizer with the REAL vetted register (`register::anthropic::VETTED_TOOLS`),
//! never a mock. Admit cases keep the deny-default gate non-vacuous.

use proxy::anthropic::{Denial, Request, recognize};
use proxy::register::anthropic::VETTED_TOOLS;

/// Recognize a body under the correct envelope + no betas (so the body is the only variable).
fn recognize_body(body: &str) -> Result<(), Denial> {
    let req = Request {
        verb: "POST",
        domain: "api.anthropic.com",
        path: "/v1/messages",
        betas: &[],
        body: body.as_bytes(),
    };
    recognize(&req, VETTED_TOOLS)
}

// ─── Admit cases (keep the deny-default gate non-vacuous) ────────────────────────────────────

#[test]
fn admits_a_plain_text_request() {
    assert_eq!(
        recognize_body(r#"{"model":"claude-x","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#),
        Ok(())
    );
}

#[test]
fn admits_a_vetted_client_tool() {
    // A type-tagged tool whose type is an exact vetted identity (from the real register).
    assert_eq!(
        recognize_body(r#"{"model":"m","messages":[],"tools":[{"type":"text_editor_20250728","name":"str_replace_based_edit_tool"}]}"#),
        Ok(())
    );
}

#[test]
fn admits_a_bare_custom_tool() {
    assert_eq!(
        recognize_body(r#"{"model":"m","messages":[],"tools":[{"name":"lookup","description":"d","input_schema":{"type":"object","properties":{}}}]}"#),
        Ok(())
    );
}

// ─── SA3 closure rows (spec §6) ──────────────────────────────────────────────────────────────

#[test]
fn sa3_row1_server_side_type_denies() {
    for t in ["web_fetch_20260318", "code_execution_20260521", "web_search_20260318"] {
        let body = format!(r#"{{"model":"m","messages":[],"tools":[{{"type":"{t}","name":"x"}}]}}"#);
        assert_eq!(recognize_body(&body), Err(Denial::UnvettedToolType), "server-side {t}");
    }
}

#[test]
fn sa3_row2_mcp_servers_denies() {
    assert_eq!(
        recognize_body(r#"{"model":"m","messages":[],"mcp_servers":[{"type":"url","url":"https://x"}]}"#),
        Err(Denial::UnknownField),
    );
}

#[test]
fn sa3_row3_container_denies() {
    assert_eq!(
        recognize_body(r#"{"model":"m","messages":[],"container":"c-123"}"#),
        Err(Denial::UnknownField),
    );
}

#[test]
fn sa3_row4_capability_beta_denies() {
    let req = Request {
        verb: "POST",
        domain: "api.anthropic.com",
        path: "/v1/messages",
        betas: &["mcp-client-2025-11-20"],
        body: br#"{"model":"m","messages":[]}"#,
    };
    assert_eq!(recognize(&req, VETTED_TOOLS), Err(Denial::BetaNotInert));
}

#[test]
fn sa3_row5_new_version_stamp_of_vetted_tool_denies() {
    // text_editor is vetted at _20250728; a new date stamp is a different exact string → denies.
    assert_eq!(
        recognize_body(r#"{"model":"m","messages":[],"tools":[{"type":"text_editor_20260101","name":"x"}]}"#),
        Err(Denial::UnvettedToolType),
    );
}

#[test]
fn sa3_row6_unknown_toplevel_key_denies() {
    assert_eq!(
        recognize_body(r#"{"model":"m","messages":[],"surprise":1}"#),
        Err(Denial::UnknownField),
    );
}

#[test]
fn sa3_row7_depth_bomb_denies() {
    // Nest arrays past MAX_DEPTH inside an inert value.
    let deep = format!(r#"{{"metadata":{}}}"#, "[".repeat(64) );
    assert_eq!(recognize_body(&deep), Err(Denial::TooDeep));
}

// Row 8 (size bomb) is proto::Reassembler's cap (reject-before-grow) — the body reaching this
// recognizer is already bounded; see the module doc. Not re-implemented here.

#[test]
fn sa3_row9_duplicate_tools_key_denies() {
    // {"tools":[inert], "tools":[server-side]} — inspect one, Anthropic executes the other.
    assert_eq!(
        recognize_body(r#"{"model":"m","messages":[],"tools":[{"name":"a","description":"d","input_schema":{}}],"tools":[{"type":"web_fetch_20260318","name":"x"}]}"#),
        Err(Denial::DuplicateKey),
    );
}

#[test]
fn sa3_row10_unicode_escaped_key_denies() {
    // "mcp_servers" — the key "mcp_servers" with its first char written as a `\u` escape, to
    // read as inert to one parser and capability to another. Identifiers reject escapes outright
    // (fail-closed), so the smuggling is impossible. (The `m` is literal in this raw string.)
    assert_eq!(
        recognize_body(r#"{"model":"m","messages":[],"\u006dcp_servers":[]}"#),
        Err(Denial::BadEscape),
    );
}

#[test]
fn sa3_row11_trailing_data_denies() {
    // A second object after the first — recognizer stops at the first; Anthropic might read more.
    assert_eq!(
        recognize_body(r#"{"model":"m","messages":[]}{"tools":[{"type":"web_fetch_20260318"}]}"#),
        Err(Denial::TrailingData),
    );
}

#[test]
fn sa3_row12_server_type_nested_in_input_schema_is_inert() {
    // A "type":"web_fetch..." buried inside a custom tool's input_schema is DATA, not a tools[]
    // entry — no server execution is selected there. Admitted (correct — it is inert).
    assert_eq!(
        recognize_body(r#"{"model":"m","messages":[],"tools":[{"name":"t","input_schema":{"type":"web_fetch_20260318"}}]}"#),
        Ok(()),
    );
}

#[test]
fn sa3_row13_typeless_entry_named_web_fetch_is_admitted_as_custom() {
    // A type-LESS entry named "web_fetch" is client-executed by construction — no server effect.
    assert_eq!(
        recognize_body(r#"{"model":"m","messages":[],"tools":[{"name":"web_fetch","description":"my own"}]}"#),
        Ok(()),
    );
}

// ─── SA8 parse-fidelity rows (spec §5) ──────────────────────────────────────────────────────

#[test]
fn sa8_duplicate_key_at_any_depth_denies() {
    // Duplicate key inside a nested (inert) object, not just top-level.
    assert_eq!(
        recognize_body(r#"{"model":"m","messages":[{"role":"user","role":"system"}]}"#),
        Err(Denial::DuplicateKey),
    );
}

#[test]
fn sa8_bad_escape_denies() {
    // Bad \u hex in a content string.
    assert_eq!(
        recognize_body(r#"{"model":"m","messages":[{"content":"\uZZZZ"}]}"#),
        Err(Denial::BadEscape),
    );
    // Lone high surrogate.
    assert_eq!(
        recognize_body(r#"{"model":"m","system":"\uD83D"}"#),
        Err(Denial::BadEscape),
    );
}

#[test]
fn sa8_bad_number_denies() {
    for bad in [
        r#"{"max_tokens":+1}"#,       // leading +
        r#"{"max_tokens":01}"#,       // leading zero
        r#"{"max_tokens":NaN}"#,      // NaN
        r#"{"max_tokens":0x1F}"#,     // hex
        r#"{"max_tokens":1.}"#,       // dangling fraction
    ] {
        let got = recognize_body(bad);
        assert!(
            matches!(got, Err(Denial::BadNumber) | Err(Denial::BadJson)),
            "non-canonical number {bad:?} must deny, got {got:?}",
        );
    }
}

#[test]
fn sa8_json5_isms_deny() {
    for bad in [
        r#"{"model":"m",}"#,               // trailing comma
        r#"{"model":"m" /* c */}"#,        // comment
        "{\"model\":\u{00A0}\"m\"}",       // exotic whitespace (non-breaking space)
        r#"{'model':'m'}"#,                 // single quotes
    ] {
        let got = recognize_body(bad);
        assert!(got.is_err(), "JSON5-ism {bad:?} must deny, got {got:?}");
    }
}

#[test]
fn valid_surrogate_pair_in_content_is_admitted() {
    // A well-formed surrogate pair (emoji) in inert content is fine — fail-CLOSED means rejecting
    // ambiguity, not rejecting valid content.
    assert_eq!(
        recognize_body(r#"{"model":"m","system":"hi 😀"}"#),
        Ok(()),
    );
}

#[test]
fn non_object_body_denies() {
    assert!(recognize_body(r#"[1,2,3]"#).is_err());
    assert!(recognize_body(r#""just a string""#).is_err());
    assert!(recognize_body("").is_err());
}
