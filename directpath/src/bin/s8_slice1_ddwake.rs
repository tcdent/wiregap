#![allow(unsafe_code)]

//! **Sprint 8 Slice 1 — the standalone DD-line device-wake proof** (MWAITX/MONITORX on AMD).
//!
//! The make-or-break: does the NIC's RX **descriptor DD write** (a device posted-write to our own memory)
//! WAKE an idled CPU armed with `MONITORX` + `MWAITX` — proven POSITIVELY by the instruments, not inferred
//! from DD-poll visibility (visibility != the monitor-wake). This is the WAKE-MECHANISM ONLY: **no reactor,
//! no transport, no DdWaitHost.** 41 sends unique-nonce frames to 63; on 63 we run
//! `arm MONITORX(next-descriptor line) -> recheck DD (LOAD) -> MWAITX(ECX=2, finite EBX) -> on wake, drain
//! DD-ready + refill -> re-arm on the new tail line`, across the full RX ring (>= 1 wrap) hundreds-to-
//! thousands of times, capturing the RAW per-wake distribution (TSC-delta, APERF-delta, DD-state,
//! RX-counter) for post-hoc classification.
//!
//! Pins (grounded 2026-07-18): MWAITX timer-enable = **ECX bit 1 (ECX=2)** (empirically pinned, APM-
//! consistent); a **finite EBX on every call** (hang-safety); the DD descriptor line is non-isolatable
//! (4x16B/64B) — arm-after-refill is strict (no store to the monitored line between MONITORX and MWAITX,
//! only the recheck LOAD); the NIC is the sole DD=1 writer (refill is read-format-only, driver/src/dma.rs:
//! 184-196). Safety: gate-minted `Allowed` for bench cards only; production `61:00.0` never touched;
//! reversible (rebind). Run: `sudo modprobe msr && sudo ./s8_slice1_ddwake`.

use directpath::datapath::{self, Layout};
use directpath::ethframe::EthSink;
use directpath::gate::{gate, Allowed, Bdf, Decision};
use directpath::gate2::validate_physaddr;
use directpath::link::{apply_restart, link_state, reset_and_init, LinkRestart};
use directpath::mem::{self, Hugepage};
use directpath::pci::{self, Bar0};
use driver::{RxRing, TxPath, VolatileRxRing};
use std::io::{self, Read};
use std::os::unix::fs::FileExt;
use std::time::{Duration, Instant};
use transport::TransmitSink;

const BENCH_41: &str = "0000:41:00.0";
const BENCH_63: &str = "0000:63:00.0";
const NONCE_LEN: usize = 8;
const ETH_HDR: usize = 14; // dst(6)+src(6)+ethertype/len(2); the nonce follows at offset 14
const N_FRAMES: usize = 2000; // hundreds-to-thousands; 2000/64 ≈ 31 ring-wraps
const PIN_CORE: usize = 4;
const APERF_MSR: u64 = 0xE8;

fn err(m: impl Into<String>) -> io::Error {
    io::Error::other(m.into())
}

// ---- MONITORX / MWAITX (AMD user-mode monitor-wait) --------------------------------------------

/// Arm address monitoring on the 64-byte line containing `addr` (MONITORX = 0F 01 FA; rAX=addr, ECX/EDX=0).
#[inline]
unsafe fn monitorx(addr: u64) {
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
/// scratch reg. A FINITE `ebx` is passed on EVERY call (hang-safety).
#[inline]
unsafe fn mwaitx(ecx: u32, ebx: u32) {
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

#[inline]
fn rdtsc() -> u64 {
    unsafe {
        let lo: u32;
        let hi: u32;
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nostack, nomem, preserves_flags));
        ((hi as u64) << 32) | lo as u64
    }
}

