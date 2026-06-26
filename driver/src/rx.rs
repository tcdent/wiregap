//! The RX descriptor path + the **adversarial guards** — the security heart (vector 2,
//! `grounding/data-path-and-events.md` §1). All of this lives **above** the seam ([`crate::RxRing`]):
//! it reads the raw untrusted fields the seam exposes and decides bound/drop, so the same guards run
//! whether the ring underneath is the sim ring or (Slice 4) the real volatile one.

use crate::seam::RxRing;

// The 82599 advanced-RX-descriptor `status_error` DWORD bits (status [19:0] + error [31:20]), from
// `reference/.../ixgbe/ixgbe_type.h` (`IXGBE_RXD_STAT_*` / `IXGBE_RXDADV_ERR_*`). `DD` is polled
// separately via `descriptor_done`. Slice 4 completes this real map under the fail-closed policy.

/// Descriptor-done (also surfaced via `descriptor_done`; enumerated so it is tolerated in `status`).
pub const RXD_STAT_DD: u32 = 1 << 0;
/// End-Of-Packet — single-buffer-per-frame requires it; a non-EOP descriptor is a multi-segment frame
/// we do not support → drop.
pub const RXD_STAT_EOP: u32 = 1 << 1;
const RXD_STAT_FLM: u32 = 1 << 2; // Flow-Director match
const RXD_STAT_VP: u32 = 1 << 3; // 802.1Q (VLAN) packet
const RXD_STAT_UDPCS: u32 = 1 << 4; // UDP checksum computed
const RXD_STAT_L4CS: u32 = 1 << 5; // L4 checksum computed
const RXD_STAT_IPCS: u32 = 1 << 6; // IPv4 checksum computed
const RXD_STAT_PIF: u32 = 1 << 7; // passed in-exact filter
const RXD_STAT_VEXT: u32 = 1 << 9; // dual-VLAN extension

/// The error field [31:20]: **any** bit set ⇒ drop (`IXGBE_RXDADV_ERR_*` — CRC/length/MAC/etc. errors).
const RXD_ERR_MASK: u32 = 0xFFF0_0000;

/// Every `status_error` bit we **understand** (status info bits + DD + the whole error field). A frame
/// is delivered only if EOP is set, **no error bit** is set, and **no bit outside this set** is present —
/// so any error or unknown/reserved bit **fails closed** (drop). The benign-info subset is from the
/// reference; the exact bits a *good* frame sets for our config are bench-confirmable, but the fail-closed
/// *policy* does not depend on that.
pub const RXD_ENUMERATED: u32 = RXD_STAT_DD
    | RXD_STAT_EOP
    | RXD_STAT_FLM
    | RXD_STAT_VP
    | RXD_STAT_UDPCS
    | RXD_STAT_L4CS
    | RXD_STAT_IPCS
    | RXD_STAT_PIF
    | RXD_STAT_VEXT
    | RXD_ERR_MASK;

/// **Fail-closed** delivery decision on the raw `status_error`: deliver iff EOP is set, no error-field
/// bit is set, and no bit outside [`RXD_ENUMERATED`] is present. (Non-EOP → drop; any error bit → drop;
/// any unenumerated/reserved bit → drop.)
fn deliverable(status: u32) -> bool {
    status & RXD_STAT_EOP != 0 && status & RXD_ERR_MASK == 0 && status & !RXD_ENUMERATED == 0
}

/// The RX descriptor path over a ring (the seam). Holds the read cursor (`tail`). The guards live here,
/// above the seam; the ring underneath is the sim or the real volatile impl.
pub struct RxPath<R: RxRing> {
    ring: R,
    tail: usize,
}

impl<R: RxRing> RxPath<R> {
    /// Wrap a descriptor ring, cursor at slot 0.
    pub fn new(ring: R) -> Self {
        Self { ring, tail: 0 }
    }

    /// Borrow the underlying ring (inspection / refill-state checks).
    pub fn ring(&self) -> &R {
        &self.ring
    }

    /// Mutably borrow the underlying ring (e.g. to simulate the device reusing a slot in a test).
    pub fn ring_mut(&mut self) -> &mut R {
        &mut self.ring
    }

