//! Closed-loop self-validation (Sprint 001, Slice 4) — the §6/§7/§8 invariants proven end-to-end.
//!
//! Two `transport` instances run the **same code** in opposite roles per session (a `box` opener/sender
//! and a `proxy` receiver), wired through a controllable in-memory fake wire and a **shared virtual
//! clock**. Pure software — no device, no real time. Uses only the public API (this is the consumer's
//! view).
//!
//! **Harness properties (stated, then asserted under):**
//! 1. **Drain-RX-before-deadline.** Each [`Loop::step`] processes all pending inbound frames
//!    (`exchange`) *before* evaluating retransmit deadlines (`poll`) — replicating the E4 reactor
//!    ordering. This is the whole justification for omitting the alternating bit (§6): a present ACK is
//!    always processed before its timer can fire, so a timer fires only on genuine loss.
//! 2. **No reorder.** Frames are delivered FIFO, in emission order, never shuffled (the wire does not
//!    reorder, §6) — so there is never a second, stale ACK arriving out of order to misread.
//!
//! Under (1)+(2) the harness asserts: no spurious retransmit when an ACK is present, and no data lost to
//! a stale ACK (every distinct payload is eventually delivered).

use core::time::Duration;
use transport::{Instant, Originator, SendError, SessionId, SessionTable, TransmitSink};

/// A one-direction fake wire: records every emitted frame verbatim, in order.
#[derive(Default)]
struct Wire {
    frames: Vec<Vec<u8>>,
}
impl TransmitSink for Wire {
    fn emit(&mut self, frame: &[u8]) {
        self.frames.push(frame.to_vec());
    }
}
impl Wire {
    /// Take all queued frames in FIFO order (no reorder).
    fn drain(&mut self) -> Vec<Vec<u8>> {
        core::mem::take(&mut self.frames)
    }
}

/// The closed loop: a box (sender) and a proxy (receiver), two channels, a shared clock.
struct Loop {
    boxs: SessionTable,
    proxy: SessionTable,
    box_out: Wire,   // frames the box emits → delivered to the proxy
    proxy_out: Wire, // frames the proxy emits (bare ACKs) → delivered to the box
    now: Instant,
    /// Payloads delivered up at the proxy (session, bytes) — duplicates allowed (no alternating bit).
    deliveries: Vec<(SessionId, Vec<u8>)>,
    /// Payloads delivered up at the BOX — i.e. data the proxy ORIGINATED (both-originate, Task 15).
    box_deliveries: Vec<(SessionId, Vec<u8>)>,
    /// When true, proxy→box frames (ACKs) are dropped on the wire (injected ACK loss).
    drop_acks: bool,
    /// Count of data frames delivered box→proxy (including retransmits).
    data_to_proxy: usize,
    /// Count of ACK frames delivered proxy→box.
    acks_to_box: usize,
}

impl Loop {
    fn new(interval: Duration, give_up: Duration) -> Self {
        Self {
            // Originator-partitioned halves (Task 15): the box draws the low half, the proxy the high half,
            // so both minting "from 0" cannot collide.
            boxs: SessionTable::with_timing(Originator::Low, interval, give_up),
            proxy: SessionTable::with_timing(Originator::High, interval, give_up),
            box_out: Wire::default(),
            proxy_out: Wire::default(),
            now: Instant(0),
            deliveries: Vec::new(),
            box_deliveries: Vec::new(),
            drop_acks: false,
            data_to_proxy: 0,
            acks_to_box: 0,
        }
    }

    /// The box opens a session by emission (allocate + send the first frame) — from the low half.
    fn open(&mut self, payload: &[u8]) -> SessionId {
        self.boxs.open(payload, self.now, &mut self.box_out)
    }

    /// The proxy opens a session by emission — from the high half (both-originate, Task 15).
    fn proxy_open(&mut self, payload: &[u8]) -> SessionId {
        self.proxy.open(payload, self.now, &mut self.proxy_out)
    }

    fn advance(&mut self, d: Duration) {
        self.now = self.now.saturating_add(d);
    }

