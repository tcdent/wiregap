//! U2 address-recognizer certification (Sprint 5, CB1–CB4). Enumeration against an INDEPENDENT
//! oracle: the scheme admit-rule (admit iff scheme==0) and byte-identity, computed from the
//! construction — never read back from `recognize`/`matches`. The per-byte alphabet floor is
//! inherited (proto's `segment_alphabet.rs`); the adversarial "unsayable" rows are proven at the
//! DECODE boundary (proto rejects), since a malformed address never reaches `recognize`.

use proto::{Reject, Wire};
use proxy::address::{AddressDenial, matches, recognize};
use proxy::vocab::{Address, Route};

/// Decode a net-address the real way (scheme u32 LE + safe stack), or return the proto Reject.
fn decode(scheme: u32, joined: &str) -> Result<Address, Reject> {
    let mut body = scheme.to_le_bytes().to_vec();
    body.extend_from_slice(joined.as_bytes());
    Address::decode(&body)
}

/// LEG-A: the scheme domain. `0` admits (yields a canonical form); every reserved value denies
/// with its raw value. Sampled across the u32 range (0 exhaustively-admits; reserved sampled).
#[test]
fn scheme_gate_against_independent_oracle() {
    for scheme in [0u32, 1, 2, 42, 0xFFFF_FFFF] {
        let addr = decode(scheme, "api.anthropic.com\u{1f}v1\u{1f}messages")
            .expect("a valid safe stack decodes regardless of scheme value");
        let got = recognize(&addr);
        // INDEPENDENT ORACLE: admit iff scheme == 0.
        if scheme == 0 {
            assert_eq!(got.as_deref(), Ok("https://api.anthropic.com/v1/messages"));
        } else {
            assert_eq!(
                got,
                Err(AddressDenial::ReservedScheme { value: scheme }),
                "reserved scheme {scheme} must deny, carrying its raw value",
            );
        }
    }
}

/// LEG-A: the byte-exact match boundary. Exact matches; case-shadow, trailing-slash, and a
/// sub/superset each fail — no normalization (`spec/address-recognizer.md` §5, residual RT-1).
#[test]
fn byte_exact_match_against_independent_oracle() {
    let route = Route::new("https://example.com/v1");
    // Independent oracle: match iff the canonical string is byte-identical to the route.
    let cases = [
        ("https://example.com/v1", true),          // exact
        ("https://Example.com/v1", false),         // case shadow (RT-1) — a correct deny
        ("https://example.com/v1/", false),         // trailing slash (not in assemble's image)
        ("https://example.com", false),             // prefix (bare domain)
        ("https://example.com/v1/extra", false),    // superset
        ("https://example.com/v2", false),          // different path
    ];
    for (canonical, oracle) in cases {
        assert_eq!(matches(canonical, &route), oracle, "match({canonical})");
    }
}

/// The recognizer round-trips against the real assembler for a decoded address (CB4 — bind to the
/// real proto stack + the real vocab assemble, not a hand-computed expected).
#[test]
fn recognize_produces_the_real_canonical_form() {
    let addr = decode(0, "docs.rs\u{1f}serde\u{1f}latest").unwrap();
    assert_eq!(recognize(&addr).as_deref(), Ok("https://docs.rs/serde/latest"));
}

/// The inherited floor: the "unsayable" adversarial rows (port/userinfo/fragment/percent/query,
/// traversal) fail at DECODE (proto rejects), so they never reach `recognize`. Proven against the
/// real proto decode — the independent oracle is proto's own alphabet.
#[test]
fn adversarial_shapes_fail_to_decode_and_never_reach_recognize() {
    // Each contains a byte outside the safe alphabet, so scan_safe (at decode) rejects it.
    for joined in [
        "api.anthropic.com:8443",       // port — ':'
        "evil@api.anthropic.com",        // userinfo — '@'
        "api.anthropic.com\u{1f}v1#x",  // fragment — '#'
        "api.anthropic.com\u{1f}%2e%2e", // percent-encoding — '%'
        "api.anthropic.com\u{1f}v1?a=1", // query — '?'
    ] {
        assert!(
            matches!(decode(0, joined), Err(Reject::BadSegmentByte { .. })),
            "adversarial shape {joined:?} must fail to decode (unsayable), never reach recognize",
        );
    }
    // Traversal is its own reject arm.
    assert!(matches!(
        decode(0, "api.anthropic.com\u{1f}.."),
        Err(Reject::TraversalSegment { .. })
    ));
}
