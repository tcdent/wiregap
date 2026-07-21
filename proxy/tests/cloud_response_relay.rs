//! Sprint 6 S3 — cloud response relay certification (MB1-MB6). MB2 byte-identity (output == input,
//! verbatim), MB3 no-inspection (the SAME wrap for any bytes — control/escape bytes relay untouched,
//! proving no neutralization on the free direction), MB5 robustness (any chunk relays + round-trips,
//! no panic). MB6 pure safe Rust.

use proto::{ErrorBody, ErrorCode, Message, NetError, NetResponse, decode_message};
use proxy::cloud_relay::{relay_error, relay_response};

/// MB2: the relayed frame decodes back to EXACTLY the input chunk — verbatim, byte-identical.
#[test]
fn response_is_relayed_byte_identical() {
    for chunk in [
        b"".as_slice(),
        b"a plain body",
        b"\x00\x01\x1b[2J binary + ESC + control \x07\xff\xfe",
        b"{\"upstream\":\"json\"}",
    ] {
        let framed = relay_response(chunk);
        match decode_message(&framed).unwrap() {
            Message::NetResponse(NetResponse(body)) => {
                assert_eq!(body, chunk, "the relayed body must equal the input, byte for byte");
            }
            other => panic!("expected NetResponse, got {other:?}"),
        }
    }
}

/// MB3: content is NOT inspected or neutralized. A body carrying a terminal ESC sequence (the
/// classic "sanitize the response" temptation) relays UNTOUCHED — hardening it would re-import the
/// confidentiality worldview the spine discards.
#[test]
fn escape_sequences_relay_untouched() {
    let malicious_looking = b"\x1b]0;pwned\x07\x1b[31mred\x1b[0m";
    let framed = relay_response(malicious_looking);
    match decode_message(&framed).unwrap() {
        Message::NetResponse(NetResponse(body)) => assert_eq!(body, malicious_looking),
        other => panic!("got {other:?}"),
    }
}

/// A net-error carries the proxy's own category, not box content.
#[test]
fn net_error_relays_the_proxy_category() {
    let framed = relay_error(ErrorBody { code: ErrorCode::Timeout, detail: Vec::new() });
    match decode_message(&framed).unwrap() {
        Message::NetError(NetError(body)) => {
            assert_eq!(body.code, ErrorCode::Timeout);
            assert!(body.detail.is_empty());
        }
        other => panic!("got {other:?}"),
    }
}

/// MB5: any chunk relays and round-trips without panic (deterministic fuzz).
#[test]
fn fuzz_any_chunk_relays_and_round_trips() {
    struct Rng(u64);
    impl Rng { fn next(&mut self) -> u64 { let mut x=self.0; x^=x<<13; x^=x>>7; x^=x<<17; self.0=x; x } }
    let mut rng = Rng(0xBEEF_F00D_1122_3344);
    for _ in 0..20_000 {
        let len = (rng.next() % 512) as usize;
        let chunk: Vec<u8> = (0..len).map(|_| (rng.next() & 0xff) as u8).collect();
        let framed = relay_response(&chunk);
        match decode_message(&framed).unwrap() {
            Message::NetResponse(NetResponse(body)) => assert_eq!(body, chunk),
            other => panic!("got {other:?}"),
        }
    }
}
