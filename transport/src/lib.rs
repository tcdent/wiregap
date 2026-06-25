#![no_std]
#![forbid(unsafe_code)]

//! `transport` — the box↔proxy session protocol (epic **E5**): a UART byte pipe plus exactly two
//! additions, a per-frame ACK and a session ID. Built as **pure software** against three seams a
//! later data path (E3) and reactor (E4) implement, so every invariant is provable with a virtual
//! clock and a fake wire — no hardware, no syscalls in this crate's logic.
//!
//! Sprint 001, **complete** (Slices 1–4): the L1 frame codec ([`frame`]) and seam definitions
//! ([`seam`]); the session table + open-by-emission + ID allocator ([`table`], [`allocator`]); the
//! **sender role** ([`sender`]) — stop-and-wait, retransmit-on-silence, timeout-only give-up; and the
//! **receiver role** (emitting bare ACKs) + the interleaving ready-set, both on [`SessionTable`]. The
//! closed-loop self-validation (two instances, opposite roles, over a fake wire + virtual clock) lives
//! in `tests/closed_loop.rs`.
//!
//! **`nic` owns the L1 frame.** The cross-component conflict with `proto` was reconciled at proto
//! sprint 010 ("excise the L1 envelope"): `proto` is now L2 `[verb][body]` only and holds no session
//! id, so there is exactly one session-id framing layer — here. The frame's `payload` carries proto's
//! L2 bytes opaquely; this crate never inspects them.
//!
//! The crate root pins the auditable-surface boundary the way `proto` does: `#![no_std]` + `alloc`
//! makes "no syscalls in the crate logic" a *compile* property (`std::{net,fs,process,thread,time}`
//! cannot link), and `#![forbid(unsafe_code)]` forecloses the pointer arithmetic an over-read would
//! need — the codec is safe Rust end to end and total over arbitrary input.

extern crate alloc;

// Test-only: the built-in test harness links `std`. Gated to `cfg(test)`, so the audited (non-test)
// library build stays `no_std`.
#[cfg(test)]
extern crate std;

mod allocator;
mod frame;
mod seam;
mod sender;
mod table;

pub use allocator::IdAllocator;
pub use frame::{Frame, FrameError, SESSION_ID_LEN, SessionId, decode_frame, encode_frame};
pub use seam::{Clock, Instant, TransmitSink};
pub use sender::{DEFAULT_GIVE_UP_AFTER, DEFAULT_RETRANSMIT_INTERVAL, SendError};
pub use table::{Delivery, Session, SessionTable};
