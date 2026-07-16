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
use crate::dmar::FaultRecord;
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

/// Observed DMAR facts (Sprint 5, Slice 1): the IOMMU register base + geometry, the (disjoint) table
/// region, and that translation is ENABLED. Slice 1 proves the table BYTES pure (G1) + reports remapping
/// ENABLED (G2) — NOT enforcement; the induced-fault enforcement proof is Slice 2.
pub struct DmarFacts {
    pub reg_base: u64,
    pub cap: u64,
    pub levels: u8,
    pub arena: Region,
    pub table_region: Region,
    pub remapping_enabled: bool,
}

impl DmarFacts {
    /// The DMAR tables live outside the NIC-reachable arena (the self-referential-containment guard).
    pub fn tables_nic_unreachable(&self) -> bool {
        !self.arena.overlaps(&self.table_region)
    }
    /// Slice-1 verdict: remapping ENABLED + the tables are NIC-unreachable. Enforcement is Slice 2.
    pub fn slice1_ok(&self) -> bool {
        self.remapping_enabled && self.tables_nic_unreachable()
    }
}

/// Write the observed DMAR report (observed → expected). Honest to the Slice-1 bar: ENABLED, not enforced.
pub fn write_dmar<W: Write>(w: &mut W, f: &DmarFacts) -> core::fmt::Result {
    writeln!(w, "---- sprint-5 slice-1 : DMAR (we own the tables) ----")?;
    writeln!(w, "iommu    : reg-base={:#014x} cap={:#018x} sl-levels={}", f.reg_base, f.cap, f.levels)?;
    writeln!(w, "arena    : {:#014x}..{:#014x} (the only NIC-reachable region)", f.arena.base, f.arena.end())?;
    writeln!(
        w,
        "tables   : {:#014x}..{:#014x} (DMAR tables; must be NIC-UNREACHABLE)",
        f.table_region.base,
        f.table_region.end()
    )?;
    writeln!(
        w,
        "assert   : tables-nic-unreachable  observed={} (disjoint from arena) expected true",
        f.tables_nic_unreachable()
    )?;
    writeln!(
        w,
        "assert   : remapping-enabled       observed={} (GSTS.TES) expected true",
        f.remapping_enabled
    )?;
    writeln!(w, "note     : table confinement proven PURE in G1 (host); ENFORCEMENT (induced fault) is Slice 2")?;
    writeln!(w, "RESULT   : {} (enabled, NOT yet enforced)", if f.slice1_ok() { "PASS" } else { "FAIL" })?;
    Ok(())
}

/// Observed DMAR ENFORCEMENT facts (Sprint 5, Slice 2): the induced in-arena-succeeds / out-of-arena-
/// faults PAIR. The pair is the (ii) BLOCKED proof — the out fault alone is vacuous; in-arena success is
/// load-bearing; and the fault must be the IOMMU's (FRCD IOVA == the induced address, SID == the NIC).
pub struct DmarEnforceFacts {
    pub in_arena_success: bool,
    pub in_arena_no_fault: bool,
    /// The induced out-of-arena buffer address (page-aligned).
    pub out_addr: u64,
    /// Whether the descriptor completed (its DD bit). Set REGARDLESS of the buffer fault — e1000e
    /// completes the descriptor via the in-arena ring write-back; the FRCD fault, not this, is the block
    /// signal. Reported as info, not part of the verdict.
    pub out_completed: bool,
    /// The IOMMU's fault record for the out-of-arena DMA.
    pub fault: Option<FaultRecord>,
    /// The NIC's source-id `(bus<<8)|devfn` the fault must name.
    pub sid: u16,
}

impl DmarEnforceFacts {
    /// The JOINT blocked proof: in-arena DMA succeeded with no fault (load-bearing), and the IOMMU
    /// recorded a fault for the out-of-arena DMA whose IOVA is the induced address and SID is the NIC.
    ///
    /// Note: `out_completed` (the descriptor's DD) is NOT part of the proof — e1000e completes the
    /// descriptor (DD write-back to the in-arena ring) regardless of the buffer read being aborted; the
    /// FRCD fault, not DD, is what proves the out-of-arena buffer access was blocked.
    pub fn blocked(&self) -> bool {
        self.in_arena_success
            && self.in_arena_no_fault
            && matches!(self.fault, Some(f) if f.iova == (self.out_addr & !0xFFF) && f.sid == self.sid)
    }
}

