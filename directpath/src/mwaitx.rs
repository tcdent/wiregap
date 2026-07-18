#![allow(unsafe_code)]

//! AMD user-mode monitor-wait primitives (MONITORX/MWAITX), the proven Sprint-8 Slice-1 wake mechanism.
//! Shared by the event-driven [`crate::ddwaithost::DdWaitHost`] and the Slice-2 live loop.
//!
//! Pins (empirically grounded on the bench CPU, Slice 1): the MWAITX **timer-enable bit is ECX bit 1
//! (`ECX=2`)** (bit 0 is interrupt-break-when-masked); a **finite EBX (TSC cycles) on EVERY call** is the
//! hang-safety (MWAITX also wakes on the OS scheduler tick ~4ms in userspace, so the recheck-loop re-arms);
//! MWAITX has **no carry-flag**, so store-vs-timeout is decided by rechecking state + comparing the clock.

/// Arm address monitoring on the 64-byte line containing `addr` (MONITORX = 0F 01 FA; rAX=addr, ECX/EDX=0).
///
/// # Safety
/// `addr` must be a readable linear address in write-back memory (a live descriptor line here); requires a
/// WAITPKG-less AMD CPU with MONITORX (CPUID Fn8000_0001_ECX[29]) — else `#UD`.
#[inline]
pub unsafe fn monitorx(addr: u64) {
    unsafe {
        core::arch::asm!(
            "monitorx",
            in("rax") addr,
            in("rcx") 0u64,
            in("rdx") 0u64,
            options(nostack, preserves_flags),
        );
    }
}

/// Wait until a store to the monitored line, the EBX TSC-cycle timeout, or an interrupt (MWAITX = 0F 01 FB;
/// EAX=0 hints, ECX=timer-enable(bit1)=2, EBX=timeout cycles). rbx is LLVM-reserved → save/restore via a
/// scratch reg. Pass a FINITE `ebx` on EVERY call (hang-safety).
///
/// # Safety
/// Must follow a [`monitorx`] arming; `ebx` must be finite (a zero/absent timer with no store risks an
/// indefinite wait); requires MONITORX/MWAITX support (CPUID Fn8000_0001_ECX[29]) — else `#UD`.
#[inline]
pub unsafe fn mwaitx(ecx: u32, ebx: u32) {
    unsafe {
        core::arch::asm!(
            "mov {save}, rbx",
            "mov ebx, {ebxv:e}",
            "mwaitx",
            "mov rbx, {save}",
            save = out(reg) _,
            ebxv = in(reg) ebx,
            in("eax") 0u32,
            in("ecx") ecx,
            options(nostack),
        );
    }
}

/// Read the invariant TSC (nonstop_tsc/constant_tsc confirmed on the bench).
#[inline]
pub fn rdtsc() -> u64 {
    unsafe {
        let lo: u32;
        let hi: u32;
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nostack, nomem, preserves_flags));
        ((hi as u64) << 32) | lo as u64
    }
}

/// The MWAITX timer-enable ECX value (bit 1). Named so the pin is unmissable at every call site.
pub const MWAITX_TIMER: u32 = 2;

/// Convert a wall-clock `Duration` to TSC cycles at the calibrated rate.
#[inline]
pub fn duration_to_cycles(d: core::time::Duration, tsc_hz: f64) -> u64 {
    (d.as_nanos() as f64 * tsc_hz / 1e9) as u64
}
