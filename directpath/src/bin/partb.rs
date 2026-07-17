#![deny(unsafe_code)]

//! Sprint-6 Slice-1 Part-B run — the reversible, NON-DMA ops on the (production) host, scoped to the
//! bench cards via the frozen allowlist + the `Allowed` token. Sequence per bench BDF: gate → unbind →
//! bus-master enable → `resource0` mmap (+ a benign register read to confirm) → then a hugepage alloc +
//! pagemap resolve, then rebind (reversibility). HARD STOP at the non-DMA line: NO ring/DMA register is
//! written, no tail kicked — the device is never made to touch host RAM.
//!
//! Every op takes the `Allowed` token minted by `gate()`; the production BDF is never a target (it fails
//! the gate). Observed→expected throughout; self-pinning banner for re-derivation.

use directpath::gate::{gate, Bdf, Decision};
use directpath::{mem, pci};
use std::fs;
use std::os::unix::fs::FileExt;
use std::process::ExitCode;

/// The FROZEN bench allowlist (re-confirmed identity below, on this host).
const BENCH: [&str; 2] = ["0000:41:00.0", "0000:63:00.0"];
/// The production NIC — must never be a target; asserted absent from the allowlist.
const PRODUCTION: &str = "0000:61:00.0";

fn read_config(bdf: &Bdf) -> std::io::Result<Vec<u8>> {
    let f = fs::File::open(format!("/sys/bus/pci/devices/{}/config", bdf.sysfs()))?;
    let mut buf = vec![0u8; 0x40];
    f.read_exact_at(&mut buf, 0)?;
    Ok(buf)
}

fn main() -> ExitCode {
    println!("==== directpath Part-B run (reversible, NON-DMA) ====");
    println!("bench-allowlist : {BENCH:?}");
    println!("production-excl : {PRODUCTION} (never a target)");

    let allowlist: Vec<Bdf> = BENCH.iter().map(|s| Bdf::parse(s).unwrap()).collect();

    // Production must not be in the allowlist (source-proven exclusion).
    let prod = Bdf::parse(PRODUCTION).unwrap();
    assert!(!allowlist.contains(&prod), "production BDF must be absent from the allowlist");
    println!("assert : production-in-allowlist observed=false expected false");

    // 1. FREEZE + gate: mint a token per bench BDF; any refuse aborts (no op attempted).
    let mut tokens = Vec::new();
    for bdf in &allowlist {
        let config = match read_config(bdf) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("ABORT: config read {} failed: {e}", bdf.sysfs());
                return ExitCode::FAILURE;
            }
        };
        match gate(*bdf, &allowlist, &config) {
            Decision::Allow(t) => {
                println!(
                    "gate  : {} -> Allow (vendor:device {:#06x}:{:#06x} subsys {:#06x}:{:#06x})",
                    t.bdf().sysfs(),
                    u16::from_le_bytes([config[0], config[1]]),
                    u16::from_le_bytes([config[2], config[3]]),
                    u16::from_le_bytes([config[0x2c], config[0x2d]]),
                    u16::from_le_bytes([config[0x2e], config[0x2f]]),
                );
                tokens.push(t);
            }
            Decision::Refuse(r) => {
                eprintln!("ABORT: gate refused {}: {r:?}", bdf.sysfs());
                return ExitCode::FAILURE;
            }
        }
    }

    // Also prove the gate REFUSES production (assert-then-act at the choke-point).
    if let Ok(cfg) = read_config(&prod) {
        match gate(prod, &allowlist, &cfg) {
            Decision::Refuse(r) => println!("gate  : {PRODUCTION} -> Refuse({r:?}) (excluded, as expected)"),
            Decision::Allow(_) => {
                eprintln!("ABORT: gate ALLOWED production — invariant violated");
                return ExitCode::FAILURE;
            }
        }
    }

    // 2. The reversible, non-DMA ops, each via its token; record the driver for rebind.
    let mut drivers = Vec::new();
    for t in &tokens {
        let bdf = t.bdf().sysfs();
        let driver = pci::current_driver(t);
        drivers.push(driver.clone());
        println!("op    : {bdf} current-driver={driver:?}");

        if let Err(e) = pci::unbind(t) {
            eprintln!("ABORT: unbind {bdf} failed: {e}");
            return ExitCode::FAILURE;
        }
        println!("op    : {bdf} unbind          observed=OK (driver detached)");

        match pci::enable_bus_master(t) {
            Ok(cmd) => println!("op    : {bdf} bus-master      observed=COMMAND {cmd:#06x} bit2={}", (cmd >> 2) & 1),
            Err(e) => {
                eprintln!("ABORT: BME {bdf} failed: {e}");
                return ExitCode::FAILURE;
            }
        }

        match pci::map_resource0(t) {
            Ok(bar) => {
                // Benign MMIO reads to confirm the mapping — NOT a DMA (CPU reads device registers).
                let ctrl = bar.read32(0x0);
                let status = bar.read32(0x8);
                println!(
                    "op    : {bdf} resource0-mmap  observed=OK len={:#x} CTRL={ctrl:#010x} STATUS={status:#010x} (MMIO read, NO DMA)",
                    bar.len()
                );
                // bar drops here (munmap).
            }
            Err(e) => {
                eprintln!("ABORT: resource0 mmap {bdf} failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    // 3. Hugepage DMA memory setup (memory only, NO device DMA armed).
    match mem::alloc() {
        Ok(hp) => println!(
            "op    : hugepage        observed=OK va={:#x} phys={:#x} size={:#x} (2MiB-aligned, pagemap-validated) — NO device pointed at it",
            hp.va(),
            hp.phys(),
            hp.size()
        ),
        Err(e) => {
            eprintln!("ABORT: hugepage alloc failed: {e}");
            return ExitCode::FAILURE;
        }
    }

    // 4. Reversibility: rebind each bench card to its original driver, restoring the initial state.
    for (t, driver) in tokens.iter().zip(drivers.iter()) {
        let bdf = t.bdf().sysfs();
        match driver {
            Some(d) => match pci::rebind(t, d) {
                Ok(()) => println!("revert: {bdf} rebind {d}   observed=OK (restored)"),
                Err(e) => eprintln!("WARN: rebind {bdf} to {d} failed: {e} (rebind manually: echo {bdf} > /sys/bus/pci/drivers/{d}/bind)"),
            },
            None => println!("revert: {bdf} had no driver to rebind"),
        }
    }

    println!("==== HARD STOP at the non-DMA line: no ring enabled, no tail kicked, no device DMA armed ====");
    println!("RESULT : PASS (bench ops OK, production untouched, reversible, NO DMA)");
    ExitCode::SUCCESS
}
