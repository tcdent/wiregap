//! Bench-only two-instance harness scaffold (compile-absent from the TCB build). The four Slice-2
//! conservation harnesses all drive the SHIPPED facade on both ends of the bench pair — box-role on one
//! card, proxy-role on the other, each [`Subsystem::run`](crate::Subsystem::run) in its own thread. This
//! module holds the one piece they share: construct both instances (exchanging MACs over a channel through
//! the [`construct`](crate::Subsystem::construct) resolver), pin each to a core, run the two roles
//! concurrently, and join. Any per-harness phase coordination (e.g. the proxy disabling its receiver for a
//! give-up) rides on channels the role closures capture — the scaffold stays minimal.

use crate::{Endpoint, Subsystem};
use crate::gate::Bdf;
use std::io::{self, Read};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Fresh-nonce length (the un-fakeable per-message tell) and the fixed test payload length.
pub const NONCE_LEN: usize = 8;
pub const PAYLOAD_LEN: usize = 24;

/// An `io::Error` from a message (shared harness error helper).
pub fn err(m: impl Into<String>) -> io::Error {
    io::Error::other(m.into())
}

/// A fresh random nonce from `/dev/urandom` (kept OUT of the timed regions by pre-generating).
pub fn read_nonce() -> io::Result<[u8; NONCE_LEN]> {
    let mut f = std::fs::File::open("/dev/urandom")?;
    let mut n = [0u8; NONCE_LEN];
    f.read_exact(&mut n)?;
    Ok(n)
}

/// The canonical test payload: the nonce followed by `0x5A` padding to [`PAYLOAD_LEN`].
pub fn payload_of(nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
    let mut p = Vec::with_capacity(PAYLOAD_LEN);
    p.extend_from_slice(nonce);
    p.extend_from_slice(&[0x5A; PAYLOAD_LEN - NONCE_LEN]);
    p
}

/// Whether a delivered payload is a well-formed test payload (nonce + `0x5A` padding, right length).
pub fn well_formed(p: &[u8]) -> bool {
    p.len() == PAYLOAD_LEN && p[NONCE_LEN..].iter().all(|&b| b == 0x5A)
}

/// Lowercase hex of a byte slice (nonce listing).
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join("")
}

/// The `p`-quantile of an already-sorted slice (`p` in `0.0..=1.0`).
pub fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[((sorted.len() as f64 * p) as usize).min(sorted.len() - 1)]
}

/// Run `role_a` over card `a` (pinned to `core_a`) and `role_b` over card `b` (pinned to `core_b`)
/// concurrently, each handed a freshly-constructed [`Subsystem`] (MACs exchanged a↔b), and return both
/// roles' results. Each `Subsystem` is constructed INSIDE its thread (never crosses a thread boundary).
// A bench-only two-instance scaffold: eight distinct, meaningful parameters (the pair, the two cores, the
// cap, the two roles) read clearer inline than bundled into a config struct.
#[allow(clippy::too_many_arguments)]
pub fn two_facade<A, B, RA, RB>(
    a: Bdf,
    b: Bdf,
    allow: &[Bdf],
    core_a: usize,
    core_b: usize,
    idle_cap: Option<Duration>,
    role_a: A,
    role_b: B,
) -> io::Result<(RA, RB)>
where
    A: FnOnce(Subsystem) -> io::Result<RA> + Send,
    B: FnOnce(Subsystem) -> io::Result<RB> + Send,
    RA: Send,
    RB: Send,
{
    let (a_tx, a_rx) = mpsc::channel::<[u8; 6]>();
    let (b_tx, b_rx) = mpsc::channel::<[u8; 6]>();
    let allow_a = allow.to_vec();
    let allow_b = allow.to_vec();
    thread::scope(|s| -> io::Result<(RA, RB)> {
        let ha = s.spawn(move || -> io::Result<RA> {
            benchtools::affinity::pin(core_a)?;
            let sys = Subsystem::construct(a, &allow_a, Endpoint::Box, idle_cap, |own| {
                a_tx.send(own).expect("publish a mac");
                b_rx.recv().expect("recv b mac")
            })?;
            role_a(sys)
        });
        let hb = s.spawn(move || -> io::Result<RB> {
            benchtools::affinity::pin(core_b)?;
            let sys = Subsystem::construct(b, &allow_b, Endpoint::Proxy, idle_cap, |own| {
                b_tx.send(own).expect("publish b mac");
                a_rx.recv().expect("recv a mac")
            })?;
            role_b(sys)
        });
        let ra = ha.join().map_err(|_| io::Error::other("role_a thread panicked"))??;
        let rb = hb.join().map_err(|_| io::Error::other("role_b thread panicked"))??;
        Ok((ra, rb))
    })
}
