//! Sprint 8 — the `NicRun` seam: the proxy's abstraction over the NIC subsystem's *serving* surface
//! (`directpath::Subsystem::run`). Sprint 10 Slice 1 widens it with the reply handle.
//!
//! This trait is the **structural lever** of the sprint's evidence model. `directpath::Subsystem` is
//! atomically opaque — `construct`/`run` are not generic or injectable, and both require the real
//! 82599 — so without an abstraction, every line that drives the subsystem would degrade to the
//! capture model. `NicRun` lets the lifecycle-sequencing and the run-loop adapter be driven by a
//! **fake** (a test double that synthesizes `Event`s), keeping that logic fresh-re-runnable by the
//! reviewer; only the real bring-up (`construct`) and the real wire round-trip (`RealNic::run` on
//! hardware) stay capture-model.
//!
//! **The reply handle (Sprint 10 Slice 1).** `Subsystem::run` hands its callback a borrowed
//! `&mut Ctx` — the origination handle by which a proxy replies (it opens a NEW proxy→box session, from
//! its own high half of the id space; it never sends data back on the received session). Sprint 8 was
//! receive-only (R-c blocked) and *discarded* that handle; Phase 4 reverses that. So `NicRun::run` now
//! hands its callback a proxy-owned [`Reply`] — a narrow `open`/`send` handle that mirrors `Ctx`'s
//! shape **without naming `directpath::Ctx` in the core**. [`RealNic`] adapts the real `&mut Ctx` to
//! `&mut dyn Reply` ([`CtxReply`], the one place `Ctx` is named in `proxy`); the test fake hands the
//! callback a recording `Reply`, so a reply is assertable **hardware-free** for the first time.
//!
//! Passing `Ctx` through raw would defeat the seam: `Ctx` cannot be constructed outside
//! `Subsystem::run`, so a fake could never hand one to a hardware-free test — the entire receive-path
//! suite would collapse onto real hardware. The narrow `Reply` trait exists precisely to avoid that
//! (Sprint 10 Non-Goals). The handle is live only for the callback invocation (`Ctx`'s own borrow
//! discipline, preserved by construction — nothing is held across calls).

use std::ops::ControlFlow;

use directpath::{Ctx, Event, SendError, SessionId, Subsystem};

/// The proxy's **in-callback reply handle** — a narrow, proxy-owned view of origination, handed to the
/// [`NicRun::run`] callback for the duration of that invocation. The proxy replies by [`open`](Reply::open)ing
/// a NEW proxy→box session (from its own high half of the originator-partitioned id space) and
/// [`send`](Reply::send)ing subsequent frames on it; it never sends data back on the received session.
///
/// Object-safe (`&mut dyn Reply`): the concrete handle is directpath's `Ctx` in production ([`CtxReply`])
/// and a recording double in tests, so the core depends on this trait, never on `directpath::Ctx`.
pub trait Reply {
    /// Open a NEW session with `payload`, returning its id — the reply mechanism (origination). The id
    /// is in this endpoint's own originator half (the proxy's high half), disjoint from the box's
    /// request ids by the allocator's partition invariant.
    fn open(&mut self, payload: &[u8]) -> SessionId;

    /// Continue an existing session `id` — one THIS endpoint opened — with another frame (multi-frame
    /// reply streaming). Errors mirror the facade: [`SendError::NotSender`] (a received session, never
    /// data back on it), [`SendError::NoSuchSession`], [`SendError::WouldBlock`] (a frame already in
    /// flight on `id`; stop-and-wait).
    fn send(&mut self, id: SessionId, payload: &[u8]) -> Result<(), SendError>;
}

/// The proxy's view of the NIC subsystem's serving surface. Generic (not `dyn`) over the implementor:
/// the composition root is monomorphized over [`RealNic`] in production / a fake in tests, so the
/// generic `run` method is fine and no object-safety is needed on `NicRun` itself.
pub trait NicRun {
    /// Drive the event loop, handing each delivered [`Event`] — and a [`Reply`] handle for originating a
    /// response — to `on_event` until it returns [`ControlFlow::Break`]. The handle is live only for
    /// each callback invocation (never held across calls; see the module doc).
    fn run<F: FnMut(&mut dyn Reply, Event) -> ControlFlow<()>>(&mut self, on_event: F);
}

/// The production implementor over a real, hardware-backed [`directpath::Subsystem`]. A newtype so the
/// proxy owns the trait impl and can adapt `Subsystem::run`'s `&mut Ctx` into `&mut dyn Reply`.
///
/// Constructing a `RealNic` requires a real `Subsystem` (built by `Subsystem::construct`, which takes
/// the 82599), so a `RealNic` never exists in a hardware-free test — the fake implementor does.
pub struct RealNic(pub Subsystem);

impl NicRun for RealNic {
    fn run<F: FnMut(&mut dyn Reply, Event) -> ControlFlow<()>>(&mut self, mut on_event: F) {
        // Adapt the real reply handle: wrap the borrowed &mut Ctx in a CtxReply for the duration of the
        // callback, so the core sees only &mut dyn Reply and never names directpath::Ctx.
        self.0.run(|ctx, event| {
            let mut reply = CtxReply(ctx);
            on_event(&mut reply, event)
        });
    }
}

/// Adapts directpath's borrowed `&mut Ctx` to the proxy-owned [`Reply`] handle. **The one place
/// `directpath::Ctx` is named in `proxy`** — the core depends only on `&mut dyn Reply`. Lives (and the
/// `Ctx` borrow lives) only for the callback invocation; never held across calls, so `Ctx`'s
/// per-invocation borrow discipline is preserved by construction.
struct CtxReply<'a, 'c>(&'a mut Ctx<'c>);

impl Reply for CtxReply<'_, '_> {
    fn open(&mut self, payload: &[u8]) -> SessionId {
        self.0.open(payload)
    }

    fn send(&mut self, id: SessionId, payload: &[u8]) -> Result<(), SendError> {
        self.0.send(id, payload)
    }
}
