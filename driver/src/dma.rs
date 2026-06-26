#![allow(unsafe_code)]

//! The **one** module permitted `unsafe` — the confined real-volatile/raw-pointer DMA ring impls
//! ([`VolatileRxRing`] / [`VolatileTxRing`]). They are **thin** volatile reads/writes that carry **zero
//! guard logic**: every adversarial guard lives above the seam (`crate::rx`, Slice 1), so the same
//! guards run over the sim ring and over this one. They honor the seam's bound obligation (`copy_out`'s
//! `len` is already `<= buffer_len`; we clamp again here defensively).
//!
//! ## Confinement (gate 5)
//!
//! `unsafe` is permitted only here (crate root is `#![deny(unsafe_code)]`). The raw pointers are
//! **private** fields; **no public method returns a raw pointer or a device-backed reference** — reads
//! return owned `u32`/`u16`/`bool`/`usize`, and `copy_out` writes into the caller's owned `dst`. So the
//! confinement cannot silently leak (re-derivable from the public signatures).
//!
//! ## Device-written fields we read — the enumeration (concerns-to-watch #5)
//!
//! From the 82599 advanced RX descriptor writeback ([`AdvRxDescWb`], `reference/.../ixgbe_type.h`):
//! - `status_error` (u32) — read for `descriptor_done` (DD = bit 0) and `status` (the guard's fail-closed
//!   decision over status [19:0] + error [31:20]).
//! - `length` (u16) — read for `reported_len` (bounded by the guard before it sizes any read).
//! - the packet **buffer bytes** — read by `copy_out` (exactly the already-bounded `len`).
//! - `lo_dword` / `hi_dword` / `vlan` are device-written but **not read** by us (listed so the
//!   enumeration is complete, not just the obvious fields).
//! - TX writeback: the descriptor status `DD` bit (`descriptor_done`) — read for per-descriptor reclaim.
//!
//! ## Crossing soundness — UNDISCHARGED, bench-gated (concerns-to-watch; gate 6)
//!
//! The SAFETY comments below discharge each block's **local** preconditions (pointer validity/alignment
//! for *that* access, the length bound). They do **NOT** discharge the **device-concurrency** soundness:
//! the NIC mutates this memory outside the language model, so (a) **volatile visibility** (is a plain
//! access even sound against concurrent device writes?), (b) the **DD→read memory barrier** (are the
//! device's field writes coherently visible before we observe DD?), and (c) **no tearing** of a
//! multi-byte field/payload are **open hazards deferred to `concerns-to-watch.md`** and provable only on
//! silicon. **This module does not claim the crossing is sound.** (A SAFETY comment asserting "we observe
//! the device writes correctly" would be a defect.) The control-flow order (poll DD, only then
//! read+copy; a DD-clear descriptor is never read/delivered) is the testable half — proven on the sim
//! (gates 2/3) — and is distinct from this memory-ordering residual.

use core::ptr;

use crate::seam::{RxRing, TxRing};

/// 82599 advanced RX descriptor, **writeback** (device→host) format (`ixgbe_type.h`
/// `union ixgbe_adv_rx_desc.wb`). 16 bytes, `#[repr(C)]`. Every field is device-written ⇒ adversarial.
/// `allow(dead_code)`: `lo_dword`/`hi_dword`/`vlan` are **layout-load-bearing** (they fix the offsets of
/// the fields we do read under `#[repr(C)]`) but are not read by us — kept named so the device-field
/// enumeration above is complete, not pruned to the obvious fields.
#[allow(dead_code)]
#[repr(C)]
pub struct AdvRxDescWb {
    /// RSS type / packet info — device-written, **not read** by us.
    lo_dword: u32,
    /// RSS hash / fragment checksum — device-written, **not read** by us.
    hi_dword: u32,
    /// status [19:0] | error [31:20]. DD = bit 0, EOP = bit 1.
    status_error: u32,
    /// frame length (low 16 of the upper qword).
    length: u16,
    /// VLAN tag — device-written, **not read** by us.
    vlan: u16,
}

const RXD_STAT_DD: u32 = 1 << 0;

