#![allow(unsafe_code)]

//! The **event-driven `Host`** — the reactor parks on the proven MWAITX-on-DD wake mechanism (Sprint 8
//! Slice 1) instead of the bounded-poll sleep. This is the backing below the frozen `Host` seam
//! (`now` + `block`); the reactor CORE (`step`/`park`/`pump_all`) and the `Host` trait are unchanged.
//!
//! **`block` waits on the NIC's own memory write** (`grounding/kernel-bypass-and-dma.md §4`): it MONITORXes
//! the 64-byte line of the **next-expected RX descriptor** — `(RDT+1) % depth`, located via the software
//! tail register `RDT` (MMIO), NOT the device head `RDH` (which races ahead of an unconsumed frame), so no
//! coupling to the frozen `RxPath` cursor — rechecks the `DD` bit (check-before-wait), then MWAITXes. The
//! NIC's DD store wakes it (→ [`WakeReason::NicMsix`]); the retransmit deadline is a
//! **self-owned bounded wait** (MWAITX's native EBX TSC timer, `ECX=2`, **finite on every call** incl.
//! `block(None)`) — MWAITX has no carry-flag, so the deadline is decided by `rdtsc() >= deadline`
//! (→ [`WakeReason::Timer`]). An OS-tick / spurious wake with neither DD nor the deadline re-arms (the
//! recheck loop). The `DD` read here is a **wakeup hint**; the reactor's `step()` does the authoritative
//! Acquire read (visibility != the monitor-wake — Slice 1).

use crate::mwaitx::{duration_to_cycles, monitorx, mwaitx, rdtsc, MWAITX_TIMER};
use core::ptr;
use core::time::Duration;
use reactor::{Host, WakeReason};
use std::time::Instant as StdInstant;
use transport::Instant;

/// Event-driven substrate seam: MONITORX/MWAITX on the next-expected RX descriptor's DD line.
pub struct DdWaitHost {
    start: StdInstant,
    /// Virtual base of the RX descriptor ring (the MONITORX target = base + slot*16, line-aligned).
    rx_desc_base: u64,
    /// The device's RX tail register (`RDT`) in the mapped BAR — the software's last-refilled descriptor;
    /// the next-expected descriptor is `(RDT+1) % depth` (not the device head `RDH`, which races ahead of
    /// an unconsumed frame).
    rdt: *const u32,
    depth: usize,
    tsc_hz: f64,
    /// Finite hang-safety EBX (cycles) — the ceiling wait per MWAITX call, incl. `block(None)`.
    max_ebx: u64,
}

impl DdWaitHost {
    /// `rx_desc_base` = `Layout::rx_desc_base_va()`; `rdt` = `datapath::rx_tail_ptr(bar)`; `depth` the ring
    /// depth; `tsc_hz` the calibrated TSC rate. `max_ebx_ms` bounds each idle wait (the re-check floor); a
    /// frame or the deadline wakes it sooner.
    pub fn new(rx_desc_base: u64, rdt: *const u32, depth: usize, tsc_hz: f64, max_ebx_ms: f64) -> Self {
        Self {
            start: StdInstant::now(),
            rx_desc_base,
            rdt,
            depth,
            tsc_hz,
            max_ebx: (tsc_hz * max_ebx_ms / 1e3) as u64,
        }
    }

    /// DD (bit 0 of `status_error` at byte 8) of descriptor `slot` — a raw hint read (the reactor's
    /// `step()` is the authoritative Acquire gate).
    #[inline]
    fn dd(&self, slot: usize) -> bool {
        let se = unsafe { ptr::read_volatile((self.rx_desc_base + (slot * 16 + 8) as u64) as *const u32) };
        se & 1 != 0
    }
}

impl Host for DdWaitHost {
    fn now(&self) -> Instant {
        Instant(self.start.elapsed().as_nanos() as u64)
    }

    fn block(&mut self, timeout: Option<Duration>) -> WakeReason {
        let deadline = timeout.map(|d| rdtsc() + duration_to_cycles(d, self.tsc_hz));
        loop {
            // Next-expected descriptor = (RDT+1) % depth (the consumer's next; never races ahead of an
            // unconsumed frame). Arm its line, then recheck DD (check-before-wait).
            let slot = (unsafe { ptr::read_volatile(self.rdt) } as usize + 1) % self.depth;
            let mon = self.rx_desc_base + (slot * 16) as u64;
            unsafe { monitorx(mon) };
            if self.dd(slot) {
                return WakeReason::NicMsix; // a frame is already there — do not park
            }
            // Finite EBX on EVERY call: bounded by the deadline if any, else the hang-safety ceiling.
            let ebx = match deadline {
                Some(dl) => dl.saturating_sub(rdtsc()).min(self.max_ebx).max(1) as u32,
                None => self.max_ebx as u32,
            };
            unsafe { mwaitx(MWAITX_TIMER, ebx) };
            if self.dd(slot) {
                return WakeReason::NicMsix; // the NIC's DD store woke us
            }
            match deadline {
                // Deadline reached (no CF: clock-compare is authoritative) → drive the retransmit.
                Some(dl) if rdtsc() >= dl => return WakeReason::Timer,
                // Deadline not yet due (an OS-tick / EBX / spurious wake) → re-arm and keep waiting.
                Some(_) => continue,
                // No deadline (ParkForever): return after one bounded low-power wait so the reactor gets
                // control back and re-polls — honors the Host contract (block(None) must not park forever,
                // else it misses the give-up/idle re-check). CPU stays idle in MWAITX (non-polling); a DD
                // store still wakes it immediately (handled above).
                None => return WakeReason::NicMsix,
            }
        }
    }
}
