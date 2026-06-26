//! The seam contracts — the read-only map of the edge between this sprint (E4) and E2/E3/E5 — plus the
//! reactor's injectable clock seam. No `unsafe` here (the crate root denies it); these are the boundary
//! the reactor's loop (Slices 2–4) is built against.
//!
//! ## Seam map
//!
//! | seam | direction | owner | contract |
//! |---|---|---|---|
//! | wakeup eventfd | in (readiness) | **E2** | fires on the **ring-non-empty edge** (§5); the reactor `epoll`s it and `read_clear`s the latch on wake. In test, [`crate::Eventfd::notify`] simulates the edge. Concrete type: [`crate::Eventfd`]. |
//! | RX-drain | in (data) | **E3** | [`RxDrain::next_frame`] yields **all currently-available frames, then `None`**, each as **OWNED** bytes (`Vec<u8>`) — never a borrow into a DMA ring, so device-writable memory structurally cannot flow past the seam (E3 does the §3.1 copy-out at the RX edge; R3). |
//! | TX-write | out (data) | **E3** | **is** [`transport::TransmitSink`] — the reactor holds ONE impl and threads it (`&mut`) into every `transport` call; emission is **synchronous** (no deferred ready-set drain). "Sole TX writer" = sole sink holder (compile-enforced via `&mut`/move). |
//! | clock | in (time) | **E4 (this crate)** | [`MonotonicClock`] over `clock_gettime(CLOCK_MONOTONIC)`, implementing [`transport::Clock`]; **injectable** so tests substitute a fake clock and all timing logic stays pure/deterministic. |
//!
//! The `transport` public surface this loop drives: `open` / `send` / `on_inbound` / `poll`
//! (all take `now: Instant` + `&mut impl TransmitSink`), `next_deadline`, `ready_set`.

use transport::{Clock, Instant};

use crate::sys;

/// The **E3 RX-drain seam**. Repeated calls yield every currently-available frame as **owned** bytes,
/// then `None` when the ring is (momentarily) empty — the shape the poll-then-block core drains against
/// (Slice 2). The owned return type is load-bearing: it makes it impossible for device-writable (DMA)
/// memory to flow past the seam into the reactor (the reactor consumes owned bytes; E3 owns the copy).
pub trait RxDrain {
    /// The next available frame as owned bytes, or `None` if none is currently available.
    fn next_frame(&mut self) -> Option<Vec<u8>>;
}

/// The reactor's **clock seam**: production reads `CLOCK_MONOTONIC` through the confined [`sys`] wrapper;
/// tests inject a fake [`transport::Clock`], so every timing decision (deadline → timeout, poll-vs-block)
/// is exercised on pure logic, never the wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct MonotonicClock;

impl Clock for MonotonicClock {
    fn now(&self) -> Instant {
        // `clock_gettime(CLOCK_MONOTONIC)` cannot fail for this fixed, valid clock id with a valid
        // timespec pointer; on the impossible error we floor to 0 (the seam returns `Instant`, not a
        // Result) — total, never panics. (The error path is unreachable on Linux.)
        Instant(sys::clock_gettime_monotonic().unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The clock seam yields non-decreasing instants (the timing surface the deadline logic rides on).
    #[test]
    fn monotonic_clock_seam_is_nondecreasing() {
        let c = MonotonicClock;
        let a = c.now();
        let b = c.now();
        assert!(b >= a, "the clock seam never goes backwards");
    }

    /// A fake clock implementing the same seam — proves the clock is injectable (tests drive `now`
    /// deterministically, no wall clock). This is the shape Slices 2–4 use for pure timing assertions.
    #[test]
    fn clock_seam_is_injectable() {
        use core::cell::Cell;

        struct FakeClock(Cell<u64>);
        impl Clock for FakeClock {
            fn now(&self) -> Instant {
                Instant(self.0.get())
            }
        }

        let fake = FakeClock(Cell::new(0));
        assert_eq!(fake.now(), Instant(0));
        fake.0.set(10_000_000);
        assert_eq!(fake.now(), Instant(10_000_000));

        // A generic consumer takes any `Clock` — the reactor (Slices 2–4) is generic over it likewise.
        fn read_now(c: &impl Clock) -> Instant {
            c.now()
        }
        assert_eq!(read_now(&fake), Instant(10_000_000));
    }
}
