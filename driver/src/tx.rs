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
    /// The **monotonic produce cursor** — the next slot to post (the software TDT position). The device
    /// consumes TDH→TDT in ring order, so the tail MUST advance monotonically; posting at the lowest free
    /// slot (out-of-order reclaim frees a low slot while TDH is ahead) would write TDT *backwards* of TDH
    /// and the device would stop transmitting / re-run stale descriptors. Wraps at ring depth.
    produce: usize,
    /// Count of frames dropped because the ring was full (back-pressure → retransmit). Observability.
    dropped: u64,
}

impl<T: TxRing> TxPath<T> {
    /// Wrap a TX ring.
    pub fn new(ring: T) -> Self {
        Self { ring, produce: 0, dropped: 0 }
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

}

impl<T: TxRing> transport::TransmitSink for TxPath<T> {
    /// Emit one L1 frame: reclaim completed descriptors (free DMA on `DD`), then post at the **monotonic
    /// produce cursor** (the software TDT) and advance it. If the cursor's slot is still in flight (the
    /// ring is full — the device has not caught up), **drop** — infallible by construction (no
    /// error/block/panic/grow); the absent ACK drives the retransmit. Posting strictly at the advancing
    /// cursor (never the lowest free slot) keeps the ring tail monotonic, as the device requires.
    fn emit(&mut self, frame: &[u8]) {
        self.reclaim_completed();
        if self.ring.is_free(self.produce) {
            self.ring.post(self.produce, frame);
            self.produce = (self.produce + 1) % self.ring.depth();
        } else {
            self.dropped += 1; // ring full at the cursor → drop; do NOT skip to a lower free slot
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

    /// Out-of-order reclaim must NOT send the produce cursor backwards: a HIGHER slot (1) completes while
    /// a LOWER one (0) is still in flight; the next emit posts at the MONOTONIC cursor (slot 2), never the
    /// freed low slot (1) that a lowest-free-slot policy would pick. On real hardware, posting slot 1 with
    /// TDT=2 while TDH is already at 2 writes TDT *backwards* and stalls the ring — the Slice-1 TDT-backwards
    /// bug this cursor fixes. (The sim has no TDH/TDT; the fix is observable as WHICH slot is posted.)
    #[test]
    fn out_of_order_reclaim_keeps_the_produce_cursor_monotonic() {
        let mut tx = TxPath::new(SimTxRing::new(4, BUF));
        tx.emit(b"f0"); // slot0
        tx.emit(b"f1"); // slot1 (both in flight; cursor now at 2)
        tx.ring_mut().complete(1); // the HIGHER slot completes first; slot0 still in flight
        tx.emit(b"f2"); // reclaims slot1, but posts at the MONOTONIC cursor (slot2), NOT the freed slot1
        assert_eq!(tx.ring().buffer(2), b"f2", "posted at the monotonic cursor (slot2)");
        assert_eq!(tx.ring().state(0), TxSlot::InFlight, "slot0 still in flight");
        assert_eq!(tx.ring().state(1), TxSlot::Free, "slot1 reclaimed but NOT reused (no backwards post)");
        assert!(tx.ring().buffer(1).is_empty(), "the reclaimed low slot left free, not overwritten");
    }

    /// The produce cursor wraps at depth: after a full cycle of completions, the next emit reuses slot 0.
    #[test]
    fn produce_cursor_wraps_at_depth() {
        let mut tx = TxPath::new(SimTxRing::new(4, BUF));
        for f in [b"0".as_slice(), b"1", b"2", b"3"] {
            tx.emit(f); // fills 0..3, cursor wraps back to 0
        }
        for s in 0..4 {
            tx.ring_mut().complete(s); // the device consumes the whole ring
        }
        tx.emit(b"4"); // cursor at 0 (wrapped) → reuses slot0
        assert_eq!(tx.ring().buffer(0), b"4", "cursor wrapped to slot0");
        assert_eq!(tx.dropped(), 0, "no drops across the wrap");
    }
}
