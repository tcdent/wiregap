//! Sprint 8 — the `NicRun` seam: the proxy's abstraction over the NIC subsystem's *serving* surface
//! (`directpath::Subsystem::send`/`run`).
//!
//! This trait is the **structural lever** of the sprint's evidence model. `directpath::Subsystem` is
//! atomically opaque — `construct`/`run` are not generic or injectable, and both require the real
//! 82599 — so without an abstraction, every line that drives the subsystem would degrade to the
//! capture model. `NicRun` lets the lifecycle-sequencing and the run-loop adapter be driven by a
//! **fake** (a test double that synthesizes `Event`s), keeping that logic fresh-re-runnable by the
//! reviewer; only the real bring-up (`construct`) and the real wire round-trip (`RealNic::run` on
//! hardware) stay capture-model.
//!
//! **`Ctx` is deliberately omitted from `run`'s callback.** `Subsystem::run` hands the callback a reply
//! handle (`&mut Ctx`), but the proxy is receive-only this sprint: it never originates in response (R-c
//! — the response path is Phase 4). So the proxy's own run abstraction takes only the `Event`, and
//! [`RealNic`] discards the unused `Ctx`.
//!
//! **`open`/`send` are forward-looking.** The trait mirrors the full facade (`open` +
//! `send(id, …)` + `run`, matching the NIC designer's ruled session model — see
//! [[box-proxy-per-session-correlation-open-send]]), but the receive-only proxy calls neither today;
//! they are here so the response path (Phase 4) has a coherent surface, and so the abstraction is a
//! faithful stand-in for the real subsystem.

use std::ops::ControlFlow;

use directpath::{Event, SendError, SessionId, Subsystem};

/// The proxy's view of the NIC subsystem's serving surface. Generic (not `dyn`): the composition root
/// is monomorphized over the concrete implementor ([`RealNic`] in production, a fake in tests), so the
/// generic `run` method is fine and no object-safety is needed.
pub trait NicRun {
    /// Originate a **new** session with `payload`, returning its id (the box's first frame of a
    /// request — the ruled `open`). Forward-looking: the receive-only proxy does not originate today.
    fn open(&mut self, payload: &[u8]) -> SessionId;

    /// Continue an **existing** session `id` with a subsequent frame (the ruled `send(id, …)`).
    /// Forward-looking: unused on the receive path. Errors mirror the facade — `NotSender` (not a
    /// session you opened), `NoSuchSession`, `WouldBlock` (a frame already in flight, stop-and-wait).
    fn send(&mut self, id: SessionId, payload: &[u8]) -> Result<(), SendError>;

    /// Drive the event loop, handing each delivered [`Event`] to `on_event` until it returns
    /// [`ControlFlow::Break`]. The reply handle is intentionally not exposed (see the module doc).
    fn run<F: FnMut(Event) -> ControlFlow<()>>(&mut self, on_event: F);
}

/// The production implementor over a real, hardware-backed [`directpath::Subsystem`]. A newtype so the
/// proxy owns the trait impl and can drop the unused `&mut Ctx` from `Subsystem::run`'s callback.
///
/// Constructing a `RealNic` requires a real `Subsystem` (built by `Subsystem::construct`, which takes
/// the 82599), so a `RealNic` never exists in a hardware-free test — the fake implementor does.
pub struct RealNic(pub Subsystem);

impl NicRun for RealNic {
    fn open(&mut self, payload: &[u8]) -> SessionId {
        self.0.open(payload)
    }

    fn send(&mut self, id: SessionId, payload: &[u8]) -> Result<(), SendError> {
        self.0.send(id, payload)
    }

    fn run<F: FnMut(Event) -> ControlFlow<()>>(&mut self, mut on_event: F) {
        // Discard the reply handle: the proxy never originates in response (R-c).
        self.0.run(|_ctx, event| on_event(event));
    }
}
