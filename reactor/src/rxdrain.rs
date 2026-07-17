//! The RX-drain seam ([`RxDrain`], the E3 driver's output seam — **kept byte-identical** from the prior
//! reactor crate, so the driver's `impl RxDrain` and the seam-diff stay frozen) and the
//! [`RxDrainAsSource`] **adapter** that presents a `RxDrain` as the fresh reactor's [`RxSource`].
//!
//! ## Why an adapter (the ruled reconciliation, Sprint 6 Slice 3)
//!
//! The driver impls `RxDrain::next_frame -> Option<Vec<u8>>` (OWNED — the §3.1 copy-out happens *at the
//! driver*), while the reactor consumes [`RxSource`] (`has_frame` + `next_frame -> Option<&[u8]>`). The
//! adapter bridges them WITHOUT editing either frozen half.
//!
//! ## The impedance mismatch (the sharp one)
//!
//! [`RxSource::has_frame`] is a **non-consuming** `&self` peek — the reactor's back-pressure
//! (`can_pull_rx`) gates the pull on it, and after an empty poll the reactor parks and later re-checks
//! `has_frame`, so it MUST stay live (report a frame that arrived after the last empty poll). But
//! [`RxDrain`] offers ONLY a consuming `next_frame(&mut)` — no peek. So the adapter carries a **one-frame
//! lookahead**: `has_frame` pulls one owned frame into `lookahead` (idempotently — never a double-pull),
//! and reports from it; `next_frame` hands that frame out. Because a fresh pull is what makes `has_frame`
//! live, the pull happens *inside* `has_frame` via interior mutability — but it only ever writes
//! `lookahead`, **never** `front` (the buffer the reactor's borrow points into). Splitting the one buffer
//! into `lookahead` (peek/pull slot) + `front` (stable borrow slot) is what lets `has_frame` stay live
//! AND keeps the reactor's borrow from ever moving under it.
//!
//! ## Structural §3.1 preservation
//!
//! `RxDrain::next_frame` returns an OWNED `Vec<u8>`, so the adapter only ever holds/lends app-owned
//! memory — a device-memory borrow leaking into the reactor is COMPILE-IMPOSSIBLE, not a discipline.
//! Cost: one app-memory→app-memory copy (driver `Vec` → reactor `pending`), which does not touch the
//! §3.1 DMA crossing; a post-PoC seam reconciliation can drop it.

extern crate alloc;
use crate::reactor::RxSource;
use alloc::vec::Vec;
use core::cell::RefCell;

/// The RX-drain seam: the next available frame as OWNED bytes (the driver copies out of DMA at this
/// boundary — device-writable memory structurally cannot flow past it; `architecture.md §3.1`).
pub trait RxDrain {
    /// The next available frame as owned bytes, or `None` if none is currently available.
    fn next_frame(&mut self) -> Option<Vec<u8>>;
}

/// Presents a [`RxDrain`] as a [`RxSource`] via a one-frame lookahead. See the module docs for why the
/// pull lives in `has_frame` and why the buffer is split in two.
pub struct RxDrainAsSource<D: RxDrain> {
    /// Behind a `RefCell` so `has_frame`/`fill` (both logically `&self` peeks) can pull.
    drain: RefCell<D>,
    /// The peeked-ahead owned frame: filled by [`Self::fill`] (pull-if-empty), consumed by `next_frame`.
    /// NOT the buffer the reactor borrows — refilling it never moves a live borrow.
    lookahead: RefCell<Option<Vec<u8>>>,
    /// Stable app-owned storage for the borrow handed to the reactor; overwritten ONLY by `next_frame`.
    front: Vec<u8>,
}

impl<D: RxDrain> RxDrainAsSource<D> {
    pub fn new(drain: D) -> Self {
        Self { drain: RefCell::new(drain), lookahead: RefCell::new(None), front: Vec::new() }
    }

    /// Pull one frame into the lookahead if it is empty. Idempotent — a full lookahead is never
    /// re-pulled (no double-pull, no dropped frame), and an empty drain simply leaves it `None`. Keeps
    /// the peek live: after an empty poll a later-arriving frame is pulled on the next call.
    fn fill(&self) {
        let mut la = self.lookahead.borrow_mut();
        if la.is_none() {
            *la = self.drain.borrow_mut().next_frame();
        }
    }
}

impl<D: RxDrain> RxSource for RxDrainAsSource<D> {
    fn has_frame(&self) -> bool {
        self.fill();
        self.lookahead.borrow().is_some()
    }

    fn next_frame(&mut self) -> Option<&[u8]> {
        self.fill(); // correct even if called without a preceding has_frame; idempotent
        let frame = self.lookahead.borrow_mut().take()?; // the pre-pulled owned frame (or None if empty)
        self.front = frame; // stable app-owned storage — the ONLY write to `front`
        Some(&self.front)
    }
}
