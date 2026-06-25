#![no_main]
//! G2 — TCB-defense totality under fuzz (sprint 005; reworked 010). The **proxy** holds the reassembly
//! buffer, so a malformed/unbounded payload sequence the box authors must not crash, over-read,
//! **unbounded-allocate**, or **hang** it. This target feeds `Reassembler::push` directly over
//! **arbitrary `(chunk, continuation)` sequences** — the transport's shape (the L1 framing is below the
//! crate's copy boundary; the crate parses no frame header) — with the reassembler at a **small cap** so
//! cap-exceed is genuinely reached. `decode_message` (on a complete buffer) is exercised total.
//!
//! A **capping global allocator** sized above the small cap is the operational proof of the alloc bound:
//! the reassembly buffer is bounded by the cap (reject-before-grow), so no allocation may exceed
//! `ALLOC_CAP`. If one ever does — an unbounded bug-alloc — the allocator aborts and libFuzzer records a
//! crash; a clean run *is* the proof. std + `unsafe` here is the harness, not the library.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use libfuzzer_sys::fuzz_target;
use proto::{Reassembler, ReassemblyState, decode_message};

/// The reassembler's capacity bound for the fuzz — **small** so an arbitrary chunk sequence genuinely
/// reaches cap-exceed (the production 32 MiB is unreachable per iteration; the mechanism is cap-agnostic
/// — the `≤ cap`/`> cap` off-by-one is value-independent, validated here and in the unit test).
const SMALL_CAP: usize = 4096;

/// Above the small cap with ample headroom (the bounded `acc` is `≤ SMALL_CAP`; the `Vec` backing may
/// be up to ~2×). An allocation beyond this is an unbounded bug — the allocator aborts.
const ALLOC_CAP: usize = SMALL_CAP * 16; // 65536

/// Bound total input so iterations stay fast (the real frame bound is the transport's; this is a fuzz
/// pacing limit, not a protocol bound).
const MAX_INPUT: usize = 1 << 16;

/// A defensive loop bound (each chunk consumes ≥ 3 header bytes, so this is already bounded by the
/// input; the guard makes the no-hang property explicit).
const MAX_CHUNKS: u32 = 4096;

static MAX_SINGLE_ALLOC: AtomicUsize = AtomicUsize::new(0);

struct CapAlloc;

unsafe impl GlobalAlloc for CapAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        MAX_SINGLE_ALLOC.fetch_max(size, Ordering::Relaxed);
        if size > ALLOC_CAP {
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
    if data.len() > MAX_INPUT {
        return;
    }
    // Parse `data` as a sequence of `(continuation, chunk)`: `[u8 cont][u16 len][len bytes]…` — the
    // transport's delivered shape (bit 0 of `cont` is the continuation signal). The crate parses no L1
    // frame; this drives `push` directly over arbitrary chunk/continuation sequences.
    let mut r = Reassembler::new(SMALL_CAP);
    let mut rest = data;
    let mut chunks = 0u32;
    while let Some((cont_byte, after)) = rest.split_first() {
        let Some((lenb, after)) = after.split_first_chunk::<2>() else {
            break;
        };
        let len = u16::from_le_bytes(*lenb) as usize;
        let Some((chunk, tail)) = after.split_at_checked(len) else {
            break;
        };
        rest = tail;
        chunks += 1;
        if chunks > MAX_CHUNKS {
            break;
        }
        let continuation = cont_byte & 1 != 0;
        // push fail-closed: Accumulating | Complete | Err(ReassemblyOverflow) — never panic/hang;
        // `acc` never exceeds the cap (the capping allocator backs that claim).
        match r.push(chunk, continuation) {
            Ok(ReassemblyState::Accumulating) => {}
            Ok(ReassemblyState::Complete(buf)) => {
                // a complete reassembled buffer is itself decode_message-total
                let _ = decode_message(&buf);
                r = Reassembler::new(SMALL_CAP);
            }
            Err(_) => {
                // fail-closed (ReassemblyOverflow): the proxy denies + drops; reset and continue
                r = Reassembler::new(SMALL_CAP);
            }
        }
    }
});
