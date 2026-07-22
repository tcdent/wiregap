#![deny(unsafe_code)]

//! **Slice 4 — the milestone: proven data transmission across the DAC** (`41:00.0` → `63:00.0`), pure
//! direct, no IOMMU. TX-first, then RX, self-pinned from the device's own counters + a byte-equal payload
//! proof through the real `copy_out`/`RxDrain` seam.
//!
//! Safety that STAYS (the hard stop was dropped by operator ruling, but device scoping + physaddr
//! validation remain): every state-changing op takes an un-forgeable [`Allowed`] token minted only by the
//! [`gate`] for the bench cards (`8086:10FB` / subsys `8086:0003`); the production NIC (`61:00.0`, subsys
//! `7a12`) can never be touched. The one Gate-2-validated 2 MiB hugepage backs BOTH devices' rings +
//! buffers as offsets. Everything is reversible: the kernel `ixgbe` driver is rebound at the end.
//!
//! Run: `sudo ./slice4` (unbind / BAR mmap / DMA / pagemap need root).

use directpath::internals::datapath::{self, Layout};
use directpath::internals::gate::{Allowed, Bdf, Decision, gate};
use directpath::internals::gate2::validate_physaddr;
use directpath::internals::link::{LinkRestart, apply_restart, link_state, reset_and_init};
use directpath::internals::mem::{self, Hugepage};
use directpath::internals::pci::{self, Bar0};
use driver::{RxPath, RxRing, TxRing};
use std::io::{self, Read};
use std::time::{Duration, Instant};

const BENCH_41: &str = "0000:41:00.0";
const BENCH_63: &str = "0000:63:00.0";
const ETHERTYPE: [u8; 2] = [0x88, 0xB5]; // IEEE 802 local-experimental ethertype
const FRAME_LEN: usize = 60; // 14 header + 46 payload (min Ethernet payload; device pads/appends CRC)
const PAYLOAD_OFF: usize = 14;
const NONCE_LEN: usize = 8;

