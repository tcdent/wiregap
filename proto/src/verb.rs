//! # The wire-numbers surface — the single editable anchor for all on-wire numbers.
//!
//! This is the **one place** the operator reviews and edits the protocol's numbers before
//! ratification. Everything numeric on the wire — the 22 verb ids and the error-body codes — is
//! defined here and referenced by the service modules ([`crate::message`], [`crate::errors`], …), so
//! a pre-ratification number change is a **one-location edit**, never scattered across modules.
//!
//! **All numbers are PROVISIONAL** until the operator ratifies the [`model.md`] §8 sets. They are
//! plain consts: change a value here and the codec, the `CATALOG`/`IMPLEMENTED` anchors, and the
//! (provisional) conformance vectors all follow. If a number moves, the affected vectors are re-authored.
//!
//! ## Verb id format
//! The verb `u32` carries the **id in the low byte**; the upper three bytes are reserved (per-verb
//! flags + id growth, zero for now) and dropped on dispatch. Ids are grouped by service block of 16.
//!
//! [`model.md`]: ../../docs/proto/model.md

// ===========================================================================================
// VERB IDS — by service block.  id  | verb name          | direction          | action (impl/stub)
// ===========================================================================================

// ---- 0x0_  control ------------------------------------------------------------------------
pub const OPEN_SESSION: u32 = 0x00; // open-session         head→box            impl (empty body)

// ---- 0x1_  terminal -----------------------------------------------------------------------
pub const CREATE_PTY: u32 = 0x10; //   create-pty           head→box            impl (dimensions)
pub const DISPLAY: u32 = 0x11; //      display              box→head            impl (cell stream)
pub const KEY: u32 = 0x12; //          key                  head→box            impl (KeySym raw u32 + text tail)
pub const RESIZE: u32 = 0x13; //       resize               head→box            impl (Dimensions reuse)
pub const VIEWPORT: u32 = 0x14; //     viewport             head→box            impl (u32 fixed)
pub const MOUSE: u32 = 0x15; //        mouse                head→box            impl (12B + carriers)
pub const PASTE: u32 = 0x16; //        paste                head→box            impl (verbatim tail)
pub const FOCUS: u32 = 0x17; //        focus                head→box            impl (raw u8)
pub const PANE_STATE: u32 = 0x18; //   pane-state           box→head            impl (fixed-record)

// ---- 0x2_  file ---------------------------------------------------------------------------
pub const FILE_WRITE: u32 = 0x20; //   file-write           box→proxy           impl (safe stack)
pub const FILE_CHUNK: u32 = 0x21; //   file-chunk           box→proxy           impl (verbatim tail)
pub const FILE_READ: u32 = 0x22; //    file-read            box→proxy           impl (unsafe stack)
pub const FILE_DATA: u32 = 0x23; //    file-data            proxy→box           impl (verbatim tail)
pub const LIST: u32 = 0x24; //         list                 box→proxy/head→box  impl (unsafe stack)
pub const LIST_ENTRY: u32 = 0x25; //   list-entry           response            impl (combined-record)
pub const REQUEST_FILE: u32 = 0x26; // request-file         head→box            impl (unsafe stack)
pub const FILE_ERROR: u32 = 0x27; //   file-error           either              impl (error body)

// ---- 0x3_  net ----------------------------------------------------------------------------
pub const NET_ADDRESS: u32 = 0x30; //  net-address          box→proxy           impl (safe stack)
pub const NET_RESPONSE: u32 = 0x31; // net-response          proxy→box           impl (verbatim; NOT reassembled)
pub const NET_ERROR: u32 = 0x32; //    net-error            proxy→box           impl (ErrorBody reuse)
pub const NET_PAYLOAD: u32 = 0x33; //  net-payload          box→proxy           impl (verbatim; reassembled upstream)

// ===========================================================================================
// DISPLAY CELL OP-SET — the top byte of a `display` (0x11) cell `u32` (display.md §1–2, §6 decided).
//   0x00–0x08 defined below; 0x09–0xFF reserved → carried raw (Cell::Reserved), never rejected.
// ===========================================================================================
pub const CELL_DRAW: u32 = 0x00; //              scalar(0–20) + width(21–23)
pub const CELL_GOTO: u32 = 0x01; //              row(12)|col(12) — write position
pub const CELL_SET_FG: u32 = 0x02; //            0xRRGGBB
pub const CELL_SET_BG: u32 = 0x03; //            0xRRGGBB
pub const CELL_SET_UNDERLINE_COLOR: u32 = 0x04; //0xRRGGBB
pub const CELL_SET_ATTRIBUTES: u32 = 0x05; //    attribute bitfield (bits 0–8 §6-open) — whole-raw
pub const CELL_SET_DIMENSIONS: u32 = 0x06; //    rows(12)|cols(12) — grid size
pub const CELL_SET_CURSOR_POS: u32 = 0x07; //    row(12)|col(12) — text cursor
pub const CELL_SET_CURSOR_STYLE: u32 = 0x08; //  shape/blink bitfield (§6-open) — whole-raw
/// The highest defined cell op; `> MAX_CELL_OP` is reserved and carried raw (`Cell::Reserved`).
pub const MAX_CELL_OP: u32 = CELL_SET_CURSOR_STYLE;

