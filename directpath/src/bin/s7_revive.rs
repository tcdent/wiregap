#![deny(unsafe_code)]

//! **Sprint 7 Slice 4a — the DORMANT-REVIVE live demo**, closing the Slice-3 observability gap. The
//! reactor's driver API is `originate`-only, so revive-on-next-frame (a dormant session sending again) is
//! not reactor-drivable; this harness drives the FROZEN transport DIRECTLY over the live rings (the
//! transport's public `open`/`send`/`on_inbound`) to demonstrate it on silicon:
//!
//! 1. 41 opens session S (frame 1) → 63 opens receiver S, delivers, ACKs → 41 gets the ACK ⇒ S is
//!    **DORMANT** (nothing in flight, `next_deadline` == None), still live.
//! 2. 41 `send`s a SECOND frame on the SAME S (**revive**) → 63 routes it to the EXISTING receiver S (the
//!    table does NOT grow), delivers byte-equal → 41 gets the ACK ⇒ S dormant again.
//!
//! Dormant ≠ dead, live: the dormant session accepted a new send and the peer revived the same session.
//! The transport + guard/copy-boundary are byte-frozen (this is a harness). Run: `sudo ./s7_revive`.

use directpath::datapath::{self, Layout};
use directpath::ethframe::{EthSink, decap};
use directpath::gate::{Allowed, Bdf, Decision, gate};
use directpath::gate2::validate_physaddr;
use directpath::link::{LinkRestart, apply_restart, link_state, reset_and_init};
use directpath::mem::{self, Hugepage};
use directpath::pci::{self, Bar0};
use driver::{RxPath, RxRing, TxPath, VolatileRxRing, VolatileTxRing};
use std::io::{self, Read};
use std::time::{Duration, Instant as StdInstant};
use transport::{Instant, SessionId, SessionTable};

const BENCH_41: &str = "0000:41:00.0";
const BENCH_63: &str = "0000:63:00.0";
const NONCE_LEN: usize = 8;
const PAYLOAD_LEN: usize = 24;

