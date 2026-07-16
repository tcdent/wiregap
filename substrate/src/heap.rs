#![allow(unsafe_code)]

//! The bare-metal global heap. The reused `transport` (session table, in-flight buffers) and the
//! reactor's copy-out use `alloc`, but after `ExitBootServices` the UEFI boot-services allocator is
//! gone — so the substrate owns the heap: a fixed static region handed to a `linked_list_allocator`
//! (real free, not a leaking bump). [`init`] must run once, before the first allocation. The only
//! `unsafe` is handing the static region to the allocator, confined here.

use linked_list_allocator::LockedHeap;

/// 1 MiB — ample for a handful of sessions + in-flight frames; sized to this NIC's stop-and-wait use.
const HEAP_SIZE: usize = 1024 * 1024;

static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Initialise the heap. Call **once**, single-threaded, before any allocation (i.e. first thing in the
/// UEFI entry, before `ExitBootServices`).
pub fn init() {
    // SAFETY: called once before any allocation; `HEAP` is a `'static` byte region handed exclusively to
    // the allocator, which thereafter owns it. `&raw mut` avoids a reference to the mutable static.
    unsafe {
        ALLOCATOR.lock().init(&raw mut HEAP as *mut u8, HEAP_SIZE);
    }
}