    /// Poll the next device-done descriptor and return the next **good** frame's bounded bytes, or
    /// `None` when the next descriptor's `DD` is clear (the ring is momentarily empty).
    ///
    /// Guards, all on the raw fields the seam exposes:
    /// - **Bound:** the copy is sized `min(reported_len, BUF)` — the raw `reported_len` **never** sizes a
    ///   read; an over-read is impossible regardless of device content.
    /// - **Fail-closed status:** a non-EOP / error / unknown-bit descriptor is dropped-and-refilled and
    ///   the scan **continues** (a hostile frame must not stall the all-available drain).
    /// - **Never deref an unchecked field:** the only field that drives memory access is the bounded len.
    ///
    /// (Slice 1 yields the bounded owned copy directly; Slice 2 wraps this as `reactor::RxDrain` and adds
    /// the copy-outlives-the-slot ownership guarantee.)
    pub fn poll_one(&mut self) -> Option<Vec<u8>> {
        let depth = self.ring.depth();
        if depth == 0 {
            return None;
        }
        loop {
            let slot = self.tail;
            if !self.ring.descriptor_done(slot) {
                return None; // DD clear → end of the all-available drain
            }
            if !deliverable(self.ring.status(slot)) {
                // Hostile / non-EOP / error / unknown → drop-and-refill, skip, keep scanning.
                self.ring.refill(slot);
                self.tail = (self.tail + 1) % depth;
                continue;
            }
            // Good frame. BOUND first: the raw reported_len never sizes the copy.
            let bounded = (self.ring.reported_len(slot) as usize).min(self.ring.buffer_len());
            let mut frame = vec![0u8; bounded];
            let n = self.ring.copy_out(slot, bounded, &mut frame);
            debug_assert!(n <= bounded, "copy_out must never exceed the bounded length");
            frame.truncate(n);
            self.ring.refill(slot);
            self.tail = (self.tail + 1) % depth;
            return Some(frame);
        }
    }
}

