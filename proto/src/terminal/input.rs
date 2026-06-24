//! The head→box **input** service (model.md §8 Input events): the head emits typed events, the box
//! decodes them through this crate. The safe direction (box-parsed, not a trusted surface) — but the
//! crate still owns the typed grammar (single-source, anti-drift).
//!
//! Sprint 004, seam **4A**: `mouse` (0x15) and its carriers. The 12B body exercises **three distinct
//! patterns** (kept apart): **carry-raw enums** (`Button`/`MouseKind` — `Other` holds only undefined
//! values, full-u8 bijective), a **reserved-bit-drop bitfield** (`Modifiers` — a 6-bool struct, bits
//! 6–15 unrepresentable by construction), and **signed POD** (`dx`/`dy` `i16`, the new wrinkle —
//! full range incl. negatives, no validation). The remaining input verbs (`resize`/`viewport`/
//! `paste`/`focus`) land in 4B.

use alloc::vec::Vec;

use crate::reject::Reject;
use crate::terminal::dimensions::Dimensions;
use crate::verb;
use crate::wire::Wire;

// ---- Button (carry-raw enum) -------------------------------------------------------------------

/// A reserved mouse button (a value `> MAX_BUTTON`), carried verbatim. Inner is **private**,
/// constructed only in-crate (value `≥ 6`), so [`Button::Other`] can never hold a defined button —
/// round-trip canonical over the full `u8` (the established enforce-by-construction pattern).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReservedButton(u8);

impl ReservedButton {
    /// The raw reserved button value (always `> MAX_BUTTON`).
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }
}

/// The mouse button ([`model.md`] §8). `0..=5` defined; an unrecognized value is **carried raw**
/// ([`Button::Other`]), never rejected. A `Scroll` event sets `button = None`.
///
/// [`model.md`]: ../../docs/proto/model.md
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    None,
    Left,
    Middle,
    Right,
    Back,
    Forward,
    /// A value `> MAX_BUTTON` — see [`ReservedButton`].
    Other(ReservedButton),
}

impl Button {
    fn from_u8(value: u8) -> Self {
        match value {
            verb::BUTTON_NONE => Button::None,
            verb::BUTTON_LEFT => Button::Left,
            verb::BUTTON_MIDDLE => Button::Middle,
            verb::BUTTON_RIGHT => Button::Right,
            verb::BUTTON_BACK => Button::Back,
            verb::BUTTON_FORWARD => Button::Forward,
            // `> MAX_BUTTON` guaranteed by the arms above — the only `ReservedButton` construction site.
            other => Button::Other(ReservedButton(other)),
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            Button::None => verb::BUTTON_NONE,
            Button::Left => verb::BUTTON_LEFT,
            Button::Middle => verb::BUTTON_MIDDLE,
            Button::Right => verb::BUTTON_RIGHT,
            Button::Back => verb::BUTTON_BACK,
            Button::Forward => verb::BUTTON_FORWARD,
            Button::Other(reserved) => reserved.get(),
        }
    }
}

// ---- MouseKind (carry-raw enum) ----------------------------------------------------------------

/// A reserved mouse kind (a value `> MAX_MOUSE_KIND`), carried verbatim. Inner private, in-crate only
/// (value `≥ 4`), so [`MouseKind::Other`] can never hold a defined kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReservedMouseKind(u8);

impl ReservedMouseKind {
    /// The raw reserved kind value (always `> MAX_MOUSE_KIND`).
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }
}

/// The mouse event kind ([`model.md`] §8). `0..=3` defined (**provisional** numbering — model §8 names
/// but does not number these); an unrecognized value is carried raw.
///
/// [`model.md`]: ../../docs/proto/model.md
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseKind {
    Down,
    Up,
    Move,
    Scroll,
    /// A value `> MAX_MOUSE_KIND` — see [`ReservedMouseKind`].
    Other(ReservedMouseKind),
}

