#![deny(unsafe_code)]

//! **Sprint 7 Slice 3 — MULTIPLEXED SESSIONS (the operational bar)** over the live DAC `41:00.0 ↔ 63:00.0`.
//! K concurrent sessions are interleaved on the one wire and demuxed by session ID (the table lookup, ZERO
//! protocol-type branching — `architecture.md §4`); each is stop-and-wait + ACK; the interleaving is
//! trivial (the transport's plain-iteration ready-set / `poll`, no scheduler). Lifecycle by
//! emission/silence: open-by-emission, no close signal, timeout give-up reclaim, **dormant ≠ dead**.
//!
//! PROVE: K sessions carry data `41 → 63` CONCURRENTLY (all in flight at once); demux correct (each unique
//! payload delivered to a session, byte-equal, no foreign / no loss); the round-trip completes for all K
//! (quiescent after ⇒ no leak); and an actively-failing (dead-peer) session is reclaimed via give-up.
//! (Dormant-survives + revive-on-next-frame is a FROZEN transport property, host-tested — the reactor's
//! driver API exposes `originate` (open), not `send`-on-existing, so it is not bench-observable here; noted.)
//!
//! Frozen: reactor algorithm + transport + guard/copy-boundary. Slice 3 adds only this runner (the K
//! concurrent sessions wire behind the existing seams — no backing change). Safety spine unchanged. Run:
//! `sudo ./s7_slice3`.

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
use std::collections::BTreeSet;
use std::io::{self, Read};
use std::time::{Duration, Instant};
use transport::SessionTable;

