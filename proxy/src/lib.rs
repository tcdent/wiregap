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

/// Sprint 20 — the upstream credential in one place: the compile-time descriptor
/// (`CredentialSource`) and the runtime value (`Credential`) with its resolution policy, co-located
/// so the one secret in the system is readable in a single sitting.
pub mod credential;

pub mod register;

pub mod address;

pub mod route;

pub mod anthropic;

pub mod dispatch;

pub mod nic;

pub mod upstream;

/// Sprint 18 — the verified TLS transport for the upstream call: a dumb, fixed pipe with no HTTP
/// knowledge, so the response can be relayed verbatim (`net.md` §3) rather than parsed and re-shaped.
/// **Outbound, client-side only** — the proxy is a TLS client to the upstream and never a TLS server;
/// the box terminates its own TLS and the project wire carries plaintext (`cloud/approach.md`).
/// Sprint 20 — the dial: the concrete Anthropic client implementing `upstream`'s seam over
/// `upstream_tls`'s verified pipe. Depends on `upstream`; `upstream` does not depend on it.
pub mod upstream_dial;

pub mod upstream_tls;

#[cfg(unix)]
pub mod head;

pub mod lifecycle;

pub mod cloud_relay;

pub mod terminal_relay;

#[cfg(unix)]
pub mod fs_read;

#[cfg(unix)]
pub mod fs_write;

pub mod leaf;

#[cfg(unix)]
pub mod open_discipline;

#[cfg(unix)]
pub mod privilege;
