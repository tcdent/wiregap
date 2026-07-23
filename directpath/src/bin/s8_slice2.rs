#![deny(unsafe_code)]

//! **Sprint 9 Slice 2 — the SHIPPED facade, two-instance conservation re-run** over the live DAC
//! `41:00.0 ↔ 63:00.0`, pure-direct, no IOMMU. This is the primary outcome-conservation re-run for the
//! facade transition: it drives the shipped [`directpath::Subsystem`] `construct` / `run` / `send` on BOTH
//! ends (box-role on 41, proxy-role on 63, each `run()` in its own thread — the box↔proxy topology), so the
//! numbers bind to the exact facade bytes, NOT a parallel harness loop.
//!
//! ONE-DIRECTIONAL workload (the faithful conservation of the original storm traffic — peak in-flight = K):
//! the box originates, the proxy receives + auto-ACKs + collects. (The transport itself supports
//! both-originate; this harness drives one direction to preserve the storm's gate invariants.) Phase 1
//! measures the send→ACK RTT by pacing on `Ctx::is_quiescent` (a bench-only completion bool); phase 2 is the
//! one-directional K-burst storm (peak in-flight = K). The facade `run()` hands the handler an OWNED `Vec` per delivery
//! (the copy-boundary), never an ACK. The Sprint-9 Fork-1 conserved ledger:
//! - CONSERVED — send→ACK RTT + throughput (via is_quiescent), idle-wakeup (every actual NIC park→wake,
//!   measured INSIDE `run()`: the sub-tick make-or-break floor + the median band), and the correctness
//!   outcomes (byte-equal delivery, device RXMPC0=0 / GPRC). The two-thread box↔proxy topology runs faster
//!   than the original single-thread loopback, so RTT/idle-wakeup values are lower (same metric kind).
//! - The R2 delivery-side dispatch sentinel gives the callback indirection a number (proxy-side).
//!
//! Safety: gate-minted `Allowed` for 41/63 only; production `61:00.0` never touched; reversible.
//! Run (built with `--features bench`): `sudo ./s8_slice2`.

use directpath::harness::{err, payload_of, pct, read_nonce, well_formed};
use directpath::internals::gate::Bdf;
use directpath::{Ctx, Event, Subsystem};
use std::collections::BTreeSet;
use std::io;
use std::ops::ControlFlow;
use std::time::Instant;

const BENCH_41: &str = "0000:41:00.0";
const BENCH_63: &str = "0000:63:00.0";
const RTT_N: usize = 200;
const STORM_K: usize = 32;
const STORM_ROUNDS: usize = 4; // 32*4 = 128 frames = 2 full ring-wraps (depth 64), each way
const BOX_CORE: usize = 4;
const PROXY_CORE: usize = 6;
/// Per-drive inactivity guard: if no progress for this long, a hardware stall is surfaced as an error
/// (checked on the Idle tick) instead of hanging `run()` forever.
const STALL: std::time::Duration = std::time::Duration::from_secs(10);

/// What the box thread reports back after its two phases.
struct BoxReport {
    rtts_us: Vec<f64>,
    tp_elapsed_s: f64,
    idle_wakeup_us: Vec<f64>,
    dispatch_us: Vec<f64>,
}

/// What the proxy thread reports back.
struct ProxyReport {
    delivered: BTreeSet<Vec<u8>>,
    total_deliveries: usize,
    foreign: bool,
    rxmpc0: u32,
    gprc: u32,
    malformed: u32,
    idle_wakeup_us: Vec<f64>,
    dispatch_us: Vec<f64>,
}

fn main() {
    match run() {
        Ok(()) => println!("\n=== SLICE 2: SHIPPED-FACADE TWO-INSTANCE CONSERVATION PROVEN ==="),
        Err(e) => {
            eprintln!("\n=== SLICE 2 FAILED: {e} ===");
            std::process::exit(1);
        }
    }
}

