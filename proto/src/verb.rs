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
pub const OPEN_SESSION: u32 = 0x00; // open-session         head→box            defer

// ---- 0x1_  terminal -----------------------------------------------------------------------
pub const CREATE_PTY: u32 = 0x10; //   create-pty           head→box            impl (dimensions)
pub const DISPLAY: u32 = 0x11; //      display              box→head            impl (cell stream)
pub const KEY: u32 = 0x12; //          key                  head→box            defer (BLOCKED: keysym table)
pub const RESIZE: u32 = 0x13; //       resize               head→box            defer
pub const VIEWPORT: u32 = 0x14; //     viewport             head→box            defer
pub const MOUSE: u32 = 0x15; //        mouse                head→box            defer
pub const PASTE: u32 = 0x16; //        paste                head→box            defer
pub const FOCUS: u32 = 0x17; //        focus                head→box            defer
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
pub const NET_RESPONSE: u32 = 0x31; // net-response          proxy→box           stub
pub const NET_ERROR: u32 = 0x32; //    net-error            proxy→box           stub
pub const NET_PAYLOAD: u32 = 0x33; //  net-payload          box→proxy           stub

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
/// 001 seeded 5 (`pane-state`, `file-chunk`, `file-read`, `net-address`, `file-error`); 002 (display)
/// adds `create-pty` (0x10) and `display` (0x11); 003 (file) adds `file-write` (0x20), `file-data`
/// (0x23), `list` (0x24), `list-entry` (0x25), `request-file` (0x26).
pub const IMPLEMENTED: [u32; 12] = [
    PANE_STATE,
    FILE_CHUNK,
    FILE_READ,
    NET_ADDRESS,
    FILE_ERROR,
    CREATE_PTY,
    DISPLAY,
    FILE_WRITE,
    FILE_DATA,
    LIST,
    LIST_ENTRY,
    REQUEST_FILE,
];
