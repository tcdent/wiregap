//! Seam 1B gates: G1 round-trip over the five seed messages (decode∘encode identity on the full
//! field set + canonical re-encode byte-identical), the carry-raw enum round-trips, the
//! reserved-bit DROP discipline, and dispatch totality (low-byte mask, unknown → `UnsupportedVerb`,
//! known-verb-with-reserved-upper still dispatches). The *dynamic* totality proof (kitted fuzz) is
//! 1C; here the discipline is exercised on representative + edge inputs.

use proto::{
    ErrorBody, ErrorCode, FileChunk, FileError, FileRead, Message, NetAddress, PaneState, Reject,
    Scheme, decode_message, encode_message, scan_safe, scan_unsafe, verb,
};

/// `decode(encode(m)) == m` on the full field set, and `encode(decode(encode(m)))` byte-identical.
fn assert_roundtrip(m: &Message) {
    let wire = encode_message(m);
    let decoded = decode_message(&wire).expect("seed message must decode");
    assert_eq!(&decoded, m, "decode(encode(m)) must equal m");
    assert_eq!(
        encode_message(&decoded),
        wire,
        "encode(decode(encode(m))) must be byte-identical"
    );
}

// ----- pane-state: fixed-record primitive, reserved-bit drop ------------------------------------

#[test]
fn pane_state_roundtrips_field_edges() {
    for m in [
        PaneState {
            scrollback_total: 0,
            viewport_offset: 0,
            wants_free_motion: false,
        },
        PaneState {
            scrollback_total: u32::MAX,
            viewport_offset: u32::MAX,
            wants_free_motion: true,
        },
        PaneState {
            scrollback_total: 1,
            viewport_offset: 7,
            wants_free_motion: true,
        },
    ] {
        assert_roundtrip(&Message::PaneState(m));
    }
}

#[test]
fn pane_state_drops_reserved_input_flag_bits() {
    // A wire pane-state with EVERY input_flags bit set must decode Ok (G2 — no reserved-bit reject),
    // keep only bit 0, and re-encode canonically (bit 0 only, reserved bits zero).
    let mut frame = Vec::new();
    frame.extend_from_slice(&verb::PANE_STATE.to_le_bytes()); // [0x18,0,0,0]
    frame.extend_from_slice(&0x1122_3344u32.to_le_bytes()); // scrollback
    frame.extend_from_slice(&0u32.to_le_bytes()); // viewport
    frame.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // input_flags: all bits set

    let decoded = decode_message(&frame).expect("arbitrary input_flags must decode Ok");
    assert_eq!(
        decoded,
        Message::PaneState(PaneState {
            scrollback_total: 0x1122_3344,
            viewport_offset: 0,
            wants_free_motion: true, // bit 0 kept; bits 1..31 dropped
        })
    );

    // canonical re-encode: input_flags is now exactly 0x0000_0001
    let canonical = encode_message(&decoded);
    assert_eq!(&canonical[12..16], &1u32.to_le_bytes(), "reserved bits zeroed");
    // and the data fields survived the drop
    assert_eq!(&canonical[4..8], &0x1122_3344u32.to_le_bytes());
}

#[test]
fn pane_state_short_and_trailing_are_structural_rejects() {
    let mut short = verb::PANE_STATE.to_le_bytes().to_vec();
    short.extend_from_slice(&[0u8; 11]); // 11 body bytes < 12
    assert_eq!(
        decode_message(&short),
        Err(Reject::ShortBody { needed: 12, got: 11 })
    );

    let mut long = verb::PANE_STATE.to_le_bytes().to_vec();
    long.extend_from_slice(&[0u8; 13]); // 13 body bytes > 12
    assert_eq!(
        decode_message(&long),
        Err(Reject::TrailingBytes { extra: 1 })
    );
}

// ----- file-chunk: verbatim tail (empty / 1B / large, incl. non-UTF-8) --------------------------

#[test]
fn file_chunk_roundtrips_tail_edges() {
    for bytes in [
        Vec::new(),
        vec![0x00],
        (0..=255u8).cycle().take(1000).collect::<Vec<u8>>(),
    ] {
        assert_roundtrip(&Message::FileChunk(FileChunk { bytes }));
    }
}

// ----- file-read: unsafe segment stack (1 / N segments), reject propagation ----------------------

