//! **Mitigation Gate 2** — physaddr pre-validation against KNOWN-GOOD RAM, before any DMA target is
//! programmed. On the no-IOMMU direct path a wrong physaddr DMAs to arbitrary host RAM, so a
//! pagemap-resolved physaddr is not trusted until: (1) it lies in a `/proc/iomem` **System RAM** range
//! (not MMIO/reserved), and (2) a **VA↔PA self-check** confirms the mapping by an INDEPENDENT path —
//! write a known pattern to the hugepage VA, read the same physaddr back via `/dev/mem`, and **REFUSE**
//! on mismatch (never warn-and-continue). If `CONFIG_STRICT_DEVMEM` blocks the `/dev/mem` read, fall back
//! to a pagemap self-consistency re-check. This gate must gate any future DMA.
//!
//! The parse + range + decision logic is pure and host-tested; the runtime reads (`/proc/iomem`,
//! `/dev/mem`, pagemap) are memory reads only — NO device, NO DMA.

use crate::pagemap::{parse_pagemap_entry, phys_addr, PAGE_SIZE};
use std::io;
use std::os::unix::fs::FileExt;

/// Parse the `System RAM` ranges from `/proc/iomem` text: lines like `00001000-0009ffff : System RAM`.
/// Pure — host-tested. Returns inclusive-exclusive `(start, end)` pairs (`end` one past the last byte).
pub fn parse_system_ram(iomem: &str) -> Vec<(u64, u64)> {
    let mut ranges = Vec::new();
    for line in iomem.lines() {
        let line = line.trim_start();
        let Some((range, label)) = line.split_once(':') else { continue };
        if label.trim() != "System RAM" {
            continue;
        }
        let Some((a, b)) = range.trim().split_once('-') else { continue };
        if let (Ok(start), Ok(last)) = (u64::from_str_radix(a.trim(), 16), u64::from_str_radix(b.trim(), 16)) {
            ranges.push((start, last + 1)); // /proc/iomem end is inclusive
        }
    }
    ranges
}

/// Whether `[phys, phys+len)` lies wholly within a single System-RAM range. Pure — host-tested.
pub fn in_system_ram(phys: u64, len: u64, ranges: &[(u64, u64)]) -> bool {
    let Some(end) = phys.checked_add(len) else { return false };
    ranges.iter().any(|&(s, e)| phys >= s && end <= e)
}

#[derive(Debug)]
pub enum Gate2Error {
    /// The physaddr is not in a `/proc/iomem` System-RAM range (MMIO/reserved/unknown).
    NotSystemRam,
    /// The VA↔PA self-check disagreed — the pagemap physaddr does not point at our VA's memory.
    VaPaMismatch,
    /// A required runtime read failed. The cause is carried for `Debug`/diagnostics (not otherwise read).
    #[allow(dead_code)]
    Io(io::Error),
}

impl From<io::Error> for Gate2Error {
    fn from(e: io::Error) -> Self {
        Gate2Error::Io(e)
    }
}

/// Validate a pagemap-resolved DMA target `[phys, phys+len)` for the memory at `va`. Fail-closed:
/// - the physaddr must be in a System-RAM range (`/proc/iomem`);
/// - and the VA↔PA mapping must be confirmed: a known pattern written at `va` must read back at `phys`
///   via `/dev/mem` (REFUSE on mismatch); if `/dev/mem` is unavailable (STRICT_DEVMEM), fall back to a
///   pagemap self-consistency re-resolve (`va` must re-resolve to the same `phys`).
///
/// `va` must be a writable page we own (the hugepage). This performs memory reads/writes only — no
/// device, no DMA.
pub fn validate_physaddr(va: u64, phys: u64, len: u64) -> Result<(), Gate2Error> {
    // (1) System-RAM range.
    let iomem = std::fs::read_to_string("/proc/iomem")?;
    if !in_system_ram(phys, len, &parse_system_ram(&iomem)) {
        return Err(Gate2Error::NotSystemRam);
    }

    // (2) VA↔PA self-check. Write a distinctive pattern at the start of `va`, read the same `phys` back.
    // SAFETY note: `va` is our owned, writable hugepage; we only touch its first 8 bytes.
    let pattern: u64 = 0xA5A5_1234_DEAD_BEEF;
    crate::mem::write_owned_u64(va, pattern);

    match read_devmem_u64(phys) {
        Ok(readback) => {
            if readback == pattern {
                Ok(())
            } else {
                Err(Gate2Error::VaPaMismatch)
            }
        }
        // /dev/mem blocked (STRICT_DEVMEM) or absent → pagemap self-consistency fallback.
        Err(_) => {
            let reresolved = resolve_phys(va)?;
            if reresolved == phys {
                Ok(())
            } else {
                Err(Gate2Error::VaPaMismatch)
            }
        }
    }
}

fn read_devmem_u64(phys: u64) -> io::Result<u64> {
    let f = std::fs::File::open("/dev/mem")?;
    let mut buf = [0u8; 8];
    f.read_exact_at(&mut buf, phys)?;
    Ok(u64::from_le_bytes(buf))
}

fn resolve_phys(va: u64) -> io::Result<u64> {
    let f = std::fs::File::open("/proc/self/pagemap")?;
    let mut entry = [0u8; 8];
    f.read_exact_at(&mut entry, (va / PAGE_SIZE) * 8)?;
    let pfn = parse_pagemap_entry(&entry)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("pagemap: {e:?}")))?;
    Ok(phys_addr(pfn, va))
}

#[cfg(test)]
mod integration_tests {
//! Mitigation Gate 2 host tests — the pure physaddr-validation logic, fail-closed. The /proc/iomem
//! System-RAM parse + the range check are proven non-vacuously with no hardware; the VA↔PA self-check's
//! refuse-on-mismatch is the runtime half (reviewed-by-reading + a sudo self-run).

use super::{in_system_ram, parse_system_ram};

const IOMEM: &str = "\
00000000-00000fff : Reserved
00001000-0009ffff : System RAM
000a0000-000fffff : Reserved
00100000-7fffffff : System RAM
80000000-8fffffff : PCI Bus 0000:00
fed90000-fed93fff : dmar0
";

#[test]
fn parse_extracts_only_system_ram_ranges() {
    let r = parse_system_ram(IOMEM);
    // Two System RAM ranges; /proc/iomem ends are inclusive → we store one-past.
    assert_eq!(r, vec![(0x1000, 0xa0000), (0x100000, 0x80000000)]);
}

#[test]
fn in_system_ram_accepts_only_wholly_contained() {
    let r = parse_system_ram(IOMEM);
    // A 2 MiB buffer wholly inside the big System-RAM range.
    assert!(in_system_ram(0x0010_0000, 0x200000, &r), "inside System RAM");
    assert!(in_system_ram(0x7fe0_0000, 0x200000, &r), "ending exactly at the range end");
}

#[test]
fn out_of_system_ram_is_refused() {
    let r = parse_system_ram(IOMEM);
    assert!(!in_system_ram(0x8000_0000, 0x1000, &r), "in the PCI-bus (MMIO) range → refuse");
    assert!(!in_system_ram(0x000a_0000, 0x1000, &r), "in a Reserved range → refuse");
    assert!(!in_system_ram(0x7fff_f000, 0x200000, &r), "straddles the range end → refuse");
    assert!(!in_system_ram(u64::MAX - 4, 0x200000, &r), "overflow → refuse");
    assert!(!in_system_ram(0x1000, 0x1000, &[]), "no ranges → refuse everything (fail-closed)");
}
}
