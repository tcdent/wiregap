#![deny(unsafe_code)]

//! `proxy` — the security-invariant core (Sprint 5, `docs/teams/proxy/005-*.md`): the total
//! functions that *are* the containment guarantee, built as pure functions over box bytes with
//! the one exception of U1's open-discipline syscall surface.
//!
//! Each accepted Sprint-4 spec becomes a built, certified module:
//! - [`leaf`] — U1, the filesystem write-path **leaf-validator** (`spec/leaf-validator.md`):
//!   `scan_safe` (proto) composed with the array-level element-count bound. **Pure**, no I/O.
//! - [`open_discipline`] — U1's second half (`spec/filesystem-write-path.md` §1): the real
//!   `openat2`/`mkdirat` confined-open primitives. The **one** module carrying `unsafe`
//!   (crate root is `deny(unsafe_code)`; that module re-`allow`s it — RQ7 Option A).
//!
//! U2 (address recognizer), U3 (route matcher), U4 (Anthropic recognizer), U5 (Rust-DSL
//! register) arrive in later slices, all pure. The **only** `unsafe` in the crate is
//! [`open_discipline`]'s syscall surface — every recognizer is safe Rust, testable without I/O.
//!
//! Certification discipline (`docs/teams/proxy/005-*.md`, CB1–CB6): each unit's proof is an
//! enumeration/corpus against an **oracle independent of the code under test** — the proto
//! `segment_alphabet.rs` model (stdlib oracle, never the code's own output).

extern crate alloc;

pub mod vocab;

pub mod register;

pub mod address;

pub mod route;

pub mod anthropic;

pub mod dispatch;

pub mod cloud_relay;

pub mod terminal_relay;

#[cfg(unix)]
pub mod fs_read;

pub mod leaf;

#[cfg(unix)]
pub mod open_discipline;
