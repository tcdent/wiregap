#![allow(unsafe_code)]

//! 82599 **datapath bring-up** (datasheet §4.6.7 receive / §4.6.8 transmit) — the register programming
//! that arms one RX and one TX queue over a **single, Gate-2-validated 2 MiB hugepage**, and constructs
//! the driver's confined volatile rings ([`driver::VolatileRxRing`] / [`VolatileTxRing`]) over that
//! memory. The init sequence follows the canonical minimal userspace `ixgbe` (`reference/ixy`
//! `init_rx`/`init_tx`/`start_*_queue`); register offsets/bits are from `ixgbe_type.h`.
//!
//! **Single-hugepage layout** (the reviewer's contiguity requirement: `depth*buffer_len <= 2 MiB`): both
//! devices' descriptor rings and packet buffers are OFFSETS within one hugepage, so validating the
//! hugepage base (Gate 2) covers every physaddr the devices are pointed at. Physaddrs are `base_phys +
//! offset`; the CPU uses `base_va + offset`.
//!
//! Confined `unsafe`: the ring construction (raw pointers into mapped DMA + the MMIO tail) and the raw
//! hugepage-memory helpers (0xFF ring pre-init, RX-buffer pre-clear/read-back). All register access goes
//! through [`Bar0`]'s checked `read32`/`write32`; MMIO is the device's own space, not host RAM.

use crate::pci::Bar0;
use driver::{AdvRxDescWb, AdvTxDesc, VolatileRxRing, VolatileTxRing};
use std::time::{Duration, Instant};

// ---- ring geometry -----------------------------------------------------------------------------

/// Descriptors per ring (power of 2, per the 82599). 64 * 16 B = 1 KiB per ring.
pub const DEPTH: usize = 64;
/// Per-buffer size (the 82599 handles 1 KiB increments; 2 KiB covers the 1518-byte MTU).
pub const BUFFER_LEN: usize = 2048;

// Offsets within the one hugepage. Descriptor rings are 128-byte-aligned (RDBAL/TDBAL require it);
// buffers are page-aligned. depth*buffer_len (128 KiB) per device << 2 MiB.
const TX_DESC_OFF: usize = 0x0000; // 1 KiB
const RX_DESC_OFF: usize = 0x0400; // 1 KiB
const TX_BUF_OFF: usize = 0x1000; // DEPTH*BUFFER_LEN = 128 KiB
const RX_BUF_OFF: usize = TX_BUF_OFF + DEPTH * BUFFER_LEN; // 0x21000

/// One hugepage's virtual + physical base, with the region offsets computed off it.
#[derive(Clone, Copy)]
pub struct Layout {
    va: u64,
    phys: u64,
}

impl Layout {
    pub fn new(va: u64, phys: u64) -> Self {
        Self { va, phys }
    }
    /// Total bytes the carve uses — must be `<= hugepage size` (asserted by the caller). Bench-only: the
    /// shipped facade gives each subsystem its own hugepage (no split); the harness splits one page in two.
    #[cfg(feature = "bench")]
    pub fn used_bytes() -> usize {
        RX_BUF_OFF + DEPTH * BUFFER_LEN
    }
    fn tx_desc_va(&self) -> u64 {
        self.va + TX_DESC_OFF as u64
    }
    fn tx_desc_phys(&self) -> u64 {
        self.phys + TX_DESC_OFF as u64
    }
    fn rx_desc_va(&self) -> u64 {
        self.va + RX_DESC_OFF as u64
    }
    /// The RX descriptor ring's **virtual** base — exposed so a wake-mechanism harness (Sprint 8) can
    /// compute the MONITORX target (the 64-byte line containing the next-expected descriptor). Read-only;
    /// the ring construction still owns the descriptor writes.
    pub fn rx_desc_base_va(&self) -> u64 {
        self.rx_desc_va()
    }
    fn rx_desc_phys(&self) -> u64 {
        self.phys + RX_DESC_OFF as u64
    }
    fn tx_buf_va(&self) -> u64 {
        self.va + TX_BUF_OFF as u64
    }
    fn tx_buf_phys(&self) -> u64 {
        self.phys + TX_BUF_OFF as u64
    }
    fn rx_buf_va(&self) -> u64 {
        self.va + RX_BUF_OFF as u64
    }
    fn rx_buf_phys(&self) -> u64 {
        self.phys + RX_BUF_OFF as u64
    }
}

// ---- register offsets (queue 0) + bits (ixgbe_type.h) ------------------------------------------

