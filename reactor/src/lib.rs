#![deny(unsafe_code)]

//! `reactor` — the single-threaded event reactor (epic **E4**): the heartbeat that drives the Sprint-1
//! `transport` crate from real readiness events. One `epoll` over the NIC idle-wakeup eventfd, the
//! retransmit deadline (composed as the `epoll_wait` timeout), and the application source fds;
//! poll-then-block; sole `TransmitSink` holder, no locks. Proven on any Linux, no NIC, with real
//! `epoll`/`eventfd`/pipes and faked E2/E3 seams.
//!
//! Sprint 002, **complete** (Slices 1–4): the crate skeleton + confined syscall wrappers ([`sys`]) +
//! seam contracts ([`seam`]); the **poll-then-block core** ([`engine`], [`Reactor`]) — busy-poll the
//! RX-drain, block on the eventfd only when empty; **deadline composition** ([`compute_timeout`]) —
//! the `epoll_wait` timeout from the transport's earliest deadline + the clock seam, driving
//! `transport::poll` on a deadline wake; and **app-fd multiplexing** — token dispatch over one
//! `epoll`, a readable app fd driving a session send, and `EPOLLONESHOT` back-pressure that pauses an
//! in-flight fd and re-arms it on ACK. `Reactor::run_once` is the audited single-threaded loop body.
//!
//! ## `unsafe` confinement — compile-enforced (deny + allow, NOT forbid)
//!
//! The crate root is `#![deny(unsafe_code)]`; the **one** [`sys`] module re-`#![allow(unsafe_code)]` at
//! its top for the syscall FFI. `deny → allow` is the compile-enforced confinement: any `unsafe`
//! outside `sys` is a **compile error**, re-derivable from the module attributes. (NB: the contract
//! said *forbid* at the root — but `forbid` cannot be locally lifted by `allow` (E0453), so it is the
//! wrong keyword for "confined, not absent". `deny` is the keyword that achieves the stated intent;
//! flagged to the lead.) The loop logic above `sys` is safe Rust.

mod engine;
mod seam;
mod sys;

pub use engine::{Reactor, compute_timeout};
pub use seam::{MonotonicClock, RxDrain};
pub use sys::{Epoll, Eventfd};

/// The E3 TX-write seam **is** the transport's emission seam — the reactor holds one impl and threads
/// it (`&mut`) into every `transport` call; emission is synchronous (no deferred drain). Re-exported so
/// the reactor's seam surface is self-contained.
pub use transport::TransmitSink;
