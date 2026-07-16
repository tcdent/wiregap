//! The self-pinning boot banner — **pure formatting, no I/O**, so it is host `cargo test`-able (harness
//! surface (a)). The UEFI entry writes the produced lines to COM1 after `ExitBootServices`; a host test
//! writes them to a `String` and asserts the exact bytes. Same inputs → same bytes.
//!
//! "Self-pinning" = the banner carries the binary's identity (`build_id`, `rustc`) so a captured serial
//! log names the exact source it came from; the observed handoff facts (`rsdp`, the usable-memory
//! summary) are the evidence that the handoff actually happened. Values are printed as **observed**,
//! never as a bare PASS.

use core::fmt::Write;

use crate::arena::{ArenaLayout, Region};
use crate::ecam::{Bar, MsixCap, CMD_BUS_MASTER, CMD_MEMORY_SPACE};
use crate::interrupts::VectorPlan;
use crate::reactor::WakeReason;

/// The facts the banner reports: the compile-time identity + what the handoff observed.
pub struct BootFacts<'a> {
    /// `git` short SHA (+ `-dirty`), from `build.rs`.
    pub build_id: &'a str,
    /// The exact `rustc` that built the binary, from `build.rs`.
    pub rustc: &'a str,
    /// The ACPI RSDP physical address read from the UEFI config table (0 if absent — a failure signal).
    pub rsdp: u64,
    /// Count of `CONVENTIONAL` (usable) memory-map regions observed at handoff.
    pub usable_regions: u64,
    /// Total usable pages (4 KiB each) across those regions.
    pub usable_pages: u64,
}

impl BootFacts<'_> {
    /// The Slice-1 acceptance predicate, computed from observed facts only: the handoff produced a live
    /// ACPI root (so ECAM/DMAR are reachable in Slice 2 / Sprint 5) and real usable memory (so the
    /// Slice-2 arena has somewhere to live). Serial working is proven by the banner arriving at all.
    pub fn handoff_ok(&self) -> bool {
        self.rsdp != 0 && self.usable_regions > 0 && self.usable_pages > 0
    }
}

/// Write the self-pinning banner + the observed Slice-1 assertions to `w`. Pure over `facts`.
pub fn write_banner<W: Write>(w: &mut W, facts: &BootFacts) -> core::fmt::Result {
    writeln!(w, "==== SUBSTRATE slice-1 : UEFI handoff ====")?;
    writeln!(w, "build-id : {}", facts.build_id)?;
    writeln!(w, "rustc    : {}", facts.rustc)?;
    writeln!(w, "handoff  : ExitBootServices reached; COM1 serial live post-exit")?;
    writeln!(w, "acpi-rsdp: {:#018x}  (kept: MCFG->ECAM in slice 2, DMAR->sprint 5)", facts.rsdp)?;
    writeln!(
        w,
        "memmap   : usable-regions={} usable-pages={} (~{} MiB)",
        facts.usable_regions,
        facts.usable_pages,
        facts.usable_pages * 4 / 1024,
    )?;
    // Observed -> expected, not a bare PASS: each line states what was observed and what was required.
    writeln!(w, "assert   : rsdp-nonzero          observed={:#018x} expected!=0", facts.rsdp)?;
    writeln!(w, "assert   : usable-regions>0      observed={} expected>0", facts.usable_regions)?;
    writeln!(w, "assert   : usable-pages>0        observed={} expected>0", facts.usable_pages)?;
    writeln!(w, "RESULT   : {}", if facts.handoff_ok() { "PASS" } else { "FAIL" })?;
    writeln!(w, "==== end banner ====")?;
    Ok(())
}

/// Observed ECAM facts for this NIC's BDF (Slice 2). `command` is COMMAND read back *after* the enable.
pub struct EcamFacts {
    pub ecam_base: u64,
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor: u16,
    pub device: u16,
    pub bar0: Bar,
    pub command: u16,
    pub msix: Option<MsixCap>,
    /// The device model actually stood up in QEMU differs from the bench 82599 — surfaced, not hidden.
    pub model_note: &'static str,
}

