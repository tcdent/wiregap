#![no_std]
#![deny(unsafe_code)]

//! `reactor` — the single-threaded event reactor (epic **E4**): the heartbeat that drives the Sprint-1
//! `transport` crate from real readiness events. The **ratified behavior, built fresh** and `no_std`
//! ([`reactor`]): poll-then-block over a minimal two-method substrate seam ([`Host`]: `now` + `block`),
//! a frame source ([`RxSource`]) and the sole sink (`transport::TransmitSink`); deadline composition to
//! a one-shot timer arm; RX back-pressure as stop-and-wait flow control. The loop is pure
//! (unsafe-free, generic over the seam), so its decision logic is host-tested (`tests/reactor.rs`).
//!
//! ## The RX seam and the driver adapter ([`rxdrain`])
//!
//! The E3 driver's output seam is [`RxDrain`] (`next_frame -> Option<Vec<u8>>`, OWNED — the §3.1
//! copy-out is *at the driver*). The reactor consumes [`RxSource`] (a non-consuming `has_frame` peek +
//! a borrowing `next_frame`). [`RxDrainAsSource`] adapts the former to the latter with a one-frame
//! lookahead, without editing either frozen half; because `RxDrain` returns owned bytes, a
//! device-memory borrow reaching the reactor is compile-impossible.
//!
//! `#![no_std]`: the reactor runs on the bare-metal substrate (its `Host` impl owns the LAPIC timer /
//! `hlt` / MSI-X). `alloc` supplies `Vec` for the frame buffers; the crate root is
//! `#![deny(unsafe_code)]` — the whole crate is safe Rust (the `unsafe` substrate lives behind `Host`).

extern crate alloc;

pub mod reactor;
pub mod rxdrain;

pub use reactor::{
    Arm, Host, Reactor, RxSource, WakeReason, arm_from_deadline, can_pull_rx,
};
pub use rxdrain::{RxDrain, RxDrainAsSource};

/// The E3 TX-write seam **is** the transport's emission seam — the reactor holds one impl and threads
/// it (`&mut`) into every `transport` call; emission is synchronous (no deferred drain). Re-exported so
/// the reactor's seam surface is self-contained.
pub use transport::TransmitSink;
