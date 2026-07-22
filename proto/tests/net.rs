//! Seam 5B gates: round-trip over the 3 net verbs through `decode_message`/`encode_message`, the
//! **per-chunk decode-then-concat** wiring of net-payload through the reassembler (each chunk is its
//! own `[verb][flag][content]` message; the proxy decodes each and pushes its content with the header's
//! `more` flag — net.md §2), the net-response **asymmetry** (never reassembled), and the binding
//! observables — a `ReassemblyOverflow` surfacing from `Reassembler::push` on cap-exceed, and an
//! `ErrorBody` reject surfacing from `net-error`.

use proto::{
    ErrorBody, ErrorCode, Message, NetError, NetPayload, NetResponse, Reassembler, ReassemblyState,
    Reject, decode_message, encode_message, verb,
};

fn assert_roundtrip(m: &Message) {
    let wire = encode_message(m);
    let decoded = decode_message(&wire).expect("must decode");
    assert_eq!(&decoded, m, "decode(encode(m)) == m");
    assert_eq!(encode_message(&decoded), wire, "canonical re-encode byte-identical");
}

#[test]
fn net_response_roundtrips_verbatim() {
    for bytes in [Vec::new(), vec![0x7B, 0x7D], vec![0xFF, 0x00, 0xFE]] {
        assert_roundtrip(&Message::NetResponse(NetResponse(bytes)));
    }
}

#[test]
fn net_error_roundtrips_through_errorbody() {
    assert_roundtrip(&Message::NetError(NetError(ErrorBody {
        code: ErrorCode::Unavailable,
        detail: b"upstream 503".to_vec(),
    })));
}

#[test]
fn net_payload_roundtrips_verbatim() {
    for more in [true, false] {
        for content in [Vec::new(), vec![0x41], vec![0xDE, 0xAD, 0xBE, 0xEF]] {
            assert_roundtrip(&Message::NetPayload(NetPayload { more, content }));
        }
    }
}

// ----- net-payload: per-chunk reassembly (decode each chunk, concat its content) -----------------

/// Decode a `net-payload` chunk message (`[0x33][flag][content]`) to its `NetPayload`.
fn decode_net_payload(wire: &[u8]) -> NetPayload {
    match decode_message(wire).unwrap() {
        Message::NetPayload(p) => p,
        other => panic!("expected NetPayload, got {other:?}"),
    }
}

/// The reassembly path (Sprint 7, per-chunk header flag): a request body spanning 3 net-payload
/// chunks. Each chunk is its **own** `[0x33][flag][content-piece]` message; the proxy decodes each to
/// `NetPayload { more, content }` and pushes the **content** to the reassembler with `more`. The
/// reassembler concatenates the per-chunk *content* (not raw L2 bytes); the final chunk (`more=false`)
/// yields the complete request **body** — the buffer the recognizer vets.
#[test]
fn net_payload_reassembles_content_across_chunks() {
    let full_body = b"POST /v1/messages {\"model\":\"claude\"} <...big body...>".to_vec();
    let cut1 = 10;
    let cut2 = 25;
    let chunks = [
        NetPayload { more: true, content: full_body[..cut1].to_vec() },
        NetPayload { more: true, content: full_body[cut1..cut2].to_vec() },
        NetPayload { more: false, content: full_body[cut2..].to_vec() },
    ];

    let mut r = Reassembler::with_capacity_bound();
    // First two chunks accumulate; each goes through the real encode→decode wire path.
    for chunk in &chunks[..2] {
        let wire = encode_message(&Message::NetPayload(chunk.clone()));
        let p = decode_net_payload(&wire);
        assert_eq!(r.push(&p.content, p.more).unwrap(), ReassemblyState::Accumulating);
    }
    // Final chunk completes → the joined content is the full request body.
    let wire = encode_message(&Message::NetPayload(chunks[2].clone()));
    let p = decode_net_payload(&wire);
    let complete = match r.push(&p.content, p.more).unwrap() {
        ReassemblyState::Complete(bytes) => bytes,
        other => panic!("expected Complete, got {other:?}"),
    };
    assert_eq!(complete, full_body, "the reassembled buffer is the full request body");
}

