#![no_std]
#![deny(unsafe_code)]

//! `reactor` — the single-threaded event reactor (epic **E4**): the heartbeat that drives the Sprint-1
//! `transport` crate from real readiness events. The **ratified behavior, built fresh** and `no_std`
//! ([`reactor`]): busy-drain-then-block ([`Reactor::pump_all`](reactor::Reactor::pump_all) drains all
//! ready work, then the caller parks on the seam when idle) over a minimal two-method substrate seam
//! ([`Host`]: `now` + `block`), a frame source ([`RxSource`]) and the sole sink
//! (`transport::TransmitSink`); deadline composition to a one-shot timer arm; RX back-pressure as
//! stop-and-wait flow control. The loop is pure (unsafe-free, generic over the seam), so its decision
//! logic is host-tested (`tests/reactor.rs`).
//!
//! ## The RX seam and the driver adapter ([`rxdrain`])
//!
//! The E3 driver's output seam is [`RxDrain`] (`next_frame -> Option<Vec<u8>>`, OWNED — the §3.1
//! copy-out is *at the driver*). The reactor consumes [`RxSource`] (a non-consuming `has_frame` peek +
//! a borrowing `next_frame`). [`RxDrainAsSource`] adapts the former to the latter with a one-frame
//! lookahead, without editing either frozen half; because `RxDrain` returns owned bytes, a
//! device-memory borrow reaching the reactor is compile-impossible.
//!
//! `#![no_std]`: the reactor core is generic over the `Host` seam and touches no OS — the substrate impl
//! lives outside it. The live impl is the pure-direct **`DdWaitHost`** (in `directpath`): a userspace
//! process on running Linux that parks by waiting on the NIC's own memory write — **MONITORX/MWAITX on
//! the RX descriptor `DD` line** (event-driven, no kernel interrupt, no APIC/IDT) — NOT a bare-metal
//! LAPIC/`hlt`/MSI-X substrate. `alloc` supplies `Vec` for the frame buffers; the crate root is
//! `#![deny(unsafe_code)]` — the whole crate is safe Rust (the `unsafe` substrate lives behind `Host`).

extern crate alloc;

pub mod reactor;
pub mod rxdrain;

pub use reactor::{
    Arm, Host, Reactor, RxSource, WakeReason, arm_from_deadline, can_pull_rx, capped_deadline,
};
pub use rxdrain::{RxDrain, RxDrainAsSource};

/// The E3 TX-write seam **is** the transport's emission seam — the reactor holds one impl and threads
/// it (`&mut`) into every `transport` call; emission is synchronous (no deferred drain). Re-exported so
/// the reactor's seam surface is self-contained.
pub use transport::TransmitSink;
