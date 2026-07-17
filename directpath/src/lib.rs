#![deny(unsafe_code)]

//! `directpath` — the pure-direct NIC control module (Sprint 6), in the `reference/ixy` `pci.c`/`memory.c`
//! shape: a userspace process on a running Linux system takes the bench 82599 and drives it directly
//! (sysfs unbind, bus-master enable, `resource0` mmap, hugepage/`pagemap` DMA — **no VFIO, no IOMMU**).
//!
//! ## Safety architecture
//!
//! Running on a **live production host with no IOMMU**, a state-changing op on the wrong device is
//! unconfined — so safety is **compiler-enforced**, not convention:
//! - [`gate`] is a pure `gate(bdf, allowlist, config) -> Decision`; `Allow` carries an **un-forgeable**
//!   [`gate::Allowed`] token (private constructor, minted only on the allow arm), **bound** to its BDF.
//! - Every state-changing op ([`pci`], [`mem`]) takes `&Allowed` and derives its target from the token,
//!   so there is **no path** to unbind/bus-master/mmap/DMA a device the gate did not allow.
//! - [`pagemap`] is the pure leg-2 core (physaddr parse + in-hugepage bounds), fail-closed and
//!   host-tested (Mitigation Gate 1) before any hardware contact.
//!
//! The crate root is `#![deny(unsafe_code)]`; only [`pci`] and [`mem`] (mmap / mlock / MMIO) re-`allow`
//! it, confined and SAFETY-noted. The gate/pagemap logic carries zero `unsafe`.
//!
//! **Part-B boundary (this slice): reversible, NON-DMA ops only** — unbind, bus-master enable,
//! `resource0` mmap, hugepage alloc + pagemap resolve. It sets up ownership + memory; it does **NOT** arm
//! any device DMA (no ring enable, no tail kick). Device DMA is later slices, operator-present.

pub mod datapath;
pub mod ethframe;
pub mod gate;
pub mod gate2;
pub mod link;
pub mod pollhost;
pub mod mem;
pub mod pagemap;
pub mod pci;