impl MouseKind {
    fn from_u8(value: u8) -> Self {
        match value {
            verb::MOUSE_KIND_DOWN => MouseKind::Down,
            verb::MOUSE_KIND_UP => MouseKind::Up,
            verb::MOUSE_KIND_MOVE => MouseKind::Move,
            verb::MOUSE_KIND_SCROLL => MouseKind::Scroll,
            other => MouseKind::Other(ReservedMouseKind(other)),
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            MouseKind::Down => verb::MOUSE_KIND_DOWN,
            MouseKind::Up => verb::MOUSE_KIND_UP,
            MouseKind::Move => verb::MOUSE_KIND_MOVE,
            MouseKind::Scroll => verb::MOUSE_KIND_SCROLL,
            MouseKind::Other(reserved) => reserved.get(),
        }
    }
}

// ---- Modifiers (reserved-bit-drop bitfield) ----------------------------------------------------

/// Keyboard modifiers held during the event ([`model.md`] §8): the 6 defined bits as named bools.
/// Bits 6–15 are reserved — **unrepresentable by construction** (there is no field for them), so a
/// non-canonical `Modifiers` cannot be built and re-encode is canonical (reserved bits zero) without
/// any masking discipline.
///
/// [`model.md`]: ../../docs/proto/model.md
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub super_: bool,
    pub caps_lock: bool,
    pub fn_: bool,
}

impl Modifiers {
    fn from_u16(bits: u16) -> Self {
        let test = |bit: u32| bits & (1u16 << bit) != 0;
        Modifiers {
            shift: test(verb::MOD_SHIFT_BIT),
            control: test(verb::MOD_CONTROL_BIT),
            alt: test(verb::MOD_ALT_BIT),
            super_: test(verb::MOD_SUPER_BIT),
            caps_lock: test(verb::MOD_CAPS_LOCK_BIT),
            fn_: test(verb::MOD_FN_BIT),
        }
    }

    fn to_u16(self) -> u16 {
        let bit = |set: bool, b: u32| if set { 1u16 << b } else { 0 };
        bit(self.shift, verb::MOD_SHIFT_BIT)
            | bit(self.control, verb::MOD_CONTROL_BIT)
            | bit(self.alt, verb::MOD_ALT_BIT)
            | bit(self.super_, verb::MOD_SUPER_BIT)
            | bit(self.caps_lock, verb::MOD_CAPS_LOCK_BIT)
            | bit(self.fn_, verb::MOD_FN_BIT)
    }
}

// ---- mouse (0x15) ------------------------------------------------------------------------------

/// `mouse` (0x15, head→box): a 12B fixed-only body
/// `[button u8][kind u8][mods u16][row u16][col u16][dx i16][dy i16]`. `dx`/`dy` carry the scroll
/// delta (0 otherwise); `row`/`col` are raw (the box clamps an out-of-grid coord).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mouse {
    pub button: Button,
    pub kind: MouseKind,
    pub mods: Modifiers,
    pub row: u16,
    pub col: u16,
    pub dx: i16,
    pub dy: i16,
}

impl Wire for Mouse {
    const VERB: u32 = verb::MOUSE;