fn run() -> io::Result<()> {
    // Self-declared provenance: binds every number below to THIS running binary's bytes (the shipped facade).
    println!("{}\n", benchtools::Provenance::capture().header("s8_slice2"));

    let bdf41 = Bdf::parse(BENCH_41).ok_or_else(|| err("bad BDF 41"))?;
    let bdf63 = Bdf::parse(BENCH_63).ok_or_else(|| err("bad BDF 63"))?;
    let allow = [bdf41, bdf63];

    // Pre-generate all payloads OUTSIDE the timed region (urandom off the hot path).
    let rtt_payloads: Vec<Vec<u8>> =
        (0..RTT_N).map(|_| read_nonce().map(|n| payload_of(&n))).collect::<io::Result<_>>()?;
    let storm_payloads: Vec<Vec<u8>> = (0..STORM_K * STORM_ROUNDS)
        .map(|_| read_nonce().map(|n| payload_of(&n)))
        .collect::<io::Result<_>>()?;
    let storm_sent: BTreeSet<Vec<u8>> = storm_payloads.iter().cloned().collect();
    let grand_total = RTT_N + STORM_K * STORM_ROUNDS;

    let (box_r, proxy_r) = directpath::harness::two_facade(
        bdf41,
        bdf63,
        &allow,
        BOX_CORE,
        PROXY_CORE,
        None, // idle-park cap: uncapped (conservation harnesses)
        // ---- BOX (41): one-directional throughout. Phase 1 send→ACK RTT (paced on quiescence), then the
        // phase 2 burst storm; the box originates, the proxy only receives + auto-ACKs (sole allocator). ----
        move |mut sys: Subsystem| -> io::Result<BoxReport> {
            // === PHASE 1: N send→ACK round-trips. RTT = send → quiescent (the ACK arrived, next_deadline
            // None). One-directional: the box receives only (hidden) ACKs, never a delivery. ===
            let mut rtts_us: Vec<f64> = Vec::with_capacity(RTT_N);
            let mut done = 0usize;
            let mut unexpected = false;
            let mut stalled = false;
            let mut last_ack = Instant::now();
            let tp0 = Instant::now();
            let mut sent_at = Instant::now();
            sys.open(&rtt_payloads[0]);
            sys.run(|ctx: &mut Ctx, ev: Event| match ev {
                Event::Delivered(..) => {
                    unexpected = true; // a delivery on the box would mean the proxy originated (defect)
                    ControlFlow::Break(())
                }
                Event::Idle => {
                    if ctx.is_quiescent() {
                        // The current send has been ACKed → record the round-trip and fire the next.
                        rtts_us.push(sent_at.elapsed().as_nanos() as f64 / 1000.0);
                        last_ack = Instant::now();
                        done += 1;
                        if done >= RTT_N {
                            ControlFlow::Break(())
                        } else {
                            sent_at = Instant::now();
                            ctx.open(&rtt_payloads[done]);
                            ControlFlow::Continue(())
                        }
                    } else if last_ack.elapsed() > STALL {
                        stalled = true;
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                }
            });
            let tp_elapsed_s = tp0.elapsed().as_secs_f64();
            if stalled {
                eprintln!(
                    "[box] STALL diag: GPTC(tx)={} GPRC(rx)={} RXMPC0={}",
                    sys.counter(directpath::internals::datapath::GPTC),
                    sys.counter(directpath::internals::datapath::GPRC),
                    sys.counter(directpath::internals::datapath::RXMPC0)
                );
                return Err(err(format!("phase 1 STALLED after {done}/{RTT_N} round-trips (no ACK — frame not flowing)")));
            }
            if unexpected {
                return Err(err("phase 1: box received a delivery (proxy must not originate — sole allocator)"));
            }

            // === PHASE 2: STORM_ROUNDS bursts of K ONE-DIRECTIONAL sessions (adjacent-line drain + re-arm).
            // One-directional (peak in-flight = K <= ring depth) is the faithful storm: an echo would put
            // ~2K in flight (ACK + echo per request) and overrun. The box is a silent sender here — it
            // paces rounds on the Idle tick, bursting the next round once the current is fully ACKed. ===
            let mut unexpected = false;
            let mut stalled2 = false;
            let mut last_round = Instant::now();
            for p in &storm_payloads[0..STORM_K] {
                sys.open(p); // prime round 0's K requests → K frames burst onto the proxy's ring
            }
            let mut round = 1usize;
            sys.run(|ctx: &mut Ctx, ev: Event| match ev {
                // One-directional: the box sees only (hidden) ACKs; a delivery here would be a defect.
                Event::Delivered(..) => {
                    unexpected = true;
                    ControlFlow::Break(())
                }
                Event::Idle => {
                    if !ctx.is_quiescent() {
                        if last_round.elapsed() > STALL {
                            stalled2 = true;
                            return ControlFlow::Break(());
                        }
                        return ControlFlow::Continue(()); // current round still has frames in flight
                    }
                    last_round = Instant::now(); // a round completed (fully ACKed) — progress
                    if round < STORM_ROUNDS {
                        let base = round * STORM_K;
                        for p in &storm_payloads[base..base + STORM_K] {
                            ctx.open(p);
                        }
                        round += 1;
                        ControlFlow::Continue(())
                    } else {
                        ControlFlow::Break(()) // all rounds sent AND fully ACKed
                    }
                }
            });
            if stalled2 {
                return Err(err(format!("phase 2 STALLED at round {round}/{STORM_ROUNDS} (a burst never fully ACKed)")));
            }
            if unexpected {
                return Err(err("phase 2: box received an unexpected delivery (storm must be one-directional)"));
            }

            Ok(BoxReport {
                rtts_us,
                tp_elapsed_s,
                idle_wakeup_us: sys.idle_wakeups_us().to_vec(),
                dispatch_us: sys.dispatch_us().to_vec(),
            })
        },
        // ---- PROXY (63): receive + auto-ACK + collect the delivered set (never originates) ----
        move |mut sys: Subsystem| -> io::Result<ProxyReport> {
            let mut delivered: BTreeSet<Vec<u8>> = BTreeSet::new();
            let mut total = 0usize;
            let mut foreign = false;
            let mut pstalled = false;
            let mut last = Instant::now();
            sys.run(|_ctx: &mut Ctx, ev: Event| {
                let req = match ev {
                    Event::Delivered(_, p) => {
                        last = Instant::now();
                        p
                    }
                    Event::Idle => {
                        if last.elapsed() > STALL {
                            pstalled = true;
                            return ControlFlow::Break(());
                        }
                        return ControlFlow::Continue(());
                    }
                };
                total += 1;
                if !well_formed(&req) {
                    foreign = true;
                    return ControlFlow::Break(());
                }
                delivered.insert(req); // the transport auto-ACKs; we only collect
                if total >= grand_total {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
            if pstalled {
                eprintln!(
                    "[proxy] STALL diag: GPRC(rx)={} RXMPC0={} GPTC(tx)={}",
                    sys.counter(directpath::internals::datapath::GPRC),
                    sys.counter(directpath::internals::datapath::RXMPC0),
                    sys.counter(directpath::internals::datapath::GPTC)
                );
                return Err(err(format!("proxy STALLED after {total} deliveries (box not sending / frames lost)")));
            }
            Ok(ProxyReport {
                delivered,
                total_deliveries: total,
                foreign,
                rxmpc0: sys.counter(directpath::internals::datapath::RXMPC0),
                gprc: sys.counter(directpath::internals::datapath::GPRC),
                malformed: sys.malformed(),
                idle_wakeup_us: sys.idle_wakeups_us().to_vec(),
                dispatch_us: sys.dispatch_us().to_vec(),
            })
        },
    )?;

    report(&box_r, &proxy_r, &storm_sent, grand_total)
}

fn report(
    box_r: &BoxReport,
    proxy_r: &ProxyReport,
    storm_sent: &BTreeSet<Vec<u8>>,
    grand_total: usize,
) -> io::Result<()> {
    // ---- CORRECTNESS (conserved) ----
    println!("--- CORRECTNESS (conserved): byte-equal delivery + device counters ---");
    if proxy_r.foreign {
        return Err(err("proxy received a malformed/foreign payload (corruption or demux mis-route)"));
    }
    if proxy_r.malformed != 0 {
        return Err(err(format!("proxy malformed={} (corruption)", proxy_r.malformed)));
    }
    if proxy_r.total_deliveries != grand_total {
        return Err(err(format!(
            "proxy delivered {} app frames, expected {grand_total} (loss or spurious re-delivery)",
            proxy_r.total_deliveries
        )));
    }
    if !storm_sent.is_subset(&proxy_r.delivered) {
        return Err(err("storm sent-set NOT fully delivered byte-equal (loss/corruption)"));
    }
    if proxy_r.rxmpc0 != 0 {
        return Err(err(format!("proxy RXMPC0={} (RX overrun under load)", proxy_r.rxmpc0)));
    }
    println!(
        "  {grand_total}/{grand_total} app frames delivered byte-equal (storm set complete); proxy RXMPC0={}, GPRC={}",
        proxy_r.rxmpc0, proxy_r.gprc
    );

    // ---- IDLE-WAKEUP (conserved): sub-tick floor + median band ----
    println!("\n--- IDLE-WAKEUP latency, measured INSIDE run() (conserved) ---");
    for (who, w) in [("proxy(63)", &proxy_r.idle_wakeup_us), ("box(41)", &box_r.idle_wakeup_us)] {
        let mut s = w.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if s.is_empty() {
            println!("  {who}: no NIC-driven parks recorded");
            continue;
        }
        let worst = s[s.len() - 1];
        let sub_tick = worst < 1000.0; // < 1ms: the make-or-break floor (330x gap to the ~4ms OS tick)
        println!(
            "  {who}: n={} median {:.1}us p90 {:.1}us max {:.1}us  [sub-tick floor (<1ms): {}]",
            s.len(),
            pct(&s, 0.5),
            pct(&s, 0.9),
            worst,
            if sub_tick { "HOLDS" } else { "VIOLATED" }
        );
        if !sub_tick {
            return Err(err(format!("{who}: a wake was NOT sub-tick (max {worst:.1}us >= 1ms) — floor VIOLATED")));
        }
    }

    // ---- R2 delivery-side dispatch sentinel (NEW baseline; gives the callback indirection a number) ----
    println!("\n--- R2 delivery-side dispatch sentinel: drain-start → all-delivered-to-handler (NEW baseline) ---");
    for (who, d) in [("proxy(63)", &proxy_r.dispatch_us), ("box(41)", &box_r.dispatch_us)] {
        let mut s = d.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if s.is_empty() {
            println!("  {who}: no delivering passes");
            continue;
        }
        println!(
            "  {who}: n={} median {:.2}us p90 {:.2}us worst-of-burst {:.2}us  (handler-inclusive; see note)",
            s.len(),
            pct(&s, 0.5),
            pct(&s, 0.9),
            s[s.len() - 1]
        );
    }

    // ---- send→ACK RTT + throughput (CONSERVED metric via is_quiescent, one-directional; the two-thread
    // box↔proxy topology runs faster than the original single-thread loopback, so the value differs) ----
    println!("\n--- send→ACK RTT + throughput (CONSERVED metric; two-thread topology faster than 1-thread loopback) ---");
    let mut rtts = box_r.rtts_us.clone();
    rtts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = rtts.iter().sum::<f64>() / rtts.len() as f64;
    let msgs_s = RTT_N as f64 / box_r.tp_elapsed_s;
    println!(
        "  send→ACK RTT: mean {mean:.1}us median {:.1}us min {:.1}us max {:.1}us (N={RTT_N})  (vs loopback ~138.8us)",
        pct(&rtts, 0.5),
        rtts[0],
        rtts[rtts.len() - 1]
    );
    println!("  throughput = {msgs_s:.0} msg/s  (send→ACK; vs the loopback ~7165 msg/s)");

    Ok(())
}
