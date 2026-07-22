#![deny(unsafe_code)]

//! `directpath` — the pure-direct NIC subsystem (Sprints 6–9), in the `reference/ixy` `pci.c`/`memory.c`
//! shape: a userspace process on a running Linux system takes the bench 82599 and drives it directly
//! (sysfs unbind, bus-master enable, `resource0` mmap, hugepage/pagemap DMA — **no VFIO, no IOMMU**).
//!
//! ## Public surface
//!
//! The entire public API is the thin integration facade [`Subsystem`] — `construct` / `run` / `send`, the
//! reply handle [`Ctx`], the [`Event`] the run loop delivers, and the transport [`SessionId`]. Every
//! hardware module (`gate`/`pci`/`mem`/`pagemap`/`link`/`datapath`/`ethframe`/`ddwaithost`/`mwaitx`) is
//! `pub(crate)` internal — off the surface — so the proxy TCB that links this crate has a minimal attack
//! surface. (The bench harness reaches the internals through a `cfg(bench)` hatch; see `internals`.)
//!
//! ## Safety architecture
//!
//! Running on a **live production host with no IOMMU**, a state-changing op on the wrong device is
//! unconfined — so safety is **compiler-enforced**, not convention:
//! - `gate` is a pure `gate(bdf, allowlist, config) -> Decision`; `Allow` carries an **un-forgeable**
//!   `Allowed` token (private constructor, minted only on the allow arm), **bound** to its BDF.
//! - Every state-changing op (`pci`, `mem`) takes `&Allowed` and derives its target from the token, so
//!   there is **no path** to unbind/bus-master/mmap/DMA a device the gate did not allow.
//! - `pagemap` is the pure leg-2 core (physaddr parse + in-hugepage bounds), fail-closed and host-tested
//!   (Mitigation Gate 1) before any hardware contact.
//!
//! The crate root is `#![deny(unsafe_code)]`; exactly FIVE modules re-`allow` it, each confined and
//! SAFETY-noted — the whole trusted-unsafe surface an auditor reads: `pci` (BAR0 mmap / MMIO), `mem`
//! (hugepage mmap / mlock), `datapath` (volatile MMIO registers + DMA descriptor rings), `mwaitx`
//! (MONITORX/MWAITX inline asm + rdtsc), and `ddwaithost` (raw-pointer RX-descriptor `DD`-line monitoring).
//! The gate/pagemap logic carries zero `unsafe`.
//!
//! ## The full pure-direct path
//!
//! [`Subsystem::construct`] assembles the whole live path over one bench card: link bring-up (`link`); the
//! 82599 RX/TX ring datapath (`datapath`, over the `driver` volatile rings); the Ethernet encap/decap
//! behind the reactor's seams (`ethframe`); and the **event-driven idle-wait** (`ddwaithost`: the
//! `DdWaitHost` that parks on **MONITORX/MWAITX over the RX descriptor `DD` line**, via `mwaitx`) that the
//! reactor blocks on instead of a poll. [`Subsystem::run`] then drives it — busy-drain, then event-driven
//! park — the whole path from `unbind` to a proven event-driven round-trip between the bench cards.

// The hardware modules are crate-internal — off the public surface entirely (the TCB never names them).
// The public API (`Subsystem`/`Ctx`/`Event`, defined below in this composition root) is the whole surface;
// the bench harness reaches the internals through the `internals` hatch (a `pub` re-export gated on the
// non-default `bench` feature).
pub(crate) mod datapath;
pub(crate) mod ddwaithost;
pub(crate) mod ethframe;
pub(crate) mod gate;
pub(crate) mod gate2;
pub(crate) mod link;
pub(crate) mod mem;
pub(crate) mod mwaitx;
pub(crate) mod pagemap;
pub(crate) mod pci;

/// Bench-only two-instance harness scaffold (the four conservation harnesses share it). Compile-absent from
/// the TCB build.
#[cfg(feature = "bench")]
#[doc(hidden)]
pub mod harness;

// The public API a trusted caller (the proxy TCB / the box) speaks: `Subsystem`/`Ctx`/`Event` are defined
// below in this composition root; `Bdf` (the device-address value `construct` names — a plain identifier,
// not a capability; the un-forgeable `Allowed` token stays private) and the transport `SessionId` are
// re-exported here. Every hardware module above is `pub(crate)` internal; nothing else is public.
pub use gate::Bdf;
pub use transport::{SendError, SessionId};

