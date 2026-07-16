// The UEFI application entry (harness surface (b)). On the bare-metal target this is a `no_std`,
// `no_main` UEFI app; on the host it is a trivial `std` binary that does nothing, so `cargo test`
// (which builds every target) still compiles — the real logic is the pure `substrate::banner`,
// host-tested in `tests/`. Keeping both behind `cfg(target_os = "uefi")` is what lets the one crate
// serve both harness surfaces without a compatibility shim.
#![cfg_attr(target_os = "uefi", no_std)]
#![cfg_attr(target_os = "uefi", no_main)]
#![deny(unsafe_code)]

#[cfg(not(target_os = "uefi"))]
fn main() {}

#[cfg(target_os = "uefi")]
extern crate alloc;

#[cfg(target_os = "uefi")]
#[allow(unsafe_code)] // confined: ExitBootServices + the QEMU debug-exit port; each block SAFETY-noted.
mod app {
    use alloc::collections::VecDeque;
    use alloc::vec::Vec;
    use core::fmt::Write;
    use core::sync::atomic::Ordering;
    use core::time::Duration;
    use substrate::acpi::Acpi;
    use substrate::arena::{self, ArenaConfig, Region};
    use substrate::banner::{
        write_arena, write_banner, write_ecam, write_interrupts, write_reactor, BootFacts, EcamFacts,
        InterruptFacts, ReactorFacts,
    };
    use substrate::calibrate::{self, Calibration};
    use substrate::ecam::Device;
    use substrate::interrupts::{self, VectorPlan, MSIX_HITS, MSIX_PENDING, TIMER_TICKS};
    use substrate::lapic::Lapic;
    use substrate::reactor::{Reactor, RxSource, WakeReason};
    use substrate::reactor_host::SubstrateHost;
    use substrate::serial::Serial;
    use transport::{SessionTable, TransmitSink};
    use uefi::boot::{self, MemoryType};
    use uefi::mem::memory_map::MemoryMap;
    use uefi::prelude::*;
    use uefi::table::cfg::{ACPI2_GUID, ACPI_GUID};

    // --- Slice-4 integration doubles: a stub RX source + a counting sink (real rings are Sprint 6) ---

    /// A stub RX source over a queue of owned frames (the driver's RX drain in production).
    struct QueueRx {
        frames: VecDeque<Vec<u8>>,
        front: Vec<u8>,
    }
    impl QueueRx {
        fn new(frames: impl IntoIterator<Item = Vec<u8>>) -> Self {
            Self { frames: frames.into_iter().collect(), front: Vec::new() }
        }
    }
    impl RxSource for QueueRx {
        fn has_frame(&self) -> bool {
            !self.frames.is_empty()
        }
        fn next_frame(&mut self) -> Option<&[u8]> {
            self.front = self.frames.pop_front()?;
            Some(&self.front)
        }
    }

    /// A sink that counts emissions (so a retransmit is observable).
    struct CountingSink {
        emits: u32,
    }
    impl TransmitSink for CountingSink {
        fn emit(&mut self, _frame: &[u8]) {
            self.emits += 1;
        }
    }

    /// A receiver-role inbound frame (`[u64 BE session id][payload]`).
    fn inbound_frame(session: u64) -> Vec<u8> {
        let mut f = session.to_be_bytes().to_vec();
        f.extend_from_slice(b"payload");
        f
    }

