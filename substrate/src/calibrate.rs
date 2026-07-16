#![allow(unsafe_code)]

//! One-shot frequency calibration against the PIT (8254), whose input clock is a fixed 1.193182 MHz.
//! The reactor's clock ([`Host::now`](crate::reactor::Host::now)) reads the TSC, and its deadline arm
//! programs the LAPIC one-shot timer — both need a tick↔time mapping, measured here by timing a known
//! PIT interval and reading the TSC + LAPIC-timer deltas across it. Rough is fine: `now` and the arm
//! derive from the same measurement, so the deadline→fire causality holds. The `unsafe` is the PIT/`0x61`
//! port I/O + `rdtsc`, confined here with SAFETY notes.

use crate::lapic::Lapic;
use x86_64::instructions::port::Port;

/// PIT input frequency (Hz), fixed by hardware.
const PIT_HZ: u64 = 1_193_182;
/// Calibration window (ms). 10 ms → a PIT count of 11932, well within the 16-bit counter.
const CAL_MS: u64 = 10;

/// Measured frequencies in kHz. `lapic_khz` is in the **divide-by-16** domain the one-shot arm uses.
#[derive(Clone, Copy, Debug)]
pub struct Calibration {
    pub tsc_khz: u64,
    pub lapic_khz: u64,
}

fn rdtsc() -> u64 {
    // SAFETY: `rdtsc` reads the timestamp counter; no memory effect, no preconditions.
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// Calibrate the TSC and the LAPIC timer against one PIT interval. Interrupts should be disabled by the
/// caller so the window is not perturbed. Leaves the LAPIC timer free-running/masked — the caller
/// re-configures it (`setup_oneshot_timer`) before use.
pub fn calibrate(lapic: &Lapic) -> Calibration {
    let count = (PIT_HZ * CAL_MS / 1000) as u16;
    lapic.timer_start_free_running();

    // SAFETY: 0x43/0x42 are the PIT command/channel-2 ports and 0x61 is the NMI-status/timer-2-gate
    // port — fixed legacy I/O. We program channel 2 (mode 0) for a one-shot `count`, gate it, and poll
    // its output bit. No memory is touched.
    unsafe {
        let mut nmi_ctrl = Port::<u8>::new(0x61);
        let mut pit_cmd = Port::<u8>::new(0x43);
        let mut pit_ch2 = Port::<u8>::new(0x42);

        // Gate low + speaker off (clear bits 0,1) before loading the count.
        let base = nmi_ctrl.read() & 0xFC;
        nmi_ctrl.write(base);
        pit_cmd.write(0xB0); // channel 2, lobyte/hibyte, mode 0 (interrupt on terminal count), binary
        pit_ch2.write((count & 0xFF) as u8);
        pit_ch2.write((count >> 8) as u8);

        // Raise the gate → counting starts; bracket the wait with TSC + LAPIC samples.
        nmi_ctrl.write(base | 0x01);
        let tsc0 = rdtsc();
        let lapic0 = lapic.timer_current();

        // Mode 0: the output (bit 5) goes high at terminal count.
        while nmi_ctrl.read() & 0x20 == 0 {
            core::hint::spin_loop();
        }

        let tsc1 = rdtsc();
        let lapic1 = lapic.timer_current();
        nmi_ctrl.write(base); // gate low again

        let tsc_ticks = tsc1.wrapping_sub(tsc0);
        let lapic_ticks = lapic0.wrapping_sub(lapic1) as u64; // counts down
        Calibration {
            tsc_khz: (tsc_ticks / CAL_MS).max(1),
            lapic_khz: (lapic_ticks / CAL_MS).max(1),
        }
    }
}