/// The confined real-volatile RX descriptor ring. Holds raw pointers into mapped DMA/MMIO (private). A
/// **thin** seam impl — no guard logic. Constructed by E1/E2 on real silicon; **not exercised this
/// sprint** (no NIC — the no-NIC validation uses [`crate::sim::SimRxRing`]); its crossing soundness is
/// the open residual above.
pub struct VolatileRxRing {
    desc: *mut AdvRxDescWb,
    bufs: *mut u8,
    depth: usize,
    buffer_len: usize,
    rdt: *mut u32,
}

impl VolatileRxRing {
    /// # Safety
    /// `desc` must point to `depth` contiguous, properly-aligned [`AdvRxDescWb`] in mapped DMA; `bufs`
    /// to `depth * buffer_len` contiguous bytes in mapped DMA; `rdt` to the RX-tail MMIO register. The
    /// region must stay mapped for `'static`. (Device-concurrency soundness of accessing it is the
    /// undischarged residual — see the module docs.)
    pub unsafe fn new(
        desc: *mut AdvRxDescWb,
        bufs: *mut u8,
        depth: usize,
        buffer_len: usize,
        rdt: *mut u32,
    ) -> Self {
        Self {
            desc,
            bufs,
            depth,
            buffer_len,
            rdt,
        }
    }
}

impl RxRing for VolatileRxRing {
    fn depth(&self) -> usize {
        self.depth
    }

    fn descriptor_done(&self, slot: usize) -> bool {
        debug_assert!(slot < self.depth);
        // SAFETY (local): `slot < depth`, so `desc.add(slot)` is in-bounds of the `depth`-element
        // descriptor array (new()'s precondition); `status_error` is an aligned `u32`; `read_volatile`
        // reads exactly it. (Device-concurrency visibility of this read is the undischarged residual.)
        let status = unsafe { ptr::read_volatile(ptr::addr_of!((*self.desc.add(slot)).status_error)) };
        status & RXD_STAT_DD != 0
    }

    fn reported_len(&self, slot: usize) -> u32 {
        debug_assert!(slot < self.depth);
        // SAFETY (local): as above; `length` is an aligned `u16` within the in-bounds descriptor.
        let len = unsafe { ptr::read_volatile(ptr::addr_of!((*self.desc.add(slot)).length)) };
        len as u32
    }

    fn status(&self, slot: usize) -> u32 {
        debug_assert!(slot < self.depth);
        // SAFETY (local): in-bounds aligned `u32` read (see `descriptor_done`).
        unsafe { ptr::read_volatile(ptr::addr_of!((*self.desc.add(slot)).status_error)) }
    }

    fn buffer_len(&self) -> usize {
        self.buffer_len
    }

    fn copy_out(&self, slot: usize, len: usize, dst: &mut [u8]) -> usize {
        debug_assert!(slot < self.depth);
        // Honor the seam's bound obligation defensively (the guard already passes len <= buffer_len).
        let n = len.min(self.buffer_len).min(dst.len());
        // SAFETY (local): `slot < depth` ⇒ the slot's buffer starts at `bufs + slot*buffer_len` and spans
        // `buffer_len` bytes (new()'s precondition); `n <= buffer_len`, so each `src.add(i)` (`i < n`) is
        // in-bounds; `u8` is align-1; `dst[..n]` is a valid writable region of `n` bytes. Each byte is a
        // `read_volatile`. (No-tearing / visibility of the device's payload write is the undischarged
        // residual — see the module docs; this is NOT a soundness claim against concurrent device writes.)
        unsafe {
            let src = self.bufs.add(slot * self.buffer_len);
            for (i, d) in dst.iter_mut().enumerate().take(n) {
                *d = ptr::read_volatile(src.add(i));
            }
        }
        n
    }

    fn refill(&mut self, slot: usize) {
        debug_assert!(slot < self.depth);
        // SAFETY (local): in-bounds aligned `u32` write clearing the writeback status (hands the slot back
        // to the device); `rdt` is the valid MMIO tail register. (Ordering of the descriptor write vs the
        // tail bump vs the device is part of the undischarged crossing residual — module docs.)
        unsafe {
            ptr::write_volatile(ptr::addr_of_mut!((*self.desc.add(slot)).status_error), 0);
            ptr::write_volatile(self.rdt, slot as u32);
        }
    }
}

