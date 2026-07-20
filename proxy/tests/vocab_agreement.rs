//! U-vocab agreement check (Sprint 5, Slice 2 lightweight gate). Not adversarial closure — the
//! point is that U2's runtime assembly and U5's compiled-in canonical routes agree **byte-for-byte
//! by construction**, and that the address vocabulary is `proto::NetAddress`'s, embedded not
//! redefined. The oracle is the pinned canonical format (`spec/trusted-address.md` §3), written
//! out by hand here — independent of `assemble`'s implementation.

use proto::{Wire, scan_safe};
use proxy::vocab::{Address, Route, Scheme, ToolIdentity, assemble};

/// Build the box's decoded `net-address` the real way — through `proto::NetAddress::decode`
/// (scheme `u32` LE prefix + safe segment stack), the actual wire path.
fn decode_address(scheme: u32, joined_segments: &str) -> Address {
    let mut body = scheme.to_le_bytes().to_vec();
    body.extend_from_slice(joined_segments.as_bytes());
    Address::decode(&body).expect("valid net-address body")
}

/// AGREEMENT: U2's runtime `assemble` of an incoming https address equals the canonical string a
/// U5 register would declare for the same destination — byte-for-byte. This is the whole reason
/// the vocabulary is pinned before the U2‖U5 fork.
#[test]
fn assemble_agrees_with_a_canonical_route() {
    let addr = decode_address(0, "api.anthropic.com\u{1f}v1\u{1f}messages");
    let assembled = assemble(&addr).expect("https address assembles");

    // The canonical route U5 would compile in for the same destination (independent oracle: the
    // §3 format written out by hand, not read from `assemble`).
    let route = Route::new("https://api.anthropic.com/v1/messages");

    assert_eq!(
        assembled.as_str(),
        route.as_str(),
        "U2's runtime assembly must equal U5's compiled-in canonical route, byte-for-byte",
    );
}

/// A bare-domain address (single segment) assembles with no trailing slash — the format has a
/// separator only *between* the domain and each path segment.
#[test]
fn bare_domain_has_no_trailing_slash() {
    let addr = decode_address(0, "example.com");
    assert_eq!(assemble(&addr).as_deref(), Some("https://example.com"));
}

/// A reserved (non-https) scheme has no canonical form — `assemble` returns `None`, leaving the
/// deny-whole-request decision to U2's scheme gate.
#[test]
fn reserved_scheme_has_no_canonical_form() {
    let addr = decode_address(1, "api.anthropic.com\u{1f}v1");
    assert!(matches!(addr.scheme, Scheme::Other(_)));
    assert_eq!(assemble(&addr), None, "a reserved scheme yields no canonical form");
}

/// Injectivity (the §3 property the shared format rests on): two distinct addresses assemble to
/// distinct canonical strings — the separator structure is proxy-supplied and unforgeable in the
/// safe alphabet, so no two distinct segment lists collide.
#[test]
fn distinct_addresses_assemble_distinctly() {
    let a = assemble(&decode_address(0, "host\u{1f}a\u{1f}b")).unwrap();
    let b = assemble(&decode_address(0, "host\u{1f}ab")).unwrap();
    assert_ne!(a, b, "distinct segment lists must not collide (injectivity)");
    assert_eq!(a, "https://host/a/b");
    assert_eq!(b, "https://host/ab");
}

/// The address vocabulary IS `proto::NetAddress`'s, embedded not redefined: a `proto::NetAddress`
/// and a `vocab::Address` are the *same type* (this compiles only if the re-export is an identity,
/// not a wrapper).
#[test]
fn address_is_proto_netaddress_not_a_redefinition() {
    let a: Address = decode_address(0, "h");
    // Assigning to the proto type binding proves they are one type, not two that convert.
    let _same: proto::NetAddress = a;
    // And the scheme vocabulary is proto's Scheme, re-exported.
    let scanned = scan_safe(b"h").unwrap();
    let built = Address {
        scheme: Scheme::Https,
        address: scanned,
    };
    assert!(assemble(&built).is_some());
}

/// The typed newtypes expose their bytes for downstream matching, and are `const`-constructible
/// (so U5's register can be compiled-in).
#[test]
fn newtypes_round_trip_and_are_const() {
    const R: Route = Route::new("https://api.anthropic.com/v1/messages");
    const T: ToolIdentity = ToolIdentity::new("text_editor_20250728");
    assert_eq!(R.as_str(), "https://api.anthropic.com/v1/messages");
    assert_eq!(T.as_str(), "text_editor_20250728");
}
