//! The descriptor-ring **seam** — drawn **above** the guards.
//!
//! It exposes the **raw, untrusted** device-written fields *up* (`reported_len`, `status`) to the shared
//! guard logic ([`crate::RxPath`]); the only *down*-call is a **bounded-length** copy ([`RxRing::copy_out`],
//! whose `len` the caller has already bounded to `<= buffer_len()`). The sim ring ([`crate::sim`], safe)
//! and the Slice-4 real volatile ring ([`crate::dma`], confined unsafe) both implement it. **All** guard
//! logic lives above this trait; the real impl is a thin volatile read with **zero** guard logic.

/// An RX descriptor ring. Every value it returns from a device-written field (`reported_len`, `status`,
/// and the buffer bytes via `copy_out`) is **adversary-influenced** (`grounding/data-path-and-events.md`
/// §1, `kernel-bypass-and-dma.md` §2 vector 2) — the guards above this trait treat it as hostile.
pub trait RxRing {
    /// Number of descriptor slots (the ring depth).
    fn depth(&self) -> usize;

    /// Whether the descriptor at `slot` is device-done (the `DD` bit). Polling only — no payload read.
    fn descriptor_done(&self, slot: usize) -> bool;

    /// The **raw, untrusted** device-reported frame length at `slot`. May exceed `buffer_len()`; the
    /// guard bounds it before any read — it never sizes a copy directly.
    fn reported_len(&self, slot: usize) -> u32;

    /// The **raw, untrusted** status/error bits at `slot` — the full 82599 advanced-RX-descriptor
    /// `status_error` DWORD (status [19:0] + error [31:20]). The guard decides delivery fail-closed over
    /// all of it. (Widened from `u16` to `u32` in Slice 4 to carry the real device field faithfully — a
    /// `u16` would drop the error bits and defeat fail-closed.)
    fn status(&self, slot: usize) -> u32;

    /// The fixed per-slot DMA buffer length (`BUF`) — the bound ceiling.
    fn buffer_len(&self) -> usize;

    /// Copy at most `len` bytes from `slot`'s DMA buffer into `dst`, returning the count copied. The
    /// caller passes an **already-bounded** `len` (`<= buffer_len()`); the impl must never read past it.
    /// (Real impl: a confined volatile/raw copy; sim impl: a safe `memcpy`.)
    fn copy_out(&self, slot: usize, len: usize, dst: &mut [u8]) -> usize;

    /// Refill `slot` — hand the buffer back to the device and clear `DD` (ready to receive again).
    fn refill(&mut self, slot: usize);
}

/// A TX descriptor ring (the transmit seam). The DMA buffer pool is **bounded** (ring-depth, >= peak
/// in-flight K) — never grown; a full ring drops (back-pressure is the absent ACK). The sim ring
/// ([`crate::sim`], safe) and the Slice-4 real volatile ring ([`crate::dma`], confined unsafe) both
/// implement it. `transport::TransmitSink::emit` (infallible) drives it through [`crate::TxPath`].
pub trait TxRing {
    /// Number of descriptor slots (the ring depth / pool size — the bound).
    fn depth(&self) -> usize;

    /// Is the descriptor at `slot` device-complete (the `RS`/`DD` writeback)? — ready to reclaim.
    fn descriptor_done(&self, slot: usize) -> bool;

    /// Is `slot` free (no in-flight or completed frame) — available to post into?
    fn is_free(&self, slot: usize) -> bool;

    /// Copy `src` (app-owned bytes) into `slot`'s DMA buffer and post the descriptor (`RS` set) for the
    /// device to DMA out. `slot` must be free. (Real impl: a confined volatile copy + descriptor write.)
    fn post(&mut self, slot: usize, src: &[u8]);

    /// Reclaim a **completed** descriptor: free its DMA buffer and mark `slot` free. **Per-descriptor**
    /// (no batch-stranding) — freed on TX `DD`, independent of any ACK.
    fn reclaim(&mut self, slot: usize);

    /// The fixed per-slot DMA buffer length.
    fn buffer_len(&self) -> usize;
}