/// 82599 advanced TX descriptor (read format we write; the device writes back DD). 16 bytes, `#[repr(C)]`.
#[repr(C)]
pub struct AdvTxDesc {
    /// buffer address (we write).
    buffer_addr: u64,
    /// command/length (we write; `RS`/`EOP`/length).
    cmd_type_len: u32,
    /// offload/status — the device writes DD back into the status byte.
    olinfo_status: u32,
}

const TXD_STAT_DD: u32 = 1 << 0;

/// The confined real-volatile TX descriptor ring. Same confinement + residual posture as the RX ring;
/// `post` copies app bytes INTO the DMA buffer (volatile), `descriptor_done` reads the TX writeback DD,
/// `reclaim` frees per-descriptor. Not exercised this sprint (no NIC).
pub struct VolatileTxRing {
    desc: *mut AdvTxDesc,
    bufs: *mut u8,
    depth: usize,
    buffer_len: usize,
    tdt: *mut u32,
}

impl VolatileTxRing {
    /// # Safety
    /// As [`VolatileRxRing::new`], for the TX descriptor array, packet buffers, and the TX-tail MMIO.
    pub unsafe fn new(
        desc: *mut AdvTxDesc,
        bufs: *mut u8,
        depth: usize,
        buffer_len: usize,
        tdt: *mut u32,
    ) -> Self {
        Self {
            desc,
            bufs,
            depth,
            buffer_len,
            tdt,
        }
    }
}

impl TxRing for VolatileTxRing {
    fn depth(&self) -> usize {
        self.depth
    }

    fn descriptor_done(&self, slot: usize) -> bool {
        debug_assert!(slot < self.depth);
        // SAFETY (local): in-bounds aligned `u32` read of the writeback status. (Visibility residual.)
        let status = unsafe { ptr::read_volatile(ptr::addr_of!((*self.desc.add(slot)).olinfo_status)) };
        status & TXD_STAT_DD != 0
    }

    fn is_free(&self, slot: usize) -> bool {
        // A slot is free once reclaimed (cmd_type_len cleared). The driver only posts into free slots.
        debug_assert!(slot < self.depth);
        // SAFETY (local): in-bounds aligned `u32` read.
        let cmd = unsafe { ptr::read_volatile(ptr::addr_of!((*self.desc.add(slot)).cmd_type_len)) };
        cmd == 0
    }

    fn post(&mut self, slot: usize, src: &[u8]) {
        debug_assert!(slot < self.depth);
        let n = src.len().min(self.buffer_len);
        // SAFETY (local): `slot < depth` ⇒ the slot's buffer spans `buffer_len`; `n <= buffer_len`, so
        // each `dstp.add(i)` (`i < n`) is in-bounds; bytes copied volatile; the descriptor fields are
        // in-bounds aligned writes. `RS` (report-status) is set so the device writes DD back. (Ordering
        // of the buffer write vs the descriptor write vs the tail bump is the undischarged residual.)
        unsafe {
            let dstp = self.bufs.add(slot * self.buffer_len);
            for (i, &b) in src.iter().take(n).enumerate() {
                ptr::write_volatile(dstp.add(i), b);
            }
            ptr::write_volatile(ptr::addr_of_mut!((*self.desc.add(slot)).buffer_addr), dstp as u64);
            ptr::write_volatile(
                ptr::addr_of_mut!((*self.desc.add(slot)).cmd_type_len),
                (n as u32) | TX_CMD_RS_EOP,
            );
            ptr::write_volatile(self.tdt, slot as u32);
        }
    }

    fn reclaim(&mut self, slot: usize) {
        debug_assert!(slot < self.depth);
        // SAFETY (local): in-bounds aligned writes clearing the descriptor (mark the slot free).
        unsafe {
            ptr::write_volatile(ptr::addr_of_mut!((*self.desc.add(slot)).cmd_type_len), 0);
            ptr::write_volatile(ptr::addr_of_mut!((*self.desc.add(slot)).olinfo_status), 0);
        }
    }

    fn buffer_len(&self) -> usize {
        self.buffer_len
    }
}

/// TX command bits: `RS` (report status, so DD is written back) | `EOP` (end of packet). Representative
/// of `IXGBE_ADVTXD_DCMD_*`.
const TX_CMD_RS_EOP: u32 = (1 << 27) | (1 << 24);
