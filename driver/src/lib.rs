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
//! `unsafe` block locally sound; their **crossing soundness stays an open, bench-gated hazard**
//! (`concerns-to-watch.md`), exercised only on the prototype cards. The no-NIC E4 integration
//! (`tests/e4_integration.rs`) drives the real reactor + the sim rings + the transport.
//!
//! ## `unsafe` confinement (deny + allow, the Sprint-2 pattern)
//!
//! The crate root is `#![deny(unsafe_code)]`; the one [`dma`] module re-`#![allow(unsafe_code)]` for the
//! Slice-4 real-volatile ring. In Slice 1 the sim ring ([`sim`]) is **safe Rust** and `dma` is an empty
//! placeholder — the confinement boundary is established now; the unsafe arrives in Slice 4.
//!
//! ## The seam is drawn ABOVE the guards
//!
//! [`RxRing`] exposes the **raw, untrusted** device-written fields *up* (`reported_len`, `status`) to
//! the shared guard logic in [`RxPath`]; the only *down*-call is a **bounded-length** copy
//! ([`RxRing::copy_out`], handed `min(reported, BUF)`). So an over-read is unsayable even in the real
//! impl, and the guards are hostile-testable in software via [`sim::SimRxRing`]. The real ring (Slice 4)
//! will be a thin volatile read with **zero** guard logic.

mod dma;
mod rx;
mod seam;
pub mod sim;
mod tx;

pub use dma::{VolatileRxRing, VolatileTxRing};
pub use rx::{RXD_ENUMERATED, RXD_STAT_EOP, RxPath};
pub use seam::{RxRing, TxRing};
pub use tx::TxPath;
