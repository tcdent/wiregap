//! The **software simulation** of an RX descriptor ring — safe Rust, normal memory. It is the test seam
//! for the whole sprint: it lets the guards be exercised with **hostile** descriptor content (any
//! reported length, any status bits) with no NIC, fully software-provable. The **production** ring is the
//! confined real-volatile impl ([`crate::dma`], Slice 4), behind the same [`RxRing`] trait — so the
//! guards above the seam are identical for both.
//!
//! It is `pub` because it is the shared validation harness across slices (and the Slice-4 no-NIC E4
//! integration), not because it is meant for production data flow.

use std::cell::RefCell;

use crate::seam::{RxRing, TxRing};

/// A recorded seam operation — lets a test assert the **control-flow order** (e.g. copy-out precedes
/// refill, the §3.1 earliest-point order). Instrumentation of the simulation; the real ring has none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RingOp {
    /// `copy_out(slot, ..)` was called.
    CopyOut { slot: usize },
    /// `refill(slot)` was called.
    Refill { slot: usize },
}

/// One simulated descriptor + its DMA buffer contents.
#[derive(Clone, Debug, Default)]
struct SimSlot {
    /// The `DD` (descriptor-done) bit.
    done: bool,
    /// The raw, device-reported length (may be hostile — exceed the buffer).
    reported_len: u32,
    /// The raw status/error bits (may be hostile) — the full status_error DWORD.
    status: u32,
    /// The DMA buffer contents the device "wrote".
    buffer: Vec<u8>,
}

/// A simulated RX descriptor ring. `post` injects a (possibly hostile) descriptor into a slot; the
/// [`RxRing`] impl exposes the raw fields up and a bounded copy down, exactly like the real ring.
#[derive(Debug)]
pub struct SimRxRing {
    buffer_len: usize,
    slots: Vec<SimSlot>,
    /// Recorded seam ops, in call order (test instrumentation; see [`RingOp`]).
    ops: RefCell<Vec<RingOp>>,
}

impl SimRxRing {
    /// A ring of `depth` slots, each with a `buffer_len`-byte DMA buffer; all descriptors start not-done.
    pub fn new(depth: usize, buffer_len: usize) -> Self {
        Self {
            buffer_len,
            slots: vec![SimSlot::default(); depth],
            ops: RefCell::new(Vec::new()),
        }
    }

    /// The recorded seam ops in call order — for asserting control-flow order (e.g. copy-out precedes
    /// refill).
    pub fn ops(&self) -> Vec<RingOp> {
        self.ops.borrow().clone()
    }

    /// Inject a descriptor at `slot`: mark it done with the given (possibly hostile) `reported_len` /
    /// `status` and DMA-buffer `contents`. This is how tests drive adversarial input through the seam.
    pub fn post(&mut self, slot: usize, reported_len: u32, status: u32, contents: Vec<u8>) {
        self.slots[slot] = SimSlot {
            done: true,
            reported_len,
            status,
            buffer: contents,
        };
    }
}

impl RxRing for SimRxRing {
    fn depth(&self) -> usize {
        self.slots.len()
    }

    fn descriptor_done(&self, slot: usize) -> bool {
        self.slots[slot].done
    }

    fn reported_len(&self, slot: usize) -> u32 {
        self.slots[slot].reported_len
    }

    fn status(&self, slot: usize) -> u32 {
        self.slots[slot].status
    }

    fn buffer_len(&self) -> usize {
        self.buffer_len
    }

    fn copy_out(&self, slot: usize, len: usize, dst: &mut [u8]) -> usize {
        // The caller (the guard) must pass an already-bounded `len`. This assert documents the seam
        // contract and would catch a guard that ever passed the raw reported length.
        debug_assert!(len <= self.buffer_len, "copy_out len must be pre-bounded to <= buffer_len");
        self.ops.borrow_mut().push(RingOp::CopyOut { slot });
        let src = &self.slots[slot].buffer;
        let n = len.min(src.len()).min(dst.len());
        dst[..n].copy_from_slice(&src[..n]);
        n
    }