const HLREG0: usize = 0x04240;
const CTRL_EXT: usize = 0x00018;
// RX
const RDBAL0: usize = 0x01000;
const RDBAH0: usize = 0x01004;
const RDLEN0: usize = 0x01008;
const RDH0: usize = 0x01010;
const RDT0: usize = 0x01018;
const RXDCTL0: usize = 0x01028;
const SRRCTL0: usize = 0x02100;
const RDRXCTL: usize = 0x02F00;
const RXCTRL: usize = 0x03000;
const RXPBSIZE0: usize = 0x03C00;
const DCA_RXCTRL0: usize = 0x02200;
const FCTRL: usize = 0x05080;
// TX
const TDBAL0: usize = 0x06000;
const TDBAH0: usize = 0x06004;
const TDLEN0: usize = 0x06008;
const TDH0: usize = 0x06010;
const TDT0: usize = 0x06018;
const TXDCTL0: usize = 0x06028;
const DMATXCTL: usize = 0x04A80;
const TXPBSIZE0: usize = 0x0CC00;
const DTXMXSZRQ: usize = 0x08100;
const RTTDCS: usize = 0x04900;
// MAC receive-address filter 0 (factory MAC, loaded from EEPROM on reset)
const RAL0: usize = 0x05400;
const RAH0: usize = 0x05404;

const RXDCTL_ENABLE: u32 = 0x0200_0000;
const TXDCTL_ENABLE: u32 = 0x0200_0000;
const DMATXCTL_TE: u32 = 0x1;
const RXCTRL_RXEN: u32 = 0x1;
const SRRCTL_DESCTYPE_ADV_ONEBUF: u32 = 0x0200_0000;
const SRRCTL_DESCTYPE_MASK: u32 = 0x0E00_0000;
const SRRCTL_DROP_EN: u32 = 0x1000_0000;
const SRRCTL_BSIZE_MASK: u32 = 0x1F; // BSIZEPACKET in 1 KiB units
const HLREG0_TXCRCEN: u32 = 0x1;
const HLREG0_RXCRCSTRP: u32 = 0x2;
const HLREG0_TXPADEN: u32 = 0x400;
const RDRXCTL_CRCSTRIP: u32 = 0x2;
const FCTRL_BAM: u32 = 0x400;
const CTRL_EXT_NS_DIS: u32 = 0x0001_0000;
const RXPBSIZE_128KB: u32 = 0x0002_0000;
const TXPBSIZE_40KB: u32 = 0x0000_A000;
const RTTDCS_ARBDIS: u32 = 0x40;
// Register-map completeness: the RAH address-valid bit, read by the bench `mac_valid` check (kept in all
// builds as part of the documented register map, so unused in the TCB build).
#[allow(dead_code)]
const RAH_AV: u32 = 0x8000_0000;

// ---- statistics counter offsets (read-to-clear / RO; harmless MMIO reads) ----------------------
// Bench-only: read exclusively through the bench instrumentation (`read_counter`). The shipped TCB path
// reads no device statistics, so these are `cfg(bench)`-absent from it.
#[cfg(feature = "bench")]
pub const GPTC: usize = 0x04080; // Good Packets Transmitted
#[cfg(feature = "bench")]
pub const GPRC: usize = 0x04074; // Good Packets Received (L2-filtered)
#[cfg(feature = "bench")]
pub const TPR: usize = 0x040D0; // Total Packets Received (regardless of filter/RXEN)
#[cfg(feature = "bench")]
pub const TORL: usize = 0x040C0; // Total Octets Received low
#[cfg(feature = "bench")]
pub const TORH: usize = 0x040C4; // Total Octets Received high
#[cfg(feature = "bench")]
pub const RXDGPC: usize = 0x02F50; // DMA Good Rx Packet — posted to host memory (0 ⇒ no host write)
#[cfg(feature = "bench")]
pub const RXMPC0: usize = 0x03FA0; // Rx Missed Packets (buffer 0): arrived-but-not-posted drop

#[cfg(feature = "bench")]
pub fn read_counter(bar: &Bar0, off: usize) -> u32 {
    bar.read32(off)
}

/// The RX tail register (`RDT`) as a raw read pointer — the software's last-refilled descriptor index.
/// The **next-expected** descriptor the consumer will read (and the device will fill) is `(RDT+1) % DEPTH`;
/// unlike the device head `RDH`, `RDT` is not advanced by the device, so it never races ahead of an
/// arrived-but-unconsumed frame. The event-driven `DdWaitHost` uses it to locate the MONITORX target
/// without coupling to the frozen `RxPath` cursor. Read-only.
pub fn rx_tail_ptr(bar: &Bar0) -> *const u32 {
    bar.reg_ptr(RDT0) as *const u32
}

