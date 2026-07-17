#![deny(unsafe_code)]

//! **Sprint 7 Slice 2 — steady-state ring management** over the live DAC `41:00.0 ↔ 63:00.0`. A sustained
//! sequential stream of round-trips exercises the rings past several wraps: the MONOTONIC-PRODUCE-CURSOR
//! TX backing (tx.rs, the TDT-backwards fix), continuous RX refill (frozen `RxPath`), TX ACK-bound reclaim
//! (frozen transport `Rc` + on-DD DMA reclaim), and the retransmit loop (~10 ms) + timeout give-up (~2 s,
//! silent reclaim). Proven: N messages ALL byte-equal (no loss over the wraps ⇒ the tail stayed monotonic);
//! zero TX drops + zero RX misses (no overrun / no starvation / no leak); retransmit fires on genuine
//! silence and the give-up reclaims.
//!
//! The reactor algorithm + transport + guard/copy-boundary stay byte-frozen; only tx.rs backing + this
//! steady-state loop are new. Safety spine unchanged (Allowed token + gate 2; production 61 untouched).
//! Run: `sudo ./s7_slice2`.

use directpath::datapath::{self, Layout};
use directpath::ethframe::{EthSink, EthSource};
use directpath::gate::{Allowed, Bdf, Decision, gate};
use directpath::gate2::validate_physaddr;
use directpath::link::{LinkRestart, apply_restart, link_state, reset_and_init};
use directpath::mem::{self, Hugepage};
use directpath::pci::{self, Bar0};
use directpath::pollhost::PollHost;
use driver::{RxPath, RxRing, TxPath, VolatileRxRing, VolatileTxRing};
use reactor::{Reactor, RxDrainAsSource};
use std::io::{self, Read};
use std::time::{Duration, Instant};
use transport::SessionTable;

const BENCH_41: &str = "0000:41:00.0";
const BENCH_63: &str = "0000:63:00.0";
const POLL_INTERVAL: Duration = Duration::from_micros(50);
const NONCE_LEN: usize = 8;
const PAYLOAD_LEN: usize = 24;
const N: usize = 128; // 2 full wraps of the depth-64 rings → proves slot reuse + monotonic tail

type LiveSink = EthSink<TxPath<VolatileTxRing>>;
type LiveSource = EthSource<RxDrainAsSource<RxPath<VolatileRxRing>>>;
type LiveReactor = Reactor<PollHost, LiveSource, LiveSink>;

