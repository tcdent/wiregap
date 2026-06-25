//! The opener's session-ID allocator: a 64-bit monotonic counter (`architecture.md` §1.1).
//!
//! The box is the **sole** allocator. IDs are unique **within an epoch** and reset on reboot — and the
//! reset needs no mechanism: a reboot constructs a fresh allocator, so the id space simply starts over
//! (no cross-epoch state, `architecture.md` §8). This mints ids for sessions THIS endpoint opens; the
//! inbound demux that instantiates a session for a *peer*-allocated id is [`crate::SessionTable`].

use crate::frame::SessionId;

/// Mints monotonic session IDs for sessions this endpoint opens. Strictly increasing within an epoch;
/// a fresh allocator (a reboot) restarts the space.
#[derive(Debug, Default)]
pub struct IdAllocator {
    next: u64,
}

impl IdAllocator {
    /// A fresh allocator for a new epoch. The first id minted is `0`.
    pub fn new() -> Self {
        Self { next: 0 }
    }

    /// Mint the next session id — strictly increasing, unique within the epoch. The id space is 2^64
    /// per epoch (unreachable in practice; a reboot resets it), so no wrap handling is carried.
    pub fn allocate(&mut self) -> SessionId {
        let id = self.next;
        self.next += 1;
        SessionId(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IDs are monotonic and unique within an epoch.
    #[test]
    fn allocate_is_monotonic_and_unique() {
        let mut a = IdAllocator::new();
        let ids: alloc::vec::Vec<SessionId> = (0..5).map(|_| a.allocate()).collect();
        assert_eq!(
            ids,
            [
                SessionId(0),
                SessionId(1),
                SessionId(2),
                SessionId(3),
                SessionId(4)
            ]
        );
        // strictly increasing => all distinct
        for w in ids.windows(2) {
            assert!(w[1] > w[0]);
        }
    }

    /// A fresh allocator (a new epoch) restarts the id space at 0 — the reset is just reconstruction,
    /// no carried state.
    #[test]
    fn fresh_allocator_restarts_epoch_space() {
        let mut epoch_a = IdAllocator::new();
        assert_eq!(epoch_a.allocate(), SessionId(0));
        assert_eq!(epoch_a.allocate(), SessionId(1));

        let mut epoch_b = IdAllocator::new();
        assert_eq!(epoch_b.allocate(), SessionId(0)); // independent of epoch_a
    }
}
