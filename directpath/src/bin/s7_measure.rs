#![deny(unsafe_code)]

//! **Sprint 7 Slice 4a — the integration MEASUREMENTS, re-run through the SHIPPED facade** over the live DAC
//! `41:00.0 ↔ 63:00.0`. Box-role on 41, proxy-role on 63, each [`directpath::Subsystem::run`] in its own
//! thread. Conserved outcomes (the three premise-gates), plus the new-baseline characterizations:
//!
//! - **M1 throughput** (NEW baseline): N sequential app round-trips (echo), wall-clock → msg/s. This is the
//!   same hidden send→ACK metric as the old 7165 msg/s (msg/s = 1/RTT), so it is a NEW facade baseline, not
//!   a conservation target (Sprint-9 Fork-1 R1).
//! - **M2/3/4 storms → gates a/b/c** (CONSERVED): for each K, `STORM_ROUNDS` rounds of K **one-directional**
//!   concurrent sessions (box bursts K, proxy sinks + auto-ACKs — the original traffic, peak in-flight = K;
//!   an echo would put ~2K in flight and break gate-c by construction). The box is a silent sender: it
//!   paces each round on the Idle tick, bursting the next once the current is fully ACKed (`Ctx::is_quiescent`).
//!   Gate a: RXMPC0 == 0 (no overrun) + no malformed. Gate b: worst round completion < the ~10 ms retransmit
//!   window. Gate c: TxPath dropped == 0 + K <= ring depth.
//!
//! Frozen: reactor algorithm + transport + guard/copy-boundary. Safety spine unchanged; production 61
//! untouched. Run (built `--features bench`): `sudo ./s7_measure`.

use directpath::harness::{err, payload_of, read_nonce, well_formed};
use directpath::internals::datapath::RXMPC0;
use directpath::internals::gate::Bdf;
use directpath::{Ctx, Event, Subsystem};
use std::io;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};
use transport::DEFAULT_RETRANSMIT_INTERVAL;

const BENCH_41: &str = "0000:41:00.0";
const BENCH_63: &str = "0000:63:00.0";
const N_THROUGHPUT: usize = 200;
const STORM_ROUNDS: usize = 10;
const STORM_KS: [usize; 2] = [32, 60]; // K=32 AND a larger K near the ring depth (64)
const DEPTH: usize = 64;
const BOX_CORE: usize = 4;
const PROXY_CORE: usize = 6;
const STALL: Duration = Duration::from_secs(10); // per-drive inactivity guard (Idle-checked)

/// Per-K storm outcome measured box-side.
struct StormK {
    k: usize,
    worst: Duration,
    rxmpc41: u32,
    dropped41: u64,
}

struct BoxReport {
    tp_elapsed_s: f64,
    storms: Vec<StormK>,
    n_nonces: usize,
}

struct ProxyReport {
    total: usize,
    foreign: bool,
    rxmpc63: u32,
    dropped63: u64,
    malformed: u32,
}

fn main() {
    match run() {
        Ok(()) => println!("\n=== SLICE 4a: MEASUREMENTS COMPLETE (shipped facade) ==="),
        Err(e) => {
            eprintln!("\n=== SLICE 4a FAILED: {e} ===");
            std::process::exit(1);
        }
    }
}