fn err(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

fn read_config(bdf: &Bdf) -> io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(format!("/sys/bus/pci/devices/{}/config", bdf.sysfs()))?;
    let mut buf = vec![0u8; 0x40];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

fn gate_bench(bdf: Bdf, allowlist: &[Bdf]) -> io::Result<Allowed> {
    match gate(bdf, allowlist, &read_config(&bdf)?) {
        Decision::Allow(tok) => Ok(tok),
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

fn build_reactor(bar: &Bar0, layout: &Layout, own_mac: [u8; 6], peer_mac: [u8; 6], name: &str) -> io::Result<LiveReactor> {
    if !datapath::init_tx(bar, layout) {
        return Err(err(format!("{name}: TXDCTL.ENABLE did not read back")));
    }
    datapath::init_rx(bar, layout);
    datapath::clear_rx_buffers(layout);
    if !datapath::rx_buffers_all_zero(layout) {
        return Err(err(format!("{name}: RX buffers not proven-cleared")));
    }
    let tx_ring = datapath::build_tx_ring(bar, layout);
    let mut rx_ring = datapath::build_rx_ring(bar, layout);
    if !datapath::start_rx(bar, layout) {
        return Err(err(format!("{name}: RXDCTL.ENABLE did not read back")));
    }
    // Arm ALL depth slots (ixy arms every num_entries buffer; RDT ends at depth-1 = one held back). Arming
    // only depth-1 would leave the last slot un-refilled, and the RX ring stalls when the consumer wraps
    // onto it (steady-state, not the single-frame path). With RDT=slot, the refill cursor tracks the
    // consumer one behind, so RDT never meets RDH.
    for slot in 0..datapath::DEPTH {
        rx_ring.refill(slot);
    }
    let sink = EthSink::new(TxPath::new(tx_ring), peer_mac, own_mac);
    let source = EthSource::new(RxDrainAsSource::new(RxPath::new(rx_ring)));
    println!("  {name}: TX+RX armed, reactor wired");
    Ok(Reactor::new(SessionTable::new(), PollHost::new(POLL_INTERVAL), source, sink))
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

/// One sequential round-trip: 41 originates the payload, 63 demux-reacts-delivers (byte-equal) + ACKs, 41
/// accepts the ACK. Errors on mismatch/timeout/malformed.
fn round_trip(r41: &mut LiveReactor, r63: &mut LiveReactor, payload: &[u8]) -> io::Result<()> {
    r41.originate(payload);
    let start = Instant::now();
    let delivered = loop {
        r63.pump_once();
        if let Some(p) = r63.take_delivery() {
            break p;
        }
        if start.elapsed() > Duration::from_secs(2) {
            return Err(err("63 did not deliver"));
        }
    };
    if delivered != payload {
        return Err(err("payload byte mismatch on 63"));
    }
    let start = Instant::now();
    let mut acked = false;
    while start.elapsed() < Duration::from_millis(50) {
        if r41.step() {
            acked = true;
            break;
        }
        r41.park();
    }
    if !acked {
        return Err(err("41 did not receive the ACK"));
    }
    if r41.take_delivery().is_some() {
        return Err(err("41: ACK must not deliver a payload up"));
    }
    Ok(())
}

fn main() {
    match run() {
        Ok(()) => println!("\n=== SLICE 2: STEADY-STATE RING MANAGEMENT PROVEN ==="),
        Err(e) => {
            eprintln!("\n=== SLICE 2 FAILED: {e} ===");
            std::process::exit(1);
        }
    }
}

fn run() -> io::Result<()> {
    let bdf41 = Bdf::parse(BENCH_41).ok_or_else(|| err("bad BDF 41"))?;
    let bdf63 = Bdf::parse(BENCH_63).ok_or_else(|| err("bad BDF 63"))?;
    let allowlist = [bdf41, bdf63];
    let tok41 = gate_bench(bdf41, &allowlist)?;
    let tok63 = gate_bench(bdf63, &allowlist)?;
    println!("gate: ALLOW {} and {}", bdf41.sysfs(), bdf63.sysfs());
    let drv41 = pci::current_driver(&tok41).unwrap_or_else(|| "ixgbe".into());
    let drv63 = pci::current_driver(&tok63).unwrap_or_else(|| "ixgbe".into());
    pci::unbind(&tok41)?;
    pci::unbind(&tok63)?;
    println!("unbound both");

    let result = steady_state(&tok41, &tok63);

    let r41 = pci::rebind(&tok41, &drv41);
    let r63 = pci::rebind(&tok63, &drv63);
    println!("rebind {}: {:?}; rebind {}: {:?}", bdf41.sysfs(), r41.is_ok(), bdf63.sysfs(), r63.is_ok());
    result?;
    r41?;
    r63?;
    Ok(())
}

fn steady_state(tok41: &Allowed, tok63: &Allowed) -> io::Result<()> {
    pci::enable_bus_master(tok41)?;
    pci::enable_bus_master(tok63)?;
    let bar41 = pci::map_resource0(tok41)?;
    let bar63 = pci::map_resource0(tok63)?;
    bring_up_link(&bar41, "41")?;
    bring_up_link(&bar63, "63")?;
    let mac41 = datapath::read_mac(&bar41);
    let mac63 = datapath::read_mac(&bar63);
    println!("MAC 41 = {}  MAC 63 = {}", hex(&mac41), hex(&mac63));

    let hp: Hugepage = mem::alloc()?;
    let split = Layout::used_bytes();
    if 2 * split > hp.size() {
        return Err(err("two carves exceed the hugepage"));
    }
    validate_physaddr(hp.va(), hp.phys(), hp.size() as u64).map_err(|e| err(format!("Gate 2 REFUSED: {e:?}")))?;
    let layout_41 = Layout::new(hp.va(), hp.phys());
    let layout_63 = Layout::new(hp.va() + split as u64, hp.phys() + split as u64);
    datapath::preinit_desc_rings(&layout_41);
    datapath::preinit_desc_rings(&layout_63);
    println!("hugepage Gate-2 validated (two carves)");

    let mut reactor_41 = build_reactor(&bar41, &layout_41, mac41, mac63, "41")?;
    let mut reactor_63 = build_reactor(&bar63, &layout_63, mac63, mac41, "63")?;

    // Baseline RXMPC on 63.
    let _ = datapath::read_counter(&bar63, datapath::RXMPC0);

    // ================= SUSTAINED STREAM: N sequential round-trips =================
    println!("\n--- SUSTAINED STREAM: {N} round-trips (2 ring wraps of depth {}) ---", datapath::DEPTH);
    let mut nonces: Vec<[u8; NONCE_LEN]> = Vec::with_capacity(N);
    for i in 0..N {
        let nonce = read_nonce()?;
        let mut payload = Vec::with_capacity(PAYLOAD_LEN);
        payload.extend_from_slice(&nonce);
        payload.extend_from_slice(&[0x5A; PAYLOAD_LEN - NONCE_LEN]);
        round_trip(&mut reactor_41, &mut reactor_63, &payload)
            .map_err(|e| err(format!("message {i} (nonce {}): {e}", hex(&nonce))))?;
        nonces.push(nonce);
    }
    println!("all {N} round-trips delivered BYTE-EQUAL (no loss across the wraps ⇒ TX tail stayed monotonic)");

    // Steady-state health.
    let drop41 = reactor_41.sink().inner().dropped();
    let drop63 = reactor_63.sink().inner().dropped();
    let mal41 = reactor_41.malformed();
    let mal63 = reactor_63.malformed();
    let rxmpc63 = datapath::read_counter(&bar63, datapath::RXMPC0);
    println!("41 TX dropped={drop41}  63 TX(ACK) dropped={drop63}  41 malformed={mal41}  63 malformed={mal63}  63 RXMPC={rxmpc63}");
    if drop41 != 0 || drop63 != 0 {
        return Err(err("TX drops occurred — the ring stalled/overran (produce cursor or reclaim leak)"));
    }
    if rxmpc63 != 0 {
        return Err(err("63 RXMPC != 0 — RX overrun (ring did not stay full / starved)"));
    }
    if mal41 != 0 || mal63 != 0 {
        return Err(err("malformed frames on the wire"));
    }
    println!("  => rings stayed FULL (63 RXMPC=0), NO overrun/starvation, TX ACK-bound reclaimed (dropped=0, no leak).");

    // List ALL fresh nonces (reporting hygiene — un-fakeable per-message).
    println!("\nALL {N} FRESH NONCES (41→63, byte-equal on 63):");
    for row in nonces.chunks(8) {
        println!("  {}", row.iter().map(|n| hex(n)).collect::<Vec<_>>().join(" "));
    }

    // ================= RETRANSMIT + GIVE-UP on genuine silence =================
    // Disable 63's receiver so it drops at the MAC (no ring fill, no PAUSE) — genuine silence to 41.
    println!("\n--- RETRANSMIT + GIVE-UP (63 receiver OFF → genuine silence) ---");
    datapath::set_rxen(&bar63, false);
    let _ = datapath::read_counter(&bar41, datapath::GPTC); // clear baseline
    let nonce = read_nonce()?;
    let mut payload = Vec::with_capacity(PAYLOAD_LEN);
    payload.extend_from_slice(&nonce);
    payload.extend_from_slice(&[0x5A; PAYLOAD_LEN - NONCE_LEN]);
    reactor_41.originate(&payload); // first emit into the void
    // Pump 41 past the ~2 s give-up bound, letting the ~10 ms retransmit fire on silence.
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(2300) {
        reactor_41.step();
        reactor_41.park();
    }
    let emits_through_giveup = datapath::read_counter(&bar41, datapath::GPTC);
    // A further window: after give-up the session is reclaimed → NO more emits.
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(600) {
        reactor_41.step();
        reactor_41.park();
    }
    let emits_after_giveup = datapath::read_counter(&bar41, datapath::GPTC);
    let drop41_rtx = reactor_41.sink().inner().dropped();
    println!("41 emits through give-up window = {emits_through_giveup} (1 original + retransmits on silence)");
    println!("41 emits AFTER give-up window  = {emits_after_giveup} (0 ⇒ gave up + reclaimed, silent)");
    println!("41 TX dropped over the retransmit burst = {drop41_rtx} (cursor held monotonic under ~{} posts)", emits_through_giveup);
    if emits_through_giveup < 2 {
        return Err(err("retransmit did not fire on silence"));
    }
    if emits_after_giveup != 0 {
        return Err(err("give-up did not reclaim — 41 kept emitting past the give-up bound"));
    }
    if drop41_rtx != 0 {
        return Err(err("TX drops during the retransmit burst (cursor/reclaim issue)"));
    }
    println!("  => retransmit FIRES on silence; give-up RECLAIMS (emits stop); no TX drop across the burst.");

    datapath::quiesce(&bar41);
    datapath::quiesce(&bar63);
    println!("\nquiesced both devices");
    Ok(())
}