    /// Drain RX in both directions (inbound), FIFO, no reorder. Box→proxy data is delivered up and bare-
    /// ACKed by the proxy; proxy→box ACKs are delivered to the box (releasing its in-flight frame),
    /// unless `drop_acks`.
    fn exchange(&mut self) {
        // Box → proxy. By ROLE (§6, table membership): a frame on a proxy SENDER session is an ACK (nothing
        // up); otherwise it is data the box originated — the proxy delivers it up + bare-ACKs.
        for f in self.box_out.drain() {
            if let Some(d) = self.proxy.on_inbound(&f, &mut self.proxy_out).unwrap() {
                self.data_to_proxy += 1;
                self.deliveries.push((d.session, d.payload.to_vec()));
            }
        }
        // Proxy → box (with optional loss). By ROLE: a frame on a box SENDER session is an ACK (nothing up,
        // counted); otherwise it is data the proxy ORIGINATED (its high-half session) — the box delivers it
        // up + bare-ACKs. The role is decided purely by table membership; the partition guarantees a
        // proxy-opened id is never in the box's sender table (both-originate, Task 15).
        for f in self.proxy_out.drain() {
            if self.drop_acks {
                continue; // injected ACK loss
            }
            match self.boxs.on_inbound(&f, &mut self.box_out).unwrap() {
                Some(d) => self.box_deliveries.push((d.session, d.payload.to_vec())),
                None => self.acks_to_box += 1,
            }
        }
    }

    /// One reactor iteration: **drain RX (inbound) before evaluating deadlines (poll)** — harness
    /// property (1).
    fn step(&mut self) {
        self.exchange();
        self.boxs.poll(self.now, &mut self.box_out);
        self.proxy.poll(self.now, &mut self.proxy_out);
    }

    /// Drive to quiescence: jump to the next armed deadline, step, repeat until nothing is armed and both
    /// wires are empty (or `max_steps`). Under persistent ACK loss this ends when give-up reclaims.
    fn run_to_quiescence(&mut self, max_steps: usize) {
        for _ in 0..max_steps {
            if let Some(d) = self.boxs.next_deadline()
                && d > self.now
            {
                self.now = d;
            }
            self.step();
            if self.boxs.next_deadline().is_none()
                && self.box_out.frames.is_empty()
                && self.proxy_out.frames.is_empty()
            {
                return;
            }
        }
    }
}

fn tiny() -> Loop {
    // Exact tiny timing for deterministic virtual-clock assertions: interval 10 ns, give-up 100 ns.
    Loop::new(Duration::from_nanos(10), Duration::from_nanos(100))
}

// ---- gate 5: interleaving ready-set, one wire, no scheduler --------------------------------------

#[test]
fn ready_set_drains_multiple_over_one_wire_no_scheduler() {
    let mut l = tiny();
    let a = l.open(b"alpha");
    let b = l.open(b"bravo");
    let c = l.open(b"charlie");

    // The ready-set is the three sessions with a frame to send.
    let mut rs: Vec<SessionId> = l.boxs.ready_set().collect();
    rs.sort();
    assert_eq!(rs, vec![a, b, c]);

    // All three data frames sit on the ONE wire, in emission order — no scheduler, no reorder.
    assert_eq!(l.box_out.frames.len(), 3, "three sessions interleaved on one wire");

    // next-deadline is the earliest armed retransmit (all armed at open(0) + interval(10 ns)).
    assert_eq!(l.boxs.next_deadline(), Some(Instant(10)));

    // Drain trivially: all delivered, all ACKed, all released → ready-set empty, nothing armed.
    l.step();
    assert_eq!(l.deliveries.len(), 3);
    assert_eq!(l.boxs.ready_set().count(), 0);
    assert_eq!(l.boxs.next_deadline(), None);
}

// ---- §6 ordering: no spurious retransmit / no stale-ACK data loss -------------------------------

