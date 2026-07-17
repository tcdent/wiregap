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
    /// The buffers' **virtual** base (CPU access — `copy_out`).
    bufs: *mut u8,
    /// The buffers' **physical** base (the device DMA target written into the descriptor's `pkt_addr`).
    /// Pure-direct has no IOMMU, so virt != phys: the CPU uses `bufs`, the device uses `bufs_phys`.
    bufs_phys: u64,
    depth: usize,
    buffer_len: usize,
    rdt: *mut u32,
}

impl VolatileRxRing {
    /// # Safety
    /// `desc` must point to `depth` contiguous, properly-aligned [`AdvRxDescWb`] in mapped DMA; `bufs`
    /// to `depth * buffer_len` contiguous bytes in mapped DMA; `bufs_phys` the **physical** address of
    /// that same buffer region (pagemap-resolved, Gate-2-validated); `rdt` to the RX-tail MMIO register.
    /// The region must stay mapped for `'static`. (Device-concurrency soundness of accessing it is the
    /// undischarged residual — see the module docs.)
    pub unsafe fn new(
        desc: *mut AdvRxDescWb,
        bufs: *mut u8,
        bufs_phys: u64,
        depth: usize,
        buffer_len: usize,
        rdt: *mut u32,
    ) -> Self {
        Self {
            desc,
            bufs,
            bufs_phys,
            depth,
            buffer_len,
            rdt,
        }
    }

