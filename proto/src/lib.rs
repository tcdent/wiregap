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

mod errors;
mod file;
mod list_entry;
mod message;
mod net;
mod reject;
mod segment;
mod terminal;
pub mod verb;
mod wire;

pub use errors::{ErrorBody, ErrorCode, ReservedCode};
pub use file::{FileChunk, FileData, FileError, FileRead, FileWrite, List, RequestFile};
pub use list_entry::{FileType, ListEntries, ListEntry, ReservedFileType};
pub use message::{Message, decode_message, encode_message};
pub use net::{NetAddress, ReservedScheme, Scheme};
pub use reject::Reject;
pub use segment::{
    SafeSegment, SafeSegmentStack, UnsafeSegment, UnsafeSegmentStack, scan_safe, scan_unsafe,
};
pub use terminal::cell::{
    AttrBits, Cell, CursorStyleBits, ReservedCell, Rgb, RowCol, Scalar, Width,
};
pub use terminal::dimensions::Dimensions;
pub use terminal::display::{CreatePty, Display};
pub use terminal::input::{
    Button, Focus, Modifiers, Mouse, MouseKind, Paste, ReservedButton, ReservedMouseKind, Resize,
    Viewport,
};
pub use terminal::pane_state::PaneState;
pub use wire::Wire;