// ===========================================================================================
// ERROR-BODY CODES — the shared `file-error` / `net-error` category (model.md §8).
//   0..=6 defined below; 7..=0xFFFFFFFF reserved (carried raw as ErrorCode::Unknown, never rejected).
// ===========================================================================================
pub const ERR_UNSPECIFIED: u32 = 0; // unspecified
pub const ERR_NOT_FOUND: u32 = 1; //   not-found    (no such path / route)
pub const ERR_DENIED: u32 = 2; //      denied       (permission / refused by recognizer)
pub const ERR_UNAVAILABLE: u32 = 3; // unavailable  (busy / no space / i/o)
pub const ERR_TIMEOUT: u32 = 4; //     timeout
pub const ERR_UNREACHABLE: u32 = 5; // unreachable
pub const ERR_MALFORMED: u32 = 6; //   malformed
/// The highest defined error code; `> MAX_ERROR_CODE` is reserved and carried raw.
pub const MAX_ERROR_CODE: u32 = ERR_MALFORMED;

// ===========================================================================================
// MOUSE / INPUT FIELD CODES — the `mouse` (0x15) field values (model.md §8 Input events).
//   The carriers in `terminal/input` reference these (no magic literals — single source).
// ===========================================================================================

// `Button` (u8) — model §8 **ratified**; `> MAX_BUTTON` carried raw (`Button::Other`).
pub const BUTTON_NONE: u8 = 0;
pub const BUTTON_LEFT: u8 = 1;
pub const BUTTON_MIDDLE: u8 = 2;
pub const BUTTON_RIGHT: u8 = 3;
pub const BUTTON_BACK: u8 = 4;
pub const BUTTON_FORWARD: u8 = 5;
/// The highest defined button; `> MAX_BUTTON` is reserved and carried raw.
pub const MAX_BUTTON: u8 = BUTTON_FORWARD;

// `MouseKind` (u8) — **PROVISIONAL** (model §8 names Down/Up/Move/Scroll but does not number them;
// pinned here like the cell op-set, pending operator ratification). `> MAX_MOUSE_KIND` carried raw.
pub const MOUSE_KIND_DOWN: u8 = 0;
pub const MOUSE_KIND_UP: u8 = 1;
pub const MOUSE_KIND_MOVE: u8 = 2;
pub const MOUSE_KIND_SCROLL: u8 = 3;
/// The highest defined mouse kind (provisional); `>` it is reserved and carried raw.
pub const MAX_MOUSE_KIND: u8 = MOUSE_KIND_SCROLL;

// `Modifiers` (u16 bitfield) — model §8 **ratified** bit positions; bits 6–15 reserved (dropped).
pub const MOD_SHIFT_BIT: u32 = 0;
pub const MOD_CONTROL_BIT: u32 = 1;
pub const MOD_ALT_BIT: u32 = 2;
pub const MOD_SUPER_BIT: u32 = 3;
pub const MOD_CAPS_LOCK_BIT: u32 = 4;
pub const MOD_FN_BIT: u32 = 5;

// `KeyKind` (u8) — **PROVISIONAL** (model §8 names Press/Repeat/Release but does not number them;
// pinned here like the cell op-set / MouseKind, pending operator ratification). `> MAX_KEY_KIND` raw.
pub const KEY_KIND_PRESS: u8 = 0;
pub const KEY_KIND_REPEAT: u8 = 1;
pub const KEY_KIND_RELEASE: u8 = 2;
/// The highest defined key kind (provisional); `>` it is reserved and carried raw.
pub const MAX_KEY_KIND: u8 = KEY_KIND_RELEASE;

// ===========================================================================================
// L1 ENVELOPE LAYOUT — the frame header the box encodes + the proxy decodes (model.md §1/§2).
//   `[session_id u32 LE][flags u32 LE][L2 [verb u32][body]]` — 8 bytes, L2 stays 4-byte aligned.
//   PROVISIONAL (session-id width, the continuation-bit position) — pending operator ratification.
// ===========================================================================================
/// The fixed L1 envelope size in bytes; the L2 message begins at this offset (4-byte aligned).
pub const ENVELOPE_LEN: usize = 8;
/// The continuation flag's bit position in the envelope `flags` u32 (bit 0). Bits 1–31 are reserved
/// and **dropped** on decode (the 002/004 reserved-bit-drop) — never read, re-encoded zero.
pub const FLAG_CONTINUATION_BIT: u32 = 0;

