//! Host tests for the bounded-poll block decision (`poll_decision`) — the `compute_timeout` analog. The
//! sleep + clock in `PollHost::block` are the live wiring; the DECISION (cap None at the interval, sleep-to
//! a near deadline as Timer, cap a far deadline) is pure and proven here.

use core::time::Duration;
use directpath::pollhost::poll_decision;
use reactor::WakeReason;

const INT: Duration = Duration::from_micros(50);

#[test]
fn park_forever_becomes_a_capped_poll_reptolling_rx() {
    // block(None) must NOT park forever (no interrupt to wake it) — sleep one interval, re-poll RX.
    assert_eq!(poll_decision(None, INT), (INT, WakeReason::NicMsix));
}

#[test]
fn near_deadline_is_slept_to_and_drives_the_retransmit() {
    let d = Duration::from_micros(20); // within the poll interval
    assert_eq!(poll_decision(Some(d), INT), (d, WakeReason::Timer));
    // exactly at the interval boundary still counts as reachable
    assert_eq!(poll_decision(Some(INT), INT), (INT, WakeReason::Timer));
}

#[test]
fn far_deadline_is_capped_at_the_poll_interval() {
    let d = Duration::from_millis(10); // the ~10ms retransmit deadline, far beyond the poll interval
    assert_eq!(poll_decision(Some(d), INT), (INT, WakeReason::NicMsix));
}
