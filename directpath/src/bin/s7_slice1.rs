#![deny(unsafe_code)]

//! **Sprint 7 Slice 1 — the reactor-driven live loop, single session round-trip** over the live DAC
//! `41:00.0 ↔ 63:00.0`, pure-direct, no IOMMU. 41 originates a session (unique-nonce payload); 63's
//! reactor RX-demuxes it, the session reacts inline (delivers the payload up + returns a bare ACK); 41's
//! reactor receives + accepts the ACK. The frozen reactor algorithm + transport + guard/copy-boundary
//! drive it; only the backing is new: the bounded-poll `Host` ([`pollhost`]) and the Ethernet encap/decap
//! ([`ethframe`]) behind the reactor's seams.
//!
//! Safety spine (unchanged): every state-change takes an `Allowed(Bdf)` token minted only by the gate for
//! the bench cards; production `61:00.0` is never touched; one Gate-2-validated 2 MiB hugepage backs both
//! cards as offsets. Reversible: both cards rebound to ixgbe at the end. Run: `sudo ./s7_slice1`.

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
const PAYLOAD_LEN: usize = 24; // 8-byte nonce + 16-byte marker; < 60 on the wire → exercises pad-strip

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
    let config = read_config(&bdf)?;
    match gate(bdf, allowlist, &config) {
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

/// Bring up one card's TX + RX over its layout and build the wired live reactor. `peer_mac` is the dst for
/// this card's TX (the other card); `own_mac` its src.
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
    for slot in 0..datapath::DEPTH - 1 {
        rx_ring.refill(slot);
    }
    let sink = EthSink::new(TxPath::new(tx_ring), peer_mac, own_mac);
    let source = EthSource::new(RxDrainAsSource::new(RxPath::new(rx_ring)));
    println!("  {name}: TX+RX armed, reactor wired (bounded-poll Host + Ethernet encap)");
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

fn main() {
    match run() {
        Ok(()) => println!("\n=== SLICE 1: SINGLE-SESSION ROUND-TRIP PROVEN ==="),
        Err(e) => {
            eprintln!("\n=== SLICE 1 FAILED: {e} ===");
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
    println!("unbound both from {drv41}/{drv63}");

    let result = round_trip(&tok41, &tok63);

    let r41 = pci::rebind(&tok41, &drv41);
    let r63 = pci::rebind(&tok63, &drv63);
    println!("rebind {}: {:?}; rebind {}: {:?}", bdf41.sysfs(), r41.is_ok(), bdf63.sysfs(), r63.is_ok());

    result?;
    r41?;
    r63?;
    Ok(())
}

fn round_trip(tok41: &Allowed, tok63: &Allowed) -> io::Result<()> {
    pci::enable_bus_master(tok41)?;
    pci::enable_bus_master(tok63)?;
    let bar41 = pci::map_resource0(tok41)?;
    let bar63 = pci::map_resource0(tok63)?;
    println!("BAR0 mapped both");

    println!("bringing up links...");
    bring_up_link(&bar41, "41")?;
    bring_up_link(&bar63, "63")?;

    let mac41 = datapath::read_mac(&bar41);
    let mac63 = datapath::read_mac(&bar63);
    println!("MAC 41 = {}  MAC 63 = {}", hex(&mac41), hex(&mac63));

    // ONE hugepage, two non-overlapping Layouts (41 at offset 0, 63 at offset used_bytes()).
    let hp: Hugepage = mem::alloc()?;
    let split = Layout::used_bytes(); // 0x41000
    if 2 * split > hp.size() {
        return Err(err("two carves exceed the 2 MiB hugepage"));
    }
    validate_physaddr(hp.va(), hp.phys(), hp.size() as u64)
        .map_err(|e| err(format!("Gate 2 REFUSED the hugepage: {e:?}")))?;
    let layout_41 = Layout::new(hp.va(), hp.phys());
    let layout_63 = Layout::new(hp.va() + split as u64, hp.phys() + split as u64);
    datapath::preinit_desc_rings(&layout_41);
    datapath::preinit_desc_rings(&layout_63);
    println!("hugepage Gate-2 validated; 41 carve [0,0x{split:x}), 63 carve [0x{split:x},0x{:x})", 2 * split);

    // Wire both reactors: 41 sends to 63's MAC, 63 sends to 41's MAC.
    println!("wiring reactors...");
    let mut reactor_41 = build_reactor(&bar41, &layout_41, mac41, mac63, "41")?;
    let mut reactor_63 = build_reactor(&bar63, &layout_63, mac63, mac41, "63")?;

    // Baseline the (read-to-clear) counters before the round-trip.
    let _ = datapath::read_counter(&bar41, datapath::GPTC);
    let _ = datapath::read_counter(&bar63, datapath::GPTC);
    let _ = datapath::read_counter(&bar41, datapath::RXDGPC);
    let _ = datapath::read_counter(&bar63, datapath::RXDGPC);

    // The unique message: 8-byte nonce + a 16-byte marker.
    let nonce = read_nonce()?;
    let mut payload = Vec::with_capacity(PAYLOAD_LEN);
    payload.extend_from_slice(&nonce);
    payload.extend_from_slice(&[0x5A; PAYLOAD_LEN - NONCE_LEN]);

    println!("\n--- ROUND TRIP ---");
    let sid = reactor_41.originate(&payload);
    println!("41 originates session {} with nonce {} → 63", sid.0, hex(&nonce));

    // Phase 1: 63 RX-demux-react → deliver the payload up (through copy_out) + emit the bare ACK.
    let start = Instant::now();
    let delivered = loop {
        reactor_63.pump_once();
        if let Some(p) = reactor_63.take_delivery() {
            break p;
        }
        if start.elapsed() > Duration::from_secs(3) {
            return Err(err("63 did not deliver the session payload within 3s"));
        }
    };
    if delivered != payload {
        return Err(err(format!("payload mismatch on 63: got {} bytes", delivered.len())));
    }
    println!("63 delivered the payload (byte-equal through copy_out): nonce {}", hex(&delivered[..NONCE_LEN]));

    // Phase 2: 41 receives + accepts the bare ACK (fast — well before the ~10 ms retransmit deadline, so
    // the first step() that does work is the ACK, not a retransmit).
    let start = Instant::now();
    let mut acked = false;
    while start.elapsed() < Duration::from_millis(50) {
        if reactor_41.step() {
            acked = true;
            break;
        }
        reactor_41.park();
    }
    if !acked {
        return Err(err("41 did not receive the ACK within 50ms"));
    }
    if reactor_41.take_delivery().is_some() {
        return Err(err("41: the ACK must not deliver a payload up"));
    }
    if reactor_41.malformed() != 0 {
        return Err(err(format!("41: {} malformed frame(s) on the ACK path", reactor_41.malformed())));
    }
    println!("41 received + accepted the bare ACK for session {} (nothing delivered up, malformed=0)", sid.0);

    // Hardware corroboration (deltas since the baseline).
    let gptc_41 = datapath::read_counter(&bar41, datapath::GPTC);
    let gptc_63 = datapath::read_counter(&bar63, datapath::GPTC);
    let rxdgpc_63 = datapath::read_counter(&bar63, datapath::RXDGPC);
    let rxdgpc_41 = datapath::read_counter(&bar41, datapath::RXDGPC);
    println!("\ncounters (deltas): 41 GPTC={gptc_41} (data, NO retransmit) | 63 RXDGPC={rxdgpc_63} (data to host)");
    println!("                   63 GPTC={gptc_63} (ACK)          | 41 RXDGPC={rxdgpc_41} (ACK to host)");
    if gptc_41 != 1 {
        return Err(err(format!("41 GPTC={gptc_41}, expected 1 (a retransmit would mean the ACK was not accepted)")));
    }
    if rxdgpc_63 < 1 || rxdgpc_41 < 1 || gptc_63 < 1 {
        return Err(err("a corroborating counter did not increment (data-to-host / ACK / ACK-to-host)"));
    }
    println!("  => ROUND TRIP: 41 → data → 63 (demux+react+deliver, byte-equal) → ACK → 41 (accepted, no retransmit).");

    datapath::quiesce(&bar41);
    datapath::quiesce(&bar63);
    println!("quiesced both devices");
    Ok(())
}
