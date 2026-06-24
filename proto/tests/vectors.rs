//! G3 — conformance vectors. **Hand-authored** golden byte sequences, one per decoder kind, named by
//! verb, **frozen** as literals in this file. They are NOT produced by `encode()` — the encoder
//! cannot be both subject and oracle (an encoder-blessed vector is a tautology). Each is authored
//! byte-by-byte from the spec field layout; the test then checks **both** directions:
//! `decode(golden) == expected` AND `encode(expected) == golden`.
//!
//! All vectors are at **zero reserved bits / canonical** form (a reserved-set input is a G2 fuzz
//! case, not a bidirectional vector). Verb ids are provisional until §8 ratification.

use proto::{
    ErrorBody, ErrorCode, FileChunk, FileError, FileRead, Message, NetAddress, PaneState, Scheme,
    decode_message, encode_message, scan_safe, scan_unsafe, verb,
};

/// fixed-record primitive — `pane-state` (verb 0x18).
/// Layout `[verb u32][scrollback u32][viewport u32][input_flags u32]`.
#[test]
fn vector_pane_state() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x18, 0x00, 0x00, 0x00, // verb 0x18 (pane-state), reserved bytes zero
        0x2C, 0x01, 0x00, 0x00, // scrollback_total = 300
        0x00, 0x00, 0x00, 0x00, // viewport_offset  = 0
        0x01, 0x00, 0x00, 0x00, // input_flags = bit0 set (wants_free_motion), reserved bits zero
    ];
    let expected = Message::PaneState(PaneState {
        scrollback_total: 300,
        viewport_offset: 0,
        wants_free_motion: true,
    });
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// verbatim tail — `file-chunk` (verb 0x21).
/// Layout `[verb u32][verbatim bytes]`.
#[test]
fn vector_file_chunk() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x21, 0x00, 0x00, 0x00, // verb 0x21 (file-chunk)
        0x68, 0x69, 0x0A,       // verbatim content: "hi\n"
    ];
    let expected = Message::FileChunk(FileChunk {
        bytes: vec![0x68, 0x69, 0x0A],
    });
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// segment stack (safe) — `net-address` (verb 0x30), scheme https.
/// Layout `[verb u32][scheme u32][safe stack: "example.com" 0x1F "api"]`.
#[test]
fn vector_net_address_https() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x30, 0x00, 0x00, 0x00, // verb 0x30 (net-address)
        0x00, 0x00, 0x00, 0x00, // scheme = 0 (https)
        0x65, 0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x2e, 0x63, 0x6f, 0x6d, // "example.com"
        0x1F,                   // segment separator
        0x61, 0x70, 0x69,       // "api"
    ];
    let expected = Message::NetAddress(NetAddress {
        scheme: Scheme::Https,
        address: scan_safe(b"example.com\x1Fapi").unwrap(),
    });
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// segment stack (unsafe) — `file-read` (verb 0x22).
/// Layout `[verb u32][unsafe stack: "my report.txt"]` (a real name — has a space).
#[test]
fn vector_file_read() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x22, 0x00, 0x00, 0x00, // verb 0x22 (file-read)
        0x6d, 0x79, 0x20, 0x72, 0x65, 0x70, 0x6f, 0x72, 0x74, 0x2e, 0x74, 0x78, 0x74, // "my report.txt"
    ];
    let expected = Message::FileRead(FileRead {
        source: scan_unsafe(b"my report.txt").unwrap(),
    });
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// fixed prefix + verbatim tail — `file-error` (verb 0x27), the shared error body.
/// Layout `[verb u32][code u32][detail tail]`. The detail carries a **non-UTF-8 `0xFF` byte** — the
/// permanent regression guard locking `detail` as verbatim `Vec<u8>` (a change to `String` breaks it).
#[test]
fn vector_file_error_non_utf8_detail() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x27, 0x00, 0x00, 0x00, // verb 0x27 (file-error)
        0x02, 0x00, 0x00, 0x00, // code = 2 (Denied)
        0xFF, 0x6F, 0x6B,       // detail = [0xFF, 'o', 'k'] — non-UTF-8, must survive verbatim
    ];
    let expected = Message::FileError(FileError(ErrorBody {
        code: ErrorCode::Denied,
        detail: vec![0xFF, 0x6F, 0x6B],
    }));
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// carry-raw guard — `net-address` (verb 0x30) with a **reserved** scheme value.
/// `Scheme::Other` is decode-only (enforce-by-construction), so this vector is decode-anchored:
/// the reserved value is carried, read via the getter, and re-encodes byte-identically.
#[test]
fn vector_net_address_reserved_scheme_carry_raw() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x30, 0x00, 0x00, 0x00, // verb 0x30 (net-address)
        0x02, 0x00, 0x00, 0x00, // scheme = 2 (reserved — carried raw, NOT rejected)
        0x68, 0x6f, 0x73, 0x74, // "host"
        0x1F,                   // separator
        0x70,                   // "p"
    ];
    let decoded = decode_message(golden).unwrap();
    match &decoded {
        Message::NetAddress(na) => match na.scheme {
            Scheme::Other(rs) => assert_eq!(rs.get(), 2),
            Scheme::Https => panic!("expected a reserved scheme"),
        },
        _ => panic!("expected NetAddress"),
    }
    assert_eq!(encode_message(&decoded), golden);
}

/// G0 / G1 coverage: the vectors above name every seed verb exactly, anchored to the source `SEED`
/// constant (not a hand-typed count).
#[test]
fn vectors_cover_every_seed_verb() {
    let covered = [
        verb::PANE_STATE,
        verb::FILE_CHUNK,
        verb::NET_ADDRESS,
        verb::FILE_READ,
        verb::FILE_ERROR,
    ];
    assert_eq!(covered.len(), verb::SEED.len());
    for v in verb::SEED {
        assert!(covered.contains(&v), "seed verb {v:#x} has no conformance vector");
    }
}