type LiveSink = EthSink<TxPath<VolatileTxRing>>;

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
        let start = StdInstant::now();
        while start.elapsed() < Duration::from_secs(4) {
            let ls = link_state(bar);
            if ls.up && ls.speed_10g {
                println!("  {name}: link UP 10G");
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Err(err(format!("{name}: no 10G link")))
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
fn payload_of(n: &[u8; NONCE_LEN]) -> Vec<u8> {
    let mut p = Vec::with_capacity(PAYLOAD_LEN);
    p.extend_from_slice(n);
    p.extend_from_slice(&[0x5A; PAYLOAD_LEN - NONCE_LEN]);
    p
}

/// A card driven at the TRANSPORT level (not via the reactor): a session table, an Ethernet sink, and the
/// raw RX path — so we can call `open`/`send`/`on_inbound` directly.
struct Endpoint {
    table: SessionTable,
    sink: LiveSink,
    rx: RxPath<VolatileRxRing>,
}

fn build_endpoint(bar: &Bar0, layout: &Layout, own_mac: [u8; 6], peer_mac: [u8; 6], name: &str) -> io::Result<Endpoint> {
    if !datapath::init_tx(bar, layout) {
        return Err(err(format!("{name}: TXDCTL enable failed")));
    }
    datapath::init_rx(bar, layout);
    datapath::clear_rx_buffers(layout);
    let tx = datapath::build_tx_ring(bar, layout);
    let mut rx_ring = datapath::build_rx_ring(bar, layout);
    if !datapath::start_rx(bar, layout) {
        return Err(err(format!("{name}: RXDCTL enable failed")));
    }
    for slot in 0..datapath::DEPTH {
        rx_ring.refill(slot);
    }
    println!("  {name}: TX+RX armed (transport-direct endpoint)");
    Ok(Endpoint { table: SessionTable::new(), sink: EthSink::new(TxPath::new(tx), peer_mac, own_mac), rx: RxPath::new(rx_ring) })
}

/// Poll the raw RX path for one frame within `dur`, returning the decap'd L1 frame (owned).
fn poll_l1(rx: &mut RxPath<VolatileRxRing>, dur: Duration) -> Option<Vec<u8>> {
    let start = StdInstant::now();
    while start.elapsed() < dur {
        if let Some(eth) = rx.poll_one() {
            return Some(decap(&eth).to_vec());
        }
        std::thread::sleep(Duration::from_micros(50));
    }
    None
}

fn main() {
    match run() {
        Ok(()) => println!("\n=== SLICE 4a DORMANT-REVIVE: DEMONSTRATED LIVE ==="),
        Err(e) => {
            eprintln!("\n=== DORMANT-REVIVE FAILED: {e} ===");
            std::process::exit(1);
        }
    }
}

fn run() -> io::Result<()> {
    let bdf41 = Bdf::parse(BENCH_41).ok_or_else(|| err("bad BDF"))?;
    let bdf63 = Bdf::parse(BENCH_63).ok_or_else(|| err("bad BDF"))?;
    let allow = [bdf41, bdf63];
    let tok41 = gate_bench(bdf41, &allow)?;
    let tok63 = gate_bench(bdf63, &allow)?;
    println!("gate: ALLOW {} and {}", bdf41.sysfs(), bdf63.sysfs());
    let drv41 = pci::current_driver(&tok41).unwrap_or_else(|| "ixgbe".into());
    let drv63 = pci::current_driver(&tok63).unwrap_or_else(|| "ixgbe".into());
    pci::unbind(&tok41)?;
    pci::unbind(&tok63)?;
    let result = demo(&tok41, &tok63);
    let r41 = pci::rebind(&tok41, &drv41);
    let r63 = pci::rebind(&tok63, &drv63);
    println!("rebind {}: {:?}; rebind {}: {:?}", bdf41.sysfs(), r41.is_ok(), bdf63.sysfs(), r63.is_ok());
    result?;
    r41?;
    r63?;
    Ok(())
}

fn demo(tok41: &Allowed, tok63: &Allowed) -> io::Result<()> {
    pci::enable_bus_master(tok41)?;
    pci::enable_bus_master(tok63)?;
    let bar41 = pci::map_resource0(tok41)?;
    let bar63 = pci::map_resource0(tok63)?;
    bring_up_link(&bar41, "41")?;
    bring_up_link(&bar63, "63")?;
    let mac41 = datapath::read_mac(&bar41);
    let mac63 = datapath::read_mac(&bar63);

    let hp: Hugepage = mem::alloc()?;
    let split = Layout::used_bytes();
    validate_physaddr(hp.va(), hp.phys(), hp.size() as u64).map_err(|e| err(format!("Gate 2 REFUSED: {e:?}")))?;
    let layout_41 = Layout::new(hp.va(), hp.phys());
    let layout_63 = Layout::new(hp.va() + split as u64, hp.phys() + split as u64);
    datapath::preinit_desc_rings(&layout_41);
    datapath::preinit_desc_rings(&layout_63);
    let mut e41 = build_endpoint(&bar41, &layout_41, mac41, mac63, "41")?;
    let mut e63 = build_endpoint(&bar63, &layout_63, mac63, mac41, "63")?;

    let clock = StdInstant::now();
    let now = || Instant(clock.elapsed().as_nanos() as u64);
    let tmo = Duration::from_secs(2);

    // ---- 1. Open session S, deliver, ACK → S DORMANT ----
    let nonce1 = read_nonce()?;
    let p1 = payload_of(&nonce1);
    println!("\n[1] 41 opens session with nonce {} (frame 1)", hex(&nonce1));
    let id: SessionId = e41.table.open(&p1, now(), &mut e41.sink);
    // 63 receives frame 1
    let f1 = poll_l1(&mut e63.rx, tmo).ok_or_else(|| err("63 did not receive frame 1"))?;
    let d1 = e63.table.on_inbound(&f1, &mut e63.sink).map_err(|e| err(format!("frame1 decode: {e:?}")))?
        .ok_or_else(|| err("frame1 delivered nothing"))?;
    if d1.payload != p1 {
        return Err(err("frame 1 payload mismatch"));
    }
    let len_after_open = e63.table.len();
    println!("    63 opened+delivered session {} byte-equal; 63 table.len()={len_after_open}", d1.session.0);
    // 41 receives the ACK → dormant
    let a1 = poll_l1(&mut e41.rx, tmo).ok_or_else(|| err("41 did not receive ACK 1"))?;
    if e41.table.on_inbound(&a1, &mut e41.sink).map_err(|e| err(format!("ack1 decode: {e:?}")))?.is_some() {
        return Err(err("ACK 1 should deliver nothing up"));
    }
    if e41.table.next_deadline().is_some() {
        return Err(err("session should be DORMANT after the ACK (no in-flight deadline)"));
    }
    if !e41.table.contains(id) {
        return Err(err("dormant session must still be live (dormant != dead)"));
    }
    println!("    41 got the ACK → session {} is DORMANT (next_deadline=None) but still LIVE", id.0);

    // ---- 2. REVIVE: send a SECOND frame on the SAME dormant session ----
    let nonce2 = read_nonce()?;
    let p2 = payload_of(&nonce2);
    println!("[2] 41 sends AGAIN on the SAME session {} (revive) with nonce {} (frame 2)", id.0, hex(&nonce2));
    e41.table.send(id, &p2, now(), &mut e41.sink).map_err(|e| err(format!("send on dormant session failed: {e:?} — revive broken")))?;
    // 63 receives frame 2 → routes to the EXISTING session (no growth), delivers byte-equal
    let f2 = poll_l1(&mut e63.rx, tmo).ok_or_else(|| err("63 did not receive frame 2"))?;
    let d2 = e63.table.on_inbound(&f2, &mut e63.sink).map_err(|e| err(format!("frame2 decode: {e:?}")))?
        .ok_or_else(|| err("frame2 delivered nothing"))?;
    if d2.payload != p2 {
        return Err(err("frame 2 payload mismatch"));
    }
    if d2.session != id {
        return Err(err("frame 2 routed to a DIFFERENT session — not a revive of the same one"));
    }
    let len_after_revive = e63.table.len();
    if len_after_revive != len_after_open {
        return Err(err(format!("63 table grew ({len_after_open}→{len_after_revive}) — revive opened a NEW session instead of routing to the existing one")));
    }
    println!("    63 routed frame 2 to the SAME session {} (table.len() still {len_after_revive}), delivered nonce {} byte-equal", d2.session.0, hex(&d2.payload[..NONCE_LEN]));
    // 41 gets ACK 2 → dormant again
    let a2 = poll_l1(&mut e41.rx, tmo).ok_or_else(|| err("41 did not receive ACK 2"))?;
    e41.table.on_inbound(&a2, &mut e41.sink).map_err(|e| err(format!("ack2 decode: {e:?}")))?;
    println!("    41 got ACK 2 → session {} dormant again", id.0);

    println!("\n  => DORMANT != DEAD, LIVE: the dormant session accepted a new send (revive), and 63 routed it to the SAME session (no table growth), byte-equal. Observability gap CLOSED.");

    datapath::quiesce(&bar41);
    datapath::quiesce(&bar63);
    println!("quiesced both devices");
    Ok(())
}
