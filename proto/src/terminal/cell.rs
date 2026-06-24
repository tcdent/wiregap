//! The display cell-op grammar ([`display.md`] §1–2): a `u32` cell, the **op in the top byte**, the
//! low 24 bits the payload. The crate owns this layout **once** so the box's encode and the head's
//! decode cannot drift (library §0) — that is why `Cell` is typed (Decided Fork #1).
//!
//! The crate decodes the **ratified field-splits** (draw scalar/width; the `RowCol` 12|12 pack for
//! goto/cursor-pos/dimensions; 24-bit RGB) and carries every payload as a **raw value** — it never
//! interprets the §6-open attribute / cursor-style bits (those are the head's). Each carrier is a
//! **bit-width-raw newtype, never a defined-value enum**, with a private interior, a public
//! width-checked constructor, and a getter — so an out-of-width carrier is **unrepresentable** and
//! `decode(encode(cell)) == cell` over the *full* `Cell` type (enforce-by-construction, Fork #1c —
//! the same pattern as 001's `ReservedCode`).
//!
//! [`display.md`]: ../../docs/proto/display.md

use crate::verb;

/// `0x00` draw — a Unicode scalar (bits 0–20), carried **raw**: an out-of-range scalar (`> 0x10FFFF`)
/// or a surrogate is the head's concern (it draws the replacement glyph), not a decode reject.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Scalar(u32);

impl Scalar {
    /// Largest 21-bit value.
    pub const MAX: u32 = 0x001F_FFFF;
    /// Construct a 21-bit scalar; `None` if `value` exceeds 21 bits.
    #[must_use]
    pub fn new(value: u32) -> Option<Self> {
        (value <= Self::MAX).then_some(Self(value))
    }
    #[must_use]
    pub fn get(self) -> u32 {
        self.0
    }
}

/// `0x00` draw — the column width (bits 21–23): `0` zero-width, `1` normal, `2` wide; `3`–`7` carried
/// raw (the head decides). A **raw 3-bit value, not an enum** (Decided Fork #1c).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Width(u8);

impl Width {
    /// Largest 3-bit value.
    pub const MAX: u8 = 0x07;
    /// Construct a 3-bit width; `None` if `value` exceeds 3 bits.
    #[must_use]
    pub fn new(value: u8) -> Option<Self> {
        (value <= Self::MAX).then_some(Self(value))
    }
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }
}

/// A `row(12)|col(12)` pair (goto / set-cursor-pos / set-dimensions). 0–4095 each, carried raw (an
/// out-of-grid value is clamped by the head). Distinct from the verb-body `Dimensions` (u16|u16).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RowCol {
    row: u16,
    col: u16,
}

impl RowCol {
    /// Largest 12-bit coordinate.
    pub const MAX: u16 = 0x0FFF;
    /// Construct a 12|12 pair; `None` if either coordinate exceeds 12 bits.
    #[must_use]
    pub fn new(row: u16, col: u16) -> Option<Self> {
        (row <= Self::MAX && col <= Self::MAX).then_some(Self { row, col })
    }
    #[must_use]
    pub fn row(self) -> u16 {
        self.row
    }
    #[must_use]
    pub fn col(self) -> u16 {
        self.col
    }
    /// Pack into the 24-bit payload (`row << 12 | col`). Both ≤ `MAX`, so it fits.
    fn packed(self) -> u32 {
        (u32::from(self.row) << 12) | u32::from(self.col)
    }
    /// Unpack from a 24-bit payload. Masks each field, so the 12-bit invariant holds by construction.
    fn from_payload(payload: u32) -> Self {
        Self {
            row: ((payload >> 12) & u32::from(Self::MAX)) as u16,
            col: (payload & u32::from(Self::MAX)) as u16,
        }
    }
}

/// Macro-free 24-bit whole-payload carrier shared shape: a private `u32`, a width-checked public
/// `new`, and a `get`. Used for RGB and the §6-open attribute / cursor-style bitfields — each kept a
/// **distinct type** for safety even though the wire width coincides.
macro_rules! payload24 {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub struct $name(u32);
        impl $name {
            /// Largest 24-bit value.
            pub const MAX: u32 = 0x00FF_FFFF;
            /// Construct from a 24-bit value; `None` if it exceeds 24 bits.
            #[must_use]
            pub fn new(value: u32) -> Option<Self> {
                (value <= Self::MAX).then_some(Self(value))
            }
            #[must_use]
            pub fn get(self) -> u32 {
                self.0
            }
        }
    };
}