fn run() -> io::Result<()> {
    println!("{}\n", benchtools::Provenance::capture().header("s7_measure"));
    let bdf41 = Bdf::parse(BENCH_41).ok_or_else(|| err("bad BDF 41"))?;
    let bdf63 = Bdf::parse(BENCH_63).ok_or_else(|| err("bad BDF 63"))?;
    let allow = [bdf41, bdf63];

    let tp_payloads: Vec<Vec<u8>> =
        (0..N_THROUGHPUT).map(|_| read_nonce().map(|n| payload_of(&n))).collect::<io::Result<_>>()?;
    let n_storm: usize = STORM_KS.iter().sum::<usize>() * STORM_ROUNDS;
    let storm_payloads: Vec<Vec<u8>> =
        (0..n_storm).map(|_| read_nonce().map(|n| payload_of(&n))).collect::<io::Result<_>>()?;
    let grand_total = N_THROUGHPUT + n_storm;

    let (box_r, proxy_r) = directpath::harness::two_facade(
        bdf41,
        bdf63,
        &allow,
        BOX_CORE,
        PROXY_CORE,
        None, // idle-park cap: uncapped (conservation harnesses)
        // ---- BOX (41): M1 echo throughput, then the one-directional Idle-paced storms ----
        move |mut sys: Subsystem| -> io::Result<BoxReport> {
            // M1: N one-directional send→ACK round-trips (paced on quiescence), wall-clocked.
            let mut done = 0usize;
            let mut unexpected = false;
            let mut stalled = false;
            let mut last_ack = Instant::now();
            let tp0 = Instant::now();
            sys.open(&tp_payloads[0]);
            sys.run(|ctx: &mut Ctx, ev: Event| match ev {
                Event::Delivered(..) => {
                    unexpected = true;
                    ControlFlow::Break(())
                }
                Event::Idle => {
                    if ctx.is_quiescent() {
                        last_ack = Instant::now();
                        done += 1;
                        if done >= N_THROUGHPUT {
                            ControlFlow::Break(())
                        } else {
                            ctx.open(&tp_payloads[done]);
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
                return Err(err(format!("M1 STALLED after {done}/{N_THROUGHPUT} (no ACK)")));
            }
            if unexpected {
                return Err(err("M1: box received a delivery (proxy must not originate)"));
            }

            // M2/3/4: one-directional storms, per K.
            let mut storms = Vec::new();
            let mut idx = 0usize;
            for &k in &STORM_KS {
                let rxmpc_b = sys.counter(RXMPC0);
                let dropped_b = sys.tx_dropped();
                let mut worst = Duration::ZERO;
                for _round in 0..STORM_ROUNDS {
                    let base = idx;
                    let t0 = Instant::now();
                    for p in &storm_payloads[base..base + k] {
                        sys.open(p);
                    }
                    idx += k;
                    sys.run(|ctx: &mut Ctx, ev: Event| match ev {
                        Event::Delivered(..) => {
                            unexpected = true;
                            ControlFlow::Break(())
                        }
                        // Round is complete once every burst frame has been ACKed (quiescent).
                        Event::Idle => {
                            if ctx.is_quiescent() {
                                ControlFlow::Break(())
                            } else if t0.elapsed() > STALL {
                                stalled = true;
                                ControlFlow::Break(())
                            } else {
                                ControlFlow::Continue(())
                            }
                        }
                    });
                    worst = worst.max(t0.elapsed());
                    if stalled {
                        return Err(err(format!("storm K={k} STALLED (a burst never fully ACKed)")));
                    }
                }
                storms.push(StormK {
                    k,
                    worst,
                    rxmpc41: sys.counter(RXMPC0) - rxmpc_b,
                    dropped41: sys.tx_dropped() - dropped_b,
                });
            }
            if unexpected {
                return Err(err("storm: box received an unexpected delivery (must be one-directional)"));
            }
            Ok(BoxReport { tp_elapsed_s, storms, n_nonces: N_THROUGHPUT + n_storm })
        },
        // ---- PROXY (63): receive + auto-ACK + count (M1 and the storms alike; never originates) ----
        move |mut sys: Subsystem| -> io::Result<ProxyReport> {
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
                // The transport auto-ACKs on receipt; the proxy only counts (no origination).
                if total >= grand_total {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
            if pstalled {
                return Err(err(format!("proxy STALLED after {total} (box not sending / frames lost)")));
            }
            Ok(ProxyReport {
                total,
                foreign,
                rxmpc63: sys.counter(RXMPC0),
                dropped63: sys.tx_dropped(),
                malformed: sys.malformed(),
            })
        },
    )?;

    report(&box_r, &proxy_r, grand_total)
}

fn report(box_r: &BoxReport, proxy_r: &ProxyReport, grand_total: usize) -> io::Result<()> {
    if proxy_r.foreign {
        return Err(err("proxy received a malformed/foreign payload"));
    }
    if proxy_r.total != grand_total {
        return Err(err(format!("proxy saw {} frames, expected {grand_total} (loss)", proxy_r.total)));
    }
    if proxy_r.malformed != 0 {
        return Err(err(format!("proxy malformed={}", proxy_r.malformed)));
    }

    // M1 — send→ACK RTT + throughput (CONSERVED metric via is_quiescent; the two-thread box↔proxy topology
    // runs faster than the original single-thread loopback, so the VALUE is lower/higher — same metric kind).
    let rtt_us = box_r.tp_elapsed_s * 1e6 / N_THROUGHPUT as f64;
    let msgs_s = N_THROUGHPUT as f64 / box_r.tp_elapsed_s;
    println!("--- M1: send→ACK RTT + throughput (CONSERVED metric; two-thread topology faster than the 1-thread loopback) ---");
    println!("  N={N_THROUGHPUT}, send→ACK RTT {rtt_us:.1}us → {msgs_s:.0} msg/s  (vs the loopback ~138.8us / 7165 msg/s)");

    // M2/3/4 — the conserved premise-gates.
    let window = DEFAULT_RETRANSMIT_INTERVAL;
    println!("\n--- M2/3/4: one-directional Idle-paced storms → gates a/b/c (CONSERVED) ---");
    if proxy_r.rxmpc63 != 0 {
        return Err(err(format!("proxy RXMPC0={} (gate a: RX overrun)", proxy_r.rxmpc63)));
    }
    if proxy_r.dropped63 != 0 {
        return Err(err(format!("proxy TxPath dropped={} (gate c)", proxy_r.dropped63)));
    }
    for s in &box_r.storms {
        // Gate a — keeps up on one core: no unbounded backlog (RXMPC=0).
        if s.rxmpc41 != 0 {
            return Err(err(format!("GATE a VIOLATED (K={}): box RXMPC0={}", s.k, s.rxmpc41)));
        }
        // Gate c — DMA pool >= peak ACK-hold: K in flight <= ring-depth pool; dropped==0 ⇒ absorbed.
        if s.dropped41 != 0 || s.k > DEPTH {
            return Err(err(format!("GATE c VIOLATED (K={}): box dropped={} vs pool {DEPTH}", s.k, s.dropped41)));
        }
        // Gate b — the ~10ms window vs the worst round completion (busy-drain ⇒ expect sub-ms).
        if s.worst >= window {
            return Err(err(format!(
                "GATE b VIOLATED (K={}): worst {:.3}ms >= {:?} window",
                s.k,
                s.worst.as_secs_f64() * 1e3,
                window
            )));
        }
        println!(
            "  [K={}] {STORM_ROUNDS} rounds; worst round completion {:.3}ms << {:?} window | box RXMPC0={} dropped={} | gates a/b/c PASS (peak in-flight {} <= pool {DEPTH})",
            s.k,
            s.worst.as_secs_f64() * 1e3,
            window,
            s.rxmpc41,
            s.dropped41,
            s.k
        );
    }
    println!(
        "  => all three premise-gates PASS at every K {STORM_KS:?}: RXMPC0=0 (a), worst-of-burst << 10ms (b), dropped=0 + peak=K<=pool (c). {} fresh frames.",
        box_r.n_nonces
    );
    Ok(())
}