/// **Bench-only internals hatch** — re-exports the hardware modules for the bench harness bins/tests. It is
/// gated behind the non-default `bench` feature and marked `#[doc(hidden)]`, so it is **compile-absent** from
/// the default (proxy/TCB) build graph: `cargo build` (no features) ships only the thin facade above, with
/// these modules unreachable. NOT part of the public API — a harness escape hatch only.
#[cfg(feature = "bench")]
#[doc(hidden)]
pub mod internals {
    // Nested re-export modules (not `pub use crate::datapath`, which E0365-rejects re-exporting a
    // `pub(crate)` module): each glob-re-exports the internal module's public items, so the harness keeps
    // its `internals::datapath::Layout` namespace while the module itself stays crate-private.
    pub mod datapath {
        pub use crate::datapath::*;
    }
    pub mod ddwaithost {
        pub use crate::ddwaithost::*;
    }
    pub mod ethframe {
        pub use crate::ethframe::*;
    }
    pub mod gate {
        pub use crate::gate::*;
    }
    pub mod gate2 {
        pub use crate::gate2::*;
    }
    pub mod link {
        pub use crate::link::*;
    }
    pub mod mem {
        pub use crate::mem::*;
    }
    pub mod mwaitx {
        pub use crate::mwaitx::*;
    }
    pub mod pagemap {
        pub use crate::pagemap::*;
    }
    pub mod pci {
        pub use crate::pci::*;
    }
}

// ============================================================================================
// The public API — the composition root (Subsystem / Ctx / Event + the run loop and the
// device-assembly `construct`). Kept in lib.rs (Rust's conventional home for a small public API +
// composition root) rather than a role-named module; the one-file TCB-audit surface is this file.
// ============================================================================================

use crate::datapath::Layout;
use crate::ddwaithost::DdWaitHost;
use crate::ethframe::{EthSink, EthSource};
use crate::gate::{gate, Allowed, Decision};
use crate::gate2::validate_physaddr;
use crate::link::{apply_restart, link_state, reset_and_init, LinkRestart};
use crate::mem::Hugepage;
use crate::mwaitx::rdtsc;
use crate::pci::Bar0;
use driver::{RxPath, RxRing, TxPath, VolatileRxRing, VolatileTxRing};
use reactor::{Reactor, RxDrainAsSource};
use std::io;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};
use transport::SessionTable;

/// The wired live reactor (event-driven `DdWaitHost`, Ethernet encap/decap over the confined driver rings).
type LiveReactor =
    Reactor<DdWaitHost, EthSource<RxDrainAsSource<RxPath<VolatileRxRing>>>, EthSink<TxPath<VolatileTxRing>>>;

fn err(m: impl Into<String>) -> io::Error {
    io::Error::other(m.into())
}

/// The pure-direct NIC subsystem — the single integration point for both the box and the proxy (symmetric,
/// same transport both ends). Construct over one bench card, then [`run`](Self::run) the event loop.
///
/// [`open`](Self::open) starts a session and emits its first frame (origination); [`send`](Self::send)`(id, ..)`
/// streams further frames on a session THIS endpoint opened (multi-frame, `architecture.md` §4/§6/§8).
/// [`SendError::NotSender`] refuses `send` on a session you *received* — a session's DATA is one-directional.
///
/// **Collision-safety (grounded in the current transport, not the model.md ideal):** the session-id
/// allocator is a single monotonic counter from 0, so origination is collision-safe for a SINGLE originator
/// per link (its own ids never self-collide). `model.md` §9's both-endpoints-originate reply model (the
/// proxy replies by *opening* a response session) additionally needs the originator-partitioned id space
/// (top-bit = originator; Task 15, designer-ruled 2026-07-21, BACKLOG — not built), so it is COMING, not
/// current: over today's shared allocator both endpoints minting from 0 collide (the Fork-1 silicon stall).
/// This API is ready for that model; drop this caveat when the partition lands.
///
/// ```no_run
/// use directpath::{Bdf, Ctx, Event, Subsystem};
/// use std::ops::ControlFlow;
///
/// # fn main() -> std::io::Result<()> {
/// let local = Bdf::parse("0000:41:00.0").expect("valid BDF");
/// let peer = Bdf::parse("0000:63:00.0").expect("valid BDF");
/// let mut sys = Subsystem::construct(local, &[local, peer], |_own_mac| [0x02, 0, 0, 0, 0, 0x63])?;
///
/// // Open a session (frame 1), then stream a further frame on it once the prior frame has ACKed
/// // (stop-and-wait; `send` returns `WouldBlock` while a frame is still in flight on that session).
/// let sid = sys.open(b"frame 1");
/// let _ = sys.send(sid, b"frame 2");
///
/// // Serve incoming deliveries, each keyed to its session id.
/// sys.run(|_ctx: &mut Ctx, ev: Event| match ev {
///     Event::Delivered(session, payload) => { let _ = (session, payload); ControlFlow::Continue(()) }
///     Event::Idle => ControlFlow::Break(()), // e.g. stop when idle (illustrative)
/// });
/// # Ok(())
/// # }
/// ```
pub struct Subsystem {
    // Declared first so it drops BEFORE its backing memory (its raw pointers reference `_hp`/`_bar`).
    reactor: LiveReactor,
    token: Allowed,
    driver: String,
    // Kept alive for the subsystem's lifetime (the reactor holds raw pointers into these mappings).
    _hp: Hugepage,
    _bar: Bar0,
    /// Bench-only conservation instrumentation, gathered INSIDE `run()` (zero effect on the shipped loop:
    /// the fields and their writes are `cfg(bench)`-absent from the TCB build). See [`Metrics`].
    #[cfg(feature = "bench")]
    metrics: Metrics,
}

