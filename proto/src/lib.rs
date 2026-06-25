#![no_std]
#![forbid(unsafe_code)]

//! `proto` — the single source of truth for the box↔proxy↔head wire codec.
//!
//! Sprint 001, seam **1A**: the segment primitive. This crate root establishes the auditable-surface
//! boundary (Decided Fork #5 / gate G6): `#![no_std]` + `alloc` makes library §0's no-effect
//! boundary (laws 5–7) a *compile error* — `std::{fs,net,process,thread,time,env}` cannot link —
//! and `#![forbid(unsafe_code)]` forecloses transmute-overlay and the pointer arithmetic an
//! over-read would need (the library is safe Rust end to end).
//!
//! 1A delivers the one addressing format shared by file and net ([`file.md`] §0): a `0x1F`-delimited
//! [`scan_safe`] / [`scan_unsafe`] that **verifies, never transforms**, producing the distinct
//! [`SafeSegment`] / [`UnsafeSegment`] types. The safe/unsafe split is a *type*, not a discipline:
//! only [`SafeSegment`] carries the safe-to-log capability ([`SafeSegment::as_str`]), so a real name
//! physically cannot reach a log line.
//!
//! [`file.md`]: ../../docs/proto/file.md

extern crate alloc;

// Test-only: the unit-test harness needs `std`. Gated to `cfg(test)`, so the audited (non-test) lib
// build stays `no_std` — `cargo build -p proto` and the bare-metal target never see this.
#[cfg(test)]
extern crate std;

// CORE modules (always compiled — universal routing + the shared foundational grammars).
mod errors;
mod message;
mod reject;
mod segment;
pub mod verb;
mod wire;

// Per-service modules (006 6B) — gated at the DECLARATION line; the module files are byte-identical
// (the cfg never goes inside them). A consumer compiles only the services it links (library §7).
#[cfg(feature = "control")]
mod control;
#[cfg(feature = "terminal")]
mod terminal;
#[cfg(feature = "file")]
mod file;
#[cfg(feature = "file")]
mod list_entry;
#[cfg(feature = "net")]
mod net;
#[cfg(feature = "net")]
mod reassembler;

// CORE re-exports (always available).
pub use errors::{ErrorBody, ErrorCode, ReservedCode};
pub use message::{Message, decode_message, encode_message};
pub use reject::Reject;
pub use segment::{
    SafeSegment, SafeSegmentStack, UnsafeSegment, UnsafeSegmentStack, scan_safe, scan_unsafe,
};
pub use wire::Wire;

// Per-service re-exports (gated to match their module).
#[cfg(feature = "control")]
pub use control::OpenSession;
#[cfg(feature = "file")]
pub use file::{FileChunk, FileData, FileError, FileRead, FileWrite, List, RequestFile};
#[cfg(feature = "file")]
pub use list_entry::{FileType, ListEntries, ListEntry, ReservedFileType};
#[cfg(feature = "net")]
pub use net::{NetAddress, NetError, NetPayload, NetResponse, ReservedScheme, Scheme};
#[cfg(feature = "net")]
pub use reassembler::{Reassembler, ReassemblyState};
#[cfg(feature = "terminal")]
pub use terminal::cell::{
    AttrBits, Cell, CursorStyleBits, ReservedCell, Rgb, RowCol, Scalar, Width,
};
#[cfg(feature = "terminal")]
pub use terminal::dimensions::Dimensions;
#[cfg(feature = "terminal")]
pub use terminal::display::{CreatePty, Display};
#[cfg(feature = "terminal")]
pub use terminal::input::{
    Button, Focus, Key, KeyKind, Modifiers, Mouse, MouseKind, Paste, ReservedButton, ReservedKeyKind,
    ReservedMouseKind, Resize, Viewport,
};
#[cfg(feature = "terminal")]
pub use terminal::pane_state::PaneState;
