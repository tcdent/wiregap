#![deny(unsafe_code)]

//! **Sprint 7 Slice 2 — steady-state ring management, re-run through the SHIPPED facade** over the live DAC
//! `41:00.0 ↔ 63:00.0`. Box-role on 41, proxy-role on 63, each [`directpath::Subsystem::run`] in its own
//! thread. Two conserved outcomes:
//!
//! - **Steady-state stream** (phase A): N sequential app round-trips (echo) past several ring wraps — all
//!   byte-equal (no loss ⇒ the TX tail stayed monotonic), zero TX drops + zero RX miss (no overrun /
//!   starvation / leak). Sequential echo = one in flight, no burst overrun.
//! - **Retransmit + give-up** (phase B): the proxy disables its receiver (genuine silence); the box sends
//!   one frame into the void and — as a SILENT sender driving `run()` on the Idle tick — retransmits on
//!   silence, then gives up + reclaims. Observed on the box's own GPTC (read-to-clear): emits through the
//!   give-up window ≥ 2 (retransmits fired), emits after == 0 (gave up, silent, reclaimed). Give-up is
//!   deliberately silent — the box detects the windows on its OWN deadline, there is no give-up event.
//!
//! Frozen: reactor algorithm + transport + guard/copy-boundary. Safety spine unchanged; production 61
//! untouched. Run (built `--features bench`): `sudo ./s7_slice2`.

use directpath::harness::{err, payload_of, read_nonce, well_formed};
use directpath::internals::datapath::{GPTC, RXMPC0};
use directpath::internals::gate::Bdf;
use directpath::{Ctx, Event, Subsystem};
use std::collections::BTreeSet;
use std::io;
use std::ops::ControlFlow;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const BENCH_41: &str = "0000:41:00.0";
const BENCH_63: &str = "0000:63:00.0";
const N: usize = 128; // 2 full wraps of the depth-64 rings → slot reuse + monotonic tail
const GIVEUP_WINDOW_MS: u64 = 2300; // past the ~2s give-up bound (retransmits fire on silence)
const POST_WINDOW_MS: u64 = 600; // after give-up: no more emits (reclaimed)
const BOX_CORE: usize = 4;
const PROXY_CORE: usize = 6;
const STALL: Duration = Duration::from_secs(10); // per-drive inactivity guard (Idle-checked)

struct BoxReport {
    rxmpc41: u32,
    dropped41: u64,
    malformed41: u32,
    emits_through_giveup: u32,
    emits_after_giveup: u32,
}

struct ProxyReport {
    received: usize,
    delivered: BTreeSet<Vec<u8>>,
    foreign: bool,
    rxmpc63: u32,
    dropped63: u64,
    malformed63: u32,
}

fn main() {
    match run() {
        Ok(()) => println!("\n=== SLICE 2: STEADY-STATE RING MANAGEMENT PROVEN (shipped facade) ==="),
        Err(e) => {
            eprintln!("\n=== SLICE 2 FAILED: {e} ===");
            std::process::exit(1);
        }
    }
}