    fn refill(&mut self, slot: usize) {
        // Hand the buffer back to the device: clear DD (ready to receive again). The ring stays full.
        self.ops.borrow_mut().push(RingOp::Refill { slot });
        self.slots[slot].done = false;
    }
}

// ============================================================================
// Simulated TX ring
// ============================================================================

/// A TX descriptor slot's state in the simulation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TxSlot {
    /// Available to post a new frame into.
    Free,
    /// Posted; the device has not yet DMA'd it out (no `DD` yet).
    InFlight,
    /// The device finished DMA'ing it out (`DD`/`RS` writeback) — awaiting per-descriptor reclaim.
    Done,
}

#[derive(Clone, Debug)]
struct SimTxSlotData {
    state: TxSlot,
    /// The DMA buffer contents (app bytes copied in by `post`).
    buffer: Vec<u8>,
}

impl Default for SimTxSlotData {
    fn default() -> Self {
        Self {
            state: TxSlot::Free,
            buffer: Vec::new(),
        }
    }
}

/// A simulated TX descriptor ring — safe Rust, the software test seam for the transmit path. The pool
/// is **bounded** (`depth` slots, never grown). Tests drive the device side with [`SimTxRing::complete`]
/// (the device finishing a frame / setting `DD`). The production ring is the confined real-volatile
/// [`crate::dma`] impl (Slice 4), behind the same [`TxRing`] trait.
#[derive(Debug)]
pub struct SimTxRing {
    buffer_len: usize,
    slots: Vec<SimTxSlotData>,
}

impl SimTxRing {
    /// A ring of `depth` slots, each with a `buffer_len`-byte DMA buffer; all slots start free.
    pub fn new(depth: usize, buffer_len: usize) -> Self {
        Self {
            buffer_len,
            slots: vec![SimTxSlotData::default(); depth],
        }
    }

    /// Simulate the **device** finishing the frame at `slot` (DMA'd out, `DD` set): `InFlight` → `Done`.
    /// (A no-op if the slot is not in flight.)
    pub fn complete(&mut self, slot: usize) {
        if self.slots[slot].state == TxSlot::InFlight {
            self.slots[slot].state = TxSlot::Done;
        }
    }

    /// The state of `slot` (test inspection).
    pub fn state(&self, slot: usize) -> TxSlot {
        self.slots[slot].state
    }

    /// The DMA buffer contents at `slot` (test inspection — empty once reclaimed).
    pub fn buffer(&self, slot: usize) -> &[u8] {
        &self.slots[slot].buffer
    }

    /// How many slots are in state `st` (test inspection).
    pub fn count(&self, st: TxSlot) -> usize {
        self.slots.iter().filter(|s| s.state == st).count()
    }
}

impl TxRing for SimTxRing {
    fn depth(&self) -> usize {
        self.slots.len()
    }

    fn descriptor_done(&self, slot: usize) -> bool {
        self.slots[slot].state == TxSlot::Done
    }

    fn is_free(&self, slot: usize) -> bool {
        self.slots[slot].state == TxSlot::Free
    }

    fn post(&mut self, slot: usize, src: &[u8]) {
        debug_assert!(self.slots[slot].state == TxSlot::Free, "post into a non-free slot");
        let n = src.len().min(self.buffer_len);
        self.slots[slot].buffer = src[..n].to_vec();
        self.slots[slot].state = TxSlot::InFlight;
    }

    fn reclaim(&mut self, slot: usize) {
        debug_assert!(self.slots[slot].state == TxSlot::Done, "reclaim a non-completed slot");
        self.slots[slot].buffer = Vec::new(); // free the DMA buffer
        self.slots[slot].state = TxSlot::Free;
    }

    fn buffer_len(&self) -> usize {
        self.buffer_len
    }
}
