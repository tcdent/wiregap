//! Seam 2B gates: the carry-raw **canonical invariant over the full `Cell` type** (round-trip over
//! each carrier's full bit-width range), enforce-by-construction (out-of-width carriers
//! unrepresentable), the decode-anchored reserved-op carry, and the binding live-path condition —
//! `Display::decode` routes through the 2A `decode_record_tail` (observable via the partial-tail reject).

use proto::{
    AttrBits, Cell, CursorStyleBits, Display, Reject, Rgb, RowCol, Scalar, Width, Wire,
};

fn encode(d: &Display) -> Vec<u8> {
    let mut out = Vec::new();
    d.encode(&mut out);
    out
}

/// Round-trip one cell through a `Display`: `decode(encode(cell)) == cell` + canonical re-encode.
fn rt(cell: Cell) {
    let d = Display(vec![cell]);
    let wire = encode(&d);
    assert_eq!(wire.len(), 4, "one cell is exactly 4 bytes");
    let back = Display::decode(&wire).expect("a valid cell stream decodes");
    assert_eq!(back, d, "decode(encode(cell)) == cell");
    assert_eq!(encode(&back), wire, "canonical re-encode is byte-identical");
}

#[test]
fn cell_roundtrips_over_full_bit_widths() {
    // draw: scalar 0..=0x1F_FFFF (21b) and width 0..=7 (3b) — they tile the 24-bit payload exactly
    for &scalar in &[0, 1, Scalar::MAX] {
        for &width in &[0, 1, 2, Width::MAX] {
            rt(Cell::Draw {
                scalar: Scalar::new(scalar).unwrap(),
                width: Width::new(width).unwrap(),
            });
        }
    }
    // row|col 12+12: corners of the 4095×4095 space
    for &(r, c) in &[(0, 0), (RowCol::MAX, RowCol::MAX), (RowCol::MAX, 0), (0, RowCol::MAX)] {
        let rc = RowCol::new(r, c).unwrap();
        rt(Cell::Goto(rc));
        rt(Cell::SetDimensions(rc));
        rt(Cell::SetCursorPos(rc));
    }
    // 24-bit whole-raw carriers: 0 and max
    for &v in &[0, 0x12_3456, Rgb::MAX] {
        rt(Cell::SetFg(Rgb::new(v).unwrap()));
        rt(Cell::SetBg(Rgb::new(v).unwrap()));
        rt(Cell::SetUnderlineColor(Rgb::new(v).unwrap()));
        rt(Cell::SetAttributes(AttrBits::new(v).unwrap()));
        rt(Cell::SetCursorStyle(CursorStyleBits::new(v).unwrap()));
    }
    // cursor shape values 0..=3 specifically (carried raw, not an enum)
    for v in 0..=3u32 {
        rt(Cell::SetCursorStyle(CursorStyleBits::new(v).unwrap()));
    }
}

#[test]
fn carriers_reject_out_of_width_by_construction() {
    assert_eq!(Scalar::new(Scalar::MAX), Scalar::new(0x1F_FFFF));
    assert!(Scalar::new(Scalar::MAX).is_some());
    assert!(Scalar::new(Scalar::MAX + 1).is_none(), "22-bit scalar unrepresentable");
    assert!(Width::new(Width::MAX).is_some());
    assert!(Width::new(Width::MAX + 1).is_none(), "4-bit width unrepresentable");
    assert!(RowCol::new(RowCol::MAX, RowCol::MAX).is_some());
    assert!(RowCol::new(RowCol::MAX + 1, 0).is_none(), "13-bit row unrepresentable");
    assert!(RowCol::new(0, RowCol::MAX + 1).is_none(), "13-bit col unrepresentable");
    assert!(Rgb::new(Rgb::MAX).is_some());
    assert!(Rgb::new(Rgb::MAX + 1).is_none(), "25-bit rgb unrepresentable");
    assert!(AttrBits::new(AttrBits::MAX + 1).is_none());
    assert!(CursorStyleBits::new(CursorStyleBits::MAX + 1).is_none());
}

#[test]
fn reserved_op_is_carried_raw_decode_anchored() {
    // op >= 0x09 -> Cell::Reserved (decode-only; no public ctor). Round-trips byte-identically.
    for op in [0x09u32, 0x10, 0x7F, 0xFF] {
        let raw = (op << 24) | 0x00AB_CDEF;
        let wire = raw.to_le_bytes();
        let d = Display::decode(&wire).expect("reserved op is carried, never rejected");
        match d.0.as_slice() {
            [Cell::Reserved(rc)] => {
                assert_eq!(rc.get(), raw, "reserved cell carries the full raw u32");
                assert!(rc.get() >> 24 >= 0x09, "reserved op byte >= 0x09");
            }
            other => panic!("expected a single Reserved cell, got {other:?}"),
        }
        assert_eq!(encode(&d), wire, "reserved re-encode is byte-identical");
    }
}

#[test]
fn known_ops_never_decode_to_reserved() {
    // a 0x05 (set-attributes) cell with a full payload is SetAttributes, not Reserved
    let wire = (0x05u32 << 24 | 0x00FF_FFFF).to_le_bytes();
    let d = Display::decode(&wire).unwrap();
    assert_eq!(d.0, vec![Cell::SetAttributes(AttrBits::new(0xFF_FFFF).unwrap())]);
}

#[test]
fn display_decode_routes_through_the_tail_decoder() {
    // empty stream -> Ok(no-op repaint)
    assert_eq!(Display::decode(&[]).unwrap(), Display(Vec::new()));
    // a partial trailing record (len % 4 != 0) -> PartialRecord — only the 2A decoder produces this,
    // so its appearance proves Display::decode routes through decode_record_tail (not an inline loop).
    for len in [1usize, 2, 3, 5, 7] {
        let buf = vec![0u8; len];
        assert_eq!(
            Display::decode(&buf),
            Err(Reject::PartialRecord {
                record_len: 4,
                remainder: len % 4,
            }),
            "len {len} must be a partial-record reject"
        );
    }
}

#[test]
fn multi_op_display_roundtrips() {
    let d = Display(vec![
        Cell::SetFg(Rgb::new(0xFF_8800).unwrap()),
        Cell::Goto(RowCol::new(3, 17).unwrap()),
        Cell::Draw {
            scalar: Scalar::new(0x61).unwrap(),
            width: Width::new(1).unwrap(),
        },
        Cell::SetAttributes(AttrBits::new(0b1_0000_0001).unwrap()),
        Cell::SetCursorPos(RowCol::new(0, 0).unwrap()),
    ]);
    let wire = encode(&d);
    assert_eq!(wire.len(), 20);
    let back = Display::decode(&wire).unwrap();
    assert_eq!(back, d);
    assert_eq!(encode(&back), wire);
}