/// Bench-only run() instrumentation (compile-absent from the TCB build). Two conserved diagnostics + one
/// new delivery-side sentinel, per the Sprint-9 Fork-1 conservation model:
/// - `idle_wakeup_us` — every actual NIC-driven park→wake latency (the make-or-break sub-tick floor +
///   the data-derived median band; the wake path is byte-identical, so an out-of-band median is a finding).
/// - `dispatch_us` — the **delivery-side sentinel** (R2): per burst, drain-start → all-delivered-to-handler,
///   the callback indirection the hidden ACK-RTT no longer shows. Measured to DELIVERY, never to ACK.
#[cfg(feature = "bench")]
#[derive(Default)]
pub struct Metrics {
    idle_wakeup_us: Vec<f64>,
    dispatch_us: Vec<f64>,
}

/// What [`run`](Subsystem::run) hands the handler each turn. `Delivered` carries the transport
/// [`SessionId`] the message is keyed to (for multiplexed demux — symmetric with
/// [`send`](Subsystem::send)'s return) plus an OWNED payload (the copy-boundary; never a device-memory
/// borrow, never an ACK). `Idle` is the tick fired when the reactor has drained and is about to park — the
/// app runs its own absence-of-data / deadline logic there (a silent sender detects a give-up on ITS OWN
/// deadline: give-up is deliberately silent, so there is no `GaveUp` event — `Idle` is the complete hook).
pub enum Event {
    /// An inbound app message: its session id and the owned payload.
    Delivered(transport::SessionId, Vec<u8>),
    /// The reactor drained; about to park. No bytes.
    Idle,
}

/// The reply handle passed to the [`run`](Subsystem::run) handler: it wraps origination so a delivery can be
/// answered without the caller ever holding the reactor (the loop stays owned by [`Subsystem`]).
pub struct Ctx<'a> {
    reactor: &'a mut LiveReactor,
}

impl Ctx<'_> {
    /// Open a NEW session with `payload` (origination, open-by-emission) — the reply mechanism from inside a
    /// handler: a proxy replies by opening a fresh response session, never by sending data back on the
    /// received one. Returns the new session id.
    pub fn open(&mut self, payload: &[u8]) -> transport::SessionId {
        self.reactor.originate(payload)
    }
    /// Continue an existing session `id` — one THIS endpoint opened — with another frame (multi-frame
    /// streaming). Errors: [`SendError::NotSender`] if `id` is a received session (never data back on it),
    /// [`SendError::NoSuchSession`], or [`SendError::WouldBlock`] (a frame is already in flight on `id`;
    /// stop-and-wait, ≤1).
    pub fn send(&mut self, id: transport::SessionId, payload: &[u8]) -> Result<(), transport::SendError> {
        self.reactor.send_on_session(id, payload)
    }
    /// Whether the reactor is quiescent (no held delivery, no in-flight retransmit) — the round-fully-ACKed
    /// completion signal. **Bench-only** measurement observability (a bool, not the ACK payload): the
    /// harness paces its one-directional bursts / measures send→ACK on the Idle tick through it, while the
    /// shipped `Ctx` surface stays just `send`.
    #[cfg(feature = "bench")]
    pub fn is_quiescent(&self) -> bool {
        self.reactor.quiescent()
    }
}