#[test]
fn no_spurious_retransmit_when_ack_is_present() {
    let mut l = tiny();
    let _id = l.open(b"x");

    // Move time to exactly the retransmit deadline, then step: the ACK is processed (exchange) BEFORE
    // the deadline check (poll), so the in-flight frame is already released → no retransmit fires.
    l.advance(Duration::from_nanos(10));
    l.step();

    assert_eq!(l.data_to_proxy, 1, "ACK present + processed first ⇒ no spurious retransmit");
    assert_eq!(l.deliveries.len(), 1, "delivered exactly once — no duplicate");
    assert_eq!(l.acks_to_box, 1, "exactly one ACK — no duplicate ACK to misread");
    assert_eq!(l.boxs.next_deadline(), None, "released");
}

#[test]
fn no_data_lost_to_stale_ack_in_a_no_loss_run() {
    // A no-loss run: each frame is ACKed before its deadline (drain-RX-before-deadline), so no
    // retransmit ⇒ no duplicate ACK ever exists to be misread. Every distinct payload is delivered
    // exactly once, in order.
    let mut l = tiny();
    let payloads: [&[u8]; 4] = [b"one", b"two", b"three", b"four"];
    for p in payloads {
        let id = l.open(p);
        l.run_to_quiescence(20);
        assert!(l.boxs.next_deadline().is_none());
        // session released and dormant (no consumer-driven completion), still present
        assert!(l.boxs.contains(id));
    }
    let got: Vec<Vec<u8>> = l.deliveries.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(got, payloads.iter().map(|p| p.to_vec()).collect::<Vec<_>>());
    assert_eq!(l.data_to_proxy, 4, "no retransmits in a no-loss run");
    assert_eq!(l.acks_to_box, 4, "exactly one ACK per frame — no duplicate ACKs");
}

// ---- §7 reliability: loss → retransmit → eventual delivery; persistent loss → give-up -----------

#[test]
fn ack_loss_then_retransmit_then_eventual_delivery() {
    let mut l = tiny();
    let id = l.open(b"hello");

    // First round: the data is delivered to the proxy, but its ACK is dropped.
    l.drop_acks = true;
    l.step();
    assert_eq!(l.deliveries.len(), 1, "delivered once already");
    assert!(l.boxs.next_deadline().is_some(), "still awaiting an ACK → retransmit armed");

    // Stop dropping; drive: the box retransmits, the proxy re-delivers (a duplicate — fine, §6) and
    // ACKs, the ACK lands, the box releases.
    l.drop_acks = false;
    l.run_to_quiescence(20);

    assert!(
        l.deliveries.iter().any(|(s, p)| *s == id && p == b"hello"),
        "the payload is eventually delivered"
    );
    assert!(l.boxs.next_deadline().is_none(), "released after recovery");
    assert!(l.boxs.contains(id), "released session is dormant, not reclaimed");
}

#[test]
fn persistent_loss_gives_up_with_positive_reclaim() {
    let mut l = tiny();
    let id = l.open(b"gone");

    // The peer's ACKs never arrive: retransmit until the give-up bound, then timeout-only reclaim.
    l.drop_acks = true;
    l.run_to_quiescence(100);

    assert!(
        !l.boxs.contains(id),
        "persistent loss ⇒ timeout-only give-up ⇒ session REMOVED from the table (positive reclaim)"
    );
    assert_eq!(l.boxs.next_deadline(), None, "nothing armed after reclaim");
}

// ---- §8 lifecycle: dormant ≠ dead; epoch reset --------------------------------------------------

#[test]
fn dormant_session_is_never_reclaimed() {
    let mut l = tiny();
    let id = l.open(b"hi");
    l.step(); // delivered + ACKed → released, dormant (nothing in flight)
    assert!(l.boxs.contains(id));

    l.advance(Duration::from_nanos(100_000)); // far past the give-up bound
    l.step();
    assert!(l.boxs.contains(id), "a dormant session (nothing in flight) is never reclaimed");
}