fn err(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

/// Read the first `0x40` config-space bytes for the gate (vendor/device/subsystem identity).
fn read_config(bdf: &Bdf) -> io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(format!("/sys/bus/pci/devices/{}/config", bdf.sysfs()))?;
    let mut buf = vec![0u8; 0x40];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

/// Gate a bench BDF: read its config, run the pure gate against the frozen 2-card allowlist. The only
/// path to an [`Allowed`] token.
fn gate_bench(bdf: Bdf, allowlist: &[Bdf]) -> io::Result<Allowed> {
    let config = read_config(&bdf)?;
    match gate(bdf, allowlist, &config) {
        Decision::Allow(tok) => Ok(tok),
        Decision::Refuse(r) => Err(err(format!("gate REFUSED {}: {r:?}", bdf.sysfs()))),
    }
}

/// Reset + bring the link up (defaults-first escalation), waiting for 10G up. Returns the raw LINKS.
fn bring_up_link(bar: &Bar0, name: &str) -> io::Result<u32> {
    let reset = reset_and_init(bar);
    if !reset.ok() {
        return Err(err(format!(
            "{name}: reset/init incomplete (reset_cleared={} eec_ard={} dmaidone={})",
            reset.reset_cleared, reset.eec_ard, reset.rdrxctl_dmaidone
        )));
    }
    for form in [LinkRestart::None, LinkRestart::RestartAn, LinkRestart::ResetPipeline] {
        apply_restart(bar, form);
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(4) {
            let ls = link_state(bar);
            if ls.up && ls.speed_10g {
                println!("  {name}: link UP 10G ({form:?}), LINKS=0x{:08x}", ls.links_raw);
                return Ok(ls.links_raw);
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

/// Build the L2 frame: dst=63's MAC, src=41's MAC, our ethertype, the unique nonce at payload offset 0,
/// zero-padded to the min length.
fn build_frame(dst: [u8; 6], src: [u8; 6], nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
    let mut f = Vec::with_capacity(FRAME_LEN);
    f.extend_from_slice(&dst);
    f.extend_from_slice(&src);
    f.extend_from_slice(&ETHERTYPE);
    f.extend_from_slice(nonce);
    f.resize(FRAME_LEN, 0x00);
    f
}

/// Post one frame on the TX ring at `slot`, then poll the (bounded) TX descriptor DD writeback.
fn transmit(tx: &mut driver::VolatileTxRing, slot: usize, frame: &[u8]) -> io::Result<()> {
    tx.post(slot, frame);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        if tx.descriptor_done(slot) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Err(err("TX descriptor DD writeback timed out"))
}

fn main() {
    match run() {
        Ok(()) => println!("\n=== SLICE 4: TRANSMISSION PROVEN ==="),
        Err(e) => {
            eprintln!("\n=== SLICE 4 FAILED: {e} ===");
            std::process::exit(1);
        }
    }
}

fn run() -> io::Result<()> {
    let bdf41 = Bdf::parse(BENCH_41).ok_or_else(|| err("bad BDF 41"))?;
    let bdf63 = Bdf::parse(BENCH_63).ok_or_else(|| err("bad BDF 63"))?;
    let allowlist = [bdf41, bdf63]; // FROZEN: only the two bench cards, ever.

    // (1) Gate BOTH — the only path to a state-changing token. Refusal ⇒ abort untouched.
    let tok41 = gate_bench(bdf41, &allowlist)?;
    let tok63 = gate_bench(bdf63, &allowlist)?;
    println!("gate: ALLOW {} and {} (bench 8086:10FB/0003)", bdf41.sysfs(), bdf63.sysfs());

    // Record the current drivers for rebinding.
    let drv41 = pci::current_driver(&tok41).unwrap_or_else(|| "ixgbe".into());
    let drv63 = pci::current_driver(&tok63).unwrap_or_else(|| "ixgbe".into());

    // (2) Unbind both from the kernel driver.
    pci::unbind(&tok41)?;
    pci::unbind(&tok63)?;
    println!("unbound both from {drv41}/{drv63}");

    // Core datapath work — bounded so its BAR mappings / rings / hugepage drop before we rebind.
    let result = datapath_run(&tok41, &tok63);

    // (3) Always rebind, leaving the system reviewable.
    let r41 = pci::rebind(&tok41, &drv41);
    let r63 = pci::rebind(&tok63, &drv63);
    println!("rebind {}: {:?}; rebind {}: {:?}", bdf41.sysfs(), r41.is_ok(), bdf63.sysfs(), r63.is_ok());

    result?;
    r41?;
    r63?;
    Ok(())
}

fn datapath_run(tok41: &Allowed, tok63: &Allowed) -> io::Result<()> {
    // Bus-master + BAR0 mmap for both.
    let cmd41 = pci::enable_bus_master(tok41)?;
    let cmd63 = pci::enable_bus_master(tok63)?;
    println!("bus-master enabled: 41 COMMAND=0x{cmd41:04x}, 63 COMMAND=0x{cmd63:04x}");
    let bar41 = pci::map_resource0(tok41)?;
    let bar63 = pci::map_resource0(tok63)?;
    println!("BAR0 mapped: 41 len=0x{:x}, 63 len=0x{:x}", bar41.len(), bar63.len());

    // Reset + link up both.
    println!("bringing up links...");
    bring_up_link(&bar41, "41")?;
    bring_up_link(&bar63, "63")?;

    // MACs from the factory receive-address filter.
    if !datapath::mac_valid(&bar41) || !datapath::mac_valid(&bar63) {
        return Err(err("a bench MAC (RAH0.AV) is not valid after reset"));
    }
    let mac41 = datapath::read_mac(&bar41);
    let mac63 = datapath::read_mac(&bar63);
    println!("MAC 41 = {}", hex_mac(&mac41));
    println!("MAC 63 = {}", hex_mac(&mac63));

    // ONE 2 MiB hugepage backs both devices' rings + buffers. Gate-2 validate its base.
    let hp: Hugepage = mem::alloc()?;
    if Layout::used_bytes() > hp.size() {
        return Err(err("carve exceeds the 2 MiB hugepage"));
    }
    validate_physaddr(hp.va(), hp.phys(), hp.size() as u64)
        .map_err(|e| err(format!("Gate 2 REFUSED the hugepage physaddr: {e:?}")))?;
    let layout = Layout::new(hp.va(), hp.phys());
    println!(
        "hugepage: va=0x{:x} phys=0x{:x} size=0x{:x} — Gate-2 validated; carve uses 0x{:x} (<= 2 MiB)",
        hp.va(),
        hp.phys(),
        hp.size(),
        Layout::used_bytes()
    );
    datapath::preinit_desc_rings(&layout); // 0xFF: no rogue DMA on premature activation

    // ================= TX-FIRST: 41 transmits, 63 RX UNARMED =================
    // Confirm the TX mechanism + wire arrival at 63 with NO host-write on 63.
    println!("\n--- TX-FIRST (63 RX UNARMED, RXEN clear) ---");
    if !datapath::init_tx(&bar41, &layout) {
        return Err(err("41: TXDCTL.ENABLE did not read back"));
    }
    let mut tx = datapath::build_tx_ring(&bar41, &layout);

    // Snapshot (read-to-clear) the counters right before transmitting.
    let gptc_b = datapath::read_counter(&bar41, datapath::GPTC);
    let tpr_b = datapath::read_counter(&bar63, datapath::TPR);
    let torl_b = datapath::read_counter(&bar63, datapath::TORL);
    let torh_b = datapath::read_counter(&bar63, datapath::TORH);
    let rxdgpc_b = datapath::read_counter(&bar63, datapath::RXDGPC);
    let gprc_b = datapath::read_counter(&bar63, datapath::GPRC);
    let rxmpc_b = datapath::read_counter(&bar63, datapath::RXMPC0);
    let _ = (gptc_b, tpr_b, torl_b, torh_b, rxdgpc_b, gprc_b, rxmpc_b); // baselines cleared

    let nonce1 = read_nonce()?;
    let frame1 = build_frame(mac63, mac41, &nonce1);
    println!("41 TX nonce1={} → 63", hex(&nonce1));
    if let Err(e) = transmit(&mut tx, 0, &frame1) {
        datapath::dump_tx_state(&bar41, &layout);
        return Err(e);
    }

    let gptc = datapath::read_counter(&bar41, datapath::GPTC);
    let tpr = datapath::read_counter(&bar63, datapath::TPR);
    let torl = datapath::read_counter(&bar63, datapath::TORL);
    let torh = datapath::read_counter(&bar63, datapath::TORH);
    let rxdgpc = datapath::read_counter(&bar63, datapath::RXDGPC);
    let gprc = datapath::read_counter(&bar63, datapath::GPRC);
    let rxmpc = datapath::read_counter(&bar63, datapath::RXMPC0);
    println!("  41 GPTC (good pkts TX)      = {gptc}");
    println!("  63 TPR  (wire pkts RX)      = {tpr}");
    println!("  63 TORL/TORH (octets RX)    = {torl} / {torh}");
    println!("  63 RXDGPC (posted to host)  = {rxdgpc}   <-- MUST be 0 (no host-write, RX unarmed)");
    println!("  63 GPRC (L2-filtered RX)    = {gprc}");
    println!("  63 RXMPC0 (missed/dropped)  = {rxmpc}");
    if gptc < 1 {
        return Err(err("TX-first: 41 GPTC did not increment (no good packet transmitted)"));
    }
    if rxdgpc != 0 {
        return Err(err("TX-first: 63 RXDGPC != 0 — a host-write occurred with RX unarmed (unexpected)"));
    }
    if tpr < 1 {
        return Err(err("TX-first: 63 TPR did not increment — no wire arrival detected"));
    }
    println!(
        "  => TX mechanism validated: 41 transmitted, frame arrived at 63's wire (TPR), NO host write (RXDGPC=0)."
    );
    println!("     Empirical: TPR ticked with RXEN CLEAR = {}", tpr >= 1);

    // ================= RX: arm 63, receive, prove byte-equal =================
    println!("\n--- RX (arm 63, receive through the copy_out/RxDrain seam) ---");
    datapath::init_rx(&bar63, &layout);
    datapath::clear_rx_buffers(&layout);
    if !datapath::rx_buffers_all_zero(&layout) {
        return Err(err("RX buffers not proven-cleared before arming"));
    }
    println!("  63 RX buffers pre-cleared + proven all-zero");
    let mut rx_ring = datapath::build_rx_ring(&bar63, &layout);
    // Enable the queue FIRST (RXDCTL+RXEN), THEN arm — RDT writes made while the queue is disabled can be
    // lost; the canonical sequence sets the tail after enable. No frame can arrive in between (41 has not
    // transmitted yet, and the ring was 0xFF-preinit against premature DMA).
    if !datapath::start_rx(&bar63, &layout) {
        return Err(err("63: RXDCTL.ENABLE did not read back"));
    }
    // Arm DEPTH-1 descriptors (hold one back): the last refill leaves RDT=DEPTH-1, so RDT never wraps to
    // meet RDH (which would read as an empty ring). The device owns [0, DEPTH-1).
    for slot in 0..datapath::DEPTH - 1 {
        rx_ring.refill(slot); // write validated physical pkt_addr + hand slot to device
    }
    // RxPath::poll_one is the guarded copy_out core that `reactor::RxDrain::next_frame` delegates to —
    // driving it exercises the exact §3.1 copy boundary + the adversarial guards.
    let mut rx = RxPath::new(rx_ring);
    println!("  63 RX armed (RXDCTL+RXEN, {} descriptors)", datapath::DEPTH);

    let rxdgpc_pre = datapath::read_counter(&bar63, datapath::RXDGPC);
    let gprc_pre = datapath::read_counter(&bar63, datapath::GPRC);
    let _ = (rxdgpc_pre, gprc_pre);

    let nonce2 = read_nonce()?;
    let frame2 = build_frame(mac63, mac41, &nonce2);
    println!("  41 TX nonce2={} → 63", hex(&nonce2));
    transmit(&mut tx, 1, &frame2)?;

    // Receive through the real seam: RxPath::next_frame → guard + copy_out (the §3.1 copy boundary).
    let start = Instant::now();
    let received = loop {
        if let Some(f) = rx.poll_one() {
            break f;
        }
        if start.elapsed() > Duration::from_secs(2) {
            datapath::dump_rx_state(&bar63, &layout);
            return Err(err("RX: no frame received within 2s"));
        }
        std::thread::sleep(Duration::from_millis(1));
    };

    let rxdgpc_post = datapath::read_counter(&bar63, datapath::RXDGPC);
    let gprc_post = datapath::read_counter(&bar63, datapath::GPRC);
    println!("  received {} bytes through copy_out/RxDrain", received.len());
    println!("  63 RXDGPC (posted to host)  = {rxdgpc_post}   <-- now >=1 (the RX host-write happened)");
    println!("  63 GPRC (good RX)           = {gprc_post}");

    // ---- the un-fakeable byte-equal proof (observed -> expected) ----
    let expect_payload = &frame2[PAYLOAD_OFF..];
    if received.len() < PAYLOAD_OFF + NONCE_LEN {
        return Err(err(format!("received frame too short: {} bytes", received.len())));
    }
    let got_dst = &received[0..6];
    let got_src = &received[6..12];
    let got_nonce = &received[PAYLOAD_OFF..PAYLOAD_OFF + NONCE_LEN];
    println!("  dst  observed={} expected={}", hex(got_dst), hex(&mac63));
    println!("  src  observed={} expected={}", hex(got_src), hex(&mac41));
    println!("  nonce observed={} expected={}", hex(got_nonce), hex(&nonce2));

    if got_dst != mac63 {
        return Err(err("dst MAC mismatch"));
    }
    if got_src != mac41 {
        return Err(err("src MAC mismatch"));
    }
    if got_nonce != nonce2 {
        return Err(err("NONCE MISMATCH — received payload is not what 41 sent"));
    }
    // Full payload byte-equality (nonce + the zero pad), bounded by what we received.
    let n = expect_payload.len().min(received.len() - PAYLOAD_OFF);
    if received[PAYLOAD_OFF..PAYLOAD_OFF + n] != expect_payload[..n] {
        return Err(err("payload byte-equality failed"));
    }
    if rxdgpc_post < 1 {
        return Err(err("RX: RXDGPC did not increment — frame not confirmed posted to host"));
    }
    println!(
        "  => BYTE-EQUAL: the unique nonce 41 sent arrived at 63 (out of pre-cleared buffers), through the\n     real copy_out/RxDrain seam, DD set by the device, RXDGPC confirms the DMA-to-host."
    );

    // Quiesce DMA before releasing (drop BARs → rebind). No-IOMMU: stop the device touching host RAM.
    datapath::quiesce(&bar41);
    datapath::quiesce(&bar63);
    println!("\nquiesced both devices (RXEN/RXDCTL/TXDCTL/TE cleared)");
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join("")
}
fn hex_mac(b: &[u8; 6]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(":")
}
