//! G3 — conformance vectors. **Hand-authored** golden byte sequences, one per decoder kind, named by
//! verb, **frozen** as literals in this file. They are NOT produced by `encode()` — the encoder
//! cannot be both subject and oracle (an encoder-blessed vector is a tautology). Each is authored
//! byte-by-byte from the spec field layout; the test then checks **both** directions:
//! `decode(golden) == expected` AND `encode(expected) == golden`.
//!
//! All vectors are at **zero reserved bits / canonical** form (a reserved-set input is a G2 fuzz
//! case, not a bidirectional vector). Verb ids are provisional until §8 ratification.

use proto::{
    Button, Cell, CreatePty, Dimensions, Display, ErrorBody, ErrorCode, FileChunk, FileData,
    FileError, FileRead, FileType, FileWrite, Focus, Key, KeyKind, List, Message, Modifiers,
    MouseKind, NetAddress, NetError, NetPayload, NetResponse, OpenSession, PaneState, Paste, Reject,
    Reassembler,
    ReassemblyState, RequestFile, Resize, Rgb, RowCol, Scalar, Scheme, Viewport, Width, decode_frame,
    decode_message, encode_frame, encode_message, scan_safe, scan_unsafe, verb,
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

/// repeated-fixed-record tail — `display` (verb 0x11): a valid multi-op cell stream.
/// Layout `[verb u32][cell u32]*`, each cell `op<<24 | payload`; the op is the **high-order LE byte**.
#[test]
fn vector_display_cell_stream() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x11, 0x00, 0x00, 0x00, // verb 0x11 (display)
        0x00, 0x88, 0xFF, 0x02, // cell 0x02FF8800 — set-fg 0xFF8800
        0x11, 0x30, 0x00, 0x01, // cell 0x01003011 — goto row=3 col=17 (3<<12 | 17)
        0x41, 0x00, 0x20, 0x00, // cell 0x00200041 — draw scalar 'A'(0x41) width 1 (1<<21 | 0x41)
    ];
    let expected = Message::Display(Display(vec![
        Cell::SetFg(Rgb::new(0xFF_8800).unwrap()),
        Cell::Goto(RowCol::new(3, 17).unwrap()),
        Cell::Draw {
            scalar: Scalar::new(0x41).unwrap(),
            width: Width::new(1).unwrap(),
        },
    ]));
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// partial-tail reject — `display` (0x11) with a body length not a multiple of 4.
/// Decode-only: a 2-byte trailing fragment past zero whole records → `PartialRecord`.
#[test]
fn vector_display_partial_tail_reject() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x11, 0x00, 0x00, 0x00, // verb 0x11 (display)
        0xAB, 0xCD,             // 2 trailing bytes — not a whole 4-byte cell
    ];
    assert_eq!(
        decode_message(golden),
        Err(Reject::PartialRecord {
            record_len: 4,
            remainder: 2,
        })
    );
}

/// reserved-op carry/ignore — `display` (0x11) with a cell whose op is `≥ 0x09`.
/// Decode-anchored (`Cell::Reserved` has no public constructor); carried raw + re-encodes identically.
#[test]
fn vector_display_reserved_op_carry() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x11, 0x00, 0x00, 0x00, // verb 0x11 (display)
        0xEF, 0xCD, 0xAB, 0x09, // cell 0x09ABCDEF — op 0x09 (reserved), payload 0xABCDEF
    ];
    let decoded = decode_message(golden).unwrap();
    match &decoded {
        Message::Display(d) => match d.0.as_slice() {
            [Cell::Reserved(rc)] => assert_eq!(rc.get(), 0x09AB_CDEF),
            other => panic!("expected one Reserved cell, got {other:?}"),
        },
        _ => panic!("expected Display"),
    }
    assert_eq!(encode_message(&decoded), golden);
}

