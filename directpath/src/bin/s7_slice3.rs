#![deny(unsafe_code)]

//! **Sprint 7 Slice 3 — multiplexed sessions, re-run through the SHIPPED facade** over the live DAC
//! `41:00.0 ↔ 63:00.0`. Box-role on 41, proxy-role on 63, each [`directpath::Subsystem::run`] in its own
//! thread. Three conserved outcomes:
//!
//! - **Multiplex demux** (phase 1): K concurrent one-directional sessions `41 → 63`, all in flight at once;
//!   63 demuxes each to its session (byte-equal, no foreign, no loss). One-directional (peak in-flight = K,
//!   the original traffic) — the box bursts K and paces to quiescence on the Idle tick.
//! - **Quiescent = no leak** (phase 2): once every session has round-tripped + been released, the box emits
//!   ZERO over a window (a silent observation on the Idle tick — a leaked/stuck session would keep emitting).
//! - **Dormant ≠ dead** (phase 3): a dead-peer session (63 receiver OFF) is reclaimed via give-up —
//!   retransmits on silence, then emits stop. Give-up is silent; the box detects the windows on its own
//!   deadline. (Dormant-survives + revive-on-next-frame is a FROZEN transport property, host-tested; the
//!   facade exposes `open` = open-by-emission, so revive-on-next-frame is not bench-observable here — noted.)
//!
//! Frozen: reactor algorithm + transport + guard/copy-boundary. Safety spine unchanged; production 61
//! untouched. Run (built `--features bench`): `sudo ./s7_slice3`.

use directpath::harness::{err, payload_of, read_nonce, well_formed};
use directpath::internals::datapath::{GPTC, RXMPC0};
use directpath::internals::gate::Bdf;
use directpath::{Ctx, Event, SessionId, Subsystem};
use std::collections::BTreeSet;
use std::io;
use std::ops::ControlFlow;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const BENCH_41: &str = "0000:41:00.0";
const BENCH_63: &str = "0000:63:00.0";
const K: usize = 32; // concurrent sessions, all in flight (< depth 64, no TX wrap during the burst)
const QUIESCENT_WINDOW_MS: u64 = 40;
const GIVEUP_WINDOW_MS: u64 = 2300;
const POST_WINDOW_MS: u64 = 600;
const BOX_CORE: usize = 4;
const PROXY_CORE: usize = 6;

struct BoxReport {
    emits_when_quiescent: u32,
    emits_through_giveup: u32,
    emits_after_giveup: u32,
    malformed41: u32,
}

struct ProxyReport {
    delivered: BTreeSet<Vec<u8>>,
    distinct_sessions: usize,
    foreign: bool,
    rxmpc63: u32,
    malformed63: u32,
}

fn main() {
    match run() {
        Ok(()) => println!("\n=== SLICE 3: MULTIPLEXED SESSIONS PROVEN (shipped facade) ==="),
        Err(e) => {
            eprintln!("\n=== SLICE 3 FAILED: {e} ===");
            std::process::exit(1);
        }
    }
}

