#![allow(unsafe_code)]

//! Minimal Local APIC (xAPIC MMIO at the architectural default base `0xFEE0_0000`) — only what this
//! NIC's interrupt path needs: enable the APIC, a one-shot LAPIC timer for retransmit deadlines, a
//! self-IPI (Slice-3 stand-in for the NIC's real MSI-X, which is Sprint 6), and EOI. Not a general APIC
//! driver. The MMIO is the confined `unsafe` (deny at the crate root, `allow` here), each access a single
//! aligned 32-bit read/write of a fixed LAPIC register with a SAFETY note.

/// Architectural xAPIC MMIO base. Firmware leaves the LAPIC mapped here; we inherit that mapping.
const BASE: u64 = 0xFEE0_0000;

// Register offsets (Intel SDM vol 3, table "Local APIC Register Address Map").
const SVR: u64 = 0x0F0; // Spurious Interrupt Vector Register (bit 8 = APIC enable)
const EOI: u64 = 0x0B0; // End Of Interrupt
const ICR_LOW: u64 = 0x300; // Interrupt Command Register [31:0]
const ICR_HIGH: u64 = 0x310; // Interrupt Command Register [63:32]
const LVT_TIMER: u64 = 0x320; // LVT Timer Register
const TIMER_INIT: u64 = 0x380; // Timer Initial Count
const TIMER_CUR: u64 = 0x390; // Timer Current Count
const TIMER_DIV: u64 = 0x3E0; // Timer Divide Configuration
const LVT_MASKED: u32 = 1 << 16; // LVT mask bit

fn write(offset: u64, value: u32) {
    // SAFETY: `BASE + offset` is a fixed, naturally-aligned LAPIC MMIO register (the LAPIC is mapped by
    // firmware at the architectural default). A volatile 32-bit write to a LAPIC register affects only
    // the APIC and touches no Rust-owned memory.
    unsafe { core::ptr::write_volatile((BASE + offset) as *mut u32, value) }
}

fn read(offset: u64) -> u32 {
    // SAFETY: as `write` — a fixed, aligned LAPIC register; a volatile read has no side effect.
    unsafe { core::ptr::read_volatile((BASE + offset) as *const u32) }
}

/// Signal End-Of-Interrupt. Called from the interrupt handlers (the LAPIC base is fixed, so this needs
/// no handle).
pub fn eoi() {
    write(EOI, 0);
}

/// A handle to the enabled Local APIC. A ZST — all access is via the fixed MMIO base, so copies are free
/// and equivalent.
#[derive(Clone, Copy)]
pub struct Lapic;

impl Lapic {
    /// Enable the APIC (SVR bit 8) with spurious vector `spurious`. Idempotent.
    pub fn enable(spurious: u8) -> Self {
        write(SVR, 0x100 | spurious as u32);
        Lapic
    }

    /// The SVR value read back (bit 8 set = enabled) — for the observed report.
    pub fn svr(&self) -> u32 {
        read(SVR)
    }

    /// Configure the LAPIC timer for one-shot delivery to `vector`, divide-by-16, **disarmed**
    /// (initial count 0). `arm` starts a countdown.
    pub fn setup_oneshot_timer(&self, vector: u8) {
        write(TIMER_DIV, 0x3); // divide by 16
        write(LVT_TIMER, vector as u32); // one-shot (bits 18:17 = 00), unmasked (bit 16 = 0)
        write(TIMER_INIT, 0); // disarmed
    }

    /// Arm the one-shot timer: it counts `count` down and fires the LVT-timer vector once at zero.
    pub fn arm(&self, count: u32) {
        write(TIMER_INIT, count);
    }

    /// Deliver a fixed-mode interrupt `vector` to **this** CPU (destination shorthand = self). The
    /// Slice-3 stand-in for the NIC's MSI-X wakeup.
    pub fn self_ipi(&self, vector: u8) {
        write(ICR_HIGH, 0);
        // vector | level-assert(bit14) | dest-shorthand=self(0b01 << 18); delivery mode fixed (000).
        write(ICR_LOW, vector as u32 | (1 << 14) | (1 << 18));
    }

    /// Start the timer free-running (max count, divide-by-16, **masked** so it raises no interrupt) so
    /// [`timer_current`](Self::timer_current) can be sampled — used once for frequency calibration.
    pub fn timer_start_free_running(&self) {
        write(TIMER_DIV, 0x3);
        write(LVT_TIMER, LVT_MASKED);
        write(TIMER_INIT, 0xFFFF_FFFF);
    }

    /// The timer's current (down-counting) value.
    pub fn timer_current(&self) -> u32 {
        read(TIMER_CUR)
    }
}