/// fixed-only body — `create-pty` (verb 0x10). Layout `[verb u32][rows u16][cols u16]`.
#[test]
fn vector_create_pty() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x10, 0x00, 0x00, 0x00, // verb 0x10 (create-pty)
        0x18, 0x00,             // rows = 24
        0x50, 0x00,             // cols = 80
    ];
    let expected = Message::CreatePty(CreatePty(Dimensions { rows: 24, cols: 80 }));
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// repeated combined-record — `list-entry` (verb 0x25): a valid 2-record listing.
/// Record layout `[size u64][mtime_secs u64][mtime_nanos u32][type_flags u32][name][0x1F]` (all LE).
#[test]
fn vector_list_entry_listing() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x25, 0x00, 0x00, 0x00, // verb 0x25 (list-entry)
        // record 0 — size 100, mtime 0, type_flags 1 (Dir), name "docs"
        0x64, 0, 0, 0, 0, 0, 0, 0, // size = 100
        0, 0, 0, 0, 0, 0, 0, 0,    // mtime_secs = 0
        0, 0, 0, 0,                // mtime_nanos = 0
        0x01, 0, 0, 0,             // type_flags = 1 (Dir)
        0x64, 0x6F, 0x63, 0x73,    // "docs"
        0x1F,                      // terminator
        // record 1 — size 42, type_flags 0 (File), name "a.txt"
        0x2A, 0, 0, 0, 0, 0, 0, 0, // size = 42
        0, 0, 0, 0, 0, 0, 0, 0,    // mtime_secs = 0
        0, 0, 0, 0,                // mtime_nanos = 0
        0x00, 0, 0, 0,             // type_flags = 0 (File)
        0x61, 0x2E, 0x74, 0x78, 0x74, // "a.txt"
        0x1F,                      // terminator
    ];
    let decoded = decode_message(golden).unwrap();
    match &decoded {
        Message::ListEntries(le) => {
            assert_eq!(le.0.len(), 2);
            assert_eq!(le.0[0].size, 100);
            assert_eq!(le.0[0].file_type, FileType::Dir);
            assert_eq!(le.0[0].name.as_bytes(), b"docs");
            assert_eq!(le.0[1].size, 42);
            assert_eq!(le.0[1].file_type, FileType::File);
            assert_eq!(le.0[1].name.as_bytes(), b"a.txt");
        }
        _ => panic!("expected ListEntries"),
    }
    // encode direction: re-encode of the decoded message reproduces the hand-authored golden.
    assert_eq!(encode_message(&decoded), golden);
}

/// terminator-semantics regression guard — `list-entry` (0x25) with a name lacking its `0x1F`.
#[test]
fn vector_list_entry_unterminated_reject() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x25, 0x00, 0x00, 0x00, // verb 0x25
        0, 0, 0, 0, 0, 0, 0, 0,  // size
        0, 0, 0, 0, 0, 0, 0, 0,  // mtime_secs
        0, 0, 0, 0,              // mtime_nanos
        0, 0, 0, 0,              // type_flags
        0x6E, 0x61, 0x6D, 0x65,  // "name" — NO trailing 0x1F before frame end
    ];
    assert_eq!(
        decode_message(golden),
        Err(Reject::UnterminatedRecord { offset: 24 })
    );
}

/// truncated-metadata reject — `list-entry` (0x25), a started record with < 24 metadata bytes.
#[test]
fn vector_list_entry_truncated_metadata_reject() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x25, 0x00, 0x00, 0x00, // verb 0x25
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 20 bytes (< 24)
    ];
    assert_eq!(
        decode_message(golden),
        Err(Reject::ShortBody { needed: 24, got: 20 })
    );
}

