//! The TX path — implements `transport::TransmitSink` (the E4/E5 TX-write seam). `emit` copies app-owned
//! bytes into a (sim) DMA buffer per emit and posts a descriptor; per-descriptor cleanup reclaims
//! completed descriptors (freed on TX `DD`, no batch-stranding).
//!
//! **`emit` is infallible by construction** (the committed `TransmitSink::emit` returns `()`): the DMA
//! TX pool is **bounded** to ring depth (>= peak in-flight K, K = concurrent sessions; provisional,
//! bench-gated `data-path-and-events.md` §6). It is **never grown**. A **full** ring **drops** the frame
//! and relies on retransmit — the absent ACK fires the sender's timer (`architecture.md` §7), reusing
//! the RX-overrun→retransmit machinery. Dropping is the *only* non-failing option: `emit` cannot error
//! (no `Result`), block (single-threaded reactor), panic, or grow the pool.
//!
//! **Reclamation split (R5):** the **DMA TX buffer is TX-completion-bound** — freed on TX `DD`, a
//! ring-depth pool. The **ACK-hold** lifetime is the *transport's* app-owned pool (Sprint-1 `Rc`
//! buffers), re-copied into a fresh DMA buffer on each retransmit; it is **not** the DMA pool.

use crate::seam::TxRing;

/// The TX path over a TX ring (the seam). The reactor holds one of these as its `TransmitSink` and
/// threads it into every `transport` call (the sole-writer property, compile-enforced by `&mut`).
pub struct TxPath<T: TxRing> {
    ring: T,
    /// Count of frames dropped because the ring was full (back-pressure → retransmit). Observability.
    dropped: u64,
}

impl<T: TxRing> TxPath<T> {
    /// Wrap a TX ring.
    pub fn new(ring: T) -> Self {
        Self { ring, dropped: 0 }
    }

    /// Borrow the ring (inspection / test).
    pub fn ring(&self) -> &T {
        &self.ring
    }

    /// Mutably borrow the ring (e.g. to simulate device completion in a test).
    pub fn ring_mut(&mut self) -> &mut T {
        &mut self.ring
    }

    /// Frames dropped because the ring was full (each is recovered by the sender's retransmit).
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// **Per-descriptor** cleanup: reclaim every completed descriptor (freed on TX `DD`), each
    /// independently — no batch-stranding (contrast ixy's "clean the whole batch or nothing").
    fn reclaim_completed(&mut self) {
        for slot in 0..self.ring.depth() {
            if self.ring.descriptor_done(slot) {
                self.ring.reclaim(slot);
            }
        }
    }

    /// The first free slot, if any.
    fn first_free(&self) -> Option<usize> {
        (0..self.ring.depth()).find(|&slot| self.ring.is_free(slot))
    }
}

