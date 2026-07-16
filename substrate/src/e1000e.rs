#![allow(unsafe_code)]

//! A **minimal** e1000e (82574L) TX DMA inducer — Slice-2 **only**, to induce the DMAR confinement test
//! (a device DMA to an in- vs out-of-arena buffer). This is NOT a driver: it programs just enough of the
//! transmit path (enable TX, point the ring, write one legacy descriptor, bump the tail) to make the
//! device DMA a buffer at a chosen address. The real 82599 driver is Sprint 6. Confined `unsafe` MMIO.
//!
//! The descriptor **ring** must be at an IOMMU-mapped (in-arena) address — the device DMA-reads it — so
//! only the descriptor's **buffer** address varies between the in-arena (succeeds) and out-of-arena
//! (faults) cases, isolating the test to the buffer DMA.

use core::ptr::{read_volatile, write_volatile};

// e1000e register offsets (from BAR0).
const TCTL: u64 = 0x0400; // Transmit Control
const TDBAL: u64 = 0x3800; // TX descriptor base low
const TDBAH: u64 = 0x3804; // TX descriptor base high
const TDLEN: u64 = 0x3808; // TX descriptor ring length (bytes)
const TDH: u64 = 0x3810; // TX descriptor head
const TDT: u64 = 0x3818; // TX descriptor tail
const TCTL_EN: u32 = 1 << 1; // transmit enable

const DESC_SIZE: u64 = 16;
// Legacy TX descriptor CMD: EOP | IFCS | RS (report status so DD is written back).
const CMD_EOP_IFCS_RS: u8 = (1 << 0) | (1 << 1) | (1 << 3);
const STA_DD: u8 = 1 << 0; // descriptor done

/// Bound on the DD poll so a faulting (never-completing) DMA cannot wedge the boot.
const POLL_MAX: u32 = 1 << 22;

/// The e1000e device, addressed by its BAR0 MMIO base.
pub struct E1000e {
    bar0: u64,
}

impl E1000e {
    pub fn new(bar0: u64) -> Self {
        Self { bar0 }
    }

    fn w(&self, off: u64, val: u32) {
        // SAFETY: `bar0 + off` is a fixed, aligned e1000e register (BAR0 from config space); a volatile
        // write affects only the device.
        unsafe { write_volatile((self.bar0 + off) as *mut u32, val) }
    }

    /// Point the TX ring at `ring_phys` (must be IOMMU-mapped, i.e. in-arena) and enable transmit.
    pub fn init_tx(&self, ring_phys: u64, ring_len: u32) {
        self.w(TDBAL, ring_phys as u32);
        self.w(TDBAH, (ring_phys >> 32) as u32);
        self.w(TDLEN, ring_len);
        self.w(TDH, 0);
        self.w(TDT, 0);
        self.w(TCTL, TCTL_EN);
    }

    /// Induce one TX DMA: write a descriptor at `ring_phys[slot]` pointing at `buf_addr`, bump TDT, and
    /// poll the descriptor's DD write-back. Returns whether the descriptor completed (DD set). NOTE:
    /// e1000e sets DD even when the buffer read faulted at the IOMMU — it completes the descriptor via the
    /// in-arena ring write-back regardless. The caller reads the IOMMU's FRCD fault, not this return, as
    /// the block signal; this is reported only as info.
    pub fn induce_tx(&self, ring_phys: u64, slot: u64, buf_addr: u64, len: u16) -> bool {
        let desc = ring_phys + slot * DESC_SIZE;
        // SAFETY: `desc` is in the in-arena, identity-mapped ring region we own; write the 16-byte legacy
        // TX descriptor (buffer address, length, command) with the status byte cleared.
        unsafe {
            write_volatile(desc as *mut u64, buf_addr);
            write_volatile((desc + 8) as *mut u16, len);
            write_volatile((desc + 10) as *mut u8, 0); // cso
            write_volatile((desc + 11) as *mut u8, CMD_EOP_IFCS_RS);
            write_volatile((desc + 12) as *mut u8, 0); // status (cleared)
            write_volatile((desc + 13) as *mut u8, 0); // css
            write_volatile((desc + 14) as *mut u16, 0); // special
        }
        self.w(TDT, (slot + 1) as u32);

        let mut n = 0;
        loop {
            // SAFETY: reading the status byte of the descriptor we wrote, in our ring region.
            let sta = unsafe { read_volatile((desc + 12) as *const u8) };
            if sta & STA_DD != 0 {
                return true;
            }
            n += 1;
            if n >= POLL_MAX {
                return false;
            }
            core::hint::spin_loop();
        }
    }
}
