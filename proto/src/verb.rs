//! # The wire-numbers surface — the single editable anchor for all on-wire numbers.
//!
//! This is the **one place** the operator reviews and edits the protocol's numbers before
//! ratification. Everything numeric on the wire — the 22 verb ids and the error-body codes — is
//! defined here and referenced by the service modules ([`crate::message`], [`crate::errors`], …), so
//! a pre-ratification number change is a **one-location edit**, never scattered across modules.
//!
//! **All numbers are PROVISIONAL** until the operator ratifies the [`model.md`] §8 sets. They are
//! plain consts: change a value here and the codec, the `CATALOG`/`SEED` anchors, and the (provisional)
//! conformance vectors all follow. If a number moves, the affected vectors are re-authored.
//!
//! ## Verb id format
//! The verb `u32` carries the **id in the low byte**; the upper three bytes are reserved (per-verb
//! flags + id growth, zero for now) and dropped on dispatch. Ids are grouped by service block of 16.
//!
//! [`model.md`]: ../../docs/proto/model.md

// ===========================================================================================
// VERB IDS — by service block.  id  | verb name          | direction          | 001 action
// ===========================================================================================

// ---- 0x0_  control ------------------------------------------------------------------------
pub const OPEN_SESSION: u32 = 0x00; // open-session         head→box            defer

// ---- 0x1_  terminal -----------------------------------------------------------------------
pub const CREATE_PTY: u32 = 0x10; //   create-pty           head→box            defer
pub const DISPLAY: u32 = 0x11; //      display              box→head            defer
pub const KEY: u32 = 0x12; //          key                  head→box            defer (BLOCKED: keysym table)
pub const RESIZE: u32 = 0x13; //       resize               head→box            defer
pub const VIEWPORT: u32 = 0x14; //     viewport             head→box            defer
pub const MOUSE: u32 = 0x15; //        mouse                head→box            defer
pub const PASTE: u32 = 0x16; //        paste                head→box            defer
pub const FOCUS: u32 = 0x17; //        focus                head→box            defer
pub const PANE_STATE: u32 = 0x18; //   pane-state           box→head            SEED (fixed-record)

// ---- 0x2_  file ---------------------------------------------------------------------------
pub const FILE_WRITE: u32 = 0x20; //   file-write           box→proxy           stub
pub const FILE_CHUNK: u32 = 0x21; //   file-chunk           box→proxy           SEED (verbatim tail)
pub const FILE_READ: u32 = 0x22; //    file-read            box→proxy           SEED (unsafe stack)
pub const FILE_DATA: u32 = 0x23; //    file-data            proxy→box           stub
pub const LIST: u32 = 0x24; //         list                 box→proxy/head→box  stub
pub const LIST_ENTRY: u32 = 0x25; //   list-entry           response            stub
pub const REQUEST_FILE: u32 = 0x26; // request-file         head→box            stub
pub const FILE_ERROR: u32 = 0x27; //   file-error           either              SEED (error body)

// ---- 0x3_  net ----------------------------------------------------------------------------
pub const NET_ADDRESS: u32 = 0x30; //  net-address          box→proxy           SEED (safe stack)
pub const NET_RESPONSE: u32 = 0x31; // net-response          proxy→box           stub
pub const NET_ERROR: u32 = 0x32; //    net-error            proxy→box           stub
pub const NET_PAYLOAD: u32 = 0x33; //  net-payload          box→proxy           stub

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

/// The verbs with a decode arm in sprint 001 — the seed set. `SEED.len()` anchors the round-trip and
/// conformance coverage counts (G0/G1). Every `SEED` verb dispatches to a real decoder; every catalog
/// verb outside `SEED` is a stub (`UnsupportedVerb`). `seed_set_matches_dispatch` ties this to the
/// actual dispatch arms.
pub const SEED: [u32; 5] = [PANE_STATE, FILE_CHUNK, FILE_READ, NET_ADDRESS, FILE_ERROR];
