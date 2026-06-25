//! Seam 5B gates: round-trip over the 3 net verbs through `decode_message`/`encode_message`, the
//! **opaque-concat + decode-once** wiring of net-payload through the reassembler, the net-response
//! **asymmetry** (never reassembled), and the binding observables — a `ReassemblyOverflow` surfacing
//! from `Reassembler::push` (fed the transport's payload chunks + continuation, the L1 framing being
//! below the crate's copy boundary), and an `ErrorBody` reject surfacing from `net-error`.

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
    for bytes in [Vec::new(), vec![0x41], vec![0xDE, 0xAD, 0xBE, 0xEF]] {
        assert_roundtrip(&Message::NetPayload(NetPayload(bytes)));
    }
}

// ----- net-payload: reassembly (opaque concat + decode ONCE), fed push-direct --------------------

/// The reassembly path: a payload spanning 3 transport chunks (the first carries verb 0x33; the
/// transport's continued chunks are raw) is concatenated opaquely, then `decode_message` runs ONCE on
/// the complete buffer → NetPayload(FULL). The transport delivers `(chunk, continuation)` per chunk.
#[test]
fn net_payload_reassembles_across_chunks_then_decodes_once() {
    // The full L2 message that *would* be sent if it fit one frame: [0x33][full payload].
    let full_payload = b"POST /v1/messages {\"model\":\"claude\"} <...big body...>".to_vec();
    let mut whole_l2 = verb::NET_PAYLOAD.to_le_bytes().to_vec();
    whole_l2.extend_from_slice(&full_payload);

    // The transport hands the crate each chunk + its continuation bool — push them directly.
    let cut1 = 10;
    let cut2 = 25;
    let mut r = Reassembler::with_capacity_bound();
    assert!(matches!(
        r.push(&whole_l2[..cut1], true).unwrap(),
        ReassemblyState::Accumulating
    ));
    assert!(matches!(
        r.push(&whole_l2[cut1..cut2], true).unwrap(),
        ReassemblyState::Accumulating
    ));
    let complete = match r.push(&whole_l2[cut2..], false).unwrap() {
        ReassemblyState::Complete(bytes) => bytes,
        other => panic!("expected Complete, got {other:?}"),
    };
    // The reassembled buffer is the WHOLE L2; decode_message runs ONCE over it.
    assert_eq!(complete, whole_l2);
    assert_eq!(
        decode_message(&complete).unwrap(),
        Message::NetPayload(NetPayload(full_payload))
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