    fn encode(&self, out: &mut Vec<u8>) {
        out.push(self.button.to_u8());
        out.push(self.kind.to_u8());
        out.extend_from_slice(&self.mods.to_u16().to_le_bytes());
        out.extend_from_slice(&self.row.to_le_bytes());
        out.extend_from_slice(&self.col.to_le_bytes());
        out.extend_from_slice(&self.dx.to_le_bytes());
        out.extend_from_slice(&self.dy.to_le_bytes());
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        let short = Reject::ShortBody {
            needed: 12,
            got: body.len(),
        };
        let (button, r) = body.split_first_chunk::<1>().ok_or(short)?;
        let (kind, r) = r.split_first_chunk::<1>().ok_or(short)?;
        let (mods, r) = r.split_first_chunk::<2>().ok_or(short)?;
        let (row, r) = r.split_first_chunk::<2>().ok_or(short)?;
        let (col, r) = r.split_first_chunk::<2>().ok_or(short)?;
        let (dx, r) = r.split_first_chunk::<2>().ok_or(short)?;
        let (dy, rest) = r.split_first_chunk::<2>().ok_or(short)?;
        if !rest.is_empty() {
            return Err(Reject::TrailingBytes { extra: rest.len() });
        }
        Ok(Mouse {
            button: Button::from_u8(u8::from_le_bytes(*button)),
            kind: MouseKind::from_u8(u8::from_le_bytes(*kind)),
            mods: Modifiers::from_u16(u16::from_le_bytes(*mods)),
            row: u16::from_le_bytes(*row),
            col: u16::from_le_bytes(*col),
            dx: i16::from_le_bytes(*dx),
            dy: i16::from_le_bytes(*dy),
        })
    }
}

// ---- the simpler verbs (4B) --------------------------------------------------------------------

/// `resize` (0x13, head→box): a window-size change to the PTY layer. Carries the shared `Dimensions`
/// grammar — the §8 grammar-reuse (create-pty + resize). Routes **through** `Dimensions::decode_body`/
/// `encode_body`; not a re-implementation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Resize(pub Dimensions);

impl Wire for Resize {
    const VERB: u32 = verb::RESIZE;

    fn encode(&self, out: &mut Vec<u8>) {
        self.0.encode_body(out);
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        Dimensions::decode_body(body).map(Resize)
    }
}

/// `viewport` (0x14, head→box): explicit scrollback navigation — `offset` lines above live (`0` =
/// live). Raw `u32` (every value valid; the box clamps).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Viewport {
    pub offset: u32,
}

impl Wire for Viewport {
    const VERB: u32 = verb::VIEWPORT;

    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.offset.to_le_bytes());
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        let (offset, rest) = body.split_first_chunk::<4>().ok_or(Reject::ShortBody {
            needed: 4,
            got: body.len(),
        })?;
        if !rest.is_empty() {
            return Err(Reject::TrailingBytes { extra: rest.len() });
        }
        Ok(Viewport {
            offset: u32::from_le_bytes(*offset),
        })
    }
}

/// `paste` (0x16, head→box): clipboard text, a **verbatim** UTF-8 tail (distinct from a text-only
/// key). Carried as `Vec<u8>` — UTF-8 validation is the consumer's (law 11), like `ErrorBody.detail`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Paste(pub Vec<u8>);

impl Wire for Paste {
    const VERB: u32 = verb::PASTE;

    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.0);
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        Ok(Paste(body.to_vec()))
    }
}

/// `focus` (0x17, head→box): a focus change. `gained` is a raw `u8` (`0`/`1` defined, `2+` reserved)
/// carried losslessly — not a `bool` (non-canonical for `≥ 2`); the consumer interprets `≠ 0`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Focus {
    pub gained: u8,
}

impl Wire for Focus {
    const VERB: u32 = verb::FOCUS;

