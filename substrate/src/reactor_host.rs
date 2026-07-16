#![allow(unsafe_code)]

//! The bare-metal [`Host`] impl — the seam between the pure reactor and the substrate. This is the one
//! place the LAPIC one-shot timer, `hlt`, the MSI-X latch, and the TSC clock meet the reactor's
//! two-method seam; the reactor loop itself stays pure. `now` reads the TSC (scaled by calibration);
//! `block` arms the one-shot timer for the deadline and parks with `hlt`, honouring the MSI-X latch so a
//! wakeup is never lost, and reports the woke-by class. The only `unsafe` is `rdtsc`, confined here.

use crate::calibrate::Calibration;
use crate::interrupts::{MSIX_PENDING, TIMER_TICKS};
use crate::lapic::Lapic;
use crate::reactor::{Host, WakeReason};
use core::sync::atomic::Ordering;
use core::time::Duration;
use transport::Instant;

fn rdtsc() -> u64 {
    // SAFETY: `rdtsc` reads the timestamp counter; no memory effect, no preconditions.
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// Drives the reactor on the real substrate. Holds the LAPIC handle, the calibration, the TSC epoch, and
/// the timer vector to arm.
pub struct SubstrateHost {
    lapic: Lapic,
    cal: Calibration,
    tsc_base: u64,
    timer_vector: u8,
    hlts: u32,
}

impl SubstrateHost {
    /// The caller must have already `setup_oneshot_timer(timer_vector)` on `lapic` (unmasked one-shot)
    /// after calibration.
    pub fn new(lapic: Lapic, cal: Calibration, timer_vector: u8) -> Self {
        Self { lapic, cal, tsc_base: rdtsc(), timer_vector, hlts: 0 }
    }

    /// Actual `hlt` parks — the gate observable: **zero while there is work**.
    pub fn hlts(&self) -> u32 {
        self.hlts
    }

    /// Convert a deadline delta to a LAPIC one-shot count (divide-by-16 domain), clamped to a live range.
    fn lapic_count(&self, d: Duration) -> u32 {
        // count = ms * lapic_khz. Sub-millisecond deadlines still arm at least one tick.
        let ms = d.as_millis() as u64;
        let count = ms.saturating_mul(self.cal.lapic_khz);
        count.clamp(1, u32::MAX as u64) as u32
    }
}

impl Host for SubstrateHost {
    fn now(&self) -> Instant {
        let ticks = rdtsc().wrapping_sub(self.tsc_base) as u128;
        // ns = ticks * 1_000_000 / tsc_khz  (u128 avoids the ×1e6 overflow).
        let ns = ticks * 1_000_000u128 / self.cal.tsc_khz as u128;
        Instant(ns as u64)
    }

    fn block(&mut self, timeout: Option<Duration>) -> WakeReason {
        // Lost-wakeup latch: a MSI-X that arrived since the last block returns immediately (no park).
        if MSIX_PENDING.swap(false, Ordering::SeqCst) {
            return WakeReason::NicMsix;
        }
        let ticks0 = TIMER_TICKS.load(Ordering::SeqCst);
        if let Some(d) = timeout {
            self.lapic.setup_oneshot_timer(self.timer_vector);
            self.lapic.arm(self.lapic_count(d));
        }

        self.hlts += 1;
        // Atomic sti;hlt so a wakeup between enabling interrupts and halting is not missed; then re-mask.
        x86_64::instructions::interrupts::enable_and_hlt();
        x86_64::instructions::interrupts::disable();

        // Classify the wake (the woke-by class).
        if MSIX_PENDING.swap(false, Ordering::SeqCst) {
            WakeReason::NicMsix
        } else if TIMER_TICKS.load(Ordering::SeqCst) != ticks0 {
            WakeReason::Timer
        } else {
            // Spurious wake (e.g. a stray interrupt) — treat as a NIC wake so the loop re-drains.
            WakeReason::NicMsix
        }
    }
}