/// **Collision-freedom — the sprint's load-bearing soundness property.** Content whose bytes equal
/// BOTH flag values (`0x00` = more, `0x01` = final), *including at position 0*, must round-trip and
/// reassemble byte-identically. The flag is a fixed-width positional header field, never a value
/// searched for in content (net.md §2, `model.md` law 11); a regression to content-scanning would read
/// one of these interior/leading bytes as a boundary and fail here. This is the permanent guard for the
/// fork resolved during planning.
#[test]
fn content_equal_to_flag_values_stays_positional() {
    // Round-trip: content leads with 0x00 and carries both flag values, under either header flag.
    let collision = vec![0x00, 0x01, 0x00];
    for more in [true, false] {
        assert_roundtrip(&Message::NetPayload(NetPayload { more, content: collision.clone() }));
    }

    // Reassembly: every chunk's content is packed with flag-valued bytes; the joined body must be the
    // exact concatenation, proving each 1-byte header flag is stripped positionally and no content byte
    // is ever mistaken for the boundary.
    let full_body = vec![0x00, 0x01, 0x00, 0xFF, 0x01, 0x00, 0x00];
    let chunks = [
        NetPayload { more: true, content: vec![0x00, 0x01, 0x00] },
        NetPayload { more: true, content: vec![0xFF, 0x01] },
        NetPayload { more: false, content: vec![0x00, 0x00] },
    ];
    let mut r = Reassembler::with_capacity_bound();
    for chunk in &chunks[..2] {
        let p = decode_net_payload(&encode_message(&Message::NetPayload(chunk.clone())));
        assert_eq!(r.push(&p.content, p.more).unwrap(), ReassemblyState::Accumulating);
    }
    let p = decode_net_payload(&encode_message(&Message::NetPayload(chunks[2].clone())));
    let complete = match r.push(&p.content, p.more).unwrap() {
        ReassemblyState::Complete(bytes) => bytes,
        other => panic!("expected Complete, got {other:?}"),
    };
    assert_eq!(complete, full_body, "flag-valued content bytes survive reassembly byte-identically");
}

/// Header-decode totality, negative arm: an EMPTY `net-payload` body (0 bytes — no flag) is a typed
/// `Reject::ShortBody`, never a panic. `take_prefix::<1>` is total on empty, but this exercises the
/// property at `NetPayload`'s own boundary (one of the reviewer's four named corpus areas).
#[test]
fn net_payload_empty_body_is_shortbody() {
    // A frame with the verb and no body → NetPayload::decode(&[]) → ShortBody{needed:1, got:0}.
    let wire = verb::NET_PAYLOAD.to_le_bytes().to_vec();
    assert_eq!(
        decode_message(&wire),
        Err(Reject::ShortBody { needed: 1, got: 0 })
    );
}

/// Total flag mapping made visible: `0x00` → `more = true`, and **any** nonzero byte (here `0x42`, not
/// just the canonical `0x01`) → `more = false`. Confirms decode reads "is-zero", not "equals-0x01".
#[test]
fn net_payload_any_nonzero_flag_is_final() {
    let mut wire = verb::NET_PAYLOAD.to_le_bytes().to_vec();
    wire.push(0x42); // arbitrary nonzero flag
    wire.extend_from_slice(b"body");
    assert_eq!(
        decode_message(&wire).unwrap(),
        Message::NetPayload(NetPayload { more: false, content: b"body".to_vec() })
    );
    // 0x00 is the sole "more" encoding.
    let mut wire0 = verb::NET_PAYLOAD.to_le_bytes().to_vec();
    wire0.push(0x00);
    wire0.extend_from_slice(b"body");
    assert_eq!(
        decode_message(&wire0).unwrap(),
        Message::NetPayload(NetPayload { more: true, content: b"body".to_vec() })
    );
}

/// Binding observable: a `ReassemblyOverflow` surfacing from `Reassembler::push` on cap-exceed — the
/// reassembler's fail-closed proof. The transport's chunks are pushed directly; a small cap makes it
/// reachable.
#[test]
fn reassembly_overflow_surfaces_from_push() {
    let mut r = Reassembler::new(8); // small reachable cap
    assert!(matches!(
        r.push(&[0u8; 6], true).unwrap(),
        ReassemblyState::Accumulating
    ));
    assert_eq!(
        r.push(&[0u8; 6], true), // 6 + 6 = 12 > 8
        Err(Reject::ReassemblyOverflow { cap: 8 })
    );
}

/// The asymmetry: net-response is a plain verbatim message — it never touches the reassembler. A
/// single 0x31 frame decodes directly (the response is relayed per-frame; the box reassembles).
#[test]
fn net_response_is_not_reassembled() {
    let mut wire = verb::NET_RESPONSE.to_le_bytes().to_vec();
    wire.extend_from_slice(b"chunk-of-streamed-response");
    assert_eq!(
        decode_message(&wire).unwrap(),
        Message::NetResponse(NetResponse(b"chunk-of-streamed-response".to_vec()))
    );
}

/// Binding observable: an `ErrorBody` reject surfacing from `net-error` via `decode_message` proves
/// the reuse-through `ErrorBody::decode_body` (not a re-implementation).
#[test]
fn net_error_routes_through_errorbody_reject() {
    // ErrorBody requires a 4-byte code; a 3-byte body → ShortBody from ErrorBody::decode_body.
    let mut wire = verb::NET_ERROR.to_le_bytes().to_vec();
    wire.extend_from_slice(&[0u8; 3]);
    assert_eq!(
        decode_message(&wire),
        Err(Reject::ShortBody { needed: 4, got: 3 })
    );
}