/// segment stack (safe) — `file-write` (verb 0x20). Layout `[verb u32][safe stack]`.
#[test]
fn vector_file_write() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x20, 0x00, 0x00, 0x00, // verb 0x20 (file-write)
        0x71, 0x75, 0x61, 0x72, 0x61, 0x6E, 0x74, 0x69, 0x6E, 0x65, // "quarantine"
        0x1F,                   // separator
        0x6F, 0x75, 0x74, 0x2E, 0x62, 0x69, 0x6E, // "out.bin"
    ];
    let expected = Message::FileWrite(FileWrite(scan_safe(b"quarantine\x1Fout.bin").unwrap()));
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// segment stack (unsafe) — `list` (verb 0x24). Layout `[verb u32][unsafe stack]`.
#[test]
fn vector_list() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x24, 0x00, 0x00, 0x00, // verb 0x24 (list)
        0x6D, 0x79, 0x20, 0x64, 0x6F, 0x63, 0x73, // "my docs"
        0x1F,                   // separator
        0x72, 0x65, 0x70, 0x6F, 0x72, 0x74, 0x73, // "reports"
    ];
    let expected = Message::List(List(scan_unsafe(b"my docs\x1Freports").unwrap()));
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// verbatim tail — `file-data` (verb 0x23). Layout `[verb u32][verbatim bytes]`.
#[test]
fn vector_file_data() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x23, 0x00, 0x00, 0x00, // verb 0x23 (file-data)
        0xDE, 0xAD, 0xBE, 0xEF, // verbatim read bytes (non-UTF-8)
    ];
    let expected = Message::FileData(FileData(vec![0xDE, 0xAD, 0xBE, 0xEF]));
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// segment stack (unsafe) — `request-file` (verb 0x26). Layout `[verb u32][unsafe stack]`.
#[test]
fn vector_request_file() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x26, 0x00, 0x00, 0x00, // verb 0x26 (request-file)
        0x72, 0x65, 0x70, 0x6F, 0x72, 0x74, 0x20, 0x66, 0x69, 0x6E, 0x61, 0x6C, 0x2E, 0x70, 0x64, 0x66, // "report final.pdf"
    ];
    let expected = Message::RequestFile(RequestFile(scan_unsafe(b"report final.pdf").unwrap()));
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// G0 / G3 coverage (re-tightened, sprint close): the vectors name **every implemented verb exactly**,
/// anchored bidirectionally to the source `IMPLEMENTED` constant (12) — fails if any implemented verb
/// lacks a vector, or a vector names an unimplemented verb.
/// signed POD + carry-raw + reserved-drop — `mouse` (verb 0x15), the headline. Decode-anchored where
/// the reserved-bit drop applies: the **input** golden carries reserved mods (`0xFFFF`); decode drops
/// them; encode produces the **canonical** golden (`0x003F`). Both byte-arrays are hand-authored, so
/// the bidirectional check is non-circular. Carries a **negative delta** (the i16 sign-bit guard).
#[test]
fn vector_mouse_negative_delta_other_and_reserved_mods() {
    #[rustfmt::skip]
    let input: &[u8] = &[
        0x15, 0x00, 0x00, 0x00, // verb 0x15 (mouse)
        0xC8,                   // button = 200 (Other, > MAX_BUTTON)
        0x63,                   // kind   = 99  (Other, > MAX_MOUSE_KIND)
        0xFF, 0xFF,             // mods   = 0xFFFF (all bits incl. reserved 6-15)
        0x0A, 0x00,             // row    = 10
        0x14, 0x00,             // col    = 20
        0xFF, 0xFF,             // dx     = -1     (i16 LE)
        0x00, 0x80,             // dy     = i16::MIN
    ];
    #[rustfmt::skip]
    let canonical: &[u8] = &[
        0x15, 0x00, 0x00, 0x00,
        0xC8, 0x63,
        0x3F, 0x00,             // mods canonical = 0x003F (reserved bits dropped)
        0x0A, 0x00, 0x14, 0x00,
        0xFF, 0xFF, 0x00, 0x80,
    ];
    let m = decode_message(input).unwrap();
    match &m {
        Message::Mouse(mo) => {
            assert!(matches!(mo.button, Button::Other(b) if b.get() == 200));
            assert!(matches!(mo.kind, MouseKind::Other(k) if k.get() == 99));
            assert_eq!(
                mo.mods,
                Modifiers { shift: true, control: true, alt: true, super_: true, caps_lock: true, fn_: true }
            );
            assert_eq!(mo.dx, -1);
            assert_eq!(mo.dy, i16::MIN);
        }
        _ => panic!("expected Mouse"),
    }
    // encode drops the reserved mod bits -> the canonical golden; the canonical decodes to the same.
    assert_eq!(encode_message(&m), canonical);
    assert_eq!(decode_message(canonical).unwrap(), m);
}