#[test]
fn file_read_roundtrips_segments() {
    for input in [&b"file.txt"[..], &b"dir\x1Fsub\x1Ffile name.txt"[..]] {
        let source = scan_unsafe(input).expect("valid unsafe stack");
        assert_roundtrip(&Message::FileRead(FileRead { source }));
    }
}

#[test]
fn file_read_propagates_segment_reject() {
    // a '/' in the body is a 1A segment reject surfacing through dispatch
    let mut frame = verb::FILE_READ.to_le_bytes().to_vec();
    frame.extend_from_slice(b"a/b");
    assert_eq!(
        decode_message(&frame),
        Err(Reject::BadSegmentByte {
            index: 0,
            offset: 1,
            byte: b'/'
        })
    );
}

// ----- net-address: scheme prefix (carry-raw) + safe stack --------------------------------------

#[test]
fn net_address_roundtrips_https() {
    let address = scan_safe(b"api.example.com\x1Fv1\x1Fmessages").expect("safe stack");
    assert_roundtrip(&Message::NetAddress(NetAddress {
        scheme: Scheme::Https,
        address,
    }));
}

#[test]
fn net_address_carries_raw_reserved_scheme() {
    // `Scheme::Other` is decode-only (private inner, enforce-by-construction) — obtain it from the
    // wire, the only constructor, and confirm it carries the raw value and round-trips.
    for raw in [2u32, u32::MAX] {
        let mut frame = verb::NET_ADDRESS.to_le_bytes().to_vec();
        frame.extend_from_slice(&raw.to_le_bytes());
        frame.extend_from_slice(b"example.com\x1Fapi");
        let m = decode_message(&frame).expect("a reserved scheme carries, never rejects");
        match &m {
            Message::NetAddress(na) => match na.scheme {
                Scheme::Other(rs) => assert_eq!(rs.get(), raw),
                Scheme::Https => panic!("expected a reserved scheme"),
            },
            _ => panic!("expected NetAddress"),
        }
        assert_eq!(encode_message(&m), frame, "carry-raw re-encode is canonical");
        assert_roundtrip(&m);
    }
}

#[test]
fn net_address_short_body_is_structural_reject() {
    let mut frame = verb::NET_ADDRESS.to_le_bytes().to_vec();
    frame.extend_from_slice(&[0u8; 3]); // < 4 scheme bytes
    assert_eq!(
        decode_message(&frame),
        Err(Reject::ShortBody { needed: 4, got: 3 })
    );
}

// ----- file-error: code (carry-raw) + verbatim detail (incl. non-UTF-8 + empty) -----------------

#[test]
fn file_error_roundtrips_known_code_and_verbatim_detail() {
    for body in [
        ErrorBody {
            code: ErrorCode::Denied,
            detail: b"permission denied".to_vec(),
        },
        ErrorBody {
            code: ErrorCode::Malformed,
            detail: vec![0xFF, 0x00, 0xFE], // non-UTF-8 detail survives verbatim
        },
        ErrorBody {
            code: ErrorCode::Unspecified,
            detail: Vec::new(), // empty detail
        },
    ] {
        assert_roundtrip(&Message::FileError(FileError(body)));
    }
}

#[test]
fn file_error_carries_raw_reserved_code() {
    // `ErrorCode::Unknown` is decode-only — obtain it from the wire and confirm carry-raw + verbatim.
    let mut frame = verb::FILE_ERROR.to_le_bytes().to_vec();
    frame.extend_from_slice(&7u32.to_le_bytes()); // reserved code 7 (out of known 0..=6)
    frame.extend_from_slice(&[0xFF, 0x6F, 0x6B]); // non-UTF-8 detail
    let m = decode_message(&frame).expect("a reserved code carries, never rejects");
    match &m {
        Message::FileError(FileError(eb)) => match eb.code {
            ErrorCode::Unknown(rc) => assert_eq!(rc.get(), 7),
            _ => panic!("expected a reserved code"),
        },
        _ => panic!("expected FileError"),
    }
    assert_eq!(encode_message(&m), frame, "carry-raw re-encode is canonical");
    assert_roundtrip(&m);
}

#[test]
fn error_code_known_values_are_canonical_not_unknown() {
    // every defined code round-trips as its named variant, never as Unknown
    for body in [
        ErrorCode::Unspecified,
        ErrorCode::NotFound,
        ErrorCode::Denied,
        ErrorCode::Unavailable,
        ErrorCode::Timeout,
        ErrorCode::Unreachable,
        ErrorCode::Malformed,
    ]
    .map(|code| ErrorBody {
        code,
        detail: Vec::new(),
    }) {
        let m = Message::FileError(FileError(body.clone()));
        let decoded = decode_message(&encode_message(&m)).unwrap();
        assert_eq!(decoded, m);
    }
}

