#![allow(unsafe_code)]

//! Hugepage DMA memory (ixy `memory.c` shape) — allocate a 2 MiB hugepage, `mlock` it, and resolve its
//! **physical** address via `/proc/self/pagemap`, validated by the pure [`crate::pagemap`] leg (Gate 1).
//! This is memory setup only: it hands back a `(virtual, physical)` region; it does **NOT** program any
//! device register or arm any DMA. The device is pointed at `phys()` only in later, operator-present
//! slices.
//!
//! Confined `unsafe`: the `mmap`/`mlock` and the fault-in touch. The physaddr trust decision is the pure,
//! host-tested pagemap logic — a resolved physaddr that fails the fail-closed checks aborts the alloc.

use crate::pagemap::{buffer_in_hugepage, parse_pagemap_entry, phys_addr, HUGE_2MB, PAGE_SIZE};
use std::io;
use std::os::unix::fs::FileExt;

/// A pinned 2 MiB hugepage with its resolved physical base.
pub struct Hugepage {
    ptr: *mut u8,
    size: usize,
    phys: u64,
}

impl Hugepage {
    pub fn va(&self) -> u64 {
        self.ptr as u64
    }
    pub fn phys(&self) -> u64 {
        self.phys
    }
    pub fn size(&self) -> usize {
        self.size
    }
}

impl Drop for Hugepage {
    fn drop(&mut self) {
        // SAFETY: `ptr`/`size` came from a successful `mmap` in `alloc`; unmapped once on drop.
        unsafe {
            libc::munmap(self.ptr as *mut libc::c_void, self.size);
        }
    }
}

/// Allocate + pin a 2 MiB hugepage and resolve its physical base (fail-closed via the pagemap leg).
pub fn alloc() -> io::Result<Hugepage> {
    let size = HUGE_2MB as usize;
    // SAFETY: an anonymous 2 MiB hugepage mapping; MAP_FAILED is checked and returned as errno. The
    // pointer is owned by `Hugepage` (unmapped on drop).
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_HUGETLB,
            -1,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    let page = ptr as *mut u8;

    // Pin it (never swap) and fault it in so the pagemap has a resident PFN.
    // SAFETY: `ptr`/`size` are the mapping just created; `mlock` pins it; the single volatile byte write
    // faults the first page in (our own memory).
    unsafe {
        if libc::mlock(ptr, size) != 0 {
            let e = io::Error::last_os_error();
            libc::munmap(ptr, size);
            return Err(e);
        }
        std::ptr::write_volatile(page, 0);
    }

    // Resolve + validate the physical base via the pure, host-tested pagemap leg (Gate 1).
    let phys = match resolve_phys(page as u64) {
        Ok(p) => p,
        Err(e) => {
            // SAFETY: unmap the mapping we own before propagating.
            unsafe { libc::munmap(ptr, size) };
            return Err(e);
        }
    };
    // The hugepage must be a valid 2 MiB-aligned DMA leaf (fail-closed if the pagemap read was bad).
    if !buffer_in_hugepage(phys, size as u64, phys) {
        unsafe { libc::munmap(ptr, size) };
        return Err(io::Error::new(io::ErrorKind::InvalidData, "hugepage physaddr failed pagemap bounds"));
    }
    Ok(Hugepage { ptr: page, size, phys })
}

/// Write a `u64` to the first 8 bytes of an owned virtual address (used by the Gate-2 VA↔PA self-check
/// on our own hugepage). Confined here because it is a raw write.
pub fn write_owned_u64(va: u64, value: u64) {
    // SAFETY: the caller guarantees `va` is the base of a writable region we own (the hugepage), so
    // writing its first 8 bytes is in-bounds of our own memory. Volatile so it is not elided before the
    // subsequent read-back.
    unsafe {
        std::ptr::write_volatile(va as *mut u64, value);
    }
}

/// Resolve the physical address of `va` via `/proc/self/pagemap`, through the fail-closed parse.
fn resolve_phys(va: u64) -> io::Result<u64> {
    let f = std::fs::File::open("/proc/self/pagemap")?;
    let mut entry = [0u8; 8];
    f.read_exact_at(&mut entry, (va / PAGE_SIZE) * 8)?;
    let pfn = parse_pagemap_entry(&entry)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("pagemap parse: {e:?}")))?;
    Ok(phys_addr(pfn, va))
}