impl EcamFacts {
    /// ECAM is sound for Slice 2 when: we addressed an Intel NIC, its BAR0 is firmware-assigned
    /// (nonzero), Bus-Master + Memory-Space are enabled, and MSI-X is present.
    pub fn ecam_ok(&self) -> bool {
        self.vendor == 0x8086
            && self.bar0.base != 0
            && self.command & (CMD_BUS_MASTER | CMD_MEMORY_SPACE) == (CMD_BUS_MASTER | CMD_MEMORY_SPACE)
            && self.msix.is_some()
    }
}

/// Write the observed ECAM report (observed → expected).
pub fn write_ecam<W: Write>(w: &mut W, f: &EcamFacts) -> core::fmt::Result {
    writeln!(w, "---- slice-2 : ECAM (this device) ----")?;
    writeln!(w, "note     : {}", f.model_note)?;
    writeln!(w, "ecam-base: {:#012x}  bdf={:02x}:{:02x}.{}", f.ecam_base, f.bus, f.dev, f.func)?;
    writeln!(
        w,
        "assert   : vendor:device       observed={:#06x}:{:#06x} expected-vendor=0x8086",
        f.vendor, f.device
    )?;
    writeln!(
        w,
        "assert   : bar0                 observed={:#014x} ({}bit{}) expected!=0",
        f.bar0.base,
        if f.bar0.is_64bit { "64" } else { "32" },
        if f.bar0.prefetchable { ",pref" } else { "" },
    )?;
    let bm = f.command & CMD_BUS_MASTER != 0;
    let ms = f.command & CMD_MEMORY_SPACE != 0;
    writeln!(
        w,
        "assert   : command             observed={:#06x} bus-master={} mem-space={} expected both set",
        f.command, bm, ms
    )?;
    match f.msix {
        Some(m) => writeln!(
            w,
            "assert   : msix-cap            observed=present @cap={:#04x} size={} bir={} table-off={:#x} expected present",
            m.cap_offset, m.table_size, m.table_bir, m.table_offset
        )?,
        None => writeln!(w, "assert   : msix-cap            observed=ABSENT expected present")?,
    }
    writeln!(w, "RESULT   : {}", if f.ecam_ok() { "PASS" } else { "FAIL" })?;
    Ok(())
}

/// Write the observed arena report (observed → expected). `usable` is the UEFI-usable region the
/// extent was carved from.
pub fn write_arena<W: Write>(w: &mut W, layout: &ArenaLayout, usable: &Region) -> core::fmt::Result {
    writeln!(w, "---- slice-2 : physical arena (this NIC's DMA) ----")?;
    writeln!(w, "usable   : {:#014x}..{:#014x} (UEFI CONVENTIONAL)", usable.base, usable.end())?;
    let e = &layout.extent;
    writeln!(w, "extent   : {:#014x}..{:#014x} len={:#x}", e.base, e.end(), e.len)?;
    let names = ["rx-ring", "tx-ring", "rx-bufs", "tx-bufs"];
    for (name, r) in names.iter().zip(layout.regions().iter()) {
        writeln!(w, "  {name} : {:#014x}..{:#014x} len={:#x}", r.base, r.end(), r.len)?;
    }
    writeln!(
        w,
        "assert   : extent-in-usable    observed={} expected true",
        layout.extent.within(usable)
    )?;
    writeln!(w, "assert   : all-within-extent   observed={} expected true", layout.all_within_extent())?;
    writeln!(w, "assert   : no-overlap          observed={} expected true", layout.no_overlap())?;
    writeln!(
        w,
        "assert   : dmar-confinable     observed={} (extent 2MiB-aligned, whole DMAR pages) expected true",
        layout.dmar_confinable()
    )?;
    writeln!(w, "RESULT   : {}", if layout.sound(usable) { "PASS" } else { "FAIL" })?;
    Ok(())
}

/// Observed interrupt facts (Slice 3): the vector policy, the LAPIC state, and the before/after handler
/// counters proving the self-IPI and the one-shot timer were taken (and that `hlt` woke).
pub struct InterruptFacts {
    pub plan: VectorPlan,
    pub svr: u32,
    pub msix_before: u32,
    pub msix_after: u32,
    pub timer_before: u32,
    pub timer_after: u32,
    pub woke_past_hlt: bool,
}