/// verbatim tail (non-UTF-8) — `paste` (verb 0x16). Locks `Vec<u8>` (a `String` would reject this).
#[test]
fn vector_paste_non_utf8() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x16, 0x00, 0x00, 0x00, // verb 0x16 (paste)
        0xFF, 0x00, 0x68, 0x69, // [0xFF, NUL, 'h', 'i'] — non-UTF-8 clipboard bytes
    ];
    let expected = Message::Paste(Paste(vec![0xFF, 0x00, 0x68, 0x69]));
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// Dimensions reuse — `resize` (verb 0x13). Layout `[verb u32][rows u16][cols u16]`.
#[test]
fn vector_resize() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x13, 0x00, 0x00, 0x00, // verb 0x13 (resize)
        0x18, 0x00,             // rows = 24
        0x50, 0x00,             // cols = 80
    ];
    let expected = Message::Resize(Resize(Dimensions { rows: 24, cols: 80 }));
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// fixed u32 — `viewport` (verb 0x14). Layout `[verb u32][offset u32]`.
#[test]
fn vector_viewport() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x14, 0x00, 0x00, 0x00, // verb 0x14 (viewport)
        0x64, 0x00, 0x00, 0x00, // offset = 100
    ];
    let expected = Message::Viewport(Viewport { offset: 100 });
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// raw u8 — `focus` (verb 0x17). Layout `[verb u32][gained u8]`.
#[test]
fn vector_focus() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x17, 0x00, 0x00, 0x00, // verb 0x17 (focus)
        0x01,                   // gained = 1
    ];
    let expected = Message::Focus(Focus { gained: 1 });
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// input reject vectors: a mouse short-body, a viewport trailing-bytes, a focus short-body.
#[test]
fn vector_input_rejects() {
    // mouse 11B (< 12)
    let mut m = verb::MOUSE.to_le_bytes().to_vec();
    m.extend_from_slice(&[0u8; 11]);
    assert_eq!(decode_message(&m), Err(Reject::ShortBody { needed: 12, got: 11 }));
    // viewport 5B (> 4)
    let mut v = verb::VIEWPORT.to_le_bytes().to_vec();
    v.extend_from_slice(&[0u8; 5]);
    assert_eq!(decode_message(&v), Err(Reject::TrailingBytes { extra: 1 }));
    // focus 0B (< 1)
    let f = verb::FOCUS.to_le_bytes().to_vec();
    assert_eq!(decode_message(&f), Err(Reject::ShortBody { needed: 1, got: 0 }));
}

/// verbatim tail — `net-response` (verb 0x31). Layout `[verb u32][response bytes]`. NOT reassembled.
#[test]
fn vector_net_response() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x31, 0x00, 0x00, 0x00, // verb 0x31 (net-response)
        0x7B, 0x22, 0x6F, 0x6B, 0x22, 0x7D, // {"ok"}
    ];
    let expected = Message::NetResponse(NetResponse(vec![0x7B, 0x22, 0x6F, 0x6B, 0x22, 0x7D]));
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// ErrorBody reuse — `net-error` (verb 0x32). Layout `[verb u32][code u32][detail tail]`.
#[test]
fn vector_net_error() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x32, 0x00, 0x00, 0x00, // verb 0x32 (net-error)
        0x05, 0x00, 0x00, 0x00, // code 5 = unreachable
        0x6E, 0x6F, 0x20, 0x72, 0x6F, 0x75, 0x74, 0x65, // "no route"
    ];
    let expected = Message::NetError(NetError(ErrorBody {
        code: ErrorCode::Unreachable,
        detail: b"no route".to_vec(),
    }));
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// verbatim tail (the COMPLETE reassembled buffer) — `net-payload` (verb 0x33). Single-frame here;
/// `[verb u32][full payload]`. (The multi-frame join is `vector_net_payload_reassembles_n_frames`.)
#[test]
fn vector_net_payload() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x33, 0x00, 0x00, 0x00, // verb 0x33 (net-payload)
        0x50, 0x4F, 0x53, 0x54, // "POST"
    ];
    let expected = Message::NetPayload(NetPayload(vec![0x50, 0x4F, 0x53, 0x54]));
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// The first **frame-sequence** golden — a payload spanning N frames reassembles to the joined bytes,
/// which decode ONCE to `NetPayload(full)`. Authored byte-by-byte: the L1 envelope `[session u32]
/// [flags u32: bit0]` per frame + the opaque join (`0x33` rides frame 1 only; continuations are raw).
#[test]
fn vector_net_payload_reassembles_n_frames() {
    // session 9; frame 1 continuation=1, frame 2 continuation=1, frame 3 continuation=0.
    #[rustfmt::skip]
    let frame1: &[u8] = &[
        0x09, 0x00, 0x00, 0x00, // session_id 9
        0x01, 0x00, 0x00, 0x00, // flags: bit0 = continuation
        0x33, 0x00, 0x00, 0x00, // L2 verb 0x33 (net-payload) — frame 1 only
        0x50, 0x4F,             // "PO"
    ];
    #[rustfmt::skip]
    let frame2: &[u8] = &[
        0x09, 0x00, 0x00, 0x00, // session_id 9
        0x01, 0x00, 0x00, 0x00, // flags: continuation
        0x53, 0x54,             // "ST" — RAW payload (no verb)
    ];
    #[rustfmt::skip]
    let frame3: &[u8] = &[
        0x09, 0x00, 0x00, 0x00, // session_id 9
        0x00, 0x00, 0x00, 0x00, // flags: clear (last frame)
        0x21,                   // "!" — RAW payload
    ];
    let mut r = Reassembler::new(64);
    assert_eq!(r.push(decode_frame(frame1).unwrap().l2, true).unwrap(), ReassemblyState::Accumulating);
    assert_eq!(r.push(decode_frame(frame2).unwrap().l2, true).unwrap(), ReassemblyState::Accumulating);
    let complete = match r.push(decode_frame(frame3).unwrap().l2, false).unwrap() {
        ReassemblyState::Complete(b) => b,
        other => panic!("{other:?}"),
    };
    // joined = [0x33][POST!] -> decode_message ONCE -> NetPayload("POST!")
    assert_eq!(complete, vec![0x33, 0x00, 0x00, 0x00, 0x50, 0x4F, 0x53, 0x54, 0x21]);
    assert_eq!(
        decode_message(&complete).unwrap(),
        Message::NetPayload(NetPayload(b"POST!".to_vec()))
    );
}

