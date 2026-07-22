#![deny(unsafe_code)]

//! `driver` — the NIC data path + the copy boundary (epic **E3**): the single piece of code standing
//! between **adversary-influenced descriptor-ring content** and the trusted host. All device-written
//! content is treated as hostile — bound the reported length, treat status as hostile (fail closed),
//! never deref an unchecked field, drop-and-refill — and each frame is copied out of DMA into app-owned
//! memory at the earliest point, so events carry the copy, never a ring pointer.
//!
//! Sprint 003, **complete** (Slices 1–4): the simulated-ring **seam** ([`RxRing`]/[`TxRing`], drawn
//! *above* the guards), the RX descriptor path ([`RxPath`]) + **adversarial guards**, the **copy
//! boundary** (each frame copied out into an app-owned `Vec` before refill; [`RxPath`] implements
//! [`reactor::RxDrain`]), the **TX path** ([`TxPath`], implementing `transport::TransmitSink`) with the
//! reclamation split + infallible drop-on-full `emit`, and the **confined real-volatile DMA rings**
//! ([`VolatileRxRing`]/[`VolatileTxRing`] in [`dma`]) — thin volatile reads, zero guard logic, each
//! `unsafe` block locally sound; their **crossing soundness has a 7b-verified account**
//! (`grounding/crossing-soundness.md`): the device-write→our-memory ordering (RX-read `DD`-acquire;
//! TX-write release-before-the-doorbell) is a reasoning artifact certified against the 82599 datasheet +
//! PCIe/SDM — **platform-scoped-sound**, with the remaining residuals tracked in `concerns-to-watch.md`.
//! The rings run on the real bench 82599 (Sprints 6–8); the no-NIC E4 integration
//! (`tests/e4_integration.rs`) drives the real reactor + the sim rings + the transport.
//!
//! ## `unsafe` confinement (deny + allow, the Sprint-2 pattern)
//!
//! The crate root is `#![deny(unsafe_code)]`; the one [`dma`] module re-`#![allow(unsafe_code)]` for the
//! real-volatile rings, while the sim ring ([`sim`]) stays **safe Rust** — the `unsafe` is compile-confined
//! to `dma` (deny at the root, allow only there).
//!
//! ## The seam is drawn ABOVE the guards
//!
//! [`RxRing`] exposes the **raw, untrusted** device-written fields *up* (`reported_len`, `status`) to
//! the shared guard logic in [`RxPath`]; the only *down*-call is a **bounded-length** copy
//! ([`RxRing::copy_out`], handed `min(reported, BUF)`). So an over-read is unsayable even in the real
//! impl, and the guards are hostile-testable in software via [`sim::SimRxRing`]. The real ring ([`dma`])
//! is a thin volatile read with **zero** guard logic.

mod dma;
mod rx;
mod seam;
pub mod sim;
mod tx;

pub use dma::{AdvRxDescWb, AdvTxDesc, VolatileRxRing, VolatileTxRing};
pub use rx::{RXD_ENUMERATED, RXD_STAT_EOP, RxPath};
pub use seam::{RxRing, TxRing};
pub use tx::TxPath;