/// Read the device's factory MAC from receive-address filter 0 (RAL0/RAH0, loaded from EEPROM on reset).
pub fn read_mac(bar: &Bar0) -> [u8; 6] {
    let low = bar.read32(RAL0);
    let high = bar.read32(RAH0); // [15:0] = MAC bytes 4-5; bit 31 = Address Valid
    [
        low as u8,
        (low >> 8) as u8,
        (low >> 16) as u8,
        (low >> 24) as u8,
        high as u8,
        (high >> 8) as u8,
    ]
}

/// Whether RAH0's Address-Valid bit is set (the factory MAC is present/valid).
#[cfg(feature = "bench")]
pub fn mac_valid(bar: &Bar0) -> bool {
    bar.read32(RAH0) & RAH_AV != 0
}

// ---- register bit helpers ----------------------------------------------------------------------

fn set_flags(bar: &Bar0, off: usize, flags: u32) {
    bar.write32(off, bar.read32(off) | flags);
}
fn clear_flags(bar: &Bar0, off: usize, flags: u32) {
    bar.write32(off, bar.read32(off) & !flags);
}
fn wait_set(bar: &Bar0, off: usize, mask: u32, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if bar.read32(off) & mask == mask {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

// ---- confined raw hugepage-memory helpers (our OWN memory) -------------------------------------

/// Pre-init both descriptor rings to `0xFF` (the Snabb/ixy trick: an all-ones pkt_addr is an address the
/// device cannot post to, preventing rogue host-RAM DMA if the queue were activated before real
/// descriptors are written). Our own hugepage memory.
pub fn preinit_desc_rings(layout: &Layout) {
    // SAFETY: the two descriptor-ring regions are DEPTH*16 bytes each at known offsets inside our owned,
    // resident hugepage; we write only those bytes.
    unsafe {
        std::ptr::write_bytes(layout.tx_desc_va() as *mut u8, 0xFF, DEPTH * 16);
        std::ptr::write_bytes(layout.rx_desc_va() as *mut u8, 0xFF, DEPTH * 16);
    }
}

/// Pre-clear the RX packet buffers to `0x00` so a later byte-equal match cannot come from stale memory.
pub fn clear_rx_buffers(layout: &Layout) {
    // SAFETY: the RX-buffer region is DEPTH*BUFFER_LEN bytes at a known offset inside our owned hugepage.
    unsafe {
        std::ptr::write_bytes(layout.rx_buf_va() as *mut u8, 0x00, DEPTH * BUFFER_LEN);
    }
}

/// Read back the RX packet buffers — used to prove they are all-zero before the device writes (the
/// un-fakeable pre-clear proof).
#[cfg(feature = "bench")]
pub fn rx_buffers_all_zero(layout: &Layout) -> bool {
    // SAFETY: read of our own hugepage RX-buffer region (DEPTH*BUFFER_LEN bytes at a known offset).
    let region =
        unsafe { std::slice::from_raw_parts(layout.rx_buf_va() as *const u8, DEPTH * BUFFER_LEN) };
    region.iter().all(|&b| b == 0)
}

// ---- TX bring-up (§4.6.8) ----------------------------------------------------------------------

/// Program + enable one TX queue over the hugepage TX region. Returns once TXDCTL.ENABLE reads back
/// (or the timeout elapses — reflected in the returned bool).
pub fn init_tx(bar: &Bar0, layout: &Layout) -> bool {
    set_flags(bar, HLREG0, HLREG0_TXCRCEN | HLREG0_TXPADEN); // append CRC, pad short frames
    bar.write32(TXPBSIZE0, TXPBSIZE_40KB);
    for i in 1..8 {
        bar.write32(TXPBSIZE0 + i * 4, 0);
    }
    bar.write32(DTXMXSZRQ, 0xFFFF);
    clear_flags(bar, RTTDCS, RTTDCS_ARBDIS);

    // Queue 0 descriptor ring (physical base into the hugepage).
    let ring_phys = layout.tx_desc_phys();
    bar.write32(TDBAL0, ring_phys as u32);
    bar.write32(TDBAH0, (ring_phys >> 32) as u32);
    bar.write32(TDLEN0, (DEPTH * 16) as u32);
    // Prefetch thresholds (pthresh 36, hthresh 8); WTHRESH = 0 is REQUIRED here: with WTHRESH>0 the device
    // coalesces descriptor write-backs in batches and the per-descriptor RS bit is ignored (datasheet
    // §7.2.3.5.1), so a single RS-marked packet would never get its DD written back. WTHRESH=0 ⇒ RS forces
    // an immediate per-descriptor DD writeback, which is what our per-descriptor DD poll relies on.
    // pthresh=36 (bits 6:0), hthresh=8 (bits 14:8); WTHRESH (bits 22:16) left CLEAR = 0 (see above).
    let txdctl = (bar.read32(TXDCTL0) & !(0x7F | (0x7F << 8) | (0x7F << 16))) | (36 | (8 << 8));
    bar.write32(TXDCTL0, txdctl);

    bar.write32(DMATXCTL, bar.read32(DMATXCTL) | DMATXCTL_TE); // global TX DMA enable

    bar.write32(TDH0, 0);
    bar.write32(TDT0, 0);
    set_flags(bar, TXDCTL0, TXDCTL_ENABLE);
    wait_set(bar, TXDCTL0, TXDCTL_ENABLE, Duration::from_millis(1000))
}

/// Construct the confined volatile TX ring over the hugepage TX region + the BAR TDT doorbell.
pub fn build_tx_ring(bar: &Bar0, layout: &Layout) -> VolatileTxRing {
    // SAFETY: `tx_desc_va` is DEPTH contiguous AdvTxDesc in the mapped hugepage; `tx_buf_va`/`tx_buf_phys`
    // the DEPTH*BUFFER_LEN buffer region (virtual/physical); `reg_ptr(TDT0)` the TX-tail MMIO. All stay
    // mapped for the run (Bar0/Hugepage owned by the caller). Device-concurrency soundness is the residual.
    unsafe {
        VolatileTxRing::new(
            layout.tx_desc_va() as *mut AdvTxDesc,
            layout.tx_buf_va() as *mut u8,
            layout.tx_buf_phys(),
            DEPTH,
            BUFFER_LEN,
            bar.reg_ptr(TDT0),
        )
    }
}

// ---- RX bring-up (§4.6.7) ----------------------------------------------------------------------

/// Set/clear the global receiver enable (RXCTRL.RXEN). Split out so the TX-first phase can run with RXEN
/// **clear** (the one-time empirical confirm that TPR still ticks on wire receipt).
#[cfg(feature = "bench")]
pub fn set_rxen(bar: &Bar0, on: bool) {
    if on {
        set_flags(bar, RXCTRL, RXCTRL_RXEN);
    } else {
        clear_flags(bar, RXCTRL, RXCTRL_RXEN);
    }
}

/// Program one RX queue's registers over the hugepage RX region (datasheet §4.6.7). Does NOT enable the
/// per-queue RXDCTL or set the global RXEN — the caller arms descriptors first, then [`start_rx`]. Leaves
/// the receiver ready but the queue disabled (no host write can occur yet).
pub fn init_rx(bar: &Bar0, layout: &Layout) {
    clear_flags(bar, RXCTRL, RXCTRL_RXEN); // configure with RX disabled
    bar.write32(RXPBSIZE0, RXPBSIZE_128KB);
    for i in 1..8 {
        bar.write32(RXPBSIZE0 + i * 4, 0);
    }
    set_flags(bar, HLREG0, HLREG0_RXCRCSTRP);
    set_flags(bar, RDRXCTL, RDRXCTL_CRCSTRIP);
    set_flags(bar, FCTRL, FCTRL_BAM); // accept broadcast (unicast-to-us is via RAL0/RAH0 exact match)

    // Queue 0: advanced one-buffer descriptors, drop-on-no-descriptor, 2 KiB buffers.
    let srrctl = (bar.read32(SRRCTL0) & !(SRRCTL_DESCTYPE_MASK | SRRCTL_BSIZE_MASK))
        | SRRCTL_DESCTYPE_ADV_ONEBUF
        | SRRCTL_DROP_EN
        | (BUFFER_LEN as u32 / 1024); // BSIZEPACKET in 1 KiB units
    bar.write32(SRRCTL0, srrctl);

    let ring_phys = layout.rx_desc_phys();
    bar.write32(RDBAL0, ring_phys as u32);
    bar.write32(RDBAH0, (ring_phys >> 32) as u32);
    bar.write32(RDLEN0, (DEPTH * 16) as u32);
    bar.write32(RDH0, 0);
    bar.write32(RDT0, 0);

    set_flags(bar, CTRL_EXT, CTRL_EXT_NS_DIS);
    clear_flags(bar, DCA_RXCTRL0, 1 << 12); // reserved bit that must be 0 (per §4.6.7)
}

/// Construct the confined volatile RX ring over the hugepage RX region + the BAR RDT doorbell.
pub fn build_rx_ring(bar: &Bar0, layout: &Layout) -> VolatileRxRing {
    // SAFETY: as `build_tx_ring`, for the RX descriptor ring / buffers / RDT MMIO. The buffers must be
    // pre-cleared (caller) and their physaddr Gate-2-validated before the queue is enabled.
    unsafe {
        VolatileRxRing::new(
            layout.rx_desc_va() as *mut AdvRxDescWb,
            layout.rx_buf_va() as *mut u8,
            layout.rx_buf_phys(),
            DEPTH,
            BUFFER_LEN,
            bar.reg_ptr(RDT0),
        )
    }
}

/// Diagnostic: dump the TX queue state + descriptor 0's command/status words. Reads GPTC (clears it).
#[cfg(feature = "bench")]
pub fn dump_tx_state(bar: &Bar0, layout: &Layout) {
    // SAFETY: read of our own hugepage TX-desc region — cmd_type_len at u32 index 2, status at index 3.
    let (cmd, sta) = unsafe {
        let base = layout.tx_desc_va() as *const u32;
        (std::ptr::read_volatile(base.add(2)), std::ptr::read_volatile(base.add(3)))
    };
    eprintln!(
        "  [diag] TDH={} TDT={} TXDCTL=0x{:08x} DMATXCTL=0x{:08x} GPTC={} | desc0 cmd_type_len=0x{cmd:08x} status=0x{sta:08x}",
        bar.read32(TDH0),
        bar.read32(TDT0),
        bar.read32(TXDCTL0),
        bar.read32(DMATXCTL),
        bar.read32(GPTC),
    );
}

/// Diagnostic: dump the RX queue state + descriptor 0's status/length. Reads RXDGPC/GPRC/RXMPC (clears).
#[cfg(feature = "bench")]
pub fn dump_rx_state(bar: &Bar0, layout: &Layout) {
    // SAFETY: read of our own hugepage RX-desc region — status_error at u32 index 2, length in index 3 low.
    let (se, len) = unsafe {
        let base = layout.rx_desc_va() as *const u32;
        (std::ptr::read_volatile(base.add(2)), std::ptr::read_volatile(base.add(3)) & 0xFFFF)
    };
    eprintln!(
        "  [diag rx] RDH={} RDT={} RXDCTL=0x{:08x} RXCTRL=0x{:08x} SRRCTL=0x{:08x} RXDGPC={} GPRC={} RXMPC0={} | desc0 status_error=0x{se:08x} len={len}",
        bar.read32(RDH0),
        bar.read32(RDT0),
        bar.read32(RXDCTL0),
        bar.read32(RXCTRL),
        bar.read32(SRRCTL0),
        bar.read32(RXDGPC),
        bar.read32(GPRC),
        bar.read32(RXMPC0),
    );
}

/// Quiesce all DMA before releasing the device (drop the BAR mapping / rebind the kernel driver): clear
/// the global receiver, both per-queue enables, and the global TX DMA enable. On a no-IOMMU host this is
/// the deliberate "stop the device writing/reading host RAM before we let go" step.
pub fn quiesce(bar: &Bar0) {
    clear_flags(bar, RXCTRL, RXCTRL_RXEN);
    clear_flags(bar, RXDCTL0, RXDCTL_ENABLE);
    clear_flags(bar, TXDCTL0, TXDCTL_ENABLE);
    clear_flags(bar, DMATXCTL, DMATXCTL_TE);
}

/// Enable the RX queue (RXDCTL.ENABLE) and the global receiver (RXEN). Call AFTER the descriptors are
/// armed with real, validated pkt_addrs — so the device never fetches a garbage descriptor. Returns
/// whether RXDCTL.ENABLE read back.
pub fn start_rx(bar: &Bar0, layout: &Layout) -> bool {
    let _ = layout;
    set_flags(bar, RXDCTL0, RXDCTL_ENABLE);
    let ok = wait_set(bar, RXDCTL0, RXDCTL_ENABLE, Duration::from_millis(1000));
    set_flags(bar, RXCTRL, RXCTRL_RXEN);
    ok
}