fn run() -> io::Result<()> {
    println!("{}\n", benchtools::Provenance::capture().header("s7_slice3"));
    let bdf41 = Bdf::parse(BENCH_41).ok_or_else(|| err("bad BDF 41"))?;
    let bdf63 = Bdf::parse(BENCH_63).ok_or_else(|| err("bad BDF 63"))?;
    let allow = [bdf41, bdf63];

    let payloads: Vec<Vec<u8>> =
        (0..K).map(|_| read_nonce().map(|n| payload_of(&n))).collect::<io::Result<_>>()?;
    let sent: BTreeSet<Vec<u8>> = payloads.iter().cloned().collect();
    let void_payload = payload_of(&read_nonce()?);

    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel::<()>();

    let (box_r, proxy_r) = directpath::harness::two_facade(
        bdf41,
        bdf63,
        &allow,
        BOX_CORE,
        PROXY_CORE,
        None, // idle-park cap: uncapped (conservation harnesses)
        // ---- BOX (41): burst K, pace to quiescence; then the no-leak window; then the dead-peer give-up ----
        move |mut sys: Subsystem| -> io::Result<BoxReport> {
            // PHASE 1: open all K sessions at once → K frames interleaved on the wire.
            let mut unexpected = false;
            for p in &payloads {
                sys.open(p);
            }
            sys.run(|ctx: &mut Ctx, ev: Event| match ev {
                Event::Delivered(..) => {
                    unexpected = true;
                    ControlFlow::Break(())
                }
                Event::Idle => {
                    if ctx.is_quiescent() {
                        ControlFlow::Break(()) // all K round-tripped + released
                    } else {
                        ControlFlow::Continue(())
                    }
                }
            });
            if unexpected {
                return Err(err("multiplex: box received an unexpected delivery (must be one-directional)"));
            }

            // PHASE 2: quiescent = no leak — zero emits over a window (a stuck session would keep emitting).
            let _ = sys.counter(GPTC); // clear
            let t0 = Instant::now();
            sys.run(|_ctx: &mut Ctx, ev: Event| match ev {
                Event::Delivered(..) => ControlFlow::Break(()),
                Event::Idle => {
                    if t0.elapsed() > Duration::from_millis(QUIESCENT_WINDOW_MS) {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                }
            });
            let emits_when_quiescent = sys.counter(GPTC);

            // PHASE 3: dead-peer give-up (wait for the proxy to go silent first).
            ready_rx.recv().map_err(|_| err("proxy did not signal receiver-off"))?;
            let _ = sys.counter(GPTC); // clear
            sys.open(&void_payload);
            let t1 = Instant::now();
            sys.run(|_ctx: &mut Ctx, ev: Event| match ev {
                Event::Delivered(..) => ControlFlow::Break(()),
                Event::Idle => {
                    if t1.elapsed() > Duration::from_millis(GIVEUP_WINDOW_MS) {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                }
            });
            let emits_through_giveup = sys.counter(GPTC);
            let t2 = Instant::now();
            sys.run(|_ctx: &mut Ctx, ev: Event| match ev {
                Event::Delivered(..) => ControlFlow::Break(()),
                Event::Idle => {
                    if t2.elapsed() > Duration::from_millis(POST_WINDOW_MS) {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                }
            });
            let emits_after_giveup = sys.counter(GPTC);
            let malformed41 = sys.malformed();
            let _ = done_tx.send(());

            Ok(BoxReport { emits_when_quiescent, emits_through_giveup, emits_after_giveup, malformed41 })
        },
        // ---- PROXY (63): demux the K sessions (assert each delivered to its OWN SessionId) + collect;
        // then go silent for the give-up ----
        move |mut sys: Subsystem| -> io::Result<ProxyReport> {
            let mut delivered: BTreeSet<Vec<u8>> = BTreeSet::new();
            let mut sessions: BTreeSet<SessionId> = BTreeSet::new();
            let mut foreign = false;
            sys.run(|_ctx: &mut Ctx, ev: Event| {
                let (session, req) = match ev {
                    Event::Delivered(s, p) => (s, p),
                    Event::Idle => return ControlFlow::Continue(()),
                };
                if !well_formed(&req) {
                    foreign = true;
                    return ControlFlow::Break(());
                }
                sessions.insert(session); // per-delivery demux: each message keyed to its transport SessionId
                delivered.insert(req);
                if delivered.len() >= K {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
            let distinct_sessions = sessions.len();
            let rxmpc63 = sys.counter(RXMPC0);
            let malformed63 = sys.malformed();

            sys.set_rx_enabled(false); // dead peer: drop at the MAC
            let _ = ready_tx.send(());
            let _ = done_rx.recv();

            Ok(ProxyReport { delivered, distinct_sessions, foreign, rxmpc63, malformed63 })
        },
    )?;

    report(&box_r, &proxy_r, &sent)
}

fn report(box_r: &BoxReport, proxy_r: &ProxyReport, sent: &BTreeSet<Vec<u8>>) -> io::Result<()> {
    // PHASE 1 — multiplex demux.
    println!("--- PHASE 1: {K} concurrent sessions, demux ---");
    if proxy_r.foreign {
        return Err(err("63 delivered a FOREIGN/malformed payload — demux mis-route"));
    }
    if &proxy_r.delivered != sent {
        return Err(err(format!("demux set mismatch: delivered {} of {K} (loss or foreign)", proxy_r.delivered.len())));
    }
    if proxy_r.distinct_sessions != K {
        return Err(err(format!(
            "per-delivery demux: {} distinct SessionIds for {K} messages (a collision/mis-route)",
            proxy_r.distinct_sessions
        )));
    }
    if proxy_r.rxmpc63 != 0 {
        return Err(err(format!("63 RXMPC0={} under the burst", proxy_r.rxmpc63)));
    }
    if box_r.malformed41 != 0 || proxy_r.malformed63 != 0 {
        return Err(err("malformed frames during multiplex"));
    }
    println!("  63 demuxed + delivered ALL {K} sessions byte-equal to {} distinct SessionIds (no foreign, no loss); RXMPC0=0", proxy_r.distinct_sessions);

    // PHASE 2 — quiescent = no leak.
    println!("\n--- PHASE 2: quiescent = no leak ---");
    println!("  41 emits over a {QUIESCENT_WINDOW_MS}ms quiescent window = {} (0 ⇒ all {K} sessions ACKed + released, no leak)", box_r.emits_when_quiescent);
    if box_r.emits_when_quiescent != 0 {
        return Err(err("41 still emitting when quiescent — a session did not complete (leak)"));
    }

    // PHASE 3 — dead-peer give-up.
    println!("\n--- PHASE 3: dormant != dead — dead-peer give-up reclaim ---");
    println!(
        "  dead-peer session: {} emits through give-up (retransmits on silence), {} after (0 ⇒ reclaimed)",
        box_r.emits_through_giveup, box_r.emits_after_giveup
    );
    if box_r.emits_through_giveup < 2 || box_r.emits_after_giveup != 0 {
        return Err(err("dead-peer give-up did not reclaim as expected"));
    }
    println!("  => {K} concurrent sessions multiplexed (demux correct, no leak); an actively-failing session gave up + reclaimed.");
    println!("     (dormant-survives + revive-on-next-frame is a FROZEN transport property, host-tested — the facade exposes only send=open-by-emission; not bench-observable here.)");
    Ok(())
}