/// Fail-closed regression guard — a CAP-EXCEED reject (small test-cap). Two frames sum past the cap →
/// `ReassemblyOverflow` (the permanent guard; the boundary is value-independent of the production cap).
#[test]
fn vector_reassembly_cap_exceed_rejects() {
    const CAP: usize = 8;
    let f1 = {
        let mut o = Vec::new();
        encode_frame(1, true, &[0u8; 6], &mut o);
        o
    };
    let f2 = {
        let mut o = Vec::new();
        encode_frame(1, true, &[0u8; 6], &mut o); // 6 + 6 = 12 > 8
        o
    };
    let mut r = Reassembler::new(CAP);
    assert_eq!(r.push(decode_frame(&f1).unwrap().l2, true).unwrap(), ReassemblyState::Accumulating);
    assert_eq!(
        r.push(decode_frame(&f2).unwrap().l2, true),
        Err(Reject::ReassemblyOverflow { cap: CAP })
    );
}

/// Fail-closed regression guard — a NEVER-ENDING sequence (continuation always set) trips the cap and
/// is denied; it does not hang awaiting a clear bit.
#[test]
fn vector_reassembly_never_ending_rejects() {
    const CAP: usize = 8;
    let f = {
        let mut o = Vec::new();
        encode_frame(1, true, &[0u8; 4], &mut o); // continuation always set
        o
    };
    let mut r = Reassembler::new(CAP);
    let mut count = 0;
    loop {
        match r.push(decode_frame(&f).unwrap().l2, true) {
            Ok(ReassemblyState::Accumulating) => {
                count += 1;
                assert!(count < 100, "must trip the cap, not loop forever");
            }
            Err(Reject::ReassemblyOverflow { cap }) => {
                assert_eq!(cap, CAP);
                break;
            }
            other => panic!("{other:?}"),
        }
    }
}

/// control empty-body — `open-session` (verb 0x00). The frame is just `[verb 0x00]` (no body).
#[test]
fn vector_open_session() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x00, 0x00, 0x00, 0x00, // verb 0x00 (open-session) — empty body
    ];
    let expected = Message::OpenSession(OpenSession);
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
    // live-path route-through: a non-empty 0x00 body → TrailingBytes from decode_message — surfaces
    // only because 0x00 dispatches to OpenSession::decode.
    assert_eq!(
        decode_message(&[0x00, 0x00, 0x00, 0x00, 0xFF]),
        Err(Reject::TrailingBytes { extra: 1 })
    );
}