    /// Drive the clean reactor through the four behavioral proofs on the real substrate.
    fn run_reactor_slice(cal: Calibration, plan: VectorPlan, lapic: Lapic) -> ReactorFacts {
        lapic.setup_oneshot_timer(plan.timer);
        MSIX_PENDING.store(false, Ordering::SeqCst);

        // A — ZERO hlt parks while there is work: a continuous RX burst never idles.
        let burst: Vec<Vec<u8>> = (0..5u64).map(|i| inbound_frame(100 + i)).collect();
        let mut ra = Reactor::new(
            SessionTable::new(),
            SubstrateHost::new(lapic, cal, plan.timer),
            QueueRx::new(burst),
            CountingSink { emits: 0 },
        );
        let mut burst_processed = 0u32;
        while ra.step() {
            if ra.take_delivery().is_some() {
                burst_processed += 1;
            }
        }
        let burst_hlts = ra.host().hlts();

        // B — one-in-flight RX back-pressure: a second frame is held while a delivery is in flight.
        let mut rb = Reactor::new(
            SessionTable::new(),
            SubstrateHost::new(lapic, cal, plan.timer),
            QueueRx::new([inbound_frame(1), inbound_frame(2)]),
            CountingSink { emits: 0 },
        );
        rb.step(); // frame 1 → delivery held
        let bp_held = rb.back_pressured() && !rb.step(); // frame 2 held (RDT not advanced)
        rb.take_delivery();
        let bp_released = !rb.back_pressured() && rb.step(); // frame 2 flows

        // C — deadline → one-shot timer → retransmit (an actual hlt park + Timer wake).
        let mut rc = Reactor::new(
            SessionTable::with_timing(Duration::from_millis(50), Duration::from_secs(5)),
            SubstrateHost::new(lapic, cal, plan.timer),
            QueueRx::new([]),
            CountingSink { emits: 0 },
        );
        rc.originate(b"retransmit-me");
        let emits0 = rc.sink().emits;
        let mut deadline_parked = false;
        let mut deadline_wake = WakeReason::NicMsix;
        for _ in 0..40 {
            while rc.step() {}
            if rc.sink().emits > emits0 {
                break;
            }
            if let Some(w) = rc.park() {
                deadline_parked = true;
                deadline_wake = w;
            }
        }
        let retransmit_emitted = rc.sink().emits > emits0;

        // D — MSI-X wakeup latch (self-IPI stand-in): the woke-by class is NIC-MSI-X.
        let mut rd = Reactor::new(
            SessionTable::new(),
            SubstrateHost::new(lapic, cal, plan.timer),
            QueueRx::new([]),
            CountingSink { emits: 0 },
        );
        lapic.setup_oneshot_timer(plan.timer); // disarm any leftover timer
        MSIX_PENDING.store(false, Ordering::SeqCst);
        x86_64::instructions::interrupts::enable();
        lapic.self_ipi(plan.nic_msix);
        for _ in 0..1_000_000u32 {
            if MSIX_PENDING.load(Ordering::SeqCst) {
                break;
            }
            core::hint::spin_loop();
        }
        x86_64::instructions::interrupts::disable();
        let msix_wake = rd.pump_once().unwrap_or(WakeReason::Timer);

        ReactorFacts {
            tsc_khz: cal.tsc_khz,
            lapic_khz: cal.lapic_khz,
            burst_processed,
            burst_hlts,
            bp_held,
            bp_released,
            deadline_wake,
            deadline_parked,
            retransmit_emitted,
            msix_wake,
        }
    }

    /// This NIC's BDF. In QEMU the stand-in NIC is placed here (see `qemu-run.sh`: `addr=3` on `pcie.0`);
    /// on the bench the real 82599 is `0000:41:00.0`. The BDF is a placement parameter, not a magic id.
    const NIC_BUS: u8 = 0;
    const NIC_DEV: u8 = 3;
    const NIC_FUNC: u8 = 0;
    /// PCI segment 0 (the only segment on q35 / the bench host).
    const PCI_SEGMENT: u16 = 0;
    /// Surfaced substitution: QEMU 7.2 has no 82599 model, so the ECAM *mechanism* is validated against
    /// e1000e (82574L); the real-82599 config read lands on the bench card in Slice 6.
    const MODEL_NOTE: &str =
        "QEMU has no 82599 model -> ECAM mechanism validated on e1000e(82574L); real 82599 = Slice 6 (bench)";