/// The RX path **is** the E4 RX-drain seam ([`reactor::RxDrain`]): `next_frame` yields the next available
/// owned frame, skipping dropped descriptors, `None` only on `DD`-clear — the all-available-then-empty
/// contract the reactor (Sprint 2) was built against. It delegates to [`RxPath::poll_one`] (the tested
/// core), so the reactor drives the exact guarded path.
impl<R: RxRing> reactor::RxDrain for RxPath<R> {
    fn next_frame(&mut self) -> Option<Vec<u8>> {
        self.poll_one()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{RingOp, SimRxRing};
    use reactor::RxDrain;

    const BUF: usize = 64;

    fn ring() -> SimRxRing {
        SimRxRing::new(4, BUF)
    }

    /// Bound fired: an over-length descriptor (reported = 2×BUF) yields a copy sized BUF — the raw 2×BUF
    /// never sizes a read.
    #[test]
    fn over_length_descriptor_is_bounded_to_buf() {
        let mut r = ring();
        r.post(0, (2 * BUF) as u32, RXD_STAT_EOP, vec![0xAB; BUF]);
        let mut rx = RxPath::new(r);
        let frame = rx.poll_one().expect("a good (over-claimed) frame is delivered, bounded");
        assert_eq!(frame.len(), BUF, "copy bounded to BUF, not the reported 2×BUF");
        assert!(frame.iter().all(|&b| b == 0xAB));
    }

    /// A within-buffer length is delivered verbatim at that length.
    #[test]
    fn in_range_length_delivered_exactly() {
        let mut r = ring();
        r.post(0, 32, RXD_STAT_EOP, vec![0xCD; BUF]);
        let mut rx = RxPath::new(r);
        assert_eq!(rx.poll_one().unwrap().len(), 32);
    }

    /// Hostile status (an error bit) → not delivered AND the descriptor is refilled (ring stays full).
    #[test]
    fn hostile_status_dropped_and_refilled() {
        let mut r = ring();
        r.post(0, 32, RXD_STAT_EOP | (1 << 29), vec![0xAB; BUF]); // EOP + an error-field bit
        let mut rx = RxPath::new(r);
        assert_eq!(rx.poll_one(), None, "hostile-status frame not delivered");
        assert!(!rx.ring().descriptor_done(0), "the dropped descriptor was refilled (DD cleared)");
    }

    /// Non-EOP (multi-segment) → dropped-and-refilled.
    #[test]
    fn non_eop_dropped_and_refilled() {
        let mut r = ring();
        r.post(0, 32, 0, vec![0xAB; BUF]); // EOP not set
        let mut rx = RxPath::new(r);
        assert_eq!(rx.poll_one(), None);
        assert!(!rx.ring().descriptor_done(0), "refilled");
    }

    /// Fail-closed: a descriptor with EOP set but an UNENUMERATED bit is dropped (not delivered).
    #[test]
    fn fail_closed_on_unenumerated_bit() {
        let mut r = ring();
        r.post(0, 32, RXD_STAT_EOP | (1 << 8), vec![0xAB; BUF]); // bit 8: reserved/unenumerated
        let mut rx = RxPath::new(r);
        assert_eq!(rx.poll_one(), None, "unenumerated status bit fails closed → drop");
        assert!(!rx.ring().descriptor_done(0));
    }

    /// A dropped descriptor does not stall the drain: the scan skips it and delivers the next good frame.
    #[test]
    fn drop_then_deliver_next() {
        let mut r = ring();
        r.post(0, 32, 0xFFFF, vec![0xAB; BUF]); // all bits set → hostile → drop
        r.post(1, 16, RXD_STAT_EOP, vec![0xEE; BUF]); // good
        let mut rx = RxPath::new(r);
        let frame = rx.poll_one().expect("skips the dropped slot and delivers the next good frame");
        assert_eq!(frame.len(), 16);
        assert!(frame.iter().all(|&b| b == 0xEE));
    }

    /// `DD` clear → `None` (end of the all-available drain).
    #[test]
    fn dd_clear_returns_none() {
        let mut rx = RxPath::new(ring()); // fresh ring: all DD clear
        assert_eq!(rx.poll_one(), None);
    }

    // ---- Slice 2: the copy boundary + reactor::RxDrain ----------------------------------------

    /// The emitted frame is app-OWNED and DISJOINT from the ring: overwriting the slot AFTER the copy
    /// leaves the frame unchanged. (Structural: poll_one returns Vec<u8> — an owned copy, never a borrow
    /// into the ring, so no ring pointer can escape.)
    #[test]
    fn emitted_frame_is_owned_and_disjoint_from_the_ring() {
        let mut r = ring();
        r.post(0, 32, RXD_STAT_EOP, vec![0xAA; BUF]);
        let mut rx = RxPath::new(r);
        let frame = rx.poll_one().unwrap();
        assert!(frame.iter().all(|&b| b == 0xAA));

        // The device reuses the (refilled) slot and DMAs new data into it.
        rx.ring_mut().post(0, 32, RXD_STAT_EOP, vec![0xBB; BUF]);
        assert!(
            frame.iter().all(|&b| b == 0xAA),
            "the copy is a disjoint allocation — a slot overwrite does not touch it"
        );
    }

    /// Copy-out PRECEDES refill (the §3.1 earliest-point order), shown on the sim's op-log.
    #[test]
    fn copy_out_precedes_refill() {
        let mut r = SimRxRing::new(2, BUF);
        r.post(0, 32, RXD_STAT_EOP, vec![0xAA; BUF]);
        let mut rx = RxPath::new(r);
        rx.poll_one().unwrap();

        let ops = rx.ring().ops();
        let copy = ops.iter().position(|o| *o == RingOp::CopyOut { slot: 0 });
        let refill = ops.iter().position(|o| *o == RingOp::Refill { slot: 0 });
        assert!(copy.is_some() && refill.is_some(), "both ops recorded");
        assert!(copy < refill, "copy-out precedes refill (copy the frame before handing the slot back)");
    }

    /// reactor::RxDrain yields all-available owned frames, SKIPPING drops, `None` only on DD-clear.
    #[test]
    fn rxdrain_yields_all_available_then_none_skipping_drops() {
        let mut r = SimRxRing::new(4, BUF);
        r.post(0, 8, RXD_STAT_EOP, vec![0x01; BUF]); // good
        r.post(1, 8, 0xFFFF, vec![0x02; BUF]); // hostile -> drop
        r.post(2, 8, RXD_STAT_EOP, vec![0x03; BUF]); // good
        // slot 3 left not-done (DD clear) -> ends the drain
        let mut rx = RxPath::new(r);

        let f1 = rx.next_frame().expect("first good frame");
        assert!(f1.iter().all(|&b| b == 0x01));
        let f2 = rx.next_frame().expect("skips the dropped slot, yields the next good frame");
        assert!(f2.iter().all(|&b| b == 0x03));
        assert_eq!(rx.next_frame(), None, "DD clear at slot 3 -> end of the all-available drain");
    }

    /// Drop-and-refill keeps the ring full: every drained or dropped slot is refilled (DD cleared).
    #[test]
    fn drop_and_refill_keeps_the_ring_full() {
        let mut r = SimRxRing::new(3, BUF);
        r.post(0, 8, RXD_STAT_EOP, vec![0x01; BUF]);
        r.post(1, 8, 0xFFFF, vec![0x02; BUF]); // hostile
        r.post(2, 8, RXD_STAT_EOP, vec![0x03; BUF]);
        let mut rx = RxPath::new(r);
        while rx.next_frame().is_some() {}
        for slot in 0..3 {
            assert!(!rx.ring().descriptor_done(slot), "slot {slot} refilled -> ring stays full");
        }
    }

    /// Adversarial sweep: **exhaustive** over the low 16 `status_error` bits (all 65 536) × a
    /// representative set of high (error/reserved) bit patterns (u32 cannot be exhausted) × edge lengths
    /// {BUF, 2×BUF}. Each poll either drops or delivers a frame of length ≤ BUF — never a panic, never a
    /// read past BUF; delivery happens iff `deliverable(status)`. (Completion is the no-panic /
    /// no-over-read proof.)
    #[test]
    fn adversarial_sweep_never_panics_or_overreads() {
        let high_patterns = [0u32, 1 << 20, 1 << 29, 0xFFF0_0000, 0x8000_0000, 0x000F_0000];
        for low in 0u32..=u16::MAX as u32 {
            for &high in &high_patterns {
                let status = low | high;
                for &reported in &[BUF as u32, (2 * BUF) as u32] {
                    let mut r = SimRxRing::new(1, BUF);
                    r.post(0, reported, status, vec![0xAB; BUF]);
                    let mut rx = RxPath::new(r);
                    match rx.poll_one() {
                        Some(frame) => {
                            assert!(frame.len() <= BUF, "delivered frame must never exceed BUF");
                            assert!(deliverable(status));
                        }
                        None => {
                            assert!(!rx.ring().descriptor_done(0));
                            assert!(!deliverable(status));
                        }
                    }
                }
            }
        }
    }
}
