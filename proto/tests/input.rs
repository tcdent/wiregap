//! Seam 4B gates: round-trip over the 5 input verbs through `decode_message`/`encode_message`, and
//! the binding cascade observables — `Mouse` `TrailingBytes` from `decode_message`'s 0x15 arm (the
//! route-through), and the `Dimensions` reject surfacing from `resize` (proving the reuse, not a
//! re-implementation).

use proto::{
    Button, Dimensions, Focus, Message, Modifiers, Mouse, MouseKind, Paste, Reject, Resize,
    Viewport, decode_message, encode_message, verb,
};

fn assert_roundtrip(m: &Message) {
    let wire = encode_message(m);
    let decoded = decode_message(&wire).expect("must decode");
    assert_eq!(&decoded, m, "decode(encode(m)) == m");
    assert_eq!(encode_message(&decoded), wire, "canonical re-encode byte-identical");
}

#[test]
fn resize_roundtrips_through_dimensions() {
    assert_roundtrip(&Message::Resize(Resize(Dimensions { rows: 24, cols: 80 })));
    assert_roundtrip(&Message::Resize(Resize(Dimensions { rows: 0, cols: u16::MAX })));
}

#[test]
fn viewport_roundtrips() {
    for offset in [0u32, 1, 4096, u32::MAX] {
        assert_roundtrip(&Message::Viewport(Viewport { offset }));
    }
}

#[test]
fn paste_roundtrips_verbatim() {
    for bytes in [Vec::new(), vec![0x41], vec![0xFF, 0x00, 0xFE, b'h', b'i']] {
        assert_roundtrip(&Message::Paste(Paste(bytes)));
    }
}

#[test]
fn focus_roundtrips_raw_u8() {
    // 0/1 defined, 2+ reserved — all carried losslessly (a bool would lose 2+)
    for gained in [0u8, 1, 2, 255] {
        assert_roundtrip(&Message::Focus(Focus { gained }));
    }
}

#[test]
fn mouse_roundtrips_via_dispatch_incl_negative_delta() {
    assert_roundtrip(&Message::Mouse(Mouse {
        button: Button::Right,
        kind: MouseKind::Scroll,
        mods: Modifiers {
            control: true,
            ..Modifiers::default()
        },
        row: 10,
        col: 20,
        dx: -1,
        dy: i16::MIN,
    }));
}

// ----- binding cascade observables ---------------------------------------------------------------

#[test]
fn mouse_trailing_bytes_surfaces_from_dispatch() {
    // 0x15 routes through Mouse::decode — a 13-byte body (>12) → TrailingBytes from decode_message.
    let mut frame = verb::MOUSE.to_le_bytes().to_vec();
    frame.extend_from_slice(&[0u8; 13]);
    assert_eq!(
        decode_message(&frame),
        Err(Reject::TrailingBytes { extra: 1 })
    );
}

#[test]
fn resize_routes_through_dimensions_reject() {
    // resize reuses Dimensions::decode_body — a 3-byte body (<4) → ShortBody surfacing from resize's
    // decode via decode_message (the Dimensions reject proves the reuse, not a re-implementation).
    let mut frame = verb::RESIZE.to_le_bytes().to_vec();
    frame.extend_from_slice(&[0u8; 3]);
    assert_eq!(
        decode_message(&frame),
        Err(Reject::ShortBody { needed: 4, got: 3 })
    );
    // and a 5-byte body → TrailingBytes (also from Dimensions)
    let mut frame = verb::RESIZE.to_le_bytes().to_vec();
    frame.extend_from_slice(&[0u8; 5]);
    assert_eq!(
        decode_message(&frame),
        Err(Reject::TrailingBytes { extra: 1 })
    );
}