impl<T: TxRing> transport::TransmitSink for TxPath<T> {
    /// Emit one L1 frame: reclaim completed descriptors (free DMA on `DD`), then copy the app-owned
    /// bytes into a free DMA buffer and post. A **full** ring (no free slot after cleanup) **drops** —
    /// infallible by construction (no error/block/panic/grow); the absent ACK drives the retransmit.
    fn emit(&mut self, frame: &[u8]) {
        self.reclaim_completed();
        match self.first_free() {
            Some(slot) => self.ring.post(slot, frame),
            None => self.dropped += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{SimTxRing, TxSlot};
    use transport::TransmitSink;

    const BUF: usize = 64;

    /// The DMA buffer is freed on TX `DD` (not before, and with no ACK in the picture — the driver has
    /// no ACK input): a not-completed slot is never reclaimed; a completed one is reclaimed on the next
    /// emit's cleanup.
    #[test]
    fn dma_buffer_freed_on_tx_dd() {
        let mut tx = TxPath::new(SimTxRing::new(2, BUF));
        tx.emit(b"f1"); // slot0 in-flight
        tx.emit(b"f2"); // slot1 in-flight; slot0 NOT done → not reclaimed
        assert_eq!(tx.ring().state(0), TxSlot::InFlight, "not freed without TX DD");
        assert_eq!(tx.ring().state(1), TxSlot::InFlight);

        // The device completes BOTH (sets DD). No ACK is involved — the driver has no ACK input.
        tx.ring_mut().complete(0);
        tx.ring_mut().complete(1);

        // The next emit's per-descriptor cleanup reclaims both (freed on DD); f3 reuses slot0.
        tx.emit(b"f3");
        assert_eq!(tx.ring().state(1), TxSlot::Free, "slot1 freed on its TX DD, not reused");
        assert!(tx.ring().buffer(1).is_empty(), "freed DMA buffer cleared");
        assert_eq!(tx.ring().buffer(0), b"f3", "slot0 freed on DD then reused for f3");
    }

    /// A retransmit re-copies from the app-pool into a FRESH DMA buffer: emitting the same app bytes
    /// again produces another independent DMA copy (the DMA buffer is per-emit, the app buffer is the
    /// transport's).
    #[test]
    fn retransmit_recopies_into_a_fresh_dma_buffer() {
        let mut tx = TxPath::new(SimTxRing::new(2, BUF));
        let app_frame = b"payload"; // the transport's app-owned (Rc) buffer, reused across retransmits
        tx.emit(app_frame); // slot0
        tx.emit(app_frame); // retransmit (slot0 still in flight) → fresh DMA copy in slot1
        assert_eq!(tx.ring().buffer(0), app_frame);
        assert_eq!(tx.ring().buffer(1), app_frame, "the retransmit is an independent DMA copy");
        assert_eq!(tx.ring().count(TxSlot::InFlight), 2);
    }

    /// A FULL ring drops without panic/block/grow: emit beyond depth → the extra frame is dropped, the
    /// ring stays at `depth` slots, all original frames remain in flight.
    #[test]
    fn full_ring_drops_without_panic_or_grow() {
        let mut tx = TxPath::new(SimTxRing::new(2, BUF));
        tx.emit(b"a"); // slot0
        tx.emit(b"b"); // slot1 — ring now full (nothing completed → nothing to reclaim)
        assert_eq!(tx.dropped(), 0);

        tx.emit(b"c"); // full → DROP (no panic, no block, no grow)
        assert_eq!(tx.dropped(), 1, "the over-capacity frame was dropped");
        assert_eq!(tx.ring().depth(), 2, "the pool did not grow");
        assert_eq!(tx.ring().count(TxSlot::InFlight), 2, "both original frames still in flight");
        assert_eq!(tx.ring().buffer(0), b"a");
        assert_eq!(tx.ring().buffer(1), b"b");
    }

    /// Per-descriptor cleanup, no stranding: with some descriptors complete and some not, the next emit
    /// reclaims ONLY the completed ones — incomplete neighbours are not stranded (contrast ixy's
    /// clean-whole-batch-or-nothing).
    #[test]
    fn per_descriptor_cleanup_no_stranding() {
        let mut tx = TxPath::new(SimTxRing::new(4, BUF));
        for f in [b"0".as_slice(), b"1", b"2", b"3"] {
            tx.emit(f);
        }
        assert_eq!(tx.ring().count(TxSlot::InFlight), 4);

        // The device completes slots 0 and 2 only (1 and 3 still in flight).
        tx.ring_mut().complete(0);
        tx.ring_mut().complete(2);

        // Next emit's cleanup reclaims 0 and 2 independently; 1 and 3 are NOT stranded.
        tx.emit(b"4");
        assert_eq!(tx.ring().state(1), TxSlot::InFlight, "slot 1 not stranded by 0/2's cleanup");
        assert_eq!(tx.ring().state(3), TxSlot::InFlight, "slot 3 not stranded");
        // 0 and 2 were freed (one reused for "4"); exactly one of them is now in flight with "4".
        let reused = (tx.ring().buffer(0) == b"4") ^ (tx.ring().buffer(2) == b"4");
        assert!(reused, "exactly one freed slot (0 or 2) was reused for the new frame");
    }
}
