#![no_main]
//! G2 — totality under fuzz. `decode_message(arbitrary)` must yield `Ok | Err`, **never** a panic,
//! over-read, or input-controlled allocation. For an `Ok`, the encode path and canonical round-trip
//! are exercised too. Built (by cargo-fuzz) with ASan + UBSan; `overflow-checks` and
//! `debug-assertions` are on (see `Cargo.toml`).
//!
//! A **capping global allocator** is the operational proof of law 11: every decode allocation is
//! frame-bounded, so no single allocation may exceed `MAX_FRAME * ALLOC_K`. If one ever does, the
//! allocator aborts and libFuzzer records a crash — so a clean run *is* the proof.
//!
//! This crate is std and uses `unsafe` for the allocator: it is the fuzz harness, NOT the library
//! (which stays `#![no_std]` + `#![forbid(unsafe_code)]`).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use libfuzzer_sys::fuzz_target;
use proto::{decode_message, encode_message};

/// The modeled maximum L2 frame the codec is ever handed. The real bound is the transport's NIC
/// frame; `proto` takes an owned slice. The harness ignores larger inputs so the cap stays meaningful.
const MAX_FRAME: usize = 4096;

/// A single decode allocation is frame-bounded (law 11): content/detail copies are ≤ frame, and the
/// segment vector's backing store is ≤ ~16×frame (one pointer+len per ≥2-byte segment). `K = 32`
/// gives ~2× headroom over that worst case; an input-*count*-controlled (unbounded) allocation would
/// exceed it.
const ALLOC_K: usize = 32;
const ALLOC_CAP: usize = MAX_FRAME * ALLOC_K; // 131072 bytes

/// High-water mark of the largest single allocation observed (for reporting).
static MAX_SINGLE_ALLOC: AtomicUsize = AtomicUsize::new(0);

struct CapAlloc;

unsafe impl GlobalAlloc for CapAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        MAX_SINGLE_ALLOC.fetch_max(size, Ordering::Relaxed);
        if size > ALLOC_CAP {
            // An input-controlled unbounded allocation — a law-11 violation. Abort so libFuzzer
            // records it as a crash; a clean run proves it never happens.
            std::process::abort();
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CapAlloc = CapAlloc;

fuzz_target!(|data: &[u8]| {
    // Model the transport frame bound; keeps the alloc cap a meaningful multiple of the frame.
    if data.len() > MAX_FRAME {
        return;
    }
    // Totality: any byte slice yields Ok or Err — reaching here at all means no panic/over-read.
    if let Ok(msg) = decode_message(data) {
        // Exercise encode + canonical idempotence. Note: `data` may carry reserved bits that decode
        // drops, so the canonical wire need not equal `data`; the invariant is wire→msg→wire fixity.
        let wire = encode_message(&msg);
        let redecoded = decode_message(&wire).expect("re-decode of a valid message must succeed");
        assert_eq!(msg, redecoded, "round-trip identity violated");
        assert_eq!(
            encode_message(&redecoded),
            wire,
            "canonical re-encode not byte-identical"
        );
    }
});