#[test]
fn epoch_reset_reuses_id_with_no_carried_state() {
    // Epoch A.
    let mut a = tiny();
    let id_a = a.open(b"a-data");
    a.run_to_quiescence(20);

    // A reboot is a brand-new pair of tables (a new epoch): the id space resets and nothing carries.
    let mut b = tiny();
    let id_b = b.open(b"b-data");
    assert_eq!(id_a, id_b, "the id space resets each epoch (both allocate the seed id)");

    b.run_to_quiescence(20);
    let got: Vec<Vec<u8>> = b.deliveries.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(got, vec![b"b-data".to_vec()], "fresh epoch delivers only its own data");
}

// ---- back-pressure end-to-end (stop-and-wait holds across the loop) -----------------------------

#[test]
fn second_send_back_pressured_until_ack_then_succeeds() {
    let mut l = tiny();
    let id = l.open(b"first");

    // A second send while the first is in flight is back-pressured — no second wire frame.
    let r = l.boxs.send(id, b"second", l.now, &mut l.box_out);
    assert_eq!(r, Err(SendError::WouldBlock));
    assert_eq!(l.box_out.frames.len(), 1, "only the first frame is on the wire");

    // Complete the first round; then the second send succeeds.
    l.run_to_quiescence(20);
    let r2 = l.boxs.send(id, b"second", l.now, &mut l.box_out);
    assert_eq!(r2, Ok(()));
    l.run_to_quiescence(20);
    assert!(l.deliveries.iter().any(|(_, p)| p == b"first"));
    assert!(l.deliveries.iter().any(|(_, p)| p == b"second"));
}

// ---- Task 15: originator-partition — BOTH endpoints originate without collision (ADDED) ----------

#[test]
fn both_originate_disjoint_no_both_from_zero_collision() {
    // The Fork-1 scenario, now safe BY CONSTRUCTION: the box AND the proxy each open a session "from 0",
    // but the box draws the LOW half and the proxy the HIGH half, so the two ids are DISJOINT. Expected ids
    // follow from the mask encoding (Low base 0, High base 1<<63).
    let mut l = tiny();
    let box_id = l.open(b"request"); // box, low half
    let proxy_id = l.proxy_open(b"response"); // proxy, high half
    assert_eq!(box_id, SessionId(0), "box opens from the low half (id 0)");
    assert_eq!(proxy_id, SessionId(1 << 63), "proxy opens from the high half (id 1<<63)");
    assert_ne!(box_id, proxy_id, "box id-0 and proxy id-0 no longer collide (disjoint halves)");

    l.run_to_quiescence(20);
    // Both directions delivered exactly their own payload — no cross-mux, no collision, both released.
    assert!(
        l.deliveries.iter().any(|(s, p)| *s == box_id && p == b"request"),
        "the box's request is delivered to the proxy"
    );
    assert!(
        l.box_deliveries.iter().any(|(s, p)| *s == proxy_id && p == b"response"),
        "the proxy's response is delivered to the box — its high-half session, NOT read as the box's ACK"
    );
    assert!(
        l.boxs.next_deadline().is_none() && l.proxy.next_deadline().is_none(),
        "both sides quiescent — every session round-tripped + released"
    );
}

#[test]
fn role_logic_peer_opened_id_delivered_not_ack_read() {
    // ROLE by table membership (§6): a proxy-originated frame arriving at the box is DATA (delivered), not
    // an ACK — the box has no SENDER for that high-half id. If the two shared an id space, the box would
    // read it as its own ACK (the Fork-1 stall, nothing delivered); the partition makes that impossible.
    let mut l = tiny();
    let proxy_id = l.proxy_open(b"hello-box");
    l.step(); // proxy → box: the box delivers it up + bare-ACKs
    assert!(
        l.box_deliveries.iter().any(|(s, p)| *s == proxy_id && p == b"hello-box"),
        "a peer-opened id is delivered as DATA (role decided by table membership, not read as an ACK)"
    );
    l.run_to_quiescence(20);
    assert!(l.proxy.next_deadline().is_none(), "the box ACKed the proxy's session → released");
}