impl Subsystem {
    /// Construct the subsystem over `local` (the bench card to own). `allowlist` is the gate's set of
    /// permitted BDFs (production devices refused). `resolve_peer` is handed this card's own MAC (read after
    /// link-up) and returns the peer MAC to send to — production passes `|_| CONFIGURED_PEER_MAC`; a
    /// two-ended harness exchanges own↔peer over a channel. Assembles gate → pci → mem/pagemap → link →
    /// datapath rings → reactor + `DdWaitHost`.
    pub fn construct(
        local: Bdf,
        allowlist: &[Bdf],
        resolve_peer: impl FnOnce([u8; 6]) -> [u8; 6],
    ) -> io::Result<Self> {
        // 1. Gate: the un-forgeable token.
        let mut cfg = vec![0u8; 0x40];
        {
            use std::io::Read;
            std::fs::File::open(format!("/sys/bus/pci/devices/{}/config", local.sysfs()))?
                .read_exact(&mut cfg)?;
        }
        let token = match gate(local, allowlist, &cfg) {
            Decision::Allow(t) => t,
            Decision::Refuse(r) => return Err(err(format!("gate REFUSED {}: {r:?}", local.sysfs()))),
        };

        // 2. pci: take the device.
        let driver = pci::current_driver(&token).unwrap_or_else(|| "ixgbe".into());
        pci::unbind(&token)?;
        pci::enable_bus_master(&token)?;
        let bar = pci::map_resource0(&token)?;

        // 4. link (needs the BAR; before the DMA rings run).
        bring_up_link(&bar)?;
        let own_mac = datapath::read_mac(&bar);
        // Resolve the peer MAC now that our own address is known (production: config; harness: channel).
        let peer_mac = resolve_peer(own_mac);

        // 3. mem/pagemap: the Gate-2-validated hugepage DMA region + the ring/buffer layout.
        let hp: Hugepage = mem::alloc()?;
        validate_physaddr(hp.va(), hp.phys(), hp.size() as u64)
            .map_err(|e| err(format!("Gate 2 REFUSED the hugepage: {e:?}")))?;
        let layout = Layout::new(hp.va(), hp.phys());
        datapath::preinit_desc_rings(&layout);

        // 5. datapath over the driver rings.
        if !datapath::init_tx(&bar, &layout) {
            return Err(err("TXDCTL.ENABLE did not read back"));
        }
        datapath::init_rx(&bar, &layout);
        datapath::clear_rx_buffers(&layout);
        let tx_ring = datapath::build_tx_ring(&bar, &layout);
        let mut rx_ring = datapath::build_rx_ring(&bar, &layout);
        if !datapath::start_rx(&bar, &layout) {
            return Err(err("RXDCTL.ENABLE did not read back"));
        }
        for slot in 0..datapath::DEPTH {
            rx_ring.refill(slot); // arm all depth slots (steady-state re-arm; the wrap path)
        }

        // 6. reactor + the event-driven DdWaitHost.
        let tsc_hz = calibrate_tsc_hz();
        let host = DdWaitHost::new(layout.rx_desc_base_va(), datapath::rx_tail_ptr(&bar), datapath::DEPTH, tsc_hz, 5.0);
        let sink = EthSink::new(TxPath::new(tx_ring), peer_mac, own_mac);
        let source = EthSource::new(RxDrainAsSource::new(RxPath::new(rx_ring)));
        let reactor = Reactor::new(SessionTable::new(), host, source, sink);

        Ok(Self {
            reactor,
            token,
            driver,
            _hp: hp,
            _bar: bar,
            #[cfg(feature = "bench")]
            metrics: Metrics::default(),
        })
    }

    /// Open a NEW session with `payload` (origination, open-by-emission) — a spontaneous / initial send,
    /// before or between `run`s. Returns the new session id.
    pub fn open(&mut self, payload: &[u8]) -> transport::SessionId {
        self.reactor.originate(payload)
    }

    /// Continue an existing session `id` — one THIS endpoint opened — with another frame (multi-frame
    /// streaming). See [`Ctx::send`] for the [`SendError`] cases.
    pub fn send(&mut self, id: transport::SessionId, payload: &[u8]) -> Result<(), transport::SendError> {
        self.reactor.send_on_session(id, payload)
    }