    /// The **physical** address of slot `slot`'s buffer (the device DMA target). Distinct from the
    /// virtual `bufs.add(slot*buffer_len)` used for CPU copy. Contiguous hugepage ⇒ base + offset.
    fn buffer_phys(&self, slot: usize) -> u64 {
        self.bufs_phys + (slot * self.buffer_len) as u64
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
        let pkt_addr = self.buffer_phys(slot);
        // SAFETY (local): `slot < depth` ⇒ `desc.add(slot)` is the in-bounds 16-byte descriptor. We write
        // its READ format — pkt_addr (u64 @ off 0) = the buffer's PHYSICAL address, hdr_addr (u64 @ off 8)
        // = 0 — which hands the slot back to the device and overwrites the writeback status (clearing DD).
        // `rdt` is the valid MMIO tail register. NEW code for the real device (the sim never needed the
        // pkt_addr write); the physaddr is the pagemap-resolved, Gate-2-validated `bufs_phys`. (Ordering
        // of the descriptor write vs the tail bump vs the device is the undischarged crossing residual.)
        unsafe {
            let d = self.desc.add(slot) as *mut u64;
            ptr::write_volatile(d, pkt_addr); // read-format pkt_addr (physical)
            ptr::write_volatile(d.add(1), 0); // read-format hdr_addr
            // RX RDT = the JUST-REFILLED slot (ixy `rx_batch`: `RDT = last_rx_index`) — NOT slot+1. As the
            // consumer advances in order, RDT tracks it monotonically, one slot behind, so RDT never meets
            // RDH (RDT==RDH reads as an empty ring). Setting slot+1 here sends RDT *backwards* the moment a
            // consumed slot is refilled after the initial arming left RDT ahead — stalling steady-state RX
            // (the Sprint-6 single-frame PoC never re-armed, so it hid this). Contrast TX, where TDT=slot+1
            // is correct (the device transmits [TDH, TDT), so the tail is one PAST the last posted).
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
    /// Buffers' **virtual** base (CPU access — the `post` byte copy).
    bufs: *mut u8,
    /// Buffers' **physical** base (the device DMA target written into the descriptor's `buffer_addr`).
    bufs_phys: u64,
    depth: usize,
    buffer_len: usize,
    tdt: *mut u32,
}

impl VolatileTxRing {
    /// # Safety
    /// As [`VolatileRxRing::new`], for the TX descriptor array, packet buffers, `bufs_phys` the buffers'
    /// physical base, and the TX-tail MMIO.
    pub unsafe fn new(
        desc: *mut AdvTxDesc,
        bufs: *mut u8,
        bufs_phys: u64,
        depth: usize,
        buffer_len: usize,
        tdt: *mut u32,
    ) -> Self {
        Self {
            desc,
            bufs,
            bufs_phys,
            depth,
            buffer_len,
            tdt,
        }
    }

    /// The **physical** address of slot `slot`'s buffer (the device DMA target). Distinct from the
    /// virtual `bufs.add(...)` used for the CPU byte copy.
    fn buffer_phys(&self, slot: usize) -> u64 {
        self.bufs_phys + (slot * self.buffer_len) as u64
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
        let buffer_addr = self.buffer_phys(slot);
        unsafe {
            // CPU byte copy via the VIRTUAL buffer pointer...
            let dstp = self.bufs.add(slot * self.buffer_len);
            for (i, &b) in src.iter().take(n).enumerate() {
                ptr::write_volatile(dstp.add(i), b);
            }
            // ...but the descriptor's DMA target is the PHYSICAL address (no IOMMU: the device reads
            // physical). Writing `dstp` (a virtual address) here would DMA to arbitrary host RAM.
            ptr::write_volatile(ptr::addr_of_mut!((*self.desc.add(slot)).buffer_addr), buffer_addr);
            // olinfo carries the payload length (PAYLEN, bits [31:14]); write it before the cmd_type_len
            // that arms the descriptor. Descriptor fields all precede the TDT doorbell below.
            ptr::write_volatile(
                ptr::addr_of_mut!((*self.desc.add(slot)).olinfo_status),
                (n as u32) << ADVTXD_PAYLEN_SHIFT,
            );
            ptr::write_volatile(
                ptr::addr_of_mut!((*self.desc.add(slot)).cmd_type_len),
                (n as u32) | TXD_CMD_TYPE,
            );
            // TDT points ONE PAST the last descriptor to transmit (next-to-fill): the device transmits
            // [TDH, TDT). Writing `slot` (not slot+1) leaves TDH==TDT for slot 0 — an empty ring the device
            // never fetches. (Wraps at `depth`.)
            ptr::write_volatile(self.tdt, ((slot + 1) % self.depth) as u32);
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

/// Advanced TX **data** descriptor command+type bits (82599, `ixgbe_type.h` `IXGBE_ADVTXD_*`), OR'd with
/// the data length in the low bits: `EOP` (end of packet) | `RS` (report status → the device writes DD
/// back) | `IFCS` (insert the Ethernet CRC) | `DEXT` (1 = advanced descriptor, not legacy) | `DTYP_DATA`
/// (advanced data descriptor). Without DEXT+DTYP_DATA the device mis-reads the descriptor as legacy;
/// without IFCS the frame is sent with no FCS. (The sim ring, being software, tolerated the earlier
/// RS|EOP placeholder; the real device does not.)
const TXD_CMD_TYPE: u32 = (1 << 24)  // EOP
    | (1 << 27)  // RS
    | (1 << 25)  // IFCS
    | (1 << 29)  // DEXT (advanced)
    | 0x0030_0000; // DTYP = advanced data descriptor
/// Advanced TX descriptor PAYLEN shift (`olinfo_status` bits [31:14]).
const ADVTXD_PAYLEN_SHIFT: u32 = 14;

#[cfg(test)]
mod virt_phys_tests {
    //! The virt/phys split (pure-direct retarget): the descriptor DMA target is the buffer's PHYSICAL
    //! address, DISTINCT from the virtual pointer used for the CPU byte copy. Proven over host memory
    //! with a synthetic physical base (no hardware). The guard/copy-boundary above the seam are frozen.
    use super::*;
    use crate::seam::{RxRing, TxRing};

    const DEPTH: usize = 4;
    const BLEN: usize = 2048;
    const SYNTH_PHYS: u64 = 0x8_4360_0000; // a synthetic hugepage physical base (!= any host VA)

    #[test]
    fn tx_post_writes_physical_buffer_addr_not_the_virtual_pointer() {
        let mut descs: Vec<AdvTxDesc> =
            (0..DEPTH).map(|_| AdvTxDesc { buffer_addr: 0, cmd_type_len: 0, olinfo_status: 0 }).collect();
        let mut bufs = vec![0u8; DEPTH * BLEN];
        let mut tdt = 0u32;
        let virt_base = bufs.as_ptr() as u64;
        let mut ring =
            unsafe { VolatileTxRing::new(descs.as_mut_ptr(), bufs.as_mut_ptr(), SYNTH_PHYS, DEPTH, BLEN, &mut tdt) };
        let slot = 2;
        ring.post(slot, b"hello");
        // the descriptor DMA target is PHYSICAL...
        assert_eq!(descs[slot].buffer_addr, SYNTH_PHYS + (slot * BLEN) as u64, "buffer_addr is physical");
        assert_ne!(descs[slot].buffer_addr, virt_base + (slot * BLEN) as u64, "NOT the virtual pointer");
        // ...while the bytes were copied to the VIRTUAL buffer.
        assert_eq!(&bufs[slot * BLEN..slot * BLEN + 5], b"hello", "copy used the virtual buffer");
        assert_eq!(tdt, ((slot + 1) % DEPTH) as u32, "tail bumped to next-to-fill (slot+1)");
        // The real-device advanced data descriptor: length in the low bits, EOP|RS|IFCS|DEXT|DTYP_DATA in
        // the command/type bits, and the payload length in olinfo PAYLEN[31:14].
        assert_eq!(descs[slot].cmd_type_len, 5u32 | TXD_CMD_TYPE, "cmd_type_len = len | advanced cmd bits");
        assert_eq!(descs[slot].cmd_type_len & (1 << 29), 1 << 29, "DEXT set (advanced, not legacy)");
        assert_eq!(descs[slot].cmd_type_len & (1 << 25), 1 << 25, "IFCS set (device appends the CRC)");
        assert_eq!(descs[slot].olinfo_status, 5u32 << ADVTXD_PAYLEN_SHIFT, "olinfo PAYLEN = payload length");
    }

    #[test]
    fn rx_refill_writes_physical_pkt_addr_new_code() {
        let mut descs: Vec<AdvRxDescWb> = (0..DEPTH)
            .map(|_| AdvRxDescWb { lo_dword: 0, hi_dword: 0, status_error: RXD_STAT_DD, length: 0, vlan: 0 })
            .collect();
        let mut bufs = vec![0u8; DEPTH * BLEN];
        let mut rdt = 0u32;
        let mut ring =
            unsafe { VolatileRxRing::new(descs.as_mut_ptr(), bufs.as_mut_ptr(), SYNTH_PHYS, DEPTH, BLEN, &mut rdt) };
        let slot = 3;
        ring.refill(slot);
        // the read-format pkt_addr (first u64 of the descriptor) = PHYSICAL, and DD is cleared.
        let pkt_addr = unsafe { ptr::read(descs.as_ptr().add(slot) as *const u64) };
        assert_eq!(pkt_addr, SYNTH_PHYS + (slot * BLEN) as u64, "pkt_addr is physical (new refill code)");
        assert!(!ring.descriptor_done(slot), "refill cleared DD (handed the slot to the device)");
        assert_eq!(rdt, slot as u32, "RX tail = the refilled slot (ixy rx_batch: RDT=last_rx_index), NOT slot+1");
    }
}