impl InterruptFacts {
    pub fn apic_enabled(&self) -> bool {
        self.svr & 0x100 != 0
    }
    pub fn msix_ran(&self) -> bool {
        self.msix_after > self.msix_before
    }
    pub fn timer_ran(&self) -> bool {
        self.timer_after > self.timer_before
    }
    /// Interrupts are sound for Slice 3 when: the vectors are valid, the APIC is enabled, the self-IPI
    /// reached the NIC-MSI-X handler, and the one-shot timer fired its handler and woke the CPU from hlt.
    pub fn ints_ok(&self) -> bool {
        self.plan.is_valid() && self.apic_enabled() && self.msix_ran() && self.timer_ran() && self.woke_past_hlt
    }
}

/// Write the observed interrupt report (observed → expected).
pub fn write_interrupts<W: Write>(w: &mut W, f: &InterruptFacts) -> core::fmt::Result {
    writeln!(w, "---- slice-3 : interrupts (IDT/LAPIC/timer, this NIC only) ----")?;
    writeln!(
        w,
        "vectors  : nic-msix={:#04x} timer={:#04x} spurious={:#04x}",
        f.plan.nic_msix, f.plan.timer, f.plan.spurious
    )?;
    writeln!(w, "assert   : vector-plan-valid   observed={} expected true", f.plan.is_valid())?;
    writeln!(
        w,
        "assert   : apic-enabled        observed={} (svr={:#06x}, bit8) expected true",
        f.apic_enabled(),
        f.svr
    )?;
    writeln!(
        w,
        "assert   : self-IPI->msix-vec  observed=ran({}->{}) expected ran",
        f.msix_before, f.msix_after
    )?;
    writeln!(
        w,
        "assert   : timer->handler+wake observed=fired({}->{}) woke-past-hlt={} expected both",
        f.timer_before, f.timer_after, f.woke_past_hlt
    )?;
    writeln!(w, "RESULT   : {}", if f.ints_ok() { "PASS" } else { "FAIL" })?;
    Ok(())
}

/// Observed reactor facts (Slice 4): the clean no_std reactor's ratified behavior, driven on the real
/// substrate (calibration + the five behavioral proofs).
pub struct ReactorFacts {
    pub tsc_khz: u64,
    pub lapic_khz: u64,
    /// Zero-parks-while-work: frames processed in a continuous burst, and the `hlt` count across it (0).
    pub burst_processed: u32,
    pub burst_hlts: u32,
    /// One-in-flight back-pressure: a second frame held while a delivery is in flight, then released.
    pub bp_held: bool,
    pub bp_released: bool,
    /// Deadline park → the one-shot timer woke it (hlt actually occurred) → the retransmit was driven.
    pub deadline_wake: WakeReason,
    pub deadline_parked: bool,
    pub retransmit_emitted: bool,
    /// The MSI-X wakeup latch (self-IPI stand-in) → the woke-by class is NIC-MSI-X.
    pub msix_wake: WakeReason,
}

impl ReactorFacts {
    pub fn reactor_ok(&self) -> bool {
        self.burst_processed > 0
            && self.burst_hlts == 0
            && self.bp_held
            && self.bp_released
            && self.deadline_wake == WakeReason::Timer
            && self.deadline_parked
            && self.retransmit_emitted
            && self.msix_wake == WakeReason::NicMsix
    }
}

/// Write the observed reactor report (observed → expected).
pub fn write_reactor<W: Write>(w: &mut W, f: &ReactorFacts) -> core::fmt::Result {
    writeln!(w, "---- slice-4 : clean no_std reactor on the substrate ----")?;
    writeln!(w, "calib    : tsc={} kHz  lapic-timer={} kHz (div16)", f.tsc_khz, f.lapic_khz)?;
    writeln!(
        w,
        "assert   : zero-park-while-work  observed=processed({}) hlts({}) expected hlts=0",
        f.burst_processed, f.burst_hlts
    )?;
    writeln!(
        w,
        "assert   : rx-back-pressure      observed=held({}) released({}) expected both true",
        f.bp_held, f.bp_released
    )?;
    writeln!(
        w,
        "assert   : deadline->timer->retx observed=parked({}) wake({:?}) retransmit({}) expected parked/Timer/true",
        f.deadline_parked, f.deadline_wake, f.retransmit_emitted
    )?;
    writeln!(
        w,
        "assert   : msix-wakeup-class     observed=wake({:?}) expected NicMsix",
        f.msix_wake
    )?;
    writeln!(w, "RESULT   : {}", if f.reactor_ok() { "PASS" } else { "FAIL" })?;
    Ok(())
}