    fn encode(&self, out: &mut Vec<u8>) {
        out.push(self.gained);
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        let (gained, rest) = body.split_first_chunk::<1>().ok_or(Reject::ShortBody {
            needed: 1,
            got: body.len(),
        })?;
        if !rest.is_empty() {
            return Err(Reject::TrailingBytes { extra: rest.len() });
        }
        Ok(Focus {
            gained: u8::from_le_bytes(*gained),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Button, Modifiers, Mouse, MouseKind, ReservedButton, ReservedMouseKind};
    use crate::reject::Reject;
    use crate::wire::Wire;
    use alloc::vec::Vec;

    fn encode(m: &Mouse) -> Vec<u8> {
        let mut out = Vec::new();
        m.encode(&mut out);
        out
    }

    #[test]
    fn button_and_kind_bijective_over_full_u8() {
        for b in 0u8..=255 {
            assert_eq!(Button::from_u8(b).to_u8(), b, "button {b} must round-trip");
            assert_eq!(MouseKind::from_u8(b).to_u8(), b, "kind {b} must round-trip");
        }
        // known vs Other
        assert_eq!(Button::from_u8(5), Button::Forward);
        assert_eq!(Button::from_u8(6), Button::Other(ReservedButton(6)));
        assert_eq!(MouseKind::from_u8(3), MouseKind::Scroll);
        assert_eq!(MouseKind::from_u8(4), MouseKind::Other(ReservedMouseKind(4)));
    }

    #[test]
    fn modifiers_drop_reserved_bits_canonically() {
        // decode(0xFFFF) -> all 6 flags set; re-encode -> 0x003F (reserved bits 6-15 zero).
        let all = Modifiers::from_u16(0xFFFF);
        assert_eq!(
            all,
            Modifiers {
                shift: true,
                control: true,
                alt: true,
                super_: true,
                caps_lock: true,
                fn_: true,
            }
        );
        assert_eq!(all.to_u16(), 0x003F);
        // a non-canonical Modifiers (reserved bits set) is unconstructible — the struct has no field
        // for bits 6-15, so the type itself forbids it.
        assert_eq!(Modifiers::from_u16(0).to_u16(), 0x0000);
        assert_eq!(Modifiers::from_u16(0x0001).to_u16(), 0x0001); // shift only
    }

    /// `decode(encode(m)) == m` + canonical re-encode for one mouse event.
    fn rt(m: Mouse) {
        let wire = encode(&m);
        assert_eq!(wire.len(), 12);
        let back = Mouse::decode(&wire).expect("12B mouse decodes");
        assert_eq!(back, m);
        assert_eq!(encode(&back), wire);
    }

    #[test]
    fn mouse_roundtrips_incl_negative_i16_deltas() {
        // THE sign-bit make-or-break: dx/dy over i16::MIN / -1 / i16::MAX (not positives only).
        for (dx, dy) in [
            (i16::MIN, i16::MAX),
            (-1, 1),
            (i16::MAX, i16::MIN),
            (0, -32768),
            (12345, -12345),
        ] {
            rt(Mouse {
                button: Button::Left,
                kind: MouseKind::Scroll,
                mods: Modifiers {
                    shift: true,
                    fn_: true,
                    ..Modifiers::default()
                },
                row: 40,
                col: 120,
                dx,
                dy,
            });
        }
        // carry-raw Other button + kind round-trip too
        rt(Mouse {
            button: Button::Other(ReservedButton(200)),
            kind: MouseKind::Other(ReservedMouseKind(99)),
            mods: Modifiers::default(),
            row: u16::MAX,
            col: 0,
            dx: 0,
            dy: 0,
        });
    }

    #[test]
    fn mouse_decode_drops_reserved_mod_bits() {
        // a wire mouse with mods = 0xFFFF decodes to all-6-flags, re-encodes mods = 0x003F.
        #[rustfmt::skip]
        let wire: &[u8] = &[
            0x01, 0x03,             // button Left, kind Scroll
            0xFF, 0xFF,             // mods 0xFFFF (all bits set, incl. reserved 6-15)
            0x00, 0x00, 0x00, 0x00, // row 0, col 0
            0xFF, 0xFF, 0x01, 0x00, // dx -1, dy 1
        ];
        let m = Mouse::decode(wire).unwrap();
        assert_eq!(m.mods.to_u16(), 0x003F);
        assert_eq!(m.dx, -1);
        // canonical re-encode zeros the reserved mod bits
        assert_eq!(&encode(&m)[2..4], &0x003Fu16.to_le_bytes());
    }

    #[test]
    fn mouse_geometry_short_and_trailing() {
        assert_eq!(
            Mouse::decode(&[0u8; 11]),
            Err(Reject::ShortBody { needed: 12, got: 11 })
        );
        assert_eq!(
            Mouse::decode(&[0u8; 13]),
            Err(Reject::TrailingBytes { extra: 1 })
        );
    }
}