fn run() -> io::Result<()> {
    println!("{}\n", benchtools::Provenance::capture().header("s7_slice2"));
    let bdf41 = Bdf::parse(BENCH_41).ok_or_else(|| err("bad BDF 41"))?;
    let bdf63 = Bdf::parse(BENCH_63).ok_or_else(|| err("bad BDF 63"))?;
    let allow = [bdf41, bdf63];

    let payloads: Vec<Vec<u8>> =
        (0..N).map(|_| read_nonce().map(|n| payload_of(&n))).collect::<io::Result<_>>()?;
    let sent: BTreeSet<Vec<u8>> = payloads.iter().cloned().collect();
    let void_payload = payload_of(&read_nonce()?);

    // Phase coordination: proxy signals "receiver off, go" → box; box signals "give-up done" → proxy.
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel::<()>();

    let (box_r, proxy_r) = directpath::harness::two_facade(
        bdf41,
        bdf63,
        &allow,
        BOX_CORE,
        PROXY_CORE,
        // ---- BOX (41): phase A one-directional send→ACK stream, then phase B silent give-up drive ----
        move |mut sys: Subsystem| -> io::Result<BoxReport> {
            // PHASE A: N one-directional send→ACK round-trips, paced on quiescence (box originates, proxy
            // receives + auto-ACKs; sole allocator — no echo).
            let mut done = 0usize;
            let mut unexpected = false;
            let mut stalled = false;
            let mut last_ack = Instant::now();
            sys.open(&payloads[0]);
            sys.run(|ctx: &mut Ctx, ev: Event| match ev {
                Event::Delivered(..) => {
                    unexpected = true;
                    ControlFlow::Break(())
                }
                Event::Idle => {
                    if ctx.is_quiescent() {
                        last_ack = Instant::now();
                        done += 1;
                        if done >= N {
                            ControlFlow::Break(())
                        } else {
                            ctx.open(&payloads[done]);
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
            if stalled {
                return Err(err(format!("phase A STALLED after {done}/{N} round-trips (no ACK)")));
            }
            if unexpected {
                return Err(err("phase A: box received a delivery (proxy must not originate — sole allocator)"));
            }
            let rxmpc41 = sys.counter(RXMPC0);
            let dropped41 = sys.tx_dropped();
            let malformed41 = sys.malformed();

            // PHASE B: wait for the proxy to go silent, then drive the give-up.
            ready_rx.recv().map_err(|_| err("proxy did not signal receiver-off"))?;
            let _ = sys.counter(GPTC); // clear the TX counter baseline (read-to-clear)
            sys.open(&void_payload); // first emit into the void
            let t0 = Instant::now();
            sys.run(|_ctx: &mut Ctx, ev: Event| match ev {
                Event::Delivered(..) => ControlFlow::Break(()), // silence expected; a delivery ends the drive
                Event::Idle => {
                    if t0.elapsed() > Duration::from_millis(GIVEUP_WINDOW_MS) {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(()) // retransmits fire underneath while we wait
                    }
                }
            });
            let emits_through_giveup = sys.counter(GPTC); // 1 original + retransmits on silence
            let t1 = Instant::now();
            sys.run(|_ctx: &mut Ctx, ev: Event| match ev {
                Event::Delivered(..) => ControlFlow::Break(()),
                Event::Idle => {
                    if t1.elapsed() > Duration::from_millis(POST_WINDOW_MS) {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                }
            });
            let emits_after_giveup = sys.counter(GPTC); // 0 ⇒ gave up + reclaimed, silent
            let _ = done_tx.send(());

            Ok(BoxReport { rxmpc41, dropped41, malformed41, emits_through_giveup, emits_after_giveup })
        },
        // ---- PROXY (63): receive + auto-ACK + collect the stream, then go silent for the give-up ----
        move |mut sys: Subsystem| -> io::Result<ProxyReport> {
            let mut received = 0usize;
            let mut delivered: BTreeSet<Vec<u8>> = BTreeSet::new();
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
                if !well_formed(&req) {
                    foreign = true;
                    return ControlFlow::Break(());
                }
                received += 1;
                delivered.insert(req);
                if received >= N {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
            if pstalled {
                return Err(err(format!("proxy STALLED after {received}/{N} (box not sending / frames lost)")));
            }
            let rxmpc63 = sys.counter(RXMPC0);
            let dropped63 = sys.tx_dropped();
            let malformed63 = sys.malformed();

            // Go silent (drop at the MAC) → genuine silence for the box's give-up, then hold until it is done.
            sys.set_rx_enabled(false);
            let _ = ready_tx.send(());
            let _ = done_rx.recv();

            Ok(ProxyReport { received, delivered, foreign, rxmpc63, dropped63, malformed63 })
        },
    )?;

    report(&box_r, &proxy_r, &sent)
}

fn report(box_r: &BoxReport, proxy_r: &ProxyReport, sent: &BTreeSet<Vec<u8>>) -> io::Result<()> {
    // PHASE A — steady-state health.
    println!("--- PHASE A: {N} round-trips (2 ring wraps), steady-state health ---");
    if proxy_r.foreign {
        return Err(err("proxy saw a malformed payload"));
    }
    if proxy_r.received != N || &proxy_r.delivered != sent {
        return Err(err(format!("proxy received {} of {N} byte-equal (loss/corruption)", proxy_r.received)));
    }
    if box_r.dropped41 != 0 || proxy_r.dropped63 != 0 {
        return Err(err(format!("TX drops (41={} 63={}) — ring stall/overrun", box_r.dropped41, proxy_r.dropped63)));
    }
    if box_r.rxmpc41 != 0 || proxy_r.rxmpc63 != 0 {
        return Err(err(format!("RXMPC (41={} 63={}) — RX overrun/starvation", box_r.rxmpc41, proxy_r.rxmpc63)));
    }
    if box_r.malformed41 != 0 || proxy_r.malformed63 != 0 {
        return Err(err("malformed frames on the wire"));
    }
    println!(
        "  all {N} round-trips byte-equal (no loss ⇒ monotonic tail); RXMPC 41={} 63={}, TX dropped 41={} 63={} (rings stayed full, no leak)",
        box_r.rxmpc41, proxy_r.rxmpc63, box_r.dropped41, proxy_r.dropped63
    );

    // PHASE B — retransmit + give-up.
    println!("\n--- PHASE B: retransmit + give-up (63 receiver OFF → genuine silence) ---");
    println!(
        "  41 emits through give-up window = {} (1 original + retransmits on silence)",
        box_r.emits_through_giveup
    );
    println!("  41 emits AFTER give-up window  = {} (0 ⇒ gave up + reclaimed, silent)", box_r.emits_after_giveup);
    if box_r.emits_through_giveup < 2 {
        return Err(err("retransmit did not fire on silence"));
    }
    if box_r.emits_after_giveup != 0 {
        return Err(err("give-up did not reclaim — 41 kept emitting past the give-up bound"));
    }
    println!("  => retransmit FIRES on silence; give-up RECLAIMS (emits stop) — driven silently on the Idle tick.");
    Ok(())
}
