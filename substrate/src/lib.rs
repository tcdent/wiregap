#![no_std]
#![deny(unsafe_code)]

//! `substrate` — the bare-metal foundation for binding **this** 82599 (epic **E2**, bare-metal
//! re-scope). No Linux kernel: the project owns the machine from the UEFI handoff down. This crate is
//! sized to exactly what driving one 82599 needs — **not** a general OS. Slice 1 is the UEFI handoff +
//! serial + the test harness ([`banner`], [`serial`]); Slice 2 adds ECAM config access for this NIC's
//! BDF ([`acpi`] → [`ecam`]) + the physical DMA [`arena`]; Slice 3 adds this NIC's interrupt path
//! ([`interrupts`] IDT + [`lapic`] one-shot timer / self-IPI); Slice 4 adds the clean no_std event
//! [`reactor`] driving the reused `transport`, over a [`heap`] + TSC/LAPIC [`calibrate`]ation, behind
//! the [`reactor_host`] seam impl.
//!
//! ## `unsafe` confinement (deny + allow, the E3/E4 convention)
//!
//! The crate root is `#![deny(unsafe_code)]`; each module that needs raw hardware access
//! re-`#![allow(unsafe_code)]`s locally, with a SAFETY note on every block. The full unsafe surface is
//! **nine** confined sites, and nothing else:
//! - [`serial`] — COM1 UART port I/O (the post-handoff console).
//! - [`acpi`] — reads of the firmware ACPI tables (RSDP → XSDT → MCFG).
//! - [`ecam`] — MMIO reads/writes of this NIC's PCIe config space.
//! - [`lapic`] — MMIO of the Local APIC (enable, one-shot timer, self-IPI, EOI).
//! - [`interrupts`] — the naked interrupt-entry stubs + loading the IDT.
//! - [`heap`] — handing the static heap region to the global allocator.
//! - [`calibrate`] — PIT/`0x61` port I/O + `rdtsc` for frequency calibration.
//! - [`reactor_host`] — `rdtsc` for the clock (the LAPIC/`hlt`/latch it uses are confined in their modules).
//! - `main.rs` (the UEFI entry) — the `ExitBootServices` call + the QEMU debug-exit port.
//!
//! The pure/security-bearing logic ([`arena`], [`banner`], the `ecam`/`arena` address arithmetic, the
//! [`interrupts::VectorPlan`] policy, and the whole [`reactor`] loop) carries **zero** `unsafe` and is
//! host-tested.
//!
//! ## Two harness surfaces
//!
//! - **(a) host `cargo test` of the pure logic.** The world-free code — here, the boot-[`banner`]
//!   formatter — is compiled for the host and asserted deterministically. The `uefi`/`x86_64` deps are
//!   gated to `cfg(target_os = "uefi")` (see `Cargo.toml`), so a host test build pulls in neither: it
//!   compiles this pure module and nothing else. This is the same no_std-lib idiom `transport` uses,
//!   established now even though the pure *reactor* logic it will carry lands in Slice 4.
//! - **(b) QEMU UEFI/OVMF serial-assert.** `main.rs` boots as a UEFI app, takes the machine at
//!   `ExitBootServices`, and reports **observed** facts over COM1 under a self-pinning banner; the
//!   runner (`qemu-run.sh`) captures the serial log for review to re-derive/re-run.

extern crate alloc;

pub mod acpi;
pub mod arena;
pub mod banner;
pub mod ecam;
pub mod interrupts;
pub mod reactor;

// Real hardware / the bare-metal runtime — present only on the target.
#[cfg(target_os = "uefi")]
pub mod calibrate;
#[cfg(target_os = "uefi")]
pub mod heap;
#[cfg(target_os = "uefi")]
pub mod lapic;
#[cfg(target_os = "uefi")]
pub mod reactor_host;
#[cfg(target_os = "uefi")]
pub mod serial;