    /// Locate the ACPI RSDP from the UEFI configuration table — **before** `ExitBootServices`, while
    /// the boot-services config table is live. Prefer ACPI 2.0+ (XSDT-rooted); fall back to legacy 1.0.
    /// Returns 0 if neither is present (a handoff failure the banner surfaces).
    fn acpi_rsdp() -> u64 {
        uefi::system::with_config_table(|entries| {
            let mut rsdp_v2 = 0u64;
            let mut rsdp_v1 = 0u64;
            for e in entries {
                if e.guid == ACPI2_GUID {
                    rsdp_v2 = e.address as u64;
                } else if e.guid == ACPI_GUID {
                    rsdp_v1 = e.address as u64;
                }
            }
            if rsdp_v2 != 0 { rsdp_v2 } else { rsdp_v1 }
        })
    }

    /// The UEFI entry: read ACPI, take the machine, then report observed facts over COM1.
    #[uefi::entry]
    fn efi_main() -> Status {
        // Stand up the heap first (before any allocation): the reactor + transport need it after exit,
        // where the UEFI allocator is gone.
        substrate::heap::init();

        // Read ACPI while boot services are alive (the config table is unreachable afterwards).
        let rsdp = acpi_rsdp();

        // Take the machine. SAFETY: after this call no boot-services function may be used; we only
        // read the returned owned memory map (its buffer stays valid) and drive raw hardware (COM1).
        // We pass `None` so LOADER_DATA backs the map, matching the UEFI/Linux-stub convention.
        let memory_map = unsafe { boot::exit_boot_services(None) };

        // Summarise usable RAM and pick the largest CONVENTIONAL region (above low memory) for the
        // arena — reads only, no allocation post-exit.
        let mut usable_regions = 0u64;
        let mut usable_pages = 0u64;
        let mut arena_region = Region { base: 0, len: 0 };
        for desc in memory_map.entries() {
            if desc.ty == MemoryType::CONVENTIONAL {
                usable_regions += 1;
                usable_pages += desc.page_count;
                let len = desc.page_count * 4096;
                if desc.phys_start >= 0x0010_0000 && len > arena_region.len {
                    arena_region = Region { base: desc.phys_start, len };
                }
            }
        }

        // Post-handoff: COM1 is the only console. The banner arriving at all proves serial works
        // after ExitBootServices.
        let mut com = Serial::com1();
        let facts = BootFacts {
            build_id: env!("SUBSTRATE_BUILD_ID"),
            rustc: env!("SUBSTRATE_RUSTC"),
            rsdp,
            usable_regions,
            usable_pages,
        };
        let _ = write_banner(&mut com, &facts);

        // Slice 2a — ECAM for this NIC's BDF (find MCFG->ECAM, read BAR0, enable bus-master, find MSI-X).
        let ecam_base = Acpi::new(rsdp).ecam_alloc(PCI_SEGMENT).map(|a| a.base).unwrap_or(0);
        let ecam_ok = if ecam_base == 0 {
            let _ = writeln!(com, "---- slice-2 : ECAM ----\nassert   : mcfg/ecam-base    observed=NOT-FOUND expected!=0\nRESULT   : FAIL");
            false
        } else {
            let nic = Device::new(ecam_base, NIC_BUS, NIC_DEV, NIC_FUNC);
            let (vendor, device) = nic.vendor_device();
            let bar0 = nic.bar0();
            let command = nic.enable_bus_master_and_memory();
            let ecam = EcamFacts {
                ecam_base,
                bus: NIC_BUS,
                dev: NIC_DEV,
                func: NIC_FUNC,
                vendor,
                device,
                bar0,
                command,
                msix: nic.find_msix(),
                model_note: MODEL_NOTE,
            };
            let _ = write_ecam(&mut com, &ecam);
            ecam.ecam_ok()
        };

        // Slice 2b — carve this NIC's DMA arena in the chosen usable region.
        let arena_ok = match arena::plan(arena_region, &ArenaConfig::NIC_82599) {
            Ok(layout) => {
                let _ = write_arena(&mut com, &layout, &arena_region);
                layout.sound(&arena_region)
            }
            Err(e) => {
                let _ = writeln!(com, "---- slice-2 : arena ----\nassert   : plan  observed=FAILED({e:?}) expected Ok\nRESULT   : FAIL");
                false
            }
        };

        // Slice 3 — this NIC's interrupt path: minimal IDT, LAPIC, one-shot timer, hlt. A self-IPI
        // stands in for the NIC's real MSI-X (driving the 82599's IVAR/EIMS is Sprint 6).
        let plan = VectorPlan::NIC;
        interrupts::init(&plan);
        let lapic = Lapic::enable(plan.spurious);
        lapic.setup_oneshot_timer(plan.timer);

        // (i) self-IPI -> the NIC MSI-X vector. Enable interrupts, deliver, wait (bounded) for the hit.
        let msix_before = MSIX_HITS.load(Ordering::SeqCst);
        x86_64::instructions::interrupts::enable();
        lapic.self_ipi(plan.nic_msix);
        for _ in 0..1_000_000u32 {
            if MSIX_HITS.load(Ordering::SeqCst) > msix_before {
                break;
            }
            core::hint::spin_loop();
        }
        let msix_after = MSIX_HITS.load(Ordering::SeqCst);

        // (ii) one-shot timer -> handler + wake. Disable, arm, then atomically sti+hlt: the timer
        // interrupt wakes the CPU (reaching the next line proves the wake).
        let timer_before = TIMER_TICKS.load(Ordering::SeqCst);
        x86_64::instructions::interrupts::disable();
        lapic.arm(1_000_000);
        x86_64::instructions::interrupts::enable_and_hlt();
        let woke_past_hlt = true;
        let timer_after = TIMER_TICKS.load(Ordering::SeqCst);
        x86_64::instructions::interrupts::disable();

        let ints = InterruptFacts {
            plan,
            svr: lapic.svr(),
            msix_before,
            msix_after,
            timer_before,
            timer_after,
            woke_past_hlt,
        };
        let _ = write_interrupts(&mut com, &ints);
        let ints_ok = ints.ints_ok();

        // Slice 4 — the clean no_std reactor on the substrate (interrupts are disabled here; calibrate
        // in that quiet window, then drive the reactor's behavioral proofs).
        let cal = calibrate::calibrate(&lapic);
        let reactor = run_reactor_slice(cal, plan, lapic);
        let _ = write_reactor(&mut com, &reactor);
        let reactor_ok = reactor.reactor_ok();

        let all_ok = facts.handoff_ok() && ecam_ok && arena_ok && ints_ok && reactor_ok;
        let _ = writeln!(com, "==== SLICE-4 OVERALL : {} ====", if all_ok { "PASS" } else { "FAIL" });
        exit_qemu(&mut com, all_ok);
    }

