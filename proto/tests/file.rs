//! Seam 3B gates: round-trip over the 5 new file verbs (incl. the full `ListEntry` via the 3A
//! decoder), the reused verbs routing *through* the 001 scans (the 001 rejects surface — proving
//! reuse, not re-implementation), and the binding observable: `UnterminatedRecord` from
//! `decode_message`'s 0x25 arm (a reject only `decode_list_entries` emits).

use proto::{
    FileData, FileWrite, List, Message, Reject, RequestFile, decode_message, encode_message,
    scan_safe, scan_unsafe, verb,
};

fn assert_roundtrip(m: &Message) {
    let wire = encode_message(m);
    let decoded = decode_message(&wire).expect("must decode");
    assert_eq!(&decoded, m, "decode(encode(m)) == m");
    assert_eq!(encode_message(&decoded), wire, "canonical re-encode byte-identical");
}

// ----- the 4 reused-decoder verbs round-trip (construct via the public 001 ctors) ----------------

#[test]
fn file_write_roundtrips_safe_stack() {
    let dest = scan_safe(b"quarantine\x1Ffile.txt").expect("safe stack");
    assert_roundtrip(&Message::FileWrite(FileWrite(dest)));
}

#[test]
fn file_data_roundtrips_verbatim() {
    for bytes in [Vec::new(), vec![0x00], vec![0xFF, 0x00, 0xFE, 0x42]] {
        assert_roundtrip(&Message::FileData(FileData(bytes)));
    }
}

#[test]
fn list_and_request_file_roundtrip_unsafe_stack() {
    let dir = scan_unsafe(b"some dir\x1Fsub").expect("unsafe stack");
    assert_roundtrip(&Message::List(List(dir)));
    let src = scan_unsafe(b"real file.txt").expect("unsafe stack");
    assert_roundtrip(&Message::RequestFile(RequestFile(src)));
}

// ----- reuse-not-reimplement: the 001 scan rejects surface through the verb's decode --------------

#[test]
fn reused_verbs_route_through_the_001_scans() {
    // file-write uses scan_safe: a '/' is a 001 safe-alphabet reject
    let mut frame = verb::FILE_WRITE.to_le_bytes().to_vec();
    frame.extend_from_slice(b"a/b");
    assert_eq!(
        decode_message(&frame),
        Err(Reject::BadSegmentByte { index: 0, offset: 1, byte: b'/' })
    );
    // list uses scan_unsafe: a '..' is the 001 traversal reject
    let mut frame = verb::LIST.to_le_bytes().to_vec();
    frame.extend_from_slice(b"..");
    assert_eq!(
        decode_message(&frame),
        Err(Reject::TraversalSegment { index: 0 })
    );
    // request-file uses scan_unsafe: a NUL is the 001 bad-byte reject
    let mut frame = verb::REQUEST_FILE.to_le_bytes().to_vec();
    frame.extend_from_slice(b"a\0b");
    assert_eq!(
        decode_message(&frame),
        Err(Reject::BadSegmentByte { index: 0, offset: 1, byte: 0 })
    );
}

// ----- list-entry: full ListEntry round-trip + the binding observable ----------------------------

/// Build a `list-entry` record: `[size u64][mtime_secs u64][mtime_nanos u32][type_flags u32][name][0x1F]`.
fn record(size: u64, type_flags: u32, name: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&size.to_le_bytes());
    b.extend_from_slice(&7u64.to_le_bytes()); // mtime_secs
    b.extend_from_slice(&3u32.to_le_bytes()); // mtime_nanos
    b.extend_from_slice(&type_flags.to_le_bytes());
    b.extend_from_slice(name);
    b.push(0x1F);
    b
}

#[test]
fn list_entries_roundtrip_decode_anchored() {
    // a 2-record canonical listing (a known type + a reserved type carried raw)
    let mut frame = verb::LIST_ENTRY.to_le_bytes().to_vec();
    frame.extend(record(100, 1, b"sub dir")); // Dir
    frame.extend(record(0, 9, b"weird\xff.bin")); // reserved type 9, non-UTF-8 name

    let m = decode_message(&frame).expect("valid listing decodes");
    match &m {
        Message::ListEntries(le) => {
            assert_eq!(le.0.len(), 2);
            assert_eq!(le.0[0].size, 100);
            assert_eq!(le.0[0].name.as_bytes(), b"sub dir");
            assert_eq!(le.0[1].name.as_bytes(), b"weird\xff.bin");
        }
        _ => panic!("expected ListEntries"),
    }
    // canonical re-encode is byte-identical (type_flags 9 had zero high bytes), and round-trips
    assert_eq!(encode_message(&m), frame);
    assert_roundtrip(&m);
}

#[test]
fn list_entry_empty_directory_roundtrips() {
    // a 0x25 frame with an empty body — an empty directory
    let frame = verb::LIST_ENTRY.to_le_bytes().to_vec();
    let m = decode_message(&frame).expect("empty directory decodes");
    assert!(matches!(&m, Message::ListEntries(le) if le.0.is_empty()));
    assert_eq!(encode_message(&m), frame);
}

#[test]
fn unterminated_record_surfaces_from_dispatch() {
    // BINDING observable: an unterminated name decoded via decode_message's 0x25 arm yields
    // UnterminatedRecord — a reject only decode_list_entries emits, proving the route-through.
    let mut frame = verb::LIST_ENTRY.to_le_bytes().to_vec();
    frame.extend_from_slice(&[0u8; 24]); // 24B metadata
    frame.extend_from_slice(b"noterminator"); // no 0x1F
    assert_eq!(
        decode_message(&frame),
        Err(Reject::UnterminatedRecord { offset: 24 })
    );
}
