//! The opener's session-ID allocator: an **originator-partitioned** session-id space (`architecture.md`
//! §1.1/§8). The top bit is the originator ([`Originator`]); the low 63 bits are a monotonic counter that
//! wraps **within** the originator's half. Both endpoints allocate from their OWN half, so a session id is
//! unique across both originators **by construction** — the two halves never collide (this removes the
//! shared-allocator both-from-0 collision, the Fork-1 stall). IDs reset on reboot: a fresh allocator
//! restarts its half (no cross-epoch state, §8). This mints ids for sessions THIS endpoint opens; the
//! inbound demux that instantiates a session for a *peer*-allocated id (the other half) is
//! [`crate::SessionTable`].

use crate::frame::SessionId;

/// Which half of the session-id space an endpoint allocates from — the top bit of every id it mints.
/// **Neutral** at the transport layer: the application maps its role onto a half (e.g. box → `Low`,
/// proxy → `High`); the transport does not name box/proxy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Originator {
    /// The low half — top bit `0`.
    Low,
    /// The high half — top bit `1`.
    High,
}

impl Originator {
    /// The top bit this originator sets on every id it mints (`0` for `Low`, `1 << 63` for `High`).
    const fn top_bit(self) -> u64 {
        match self {
            Originator::Low => 0,
            Originator::High => 1 << 63,
        }
    }
}

/// Mints monotonic session IDs for sessions this endpoint opens, within its [`Originator`] half. Each id is
/// `top_bit | (counter & (2^63 - 1))`: the top bit is definitionally the originator and the low 63 bits are
/// a monotonic counter that wraps within the half — so a minted id **cannot** escape its half (the mask is
/// the invariant; no runtime assertion needed). A fresh allocator restarts the half.
#[derive(Debug)]
pub struct IdAllocator {
    top_bit: u64,
    /// The next low-63-bit counter value; always in `0..2^63` (masked on increment).
    next: u64,
}

/// The 63-bit counter mask — the low half of the id, below the originator top bit.
const LOW63: u64 = (1 << 63) - 1;

impl IdAllocator {
    /// A fresh allocator for `originator`'s half. The first id minted is that half's base (`0` for `Low`,
    /// `1 << 63` for `High`).
    pub fn new(originator: Originator) -> Self {
        Self { top_bit: originator.top_bit(), next: 0 }
    }

    /// Mint the next session id — `top_bit | (counter & (2^63 - 1))`. The counter is monotonic and wraps
    /// within the half on overflow (2^63 ids/epoch, unreachable in practice); the top bit keeps the id in
    /// its half by construction.
    pub fn allocate(&mut self) -> SessionId {
        let id = self.top_bit | self.next;
        self.next = (self.next + 1) & LOW63;
        SessionId(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IDs are monotonic and unique within an originator's half (proposition conserved from the pre-partition
    /// allocator; now the `Low` half is `0,1,2,…`).
    #[test]
    fn allocate_is_monotonic_and_unique() {
        let mut a = IdAllocator::new(Originator::Low);
        let ids: alloc::vec::Vec<SessionId> = (0..5).map(|_| a.allocate()).collect();
        assert_eq!(
            ids,
            [SessionId(0), SessionId(1), SessionId(2), SessionId(3), SessionId(4)]
        );
        for w in ids.windows(2) {
            assert!(w[1] > w[0]); // strictly increasing ⇒ all distinct
        }
    }

    /// A fresh allocator (a new epoch) restarts its half at the half's base — the reset is just
    /// reconstruction, no carried state (proposition conserved; shown here for the `High` half).
    #[test]
    fn fresh_allocator_restarts_its_half() {
        let base = 1u64 << 63;
        let mut epoch_a = IdAllocator::new(Originator::High);
        assert_eq!(epoch_a.allocate(), SessionId(base));
        assert_eq!(epoch_a.allocate(), SessionId(base + 1));

        let mut epoch_b = IdAllocator::new(Originator::High);
        assert_eq!(epoch_b.allocate(), SessionId(base)); // independent of epoch_a, restarts the High half
    }

    /// ADDED (Task 15): the two halves are DISJOINT by construction — both endpoints minting "from 0" no
    /// longer collide (the Fork-1 stall is impossible). Expected values follow from the mask encoding.
    #[test]
    fn halves_are_disjoint_no_both_from_zero_collision() {
        let mut lo = IdAllocator::new(Originator::Low);
        let mut hi = IdAllocator::new(Originator::High);
        let los: alloc::vec::Vec<SessionId> = (0..5).map(|_| lo.allocate()).collect();
        let his: alloc::vec::Vec<SessionId> = (0..5).map(|_| hi.allocate()).collect();
        let base = 1u64 << 63;
        assert_eq!(his[0], SessionId(base), "the High half bases at 1<<63");
        assert_ne!(los[0], his[0], "box id-0 and proxy id-0 no longer collide (disjoint halves)");
        for l in &los {
            for h in &his {
                assert_ne!(l, h, "no Low id equals any High id — the top bit partitions the space");
            }
        }
    }

    /// ADDED (Task 15): the counter wraps WITHIN the half — an id never escapes its originator's half.
    #[test]
    fn counter_wraps_within_the_half() {
        let mut lo = IdAllocator { top_bit: 0, next: LOW63 };
        assert_eq!(lo.allocate(), SessionId(LOW63), "last id of the Low half is all-low-bits-set, top bit 0");
        assert_eq!(lo.allocate(), SessionId(0), "the counter wraps within the half, back to base 0");
    }
}