/// control empty-body — `open-session` placeholder kept above; the **`key`** vectors (0x12, the final
/// verb) follow, then the coverage gate.
///
/// raw-u32 keysym + reuse + carry-raw + verbatim — `key` (verb 0x12). The canonical bidirectional
/// vector: keysym above the `0x10FFFF` threshold (a named key), `kind` Press, defined mods, rsv 0,
/// text "A".
#[test]
fn vector_key() {
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x12, 0x00, 0x00, 0x00, // verb 0x12 (key)
        0x00, 0x00, 0x11, 0x00, // KeySym 0x110000 (a named functional key, > 0x10FFFF)
        0x01, 0x00,             // mods = Shift
        0x00,                   // kind = Press
        0x00,                   // reserved
        0x41,                   // text "A"
    ];
    let expected = Message::Key(Key {
        keysym: 0x0011_0000,
        mods: Modifiers { shift: true, ..Modifiers::default() },
        kind: KeyKind::Press,
        text: b"A".to_vec(),
    });
    assert_eq!(decode_message(golden).unwrap(), expected);
    assert_eq!(encode_message(&expected), golden);
}

/// Decode-anchored — the Modifiers-reuse canonicalization AND the reserved-byte drop in one non-circular
/// vector: the **input** golden carries `mods=0xFFFF` + `rsv=0xFF`; decode drops both; encode produces
/// the hand-authored **canonical** golden (`mods=0x003F`, `rsv=0x00`). Both byte-arrays hand-authored.
#[test]
fn vector_key_reserved_canonical() {
    #[rustfmt::skip]
    let input: &[u8] = &[
        0x12, 0x00, 0x00, 0x00, // verb 0x12 (key)
        0x69, 0x00, 0x00, 0x00, // KeySym 0x69 ('i')
        0xFF, 0xFF,             // mods 0xFFFF (all bits incl. reserved 6-15)
        0x02,                   // kind = Release
        0xFF,                   // reserved byte = 0xFF (dropped)
        0x68, 0x69,             // text "hi"
    ];
    #[rustfmt::skip]
    let canonical: &[u8] = &[
        0x12, 0x00, 0x00, 0x00,
        0x69, 0x00, 0x00, 0x00,
        0x3F, 0x00,             // mods canonical 0x003F
        0x02,
        0x00,                   // reserved canonical 0
        0x68, 0x69,
    ];
    let m = decode_message(input).unwrap();
    match &m {
        Message::Key(k) => {
            assert_eq!(k.keysym, 0x69);
            assert_eq!(
                k.mods,
                Modifiers { shift: true, control: true, alt: true, super_: true, caps_lock: true, fn_: true }
            );
            assert_eq!(k.kind, KeyKind::Release);
            assert_eq!(k.text, b"hi");
        }
        _ => panic!("expected Key"),
    }
    assert_eq!(encode_message(&m), canonical);
    assert_eq!(decode_message(canonical).unwrap(), m);
}

/// G0 / G3 coverage (re-tightened, the crate COMPLETE): every implemented verb has a vector, both ways,
/// anchored to `IMPLEMENTED(22)` — fails if any of the 22 lacks a vector.
#[test]
fn vectors_cover_every_implemented_verb() {
    let covered = [
        verb::OPEN_SESSION,
        verb::KEY,
        verb::PANE_STATE,
        verb::FILE_CHUNK,
        verb::NET_ADDRESS,
        verb::FILE_READ,
        verb::FILE_ERROR,
        verb::DISPLAY,
        verb::CREATE_PTY,
        verb::FILE_WRITE,
        verb::FILE_DATA,
        verb::LIST,
        verb::LIST_ENTRY,
        verb::REQUEST_FILE,
        verb::RESIZE,
        verb::VIEWPORT,
        verb::MOUSE,
        verb::PASTE,
        verb::FOCUS,
        verb::NET_RESPONSE,
        verb::NET_ERROR,
        verb::NET_PAYLOAD,
    ];
    assert_eq!(covered.len(), verb::IMPLEMENTED.len());
    // `IMPLEMENTED` is now `&[u32]` (006 6B) — iterate by ref so the element stays `u32`.
    for &v in verb::IMPLEMENTED {
        assert!(covered.contains(&v), "implemented verb {v:#x} has no conformance vector");
    }
    for v in covered {
        assert!(verb::IMPLEMENTED.contains(&v), "vector names unimplemented verb {v:#x}");
    }
}