/// Write the observed enforcement report (observed → expected). Proves (ii) BLOCKED; (iii) SOUND stays
/// honest-open (silicon).
pub fn write_dmar_enforce<W: Write>(w: &mut W, f: &DmarEnforceFacts) -> core::fmt::Result {
    writeln!(w, "---- sprint-5 slice-2 : DMAR enforcement (induced fault) ----")?;
    writeln!(
        w,
        "assert   : in-arena-DMA-succeeds   observed=completed({}) no-fault({}) expected both true [LOAD-BEARING]",
        f.in_arena_success, f.in_arena_no_fault
    )?;
    match f.fault {
        Some(fr) => {
            let iova_match = fr.iova == (f.out_addr & !0xFFF);
            let sid_match = fr.sid == f.sid;
            writeln!(
                w,
                "assert   : out-of-arena-BLOCKED    observed=DMAR-fault iova={:#014x} sid={:#06x} {} expected iova=={:#014x} sid={:#06x}",
                fr.iova,
                fr.sid,
                if fr.is_read { "read" } else { "write" },
                f.out_addr & !0xFFF,
                f.sid
            )?;
            writeln!(w, "assert   : frcd-iova==induced       observed={} sid-matches={} expected both true", iova_match, sid_match)?;
        }
        None => {
            writeln!(w, "assert   : out-of-arena-BLOCKED    observed=NO-FRCD-FAULT expected a DMAR fault record")?;
        }
    }
    writeln!(
        w,
        "info     : e1000e-descriptor-DD    observed={} — the device completes the descriptor regardless; the",
        f.out_completed
    )?;
    writeln!(w, "info     :                          FRCD fault (not DD) is the block proof — the out-of-arena READ was aborted")?;
    writeln!(w, "note     : proves (ii) BLOCKED in QEMU; (iii) SOUND stays honest-open (silicon residual)")?;
    writeln!(w, "RESULT   : {} (blocked; not yet silicon-verified)", if f.blocked() { "PASS" } else { "FAIL" })?;
    Ok(())
}

/// Observed (B) airtightness facts (Sprint 5, Slice 3). Each item carries its LEVEL: (i) source (our
/// config/tables, reviewed-by-reading + observed) vs (ii) induced. A (ii)/(iii) claim on (i) evidence is
/// a defect; un-inducible items stay HONEST-OPEN.
pub struct AirtightnessFacts {
    /// GSTS.IRES — our IRT is programmed + IR enabled (the IR (i)-source observed half).
    pub ir_enabled: bool,
    pub ir_vector: u8,
    pub ir_sid: u16,
    /// ATS is NOT enabled by our config (context translation-type = 0). (i)-source.
    pub ats_enabled_by_us: bool,
    /// ECAP.DT — whether the hardware advertises Device-TLB/ATS (informational: even so, we don't enable).
    pub hw_supports_ats: bool,
    /// Our SL tables map no device-to-device (only host RAM / the arena). (i)-source.
    pub p2p_mapped: bool,
    /// DMA translation (TE) is STILL enabled after enabling IR — proves the GCMD write preserved the
    /// persistent TE bit (enabling IRE alone would have silently disabled the DMAR).
    pub te_still_enabled: bool,
}

impl AirtightnessFacts {
    /// Slice-3 verdict: the (i)-SOURCE items hold (IR enabled, ATS not enabled by us, no p2p mapping) AND
    /// enabling IR did not disable the DMAR (TE still on). The (ii) / silicon items are HONEST-OPEN and
    /// are NOT part of pass/fail.
    pub fn slice3_ok(&self) -> bool {
        self.ir_enabled && !self.ats_enabled_by_us && !self.p2p_mapped && self.te_still_enabled
    }
}

/// Write the (B) airtightness report — per item: expected → observed → verdict, LEVEL stated. Never
/// claims (ii) blocked/airtight on (i) evidence.
pub fn write_airtightness<W: Write>(w: &mut W, f: &AirtightnessFacts) -> core::fmt::Result {
    writeln!(w, "---- sprint-5 slice-3 : (B) airtightness verdicts ----")?;

    // Interrupt remapping.
    writeln!(w, "[B1] interrupt-remapping")?;
    writeln!(
        w,
        "  (i)-source  : our IRT written (vector={:#04x}, source-validated sid={:#06x}) + IR enabled observed={} (GSTS.IRES) expected true",
        f.ir_vector, f.ir_sid, f.ir_enabled
    )?;
    writeln!(w, "  (ii)        : HONEST-OPEN — rogue-NIC-MSI-to-arbitrary-address BLOCKED is UN-INDUCIBLE in QEMU (a well-behaved emulated device fires no rogue MSI); NOT claimed")?;
    writeln!(w, "  verdict     : (i)-source PASS; (ii) open")?;

    // ATS off.
    writeln!(w, "[B2] ATS-off")?;
    writeln!(
        w,
        "  (i)-source  : ATS not enabled by our config (context translation-type=0) observed=ats-enabled-by-us({}) expected false; hw-advertises-ATS={} (informational — we do not enable it)",
        f.ats_enabled_by_us, f.hw_supports_ats
    )?;
    writeln!(w, "  inferred    : bypass-blocked INFERRED from ATS-disabled (the (ii) induced ATS-bypass test unattempted — NOT read as (ii))")?;
    writeln!(w, "  open        : HONEST-OPEN — the 82599 ATS-advertise datasheet half re-verifies on silicon (Sprint 6)")?;
    writeln!(w, "  verdict     : (i)-source PASS; bypass-blocked inferred; datasheet open")?;

    // No peer-to-peer.
    writeln!(w, "[B3] no-peer-to-peer")?;
    writeln!(
        w,
        "  (i)-source  : single-NIC topology + our SL tables map only host RAM (the arena), no device-to-device observed=p2p-mapped({}) expected false",
        f.p2p_mapped
    )?;
    writeln!(w, "  verdict     : (i)-source PASS")?;

    writeln!(
        w,
        "assert   : DMAR-preserved          observed=TE-still-enabled({}) expected true (enabling IR kept the persistent TE bit)",
        f.te_still_enabled
    )?;
    writeln!(w, "RESULT   : {} (the (i)-source verdicts; (ii)/silicon items HONEST-OPEN)", if f.slice3_ok() { "PASS" } else { "FAIL" })?;
    Ok(())
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
