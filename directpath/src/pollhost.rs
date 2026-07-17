//! The **bounded-poll `Host`** — the pure-direct substrate seam impl the reactor parks on. There is **no
//! interrupt delivery** on the pure-direct path (`kernel-bypass-and-dma.md §4`), so "block" is a **capped
//! poll**, never an infinite park:
//! - `block(None)` (the reactor's `ParkForever`, no deadline) → sleep ONE poll interval and report
//!   [`WakeReason::NicMsix`] so the loop re-polls the RX ring. An infinite park would miss every RX frame
//!   (nothing would wake it).
//! - `block(Some(d))` → if the deadline is within the poll interval, sleep to it and report
//!   [`WakeReason::Timer`] (the reactor then drives the retransmit); else cap at the interval and report
//!   `NicMsix` (re-poll RX, re-check the deadline next time).
//!
//! [`poll_decision`] is the pure `compute_timeout` analog (the block-shape logic), host-tested; only
//! [`PollHost::block`] adds the sleep + the monotonic clock.

use core::time::Duration;
use reactor::{Host, WakeReason};
use std::time::Instant as StdInstant;
use transport::Instant;

/// Pure block decision: `(how long to sleep, which wake reason)`, capping the reactor's requested park
/// `timeout` at `poll_interval`. `None` (park-forever) becomes a capped poll → re-poll RX; a deadline
/// within reach is slept-to and reported as `Timer`; a farther deadline caps at the poll interval.
pub fn poll_decision(timeout: Option<Duration>, poll_interval: Duration) -> (Duration, WakeReason) {
    match timeout {
        None => (poll_interval, WakeReason::NicMsix),
        Some(d) if d <= poll_interval => (d, WakeReason::Timer),
        Some(_) => (poll_interval, WakeReason::NicMsix),
    }
}

/// The bounded-poll substrate seam. Monotonic `now` off a start instant; `block` = the capped-poll sleep.
pub struct PollHost {
    start: StdInstant,
    poll_interval: Duration,
}

impl PollHost {
    pub fn new(poll_interval: Duration) -> Self {
        Self { start: StdInstant::now(), poll_interval }
    }
}

impl Host for PollHost {
    fn now(&self) -> Instant {
        Instant(self.start.elapsed().as_nanos() as u64)
    }

    fn block(&mut self, timeout: Option<Duration>) -> WakeReason {
        let (sleep, reason) = poll_decision(timeout, self.poll_interval);
        std::thread::sleep(sleep);
        reason
    }
}