    /// QEMU `isa-debug-exit` codes (see `qemu-run.sh`, `-device isa-debug-exit,iobase=0xf4`). QEMU
    /// terminates with host status `(code << 1) | 1`, so 0x10 -> 33 (pass) and 0x11 -> 35 (fail).
    const EXIT_PASS: u32 = 0x10;
    const EXIT_FAIL: u32 = 0x11;

    /// Terminate the run: signal QEMU to exit with a pass/fail code, then park if that device is
    /// absent (a real machine has no `isa-debug-exit`) so we never fall off the end.
    fn exit_qemu(com: &mut Serial, ok: bool) -> ! {
        let code = if ok { EXIT_PASS } else { EXIT_FAIL };
        let _ = writeln!(com, "exit     : isa-debug-exit code={code:#x} ({})", if ok { "pass" } else { "fail" });
        // SAFETY: 0xf4 is QEMU's isa-debug-exit port, configured on the QEMU command line. Writing the
        // code triggers QEMU shutdown; it has no memory effect. On real hardware the port is inert.
        unsafe {
            let mut port = x86_64::instructions::port::Port::new(0xf4);
            port.write(code);
        }
        loop {
            x86_64::instructions::hlt();
        }
    }

    /// Post-handoff panic path: re-init COM1 and report, then fail-exit. No unwinding (panic = abort).
    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        let mut com = Serial::com1();
        let _ = writeln!(com, "PANIC    : {info}");
        exit_qemu(&mut com, false);
    }
}