payload24!(
    /// A 24-bit `0xRRGGBB` color (set-fg / set-bg / set-underline-color).
    Rgb
);
payload24!(
    /// The set-attributes payload (bits 0–8 §6-open), carried **whole-raw** — the head interprets the
    /// bits, so the unratified assignments do not gate the crate.
    AttrBits
);
payload24!(
    /// The set-cursor-style payload (shape/blink, §6-open), carried **whole-raw**.
    CursorStyleBits
);

/// `0x09`–`0xFF` reserved cell op — the whole cell `u32`, carried raw and re-emitted byte-identical
/// (display.md §1 `_` ignored). Its inner is **private** and constructed only in-crate when the op is
/// `≥ 0x09`, so a `Cell::Reserved` can never hold a *known* op — the round-trip stays canonical (the
/// same enforce-by-construction as 001's `ReservedCode`). Decode-only: the box emits known ops.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReservedCell(u32);

impl ReservedCell {
    /// The raw reserved cell `u32` (its op byte is always `≥ 0x09`).
    #[must_use]
    pub fn get(self) -> u32 {
        self.0
    }
}

/// One display cell op. Typed op + ratified field-splits; §6-open payloads carried whole-raw; unknown
/// ops carried (never rejected).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cell {
    Draw { scalar: Scalar, width: Width },
    Goto(RowCol),
    SetFg(Rgb),
    SetBg(Rgb),
    SetUnderlineColor(Rgb),
    SetAttributes(AttrBits),
    SetDimensions(RowCol),
    SetCursorPos(RowCol),
    SetCursorStyle(CursorStyleBits),
    Reserved(ReservedCell),
}

impl Cell {
    /// Decode one cell `u32`. Total: every value yields a `Cell` (an unknown op → `Reserved`), never a
    /// reject. In-crate constructors mask each field, so every carrier's bit-width invariant holds.
    pub(crate) fn from_u32(cell: u32) -> Self {
        let op = cell >> 24;
        let payload = cell & 0x00FF_FFFF;
        match op {
            verb::CELL_DRAW => Cell::Draw {
                scalar: Scalar(payload & Scalar::MAX),
                width: Width(((payload >> 21) & u32::from(Width::MAX)) as u8),
            },
            verb::CELL_GOTO => Cell::Goto(RowCol::from_payload(payload)),
            verb::CELL_SET_FG => Cell::SetFg(Rgb(payload)),
            verb::CELL_SET_BG => Cell::SetBg(Rgb(payload)),
            verb::CELL_SET_UNDERLINE_COLOR => Cell::SetUnderlineColor(Rgb(payload)),
            verb::CELL_SET_ATTRIBUTES => Cell::SetAttributes(AttrBits(payload)),
            verb::CELL_SET_DIMENSIONS => Cell::SetDimensions(RowCol::from_payload(payload)),
            verb::CELL_SET_CURSOR_POS => Cell::SetCursorPos(RowCol::from_payload(payload)),
            verb::CELL_SET_CURSOR_STYLE => Cell::SetCursorStyle(CursorStyleBits(payload)),
            // op ≥ 0x09 — carried raw, never rejected.
            _ => Cell::Reserved(ReservedCell(cell)),
        }
    }

    /// Encode one cell to its `u32`. Canonical: every field fits its bit range, so the op byte and
    /// payload never collide.
    pub(crate) fn to_u32(self) -> u32 {
        match self {
            Cell::Draw { scalar, width } => {
                (verb::CELL_DRAW << 24) | (u32::from(width.get()) << 21) | scalar.get()
            }
            Cell::Goto(rc) => (verb::CELL_GOTO << 24) | rc.packed(),
            Cell::SetFg(c) => (verb::CELL_SET_FG << 24) | c.get(),
            Cell::SetBg(c) => (verb::CELL_SET_BG << 24) | c.get(),
            Cell::SetUnderlineColor(c) => (verb::CELL_SET_UNDERLINE_COLOR << 24) | c.get(),
            Cell::SetAttributes(a) => (verb::CELL_SET_ATTRIBUTES << 24) | a.get(),
            Cell::SetDimensions(rc) => (verb::CELL_SET_DIMENSIONS << 24) | rc.packed(),
            Cell::SetCursorPos(rc) => (verb::CELL_SET_CURSOR_POS << 24) | rc.packed(),
            Cell::SetCursorStyle(s) => (verb::CELL_SET_CURSOR_STYLE << 24) | s.get(),
            Cell::Reserved(rc) => rc.get(),
        }
    }
}