// ===========================================================================================
// REASSEMBLY CAPACITY BOUND — the fail-closed total reassembled-request cap (5A).
//   This is a CAPACITY / SECURITY tuning (memory buffered on the trusted proxy vs the largest
//   legitimate request), DISTINCT IN KIND from the protocol numbers above (verb-ids / op-set /
//   error-codes / mouse field codes / envelope-layout) — review it with the capacity/security lens.
//   PROVISIONAL = 32 MiB: the Anthropic Messages API's own ~32 MB request limit, so the proxy fails
//   closed exactly where the upstream would and admits image-bearing POSTs. DISTINCT from the
//   per-frame/per-alloc bound (`MAX_FRAME * ALLOC_K` in the fuzz target) — this bounds the TOTAL
//   reassembled request across frames.
// ===========================================================================================
pub const REASSEMBLY_CAP: usize = 32 * 1024 * 1024;

// ===========================================================================================
// DERIVED ANCHORS — keep in sync with the lists above (asserted by tests).
// ===========================================================================================

/// Every declared verb id, the catalog of record. `CATALOG.len() == 22` anchors build coherence
/// (G0). `list-entry` (`0x25`) is a first-class verb — no special-casing (law 1).
pub const CATALOG: [u32; 22] = [
    OPEN_SESSION,
    CREATE_PTY,
    DISPLAY,
    KEY,
    RESIZE,
    VIEWPORT,
    MOUSE,
    PASTE,
    FOCUS,
    PANE_STATE,
    FILE_WRITE,
    FILE_CHUNK,
    FILE_READ,
    FILE_DATA,
    LIST,
    LIST_ENTRY,
    REQUEST_FILE,
    FILE_ERROR,
    NET_ADDRESS,
    NET_RESPONSE,
    NET_ERROR,
    NET_PAYLOAD,
];

/// The verbs with a decode arm — the **implemented** set, which grows each service sprint.
/// `IMPLEMENTED.len()` anchors the round-trip and conformance coverage counts (G0/G1). Every
/// `IMPLEMENTED` verb dispatches to a real decoder; every catalog verb outside it is a stub
/// (`UnsupportedVerb`). `seed_set_matches_dispatch` ties this to the actual dispatch arms.
///
/// 001 seeded 5; 002 (display) adds `create-pty`/`display`; 003 (file) adds `file-write`/`file-data`/
/// `list`/`list-entry`/`request-file`; 004 (input) adds `resize`/`viewport`/`mouse`/`paste`/`focus`;
/// 005 (net) adds `net-response`/`net-error`/`net-payload`; 006 (control) adds `open-session` (0x00);
/// 007 (input) adds `key` (0x12) — the final verb, the crate now complete (22/22). (006 6B made this a
/// `&[u32]` with `#[cfg]` on its elements so a gated build holds only its slice.)
/// 006 6B: a `&[u32]` (not a fixed array) so each element can be `#[cfg]`'d to its service — a gated
/// build holds only its slice's implemented set (all-features = 21; core-only = `&[]`). `CATALOG`
/// stays the full fixed `[u32; 22]` (the id space is universal). This is declaration-level metadata
/// (the implemented-set per slice), not decode behavior.
pub const IMPLEMENTED: &[u32] = &[
    #[cfg(feature = "control")]
    OPEN_SESSION,
    #[cfg(feature = "terminal")]
    KEY,
    #[cfg(feature = "terminal")]
    PANE_STATE,
    #[cfg(feature = "terminal")]
    CREATE_PTY,
    #[cfg(feature = "terminal")]
    DISPLAY,
    #[cfg(feature = "terminal")]
    RESIZE,
    #[cfg(feature = "terminal")]
    VIEWPORT,
    #[cfg(feature = "terminal")]
    MOUSE,
    #[cfg(feature = "terminal")]
    PASTE,
    #[cfg(feature = "terminal")]
    FOCUS,
    #[cfg(feature = "file")]
    FILE_WRITE,
    #[cfg(feature = "file")]
    FILE_CHUNK,
    #[cfg(feature = "file")]
    FILE_READ,
    #[cfg(feature = "file")]
    FILE_DATA,
    #[cfg(feature = "file")]
    LIST,
    #[cfg(feature = "file")]
    LIST_ENTRY,
    #[cfg(feature = "file")]
    REQUEST_FILE,
    #[cfg(feature = "file")]
    FILE_ERROR,
    #[cfg(feature = "net")]
    NET_ADDRESS,
    #[cfg(feature = "net")]
    NET_RESPONSE,
    #[cfg(feature = "net")]
    NET_ERROR,
    #[cfg(feature = "net")]
    NET_PAYLOAD,
];