const BENCH_41: &str = "0000:41:00.0";
const BENCH_63: &str = "0000:63:00.0";
const POLL_INTERVAL: Duration = Duration::from_micros(50);
const NONCE_LEN: usize = 8;
const PAYLOAD_LEN: usize = 24;
const K: usize = 32; // concurrent sessions (all in flight at once; < depth-64, no TX wrap during the burst)

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
    for slot in 0..datapath::DEPTH {
        rx_ring.refill(slot); // arm ALL depth slots (steady-state ring, Slice-2 discipline)
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

fn payload_of(nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
    let mut p = Vec::with_capacity(PAYLOAD_LEN);
    p.extend_from_slice(nonce);
    p.extend_from_slice(&[0x5A; PAYLOAD_LEN - NONCE_LEN]);
    p
}

fn main() {
    match run() {
        Ok(()) => println!("\n=== SLICE 3: MULTIPLEXED SESSIONS PROVEN (operational) ==="),
        Err(e) => {
            eprintln!("\n=== SLICE 3 FAILED: {e} ===");
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

    let result = multiplex(&tok41, &tok63);

    let r41 = pci::rebind(&tok41, &drv41);
    let r63 = pci::rebind(&tok63, &drv63);
    println!("rebind {}: {:?}; rebind {}: {:?}", bdf41.sysfs(), r41.is_ok(), bdf63.sysfs(), r63.is_ok());
    result?;
    r41?;
    r63?;
    Ok(())
}

fn multiplex(tok41: &Allowed, tok63: &Allowed) -> io::Result<()> {
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

    // ================= K CONCURRENT SESSIONS =================
    println!("\n--- MULTIPLEX: {K} concurrent sessions, all in flight, interleaved on the one wire ---");
    let mut nonces: Vec<[u8; NONCE_LEN]> = Vec::with_capacity(K);
    let mut sent: BTreeSet<Vec<u8>> = BTreeSet::new();
    for _ in 0..K {
        let n = read_nonce()?;
        nonces.push(n);
        sent.insert(payload_of(&n));
    }
    let _ = datapath::read_counter(&bar41, datapath::GPTC); // baseline

    // Open all K sessions at once → K frames interleaved on the wire (each an independent session ID).
    let mut ids = Vec::with_capacity(K);
    for n in &nonces {
        ids.push(reactor_41.originate(&payload_of(n)).0);
    }
    println!("41 originated {K} sessions (ids {}..={}), all frames in flight", ids[0], ids[K - 1]);

    // 63 demuxes each frame to its session (byte-equal); 41 processes the returning ACKs. Interleave both.
    let mut delivered: BTreeSet<Vec<u8>> = BTreeSet::new();
    let mut total_deliveries = 0usize;
    let start = Instant::now();
    while delivered.len() < K {
        reactor_63.pump_once();
        if let Some(p) = reactor_63.take_delivery() {
            if !sent.contains(&p) {
                return Err(err("63 delivered a FOREIGN payload — demux mis-routed a frame"));
            }
            delivered.insert(p);
            total_deliveries += 1;
        }
        reactor_41.step(); // process ACKs (and any retransmit) as they arrive
        if start.elapsed() > Duration::from_secs(3) {
            return Err(err(format!("only {}/{K} sessions demuxed+delivered", delivered.len())));
        }
    }
    let dups = total_deliveries - K;
    println!("63 demuxed + delivered ALL {K} sessions byte-equal (unique set complete, no foreign); {dups} retransmit-duplicate(s)");

    // Settle: drain the remaining ACKs on both, then confirm QUIESCENT (no ongoing retransmit ⇒ every
    // session round-tripped + was released — no leak).
    let settle = Instant::now();
    while settle.elapsed() < Duration::from_millis(40) {
        reactor_63.pump_once();
        let _ = reactor_63.take_delivery();
        reactor_41.step();
    }
    let _ = datapath::read_counter(&bar41, datapath::GPTC); // clear
    let q1 = Instant::now();
    while q1.elapsed() < Duration::from_millis(40) {
        reactor_41.step();
        reactor_41.park();
    }
    let emits_when_quiescent = datapath::read_counter(&bar41, datapath::GPTC);
    println!("41 emits over a 40ms quiescent window = {emits_when_quiescent} (0 ⇒ all {K} sessions ACKed + released, no leak)");
    if emits_when_quiescent != 0 {
        return Err(err("41 still emitting when it should be quiescent — a session did not complete (leak)"));
    }
    if reactor_41.malformed() != 0 || reactor_63.malformed() != 0 {
        return Err(err("malformed frames during multiplex"));
    }
    println!("  => {K} CONCURRENT sessions multiplexed: demux correct (byte-equal, no foreign, no loss), interleaving trivial, all round-tripped (quiescent = no leak).");

    // ================= DORMANT != DEAD: dead-peer give-up reclaim =================
    println!("\n--- DORMANT != DEAD: an actively-failing (dead-peer) session is reclaimed via give-up ---");
    datapath::set_rxen(&bar63, false); // dead peer: 63 drops at the MAC
    let _ = datapath::read_counter(&bar41, datapath::GPTC);
    let n = read_nonce()?;
    reactor_41.originate(&payload_of(&n)); // a session whose peer never ACKs
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(2300) {
        reactor_41.step();
        reactor_41.park();
    }
    let emits_through = datapath::read_counter(&bar41, datapath::GPTC);
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(600) {
        reactor_41.step();
        reactor_41.park();
    }
    let emits_after = datapath::read_counter(&bar41, datapath::GPTC);
    println!("dead-peer session: {emits_through} emits through give-up (retransmits on silence), {emits_after} after (0 ⇒ reclaimed)");
    if emits_through < 2 || emits_after != 0 {
        return Err(err("dead-peer give-up did not reclaim as expected"));
    }
    println!("  => the actively-failing session gave up + reclaimed (no leak); a DORMANT session is instead never reaped");
    println!("     (dormant-survives + revive-on-next-frame is a FROZEN transport property, host-tested: the reactor exposes originate, not send-on-existing — noted, not bench-observable here).");

    // List all nonces.
    println!("\nALL {K} MULTIPLEXED SESSION NONCES (each demuxed byte-equal on 63):");
    for row in nonces.chunks(8) {
        println!("  {}", row.iter().map(|x| hex(x)).collect::<Vec<_>>().join(" "));
    }

    datapath::quiesce(&bar41);
    datapath::quiesce(&bar63);
    println!("\nquiesced both devices");
    Ok(())
}