/// APERF (active-clock cycles; freezes in a C-state) via /dev/cpu/<core>/msr. `None` if unavailable.
fn read_aperf(msr: &std::fs::File) -> Option<u64> {
    let mut b = [0u8; 8];
    msr.read_at(&mut b, APERF_MSR).ok().map(|_| u64::from_le_bytes(b))
}

fn pin_to_core(core: usize) -> io::Result<()> {
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_ZERO(&mut set);
        libc::CPU_SET(core, &mut set);
        if libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

// ---- bring-up (reused pattern from s7) ---------------------------------------------------------

fn read_config(bdf: &Bdf) -> io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(format!("/sys/bus/pci/devices/{}/config", bdf.sysfs()))?;
    let mut buf = vec![0u8; 0x40];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

fn gate_bench(bdf: Bdf, allow: &[Bdf]) -> io::Result<Allowed> {
    let cfg = read_config(&bdf)?;
    match gate(bdf, allow, &cfg) {
        Decision::Allow(t) => Ok(t),
        Decision::Refuse(r) => Err(err(format!("gate REFUSED {}: {r:?}", bdf.sysfs()))),
    }
}

fn bring_up_link(bar: &Bar0, name: &str) -> io::Result<()> {
    if !reset_and_init(bar).ok() {
        return Err(err(format!("{name}: reset/init incomplete")));
    }
    for form in [LinkRestart::None, LinkRestart::RestartAn, LinkRestart::ResetPipeline] {
        apply_restart(bar, form);
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(4) {
            let ls = link_state(bar);
            if ls.up && ls.speed_10g {
                println!("  {name}: link UP 10G ({form:?})");
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Err(err(format!("{name}: link did not come up 10G")))
}

fn read_nonce() -> io::Result<[u8; NONCE_LEN]> {
    let mut f = std::fs::File::open("/dev/urandom")?;
    let mut n = [0u8; NONCE_LEN];
    f.read_exact(&mut n)?;
    Ok(n)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join("")
}

/// One captured MWAITX wake.
#[derive(Clone, Copy)]
struct Wake {
    frame: u32,
    call: u16,   // which re-arm within the frame (0 = first)
    tail: u16,   // the descriptor slot armed
    tsc_delta: u64,
    aperf_delta: u64,
    dd: bool,    // DD set on recheck after this wake
    rxdgpc: u32, // read-to-clear counter delta at this wake
}

/// Device-independent APERF-idle proof + the slow-wake population. Arm a quiet heap line, MWAITX to a ~1ms
/// EBX timer (no store ever), measure active(APERF) vs wall(TSC) across the wait. The idle dominates the
/// entry/exit overhead over ~1ms, so a low APERF/TSC ratio proves the core genuinely idles (a spin would
/// give APERF ~= TSC). The observed wake times (~1ms EBX / ~4ms tick) are the slow population the ~us
/// store-wakes separate from.
fn idle_proof(msr: &std::fs::File, tsc_hz: f64) {
    let quiet = Box::new(0u64);
    let addr = &*quiet as *const u64 as u64;
    let ebx = (tsc_hz * 0.001) as u32; // 1 ms timer
    let mut ratios: Vec<f64> = Vec::new();
    let mut wait_us: Vec<f64> = Vec::new();
    for _ in 0..64 {
        let a0 = read_aperf(msr).unwrap_or(0);
        let t0 = rdtsc();
        unsafe { monitorx(addr) };
        // recheck LOAD only (never a store) — strict arm-after-refill discipline.
        std::hint::black_box(unsafe { core::ptr::read_volatile(&*quiet) });
        unsafe { mwaitx(2, ebx) };
        let t1 = rdtsc();
        let a1 = read_aperf(msr).unwrap_or(0);
        let tsc_d = (t1 - t0) as f64;
        let ap_d = a1.saturating_sub(a0) as f64;
        if tsc_d > tsc_hz * 100e-6 {
            // only judge waits > 100us (idle dominates overhead)
            ratios.push(ap_d / tsc_d);
            wait_us.push(tsc_d / (tsc_hz / 1e6));
        }
    }
    ratios.sort_by(|a, b| a.partial_cmp(b).unwrap());
    wait_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if ratios.is_empty() {
        println!("APERF-idle proof: no long waits captured");
        return;
    }
    let med_r = ratios[ratios.len() / 2];
    let max_r = *ratios.last().unwrap();
    println!(
        "APERF-idle proof ({} long waits, ~{:.0}us each): active/wall ratio median {:.4}, max {:.4}  -> {} (spin would be ~1.0)",
        ratios.len(),
        wait_us[wait_us.len() / 2],
        med_r,
        max_r,
        if max_r < 0.05 { "TRULY IDLE" } else { "not-clearly-idle (surface)" }
    );
}

fn main() {
    match run() {
        Ok(()) => println!("\n=== SLICE 1 (DD-wake) COMPLETE ==="),
        Err(e) => {
            eprintln!("\n=== SLICE 1 FAILED: {e} ===");
            std::process::exit(1);
        }
    }
}

fn run() -> io::Result<()> {
    pin_to_core(PIN_CORE)?;
    let msr = std::fs::File::open(format!("/dev/cpu/{PIN_CORE}/msr"))
        .map_err(|e| err(format!("open /dev/cpu/{PIN_CORE}/msr (modprobe msr?): {e}")))?;

    // TSC calibration (invariant/nonstop TSC confirmed on this host).
    let a = rdtsc();
    std::thread::sleep(Duration::from_millis(200));
    let tsc_hz = (rdtsc() - a) as f64 / 0.2;
    let ebx: u32 = (tsc_hz * 0.005) as u32; // 5 ms hang-safety timer, finite on every call
    println!("pinned core {PIN_CORE}; TSC = {:.3} GHz; EBX timer = {ebx} cyc (~5ms); N = {N_FRAMES}", tsc_hz / 1e9);

    // --- APERF-idle proof (device-independent): long MWAITX-to-timer waits on a quiet line. A ~1ms wait
    // makes the idle dominate the ~us entry/exit overhead, so active(APERF) << wall(TSC) is unambiguous.
    // Also directly shows the SLOW-wake population (~1ms EBX timer / ~4ms OS tick) that the store-wakes
    // (~us) separate from. (Store-wait APERF is too short at ~4us to judge — this is the clean idle proof.)
    idle_proof(&msr, tsc_hz);

    let bdf41 = Bdf::parse(BENCH_41).ok_or_else(|| err("bad BDF 41"))?;
    let bdf63 = Bdf::parse(BENCH_63).ok_or_else(|| err("bad BDF 63"))?;
    let allow = [bdf41, bdf63];
    let tok41 = gate_bench(bdf41, &allow)?;
    let tok63 = gate_bench(bdf63, &allow)?;
    println!("gate: ALLOW {} (sender) and {} (receiver)", bdf41.sysfs(), bdf63.sysfs());

    let drv41 = pci::current_driver(&tok41).unwrap_or_else(|| "ixgbe".into());
    let drv63 = pci::current_driver(&tok63).unwrap_or_else(|| "ixgbe".into());
    pci::unbind(&tok41)?;
    pci::unbind(&tok63)?;

    let result = measure(&tok41, &tok63, &msr, tsc_hz, ebx);

    let r41 = pci::rebind(&tok41, &drv41);
    let r63 = pci::rebind(&tok63, &drv63);
    println!("rebind {}: {:?}; rebind {}: {:?}", bdf41.sysfs(), r41.is_ok(), bdf63.sysfs(), r63.is_ok());
    result?;
    r41?;
    r63?;
    Ok(())
}

fn measure(tok41: &Allowed, tok63: &Allowed, msr: &std::fs::File, tsc_hz: f64, ebx: u32) -> io::Result<()> {
    pci::enable_bus_master(tok41)?;
    pci::enable_bus_master(tok63)?;
    let bar41 = pci::map_resource0(tok41)?;
    let bar63 = pci::map_resource0(tok63)?;
    bring_up_link(&bar41, "41")?;
    bring_up_link(&bar63, "63")?;
    let mac41 = datapath::read_mac(&bar41);
    let mac63 = datapath::read_mac(&bar63);
    println!("MAC 41 = {}  MAC 63 = {}", hex(&mac41), hex(&mac63));

    // One hugepage, two carves (41 sender TX, 63 receiver RX).
    let hp: Hugepage = mem::alloc()?;
    let split = Layout::used_bytes();
    validate_physaddr(hp.va(), hp.phys(), hp.size() as u64)
        .map_err(|e| err(format!("Gate 2 REFUSED the hugepage: {e:?}")))?;
    let layout_41 = Layout::new(hp.va(), hp.phys());
    let layout_63 = Layout::new(hp.va() + split as u64, hp.phys() + split as u64);
    datapath::preinit_desc_rings(&layout_41);
    datapath::preinit_desc_rings(&layout_63);

    // 41: TX sender.
    if !datapath::init_tx(&bar41, &layout_41) {
        return Err(err("41: TXDCTL.ENABLE did not read back"));
    }
    let tx41 = datapath::build_tx_ring(&bar41, &layout_41);
    let mut sink = EthSink::new(TxPath::new(tx41), mac63, mac41); // 41 -> 63

    // 63: RX receiver, descriptors armed.
    datapath::init_rx(&bar63, &layout_63);
    datapath::clear_rx_buffers(&layout_63);
    if !datapath::rx_buffers_all_zero(&layout_63) {
        return Err(err("63: RX buffers not proven-cleared"));
    }
    let mut rx63: VolatileRxRing = datapath::build_rx_ring(&bar63, &layout_63);
    if !datapath::start_rx(&bar63, &layout_63) {
        return Err(err("63: RXDCTL.ENABLE did not read back"));
    }
    let depth = rx63.depth();
    // Arm ALL depth slots (matches steady-state s7_slice2, NOT the single-frame s7_slice1 depth-1): with
    // RDT=slot the refill cursor tracks the consumer one behind, so RDT never meets RDH. Arming depth-1
    // leaves the last slot un-refilled and the ring STALLS when the consumer wraps onto it — the RX-RDT
    // re-arm bug that a single-frame PoC never exercises (exactly what this harness must exercise).
    for slot in 0..depth {
        rx63.refill(slot);
    }
    let rx_desc_base = layout_63.rx_desc_base_va();
    println!("  63: RX armed ({depth} descriptors); rx_desc_base_va=0x{rx_desc_base:x}");
    println!("  wake loop: MONITORX(tail line) -> recheck DD -> MWAITX(ECX=2, EBX) -> drain+refill -> re-arm\n");

    let _ = datapath::read_counter(&bar63, datapath::RXDGPC); // baseline the read-to-clear counter

    let mut wakes: Vec<Wake> = Vec::with_capacity(N_FRAMES + 512);
    let mut tail = 0usize;
    let mut wraps = 0usize;
    let mut arrived = 0usize; // frames with DD=1 + counter++
    let mut nonce_ok = 0usize;
    let mut nonce_bad = 0usize;
    let hang_guard = (tsc_hz * 0.050) as u64; // 50ms per-frame guard (well above EBX)

    for frame in 0..N_FRAMES {
        // Send one unique-nonce frame 41 -> 63. The nonce lands at offset ETH_HDR in 63's buffer.
        let nonce = read_nonce()?;
        let mut payload = Vec::with_capacity(NONCE_LEN + 16);
        payload.extend_from_slice(&nonce);
        payload.extend_from_slice(&[0x5A; 16]);
        sink.emit(&payload);

        // The frame's wire+DMA latency (~hundreds of us) >> the send->arm code path (~us), so the arm
        // reliably precedes the DD store. Arm on the tail-descriptor line and wait.
        let mon_addr = rx_desc_base + (tail * 16) as u64;
        let first_arm = rdtsc();
        let mut call: u16 = 0;
        loop {
            let a0 = read_aperf(msr).unwrap_or(0);
            let t0 = rdtsc();
            // ARM, then ONLY a recheck LOAD before MWAITX — strict arm-after-refill (no store between).
            unsafe { monitorx(mon_addr) };
            if rx63.descriptor_done(tail) {
                // Store already landed before we could wait: record a pre-wait catch (delta ~ 0).
                let dd_counter = datapath::read_counter(&bar63, datapath::RXDGPC);
                wakes.push(Wake { frame: frame as u32, call, tail: tail as u16, tsc_delta: rdtsc() - t0, aperf_delta: 0, dd: true, rxdgpc: dd_counter });
                break;
            }
            unsafe { mwaitx(2, ebx) };
            let t1 = rdtsc();
            let a1 = read_aperf(msr).unwrap_or(0);
            let dd = rx63.descriptor_done(tail);
            let rxdgpc = datapath::read_counter(&bar63, datapath::RXDGPC); // read-to-clear delta this wake
            wakes.push(Wake { frame: frame as u32, call, tail: tail as u16, tsc_delta: t1 - t0, aperf_delta: a1.saturating_sub(a0), dd, rxdgpc });
            if dd {
                break;
            }
            if rdtsc() - first_arm > hang_guard {
                break; // frame-level guard (should never trip: EBX bounds each wait)
            }
            call = call.saturating_add(1);
        }

        // Drain the awaited descriptor (and any others on the line that are DD-ready) + refill.
        let mut got_this_frame = false;
        while rx63.descriptor_done(tail) {
            let len = (rx63.reported_len(tail) as usize).min(rx63.buffer_len());
            let mut buf = vec![0u8; len];
            let n = rx63.copy_out(tail, len, &mut buf);
            buf.truncate(n);
            if n >= ETH_HDR + NONCE_LEN && buf[ETH_HDR..ETH_HDR + NONCE_LEN] == nonce {
                nonce_ok += 1;
            } else if !got_this_frame {
                nonce_bad += 1;
            }
            got_this_frame = true;
            rx63.refill(tail);
            tail = (tail + 1) % depth;
            if tail == 0 {
                wraps += 1;
            }
        }
        if got_this_frame {
            arrived += 1;
        }
    }

    // --- adjacent-descriptor demonstration: a burst of 4 frames fills 4 descriptors on/around the tail
    // line (4x16B/64B); ONE MWAITX wake drains ALL DD-ready + re-arms on the next line. Proves the
    // non-isolatable line is handled: a wake drains the whole line's arrivals, then re-arms. ---
    println!("\n=== ADJACENT-DESCRIPTOR burst demo (4 frames/burst; tail line holds 4 descriptors) ===");
    for burst in 0..6 {
        for _ in 0..4 {
            let nonce = read_nonce()?;
            let mut p = nonce.to_vec();
            p.extend_from_slice(&[0x5A; 16]);
            sink.emit(&p);
        }
        let mon_addr = rx_desc_base + (tail * 16) as u64;
        let start_tail = tail;
        let first = rdtsc();
        loop {
            unsafe { monitorx(mon_addr) };
            if rx63.descriptor_done(tail) {
                break;
            }
            unsafe { mwaitx(2, ebx) };
            if rx63.descriptor_done(tail) || rdtsc() - first > (tsc_hz * 0.02) as u64 {
                break;
            }
        }
        // brief settle so all 4 of the burst land, then drain all DD-ready on this line + refill.
        let settle = rdtsc();
        while rdtsc() - settle < (tsc_hz * 50e-6) as u64 {}
        let mut drained = 0usize;
        while rx63.descriptor_done(tail) {
            let len = (rx63.reported_len(tail) as usize).min(rx63.buffer_len());
            let mut buf = vec![0u8; len];
            rx63.copy_out(tail, len, &mut buf);
            rx63.refill(tail);
            tail = (tail + 1) % depth;
            drained += 1;
        }
        println!("  burst {burst}: 4 sent; ONE wake drained {drained} adjacent descriptors (tail {start_tail}->{tail}, re-armed)");
    }

    // --- dump the RAW per-wake records (for the reviewer's re-analysis) + post-hoc analysis ---
    let raw_path = "/tmp/agent-message/zero.nic-dev/s8-slice1-raw-wakes.csv";
    let mut csv = String::from("frame,call,tail,tsc_delta_us,aperf_delta,dd,rxdgpc\n");
    for w in &wakes {
        csv.push_str(&format!(
            "{},{},{},{:.3},{},{},{}\n",
            w.frame, w.call, w.tail, w.tsc_delta as f64 / (tsc_hz / 1e6), w.aperf_delta, w.dd as u8, w.rxdgpc
        ));
    }
    if std::fs::write(raw_path, &csv).is_ok() {
        println!("raw per-wake records written: {raw_path} ({} wakes)", wakes.len());
    }
    println!("63 RXMPC0 (missed/no-descriptor drops) = {}", datapath::read_counter(&bar63, datapath::RXMPC0));
    analyze(&wakes, tsc_hz, ebx, arrived, nonce_ok, nonce_bad, wraps, N_FRAMES);

    datapath::quiesce(&bar63);
    Ok(())
}

/// Post-hoc classification over the raw per-wake distribution. Cutoff derived from the observed gap;
/// joint attribution (sub-cutoff AND DD=1 AND counter++); spurious-wake rate secondary.
#[allow(clippy::too_many_arguments)]
fn analyze(wakes: &[Wake], tsc_hz: f64, ebx: u32, arrived: usize, nonce_ok: usize, nonce_bad: usize, wraps: usize, n: usize) {
    let us = |c: u64| c as f64 / (tsc_hz / 1e6);
    // Histogram (log-ish buckets in us).
    let edges_us = [1.0, 3.0, 10.0, 30.0, 100.0, 300.0, 500.0, 800.0, 1200.0, 2000.0, 3500.0, 4500.0, 6000.0, 1e9];
    let mut hist = vec![0usize; edges_us.len()];
    let mut hist_dd = vec![0usize; edges_us.len()];
    for w in wakes {
        let d = us(w.tsc_delta);
        let b = edges_us.iter().position(|&e| d < e).unwrap_or(edges_us.len() - 1);
        hist[b] += 1;
        if w.dd {
            hist_dd[b] += 1;
        }
    }
    println!("=== RAW per-wake TSC-delta histogram ({} wakes) ===", wakes.len());
    println!("  bucket(<us)   all-wakes   DD=1-wakes");
    let mut lo = 0.0;
    for (i, &e) in edges_us.iter().enumerate() {
        if hist[i] > 0 {
            println!("  {:>7.0}..{:<7} {:>8}   {:>8}", lo, if e > 1e8 { f64::INFINITY } else { e }, hist[i], hist_dd[i]);
        }
        lo = e;
    }

    // Store-wake candidates: the DD=1 wakes (a device DD write happened at this wake) with counter++.
    // Classify by the observed sub-tick gap. Populations of interest: sub-ms store vs ~4ms tick vs EBX.
    let ebx_us = us(ebx as u64);
    let cutoff_us = 1000.0; // sub-tick (< the ~4ms OS-tick floor); the gap check below validates it
    let mut store = 0usize; // DD=1 + counter++ + sub-cutoff
    let mut slow_arrived = 0usize; // DD=1 + counter++ + >= cutoff (tick/EBX = MISS)
    let mut spurious = 0usize; // sub-cutoff wake with DD=0 (noisy-monitor: eviction/IPI)
    let mut aperf_idle_ok = 0usize;
    let mut aperf_total = 0usize;
    let mut store_deltas_us: Vec<f64> = Vec::new();
    for w in wakes {
        let d = us(w.tsc_delta);
        // APERF-idle: active cycles << wall-clock cycles across the wait (waits long enough to judge).
        if w.tsc_delta > (tsc_hz * 20e-6) as u64 {
            aperf_total += 1;
            if (w.aperf_delta as f64) < 0.2 * (w.tsc_delta as f64) {
                aperf_idle_ok += 1;
            }
        }
        if w.dd && w.rxdgpc >= 1 {
            if d < cutoff_us {
                store += 1;
                store_deltas_us.push(d);
            } else {
                slow_arrived += 1;
            }
        } else if !w.dd && d < cutoff_us {
            spurious += 1;
        }
    }
    store_deltas_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |x: usize, d: usize| if d == 0 { 0.0 } else { 100.0 * x as f64 / d as f64 };
    let (smin, smed, smax) = if store_deltas_us.is_empty() {
        (0.0, 0.0, 0.0)
    } else {
        (store_deltas_us[0], store_deltas_us[store_deltas_us.len() / 2], store_deltas_us[store_deltas_us.len() - 1])
    };

    println!("\n=== CLASSIFICATION (cutoff {cutoff_us:.0}us sub-tick; EBX~{ebx_us:.0}us; joint DD=1 + counter++) ===");
    println!("  frames sent          = {n}");
    println!("  frames ARRIVED (DD=1+counter++) = {arrived}  ({:.1}% of sent)", pct(arrived, n));
    println!("  ring-wraps completed = {wraps}  (re-arm across the tail 63->0 line jump exercised {wraps}x)");
    println!("  nonce match (device wrote our frame) = {nonce_ok} ok, {nonce_bad} mismatch");
    println!("  --- of ARRIVED frames, the DD=1 store-wake classification ---");
    println!("  STORE-WAKE (sub-tick, event-driven)  = {store}   ({:.2}% of arrived)", pct(store, arrived));
    println!("  SLOW/MISSED (tick/EBX despite DD=1)   = {slow_arrived}   ({:.2}% of arrived)  <- intermittent if non-trivial", pct(slow_arrived, arrived));
    println!("  store-wake latency: min {smin:.1}us  median {smed:.1}us  max {smax:.1}us");
    println!("  --- secondary ---");
    println!("  SPURIOUS sub-tick wakes (DD=0; eviction/IPI, re-armed) = {spurious}  ({:.2} per arrived frame)", spurious as f64 / arrived.max(1) as f64);
    println!("  APERF-idle across wait (>20us waits): {aperf_idle_ok}/{aperf_total} had active<<wall ({:.1}%)", pct(aperf_idle_ok, aperf_total.max(1)));

    // Gap check: is there a clean gap between the store cluster (tail) and the ~4ms tick / EBX?
    let tick_floor_us = 3000.0;
    if smax > 0.0 && smax < tick_floor_us && store > 0 {
        println!("\n  GAP: store tail {smax:.1}us << tick floor ~4000us -> CLEAN sub-tick separation.");
    } else if store == 0 {
        println!("\n  NO STORE-WAKES -> device DD store does NOT wake the monitor sub-tick (BLOCKER).");
    } else {
        println!("\n  WARNING: store tail {smax:.1}us bleeds toward the tick floor -> ambiguous gap, SURFACE (do not fudge cutoff).");
    }
    let verdict = if arrived > 0 && pct(store, arrived) >= 99.0 && smax < tick_floor_us {
        "PASS: ~100% of arrived frames woke sub-tick (event-driven), re-arm exercised across wraps."
    } else if store == 0 {
        "BLOCKER: no store-wakes."
    } else {
        "INTERMITTENT: a non-trivial fraction of arrived frames woke at tick/EBX -> latency FINDING to surface."
    };
    println!("\n  >>> {verdict} <<<");
}