    /// Run the event loop until the handler asks to stop. Each iteration busy-drains all ready deliveries
    /// (the reactor's `pump_all`), dispatches each as [`Event::Delivered`], then fires one [`Event::Idle`]
    /// before parking event-driven (the `DdWaitHost` park). The handler replies via [`Ctx`] and returns
    /// [`ControlFlow::Break`] to stop.
    ///
    /// The `Idle` tick is the app's hook for absence-of-data / deadline logic: a silent sender (no reply
    /// coming) breaks on its OWN deadline here while `run` keeps driving the shipped `pump_all` + park +
    /// retransmit + give-up underneath. Give-up is silent by design — there is no give-up event.
    ///
    /// (Deliveries are collected out of `pump_all` first, then handled — so the handler can reply via `Ctx`
    /// without re-entering the reactor while `pump_all` still borrows it. The stop-and-wait back-pressure
    /// stays inside the reactor; the caller never sees it.)
    pub fn run(&mut self, mut on_event: impl FnMut(&mut Ctx<'_>, Event) -> ControlFlow<()>) {
        let mut deliveries: Vec<(transport::SessionId, Vec<u8>)> = Vec::new();
        loop {
            #[cfg(feature = "bench")]
            let drain_start = Instant::now();
            self.reactor.pump_all(|session, payload| deliveries.push((session, payload)));
            #[cfg(feature = "bench")]
            let had_delivery = !deliveries.is_empty();
            for (session, payload) in deliveries.drain(..) {
                let mut ctx = Ctx { reactor: &mut self.reactor };
                if on_event(&mut ctx, Event::Delivered(session, payload)).is_break() {
                    return;
                }
            }
            // R2 sentinel: drain-start -> all-delivered-to-handler (the delivery-side callback path). Only
            // bursts that actually delivered are recorded (a pure-ACK/idle pass is not a delivery latency).
            #[cfg(feature = "bench")]
            if had_delivery {
                self.metrics.dispatch_us.push(drain_start.elapsed().as_nanos() as f64 / 1000.0);
            }
            // Idle tick (drained, about to park): the app's absence-of-data / deadline hook.
            {
                let mut ctx = Ctx { reactor: &mut self.reactor };
                if on_event(&mut ctx, Event::Idle).is_break() {
                    return;
                }
            }
            // Idle-wakeup: the actual NIC-driven park->wake latency (the conserved sub-tick floor + band).
            #[cfg(feature = "bench")]
            let park_start = Instant::now();
            let _wake = self.reactor.park();
            #[cfg(feature = "bench")]
            if _wake == Some(reactor::WakeReason::NicMsix) {
                self.metrics.idle_wakeup_us.push(park_start.elapsed().as_nanos() as f64 / 1000.0);
            }
            #[cfg(not(feature = "bench"))]
            self.reactor.park();
        }
    }
}

/// Bench-only instrumentation surface (compile-absent from the TCB build): the conservation diagnostics
/// gathered inside [`run`](Subsystem::run) and the device/reactor counters the re-run reads. NONE of this
/// widens the shipped surface — every item is `cfg(bench)`.
#[cfg(feature = "bench")]
impl Subsystem {
    /// Every actual NIC-driven park->wake latency (us), in order — the idle-wakeup distribution.
    pub fn idle_wakeups_us(&self) -> &[f64] {
        &self.metrics.idle_wakeup_us
    }
    /// The R2 delivery-side sentinel samples (us): per delivering burst, drain-start -> all-dispatched.
    pub fn dispatch_us(&self) -> &[f64] {
        &self.metrics.dispatch_us
    }
    /// Device counter read (RXMPC0 / GPRC / GPTC / RXDGPC ...), for the conserved correctness outcomes.
    pub fn counter(&self, off: usize) -> u32 {
        datapath::read_counter(&self._bar, off)
    }
    /// TX frames the sink dropped (pool exhaustion) — the gate-c observable.
    pub fn tx_dropped(&self) -> u64 {
        self.reactor.sink().inner().dropped()
    }
    /// Malformed inbound frames dropped by the transport (totality).
    pub fn malformed(&self) -> u32 {
        self.reactor.malformed()
    }
    /// Enable/disable this card's receiver — the dead-peer / genuine-silence induction for the give-up path.
    pub fn set_rx_enabled(&self, on: bool) {
        datapath::set_rxen(&self._bar, on);
    }
}

impl Drop for Subsystem {
    /// Reversible teardown: quiesce DMA and rebind the kernel driver (the device is returned to the host).
    fn drop(&mut self) {
        datapath::quiesce(&self._bar);
        let _ = pci::rebind(&self.token, &self.driver);
    }
}

/// Calibrate the invariant TSC rate for the `DdWaitHost` deadline conversion.
fn calibrate_tsc_hz() -> f64 {
    let a = rdtsc();
    std::thread::sleep(Duration::from_millis(200));
    (rdtsc() - a) as f64 / 0.2
}

fn bring_up_link(bar: &Bar0) -> io::Result<()> {
    if !reset_and_init(bar).ok() {
        return Err(err("reset/init incomplete"));
    }
    for form in [LinkRestart::None, LinkRestart::RestartAn, LinkRestart::ResetPipeline] {
        apply_restart(bar, form);
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(4) {
            let ls = link_state(bar);
            if ls.up && ls.speed_10g {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Err(err("link did not come up 10G"))
}