// ----- dispatch totality ------------------------------------------------------------------------

#[test]
fn unknown_low_byte_is_unsupported_verb() {
    // any declared-but-unimplemented catalog verb is a stub → UnsupportedVerb. Derived from the
    // catalog so it never churns as service sprints implement specific ids.
    let stub = verb::CATALOG
        .iter()
        .copied()
        .find(|v| !verb::IMPLEMENTED.contains(v))
        .expect("an unimplemented stub verb exists");
    assert_eq!(
        decode_message(&stub.to_le_bytes()),
        Err(Reject::UnsupportedVerb { verb: stub })
    );
    // a fully unused id
    let frame = 0x99u32.to_le_bytes().to_vec();
    assert_eq!(
        decode_message(&frame),
        Err(Reject::UnsupportedVerb { verb: 0x99 })
    );
}

#[test]
fn known_verb_with_reserved_upper_bytes_still_dispatches() {
    // verb 0xAB00_0018: low byte 0x18 (pane-state), reserved upper bytes nonzero
    let mut frame = 0xAB00_0018u32.to_le_bytes().to_vec();
    frame.extend_from_slice(&[0u8; 12]); // valid pane-state body

    let decoded = decode_message(&frame).expect("reserved-upper must NOT reject a known verb");
    assert!(matches!(decoded, Message::PaneState(_)));

    // re-encode zeros the reserved upper bytes -> canonical verb 0x0000_0018
    let canonical = encode_message(&decoded);
    assert_eq!(&canonical[0..4], &verb::PANE_STATE.to_le_bytes());
}

#[test]
fn frame_shorter_than_verb_is_short_body() {
    assert_eq!(
        decode_message(&[0x18, 0x00, 0x00]),
        Err(Reject::ShortBody { needed: 4, got: 3 })
    );
    assert_eq!(
        decode_message(&[]),
        Err(Reject::ShortBody { needed: 4, got: 0 })
    );
}

// ----- catalog coherence (G0 anchor) ------------------------------------------------------------

#[test]
fn catalog_has_22_distinct_low_byte_ids() {
    assert_eq!(verb::CATALOG.len(), 22, "CATALOG anchors G0 at 22 verbs");
    for (i, &a) in verb::CATALOG.iter().enumerate() {
        assert!(a <= 0xFF, "verb id {a:#x} must fit the low byte");
        for &b in &verb::CATALOG[i + 1..] {
            assert_ne!(a, b, "verb ids must be distinct");
        }
    }
}

#[test]
fn error_codes_are_contiguous_zero_to_max() {
    // the wire-numbers surface for error codes is complete and contiguous 0..=MAX
    let codes = [
        verb::ERR_UNSPECIFIED,
        verb::ERR_NOT_FOUND,
        verb::ERR_DENIED,
        verb::ERR_UNAVAILABLE,
        verb::ERR_TIMEOUT,
        verb::ERR_UNREACHABLE,
        verb::ERR_MALFORMED,
    ];
    for (i, &c) in codes.iter().enumerate() {
        assert_eq!(c, i as u32, "error codes must be contiguous from 0");
    }
    assert_eq!(verb::MAX_ERROR_CODE, 6);
}

#[test]
fn implemented_set_matches_dispatch() {
    // G0 coherence: the source `IMPLEMENTED` constant must equal the set of verbs with a real decode
    // arm. An implemented verb with an empty body decodes to Ok or a *structural* reject (it has an
    // arm); every other catalog verb decodes to UnsupportedVerb (it is a stub).
    assert_eq!(verb::IMPLEMENTED.len(), 21);
    assert!(verb::IMPLEMENTED.iter().all(|s| verb::CATALOG.contains(s)));
    for &v in &verb::CATALOG {
        let result = decode_message(&v.to_le_bytes());
        let is_unsupported = matches!(result, Err(Reject::UnsupportedVerb { .. }));
        assert_eq!(
            verb::IMPLEMENTED.contains(&v),
            !is_unsupported,
            "verb {v:#x}: IMPLEMENTED membership must match having a decode arm"
        );
    }
}
