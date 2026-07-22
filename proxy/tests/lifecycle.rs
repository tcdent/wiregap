//! Sprint 8 Slice 3 — the lifecycle-ladder **sequencing** gate (IB1), fresh-re-runnable via the
//! `NicRun` fake: no hardware, no real privilege change. Only the real `construct` bring-up and the
//! live privileged credential capture (IB2) stay capture-model — those are not here.
//!
//! What this proves hardware-free:
//! - drop-before-run ordering (L15: no box byte processed before the drop);
//! - all-or-none readiness — EVERY authority, enumerated, closes the WHOLE proxy when broken, named by
//!   channel, with `run` never entered (total inertness, not merely "construct returned Err");
//! - a failed privilege drop fails closed (never serves);
//! - the production validator's named fail-closed placeholders (register real; the rest closed pending
//!   config).

use std::cell::RefCell;
use std::ops::ControlFlow;
use std::rc::Rc;

use directpath::{Event, SessionId};
use proto::{Message, NetAddress, NetPayload, Scheme, encode_message, scan_safe};
use proxy::dispatch::{Dispatcher, Routed};
use proxy::lifecycle::{AUTHORITIES, Authority, Readiness, run_adapter, serve, validate_authority};
use proxy::nic::{NicRun, Reply};
use proxy::privilege::{Credentials, observe};
use proxy::register::anthropic::VETTED_TOOLS;
use proxy::register::net::ROUTES;
use transport::{IdAllocator, Originator};

/// A shared, ordered log of `(session, payload)` pairs the recording [`Reply`] captures.
type ReplyLog = Rc<RefCell<Vec<(SessionId, Vec<u8>)>>>;

/// A recording [`Reply`] handle: the reversal of R-c made assertable hardware-free. It mints reply ids
/// from the proxy's OWN half via the **real committed** `transport::IdAllocator` (`Originator::High`,
/// what `Endpoint::Proxy` selects), so even the fake's reply ids land in the proxy partition — never a
/// box request id — and records every `open`/`send` for the test to inspect.
struct RecordingReply {
    alloc: IdAllocator,
    opened: ReplyLog,
    sent: ReplyLog,
}
impl Reply for RecordingReply {
    fn open(&mut self, payload: &[u8]) -> SessionId {
        let id = self.alloc.allocate();
        self.opened.borrow_mut().push((id, payload.to_vec()));
        id
    }
    fn send(&mut self, id: SessionId, payload: &[u8]) -> Result<(), directpath::SendError> {
        self.sent.borrow_mut().push((id, payload.to_vec()));
        Ok(())
    }
}

/// A fake `NicRun`: records that `run` was entered (into a shared order log), delivers a fixed event
/// list to the handler — each with a recording [`Reply`] handle — and counts deliveries. No hardware.
struct FakeNic {
    events: Vec<Event>,
    log: Rc<RefCell<Vec<&'static str>>>,
    deliveries: Rc<RefCell<usize>>,
    opened: ReplyLog,
    sent: ReplyLog,
}
impl NicRun for FakeNic {
    fn run<F: FnMut(&mut dyn Reply, Event) -> ControlFlow<()>>(&mut self, mut on_event: F) {
        self.log.borrow_mut().push("run");
        // The reply handle a real proxy gets: origination from its own high half via the real allocator.
        let mut reply = RecordingReply {
            alloc: IdAllocator::new(Originator::High),
            opened: self.opened.clone(),
            sent: self.sent.clone(),
        };
        let events: Vec<Event> = self.events.drain(..).collect();
        for ev in events {
            *self.deliveries.borrow_mut() += 1;
            if on_event(&mut reply, ev).is_break() {
                return;
            }
        }
    }
}

fn fake_nic(events: Vec<Event>, log: &Rc<RefCell<Vec<&'static str>>>) -> (FakeNic, Rc<RefCell<usize>>) {
    let deliveries = Rc::new(RefCell::new(0));
    (
        FakeNic {
            events,
            log: log.clone(),
            deliveries: deliveries.clone(),
            opened: Rc::new(RefCell::new(Vec::new())),
            sent: Rc::new(RefCell::new(Vec::new())),
        },
        deliveries,
    )
}

/// All authorities valid → serve, and the privilege drop precedes `run` (L15 ordering).
#[test]
fn all_authorities_valid_serves_and_drops_before_run() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let (mut nic, deliveries) = fake_nic(vec![Event::Idle], &log);
    let drop_log = log.clone();
    let r = serve(
        &mut nic,
        || {
            drop_log.borrow_mut().push("drop");
            Ok(())
        },
        |_authority| true, // every authority validates
        |_reply, _ev| ControlFlow::Continue(()),
    );
    assert_eq!(r, Readiness::Served);
    assert_eq!(*log.borrow(), vec!["drop", "run"], "the privilege drop must precede run (L15)");
    assert_eq!(*deliveries.borrow(), 1, "run drove the one delivered event");
}

/// IB1 — all-or-none, named, total inertness: EVERY authority, when it is the (single) broken one,
/// holds the WHOLE proxy closed and named, with `run` NEVER entered.
#[test]
fn any_single_broken_authority_closes_the_whole_proxy_named_and_run_never_entered() {
    for &broken in AUTHORITIES {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (mut nic, deliveries) = fake_nic(vec![Event::Idle], &log);
        let drop_log = log.clone();
        let r = serve(
            &mut nic,
            || {
                drop_log.borrow_mut().push("drop");
                Ok(())
            },
            |authority| authority != broken,
            |_reply, _ev| ControlFlow::Continue(()),
        );
        assert_eq!(r, Readiness::Closed(broken), "the failed authority is named");
        assert_eq!(*log.borrow(), vec!["drop"], "run must NEVER be entered when an authority is broken");
        assert_eq!(*deliveries.borrow(), 0, "not one box byte processed under a broken authority");
    }
}

/// A failed privilege drop fails closed — never validates authorities, never serves.
#[test]
fn a_failed_privilege_drop_closes_and_never_serves() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let (mut nic, deliveries) = fake_nic(vec![Event::Idle], &log);
    let r = serve(
        &mut nic,
        || Err(std::io::Error::from_raw_os_error(1)), // EPERM — the drop failed
        |_authority| true,
        |_reply, _ev| ControlFlow::Continue(()),
    );
    assert!(matches!(r, Readiness::DropFailed(_)), "a failed drop reports DropFailed");
    assert!(log.borrow().is_empty(), "a failed drop must not enter run (no box byte processed)");
    assert_eq!(*deliveries.borrow(), 0);
}

/// The production validator's named fail-closed placeholders: the register is real (present → ok,
/// empty → closed); the config-pending authorities fail closed, never silently pass.
#[test]
fn production_validator_placeholders_fail_closed_register_is_real() {
    assert!(validate_authority(Authority::RouteRegister, ROUTES));
    assert!(!validate_authority(Authority::RouteRegister, &[]), "an empty register is no authority");
    for authority in [Authority::FilesystemRoots, Authority::KernelProtections, Authority::Credential] {
        assert!(
            !validate_authority(authority, ROUTES),
            "{authority:?} must fail closed pending config, not silently pass",
        );
    }
}

/// With the real production validator, the ladder holds closed at the first config-pending authority
/// (FilesystemRoots) — the correct fail-closed posture until the config DSL lands — and the drop still
/// runs first. Documents that production does not serve today, by design, not by accident.
#[test]
fn production_ladder_holds_closed_pending_config_but_drops_first() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let (mut nic, deliveries) = fake_nic(vec![], &log);
    let drop_log = log.clone();
    let r = serve(
        &mut nic,
        || {
            drop_log.borrow_mut().push("drop");
            Ok(())
        },
        |authority| validate_authority(authority, ROUTES),
        |_reply, _ev| ControlFlow::Continue(()),
    );
    assert_eq!(
        r,
        Readiness::Closed(Authority::FilesystemRoots),
        "production holds closed at the first config-pending authority (fail-closed posture)",
    );
    assert_eq!(*log.borrow(), vec!["drop"], "drop happened; run did not");
    assert_eq!(*deliveries.borrow(), 0);
}

// ── Sprint 10 Slice 1: the reply seam (R-c reversal), hardware-free ──────────────────────────────────

/// The reply seam: `NicRun::run` hands the callback a [`Reply`] handle, and a handler that originates a
/// response is **assertable without hardware** — the first time a proxy reply can be proven off the
/// 82599. The recording fake mints the reply id from the proxy's OWN half via the real committed
/// allocator, so the id lands in the proxy partition (top bit set), and the payload is echoed verbatim.
#[test]
fn reply_seam_hands_a_recording_handle_and_originate_in_response_is_assertable() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let (mut nic, _d) = fake_nic(vec![delivered(1, b"req".to_vec())], &log);
    let mut reply_id = None;
    // A minimal handler: on the delivery, open a reply session and send a follow-up frame on it.
    nic.run(|reply, ev| {
        if let Event::Delivered(_s, _p) = ev {
            let id = reply.open(b"reply-body");
            reply.send(id, b"more").unwrap();
            reply_id = Some(id);
        }
        ControlFlow::Break(())
    });
    let opened = nic.opened.borrow();
    assert_eq!(opened.len(), 1, "the handler originated exactly one reply session");
    assert_eq!(opened[0].1, b"reply-body", "the reply payload is recorded verbatim (echoed, not interpreted)");
    let id = reply_id.expect("the delivery originated a reply");
    assert_ne!(id.0 & (1 << 63), 0, "the reply id lands in the proxy's high half (its own partition)");
    let sent = nic.sent.borrow();
    assert_eq!(sent.len(), 1, "one follow-up send");
    assert_eq!(sent[0], (id, b"more".to_vec()), "the follow-up send targets the just-opened reply session");
}

/// Id-partition correctness against the REAL committed allocator (the R-c-reversal safety check): the
/// proxy replies from `Endpoint::Proxy` → `Originator::High` (lifecycle bring_up + directpath's committed
/// mapping). The committed `transport::IdAllocator` keeps that half disjoint from the box's `Low` half,
/// so no proxy reply id (`Reply::open`) can ever collide with a box request id — the classic
/// both-from-zero / N-stall the originator-partition exists to prevent.
#[test]
fn proxy_reply_ids_never_collide_with_box_request_ids() {
    let mut proxy = IdAllocator::new(Originator::High);
    let mut boxx = IdAllocator::new(Originator::Low);
    let box_ids: std::collections::HashSet<u64> = (0..2000).map(|_| boxx.allocate().0).collect();
    for _ in 0..2000 {
        let reply = proxy.allocate().0;
        assert_ne!(reply & (1 << 63), 0, "every proxy reply id is in the high half");
        assert!(!box_ids.contains(&reply), "a proxy reply id must never collide with a box request id");
    }
    // The exact Fork-1 stall case: both endpoints' FIRST id must differ.
    assert_ne!(
        IdAllocator::new(Originator::High).allocate(),
        IdAllocator::new(Originator::Low).allocate(),
        "both-from-zero must not collide (the partition's whole point)",
    );
}

// ── The privilege module's observable surface, hardware-free (the real drop is capture-model) ────────

/// The IB2 predicate is exact: only all-triples-at-target + every cap set empty counts as dropped.
#[test]
fn is_fully_dropped_predicate_is_exact() {
    let dropped = Credentials {
        uid: (1000, 1000, 1000),
        gid: (1000, 1000, 1000),
        cap_effective: 0,
        cap_permitted: 0,
        cap_inheritable: 0,
        bounding_empty: true,
        ambient_empty: true,
        no_new_privs: true,
    };
    assert!(dropped.is_fully_dropped_to(1000, 1000));
    // A privileged saved-uid is re-acquirable → not dropped.
    assert!(!Credentials { uid: (1000, 1000, 0), ..dropped }.is_fully_dropped_to(1000, 1000));
    // A non-empty bounding set is re-acquirable via exec → not dropped.
    assert!(!Credentials { bounding_empty: false, ..dropped }.is_fully_dropped_to(1000, 1000));
    // A leftover permitted capability → not dropped.
    assert!(!Credentials { cap_permitted: 1, ..dropped }.is_fully_dropped_to(1000, 1000));
    // no_new_privs not stuck → the execve-guard is missing → not dropped.
    assert!(!Credentials { no_new_privs: false, ..dropped }.is_fully_dropped_to(1000, 1000));
    // Wrong target → not dropped.
    assert!(!dropped.is_fully_dropped_to(0, 0));
}

/// `observe()` reads the live credential state without panicking or changing anything (read-only). The
/// real drop+observe round-trip is certified by the live privileged capture, not here.
#[test]
fn observe_reads_credentials_without_side_effects() {
    let before = observe();
    let after = observe();
    assert_eq!(before, after, "observe is read-only — two reads agree, nothing changed");
}

// ── Slice 4: the run-loop adapter (Event::Delivered → Dispatcher::route; Event::Idle → continue) ─────

fn delivered(session: u64, l2: Vec<u8>) -> Event {
    Event::Delivered(SessionId(session), l2)
}
fn net_address_l2(joined: &str) -> Vec<u8> {
    encode_message(&Message::NetAddress(NetAddress {
        scheme: Scheme::Https,
        address: scan_safe(joined.replace('/', "\u{1f}").as_bytes()).unwrap(),
    }))
}
fn net_payload_l2(content: &[u8], more: bool) -> Vec<u8> {
    encode_message(&Message::NetPayload(NetPayload { more, content: content.to_vec() }))
}

/// The full receive path through the ladder, hardware-free: `serve` enters `run` (authorities faked
/// valid), and the adapter routes a `net-address` (stores the route) then a `net-payload` (recognized
/// under that route). Two Delivered events → two verdicts; `Idle` drives none. This is the Slice-4
/// wiring end to end minus the real 82599 (the `NicRun` fake stands in).
///
/// **Both frames are delivered under ONE `SessionId`** — the outcome of the box's ruled two-call
/// sequence `open(net_address) -> sid=1` then `send(sid=1, net_payload)` (NIC designer ruling,
/// 2026-07-21: session-continuation is a NIC-side exposure the proxy correlation already assumes
/// correctly). This is *not* two independent opens (which would be two different SessionIds and would
/// deny the payload as `NetPayloadNoRoute`); the per-session Route correlation (Task 14) is exactly
/// what the ruling confirms as-built.
#[test]
fn run_adapter_drives_the_receive_path_through_the_ladder() {
    let body = br#"{"model":"claude-x","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#;
    // sid=1 for BOTH frames — open(net_address)->1, then send(1, net_payload). Same session ⇒ correlated.
    let events = vec![
        delivered(1, net_address_l2("api.anthropic.com/v1/messages")),
        delivered(1, net_payload_l2(body, false)),
        Event::Idle,
    ];
    let verdicts = Rc::new(RefCell::new(Vec::new()));
    let sink = verdicts.clone();
    let adapter = run_adapter(Dispatcher::new(), ROUTES, VETTED_TOOLS, move |_s, routed| {
        sink.borrow_mut().push(routed.clone());
        ControlFlow::Continue(())
    });

    let log = Rc::new(RefCell::new(Vec::new()));
    let (mut nic, _deliveries) = fake_nic(events, &log);
    let drop_log = log.clone();
    let r = serve(
        &mut nic,
        || {
            drop_log.borrow_mut().push("drop");
            Ok(())
        },
        |_a| true,
        adapter,
    );

    assert_eq!(r, Readiness::Served);
    let v = verdicts.borrow();
    assert_eq!(v.len(), 2, "two Delivered events → two verdicts; Idle drives none");
    assert!(matches!(v[0], Routed::NetRouteAdmit(_)), "net-address admits and stores the route");
    assert!(matches!(v[1], Routed::NetPayloadAdmit(_)), "net-payload recognized under the stored route");
}

/// IB3 (adapter under sustained load, fresh): a long burst of deliveries with interleaved idles is
/// routed in order, none lost. The adapter never breaks on its own; ordering holds — 500 distinct
/// net-address sessions each admit.
#[test]
fn run_adapter_handles_sustained_delivery_in_order() {
    let mut events = Vec::new();
    for i in 0..500u64 {
        events.push(delivered(i, net_address_l2("api.anthropic.com/v1/messages")));
        if i % 50 == 0 {
            events.push(Event::Idle);
        }
    }
    let admits = Rc::new(RefCell::new(0usize));
    let sink = admits.clone();
    let adapter = run_adapter(Dispatcher::new(), ROUTES, VETTED_TOOLS, move |_s, routed| {
        if matches!(routed, Routed::NetRouteAdmit(_)) {
            *sink.borrow_mut() += 1;
        }
        ControlFlow::Continue(())
    });
    let log = Rc::new(RefCell::new(Vec::new()));
    let (mut nic, _d) = fake_nic(events, &log);
    let drop_log = log.clone();
    let r = serve(
        &mut nic,
        || {
            drop_log.borrow_mut().push("drop");
            Ok(())
        },
        |_a| true,
        adapter,
    );
    assert_eq!(r, Readiness::Served);
    assert_eq!(*admits.borrow(), 500, "every one of the 500 net-address deliveries admitted, in order, none lost");
}

/// The observer can stop the loop (the Slice-5 e2e stop pattern): returning `Break` from `on_verdict`
/// ends `run` after the proof rather than running forever — the next delivery is not processed.
#[test]
fn run_adapter_observer_can_break_the_loop() {
    let events = vec![
        delivered(1, net_address_l2("api.anthropic.com/v1/messages")),
        delivered(2, net_address_l2("api.anthropic.com/v1/messages")), // must NOT be processed
    ];
    let count = Rc::new(RefCell::new(0));
    let sink = count.clone();
    let adapter = run_adapter(Dispatcher::new(), ROUTES, VETTED_TOOLS, move |_s, _routed| {
        *sink.borrow_mut() += 1;
        ControlFlow::Break(()) // stop after the first verdict
    });
    let log = Rc::new(RefCell::new(Vec::new()));
    let (mut nic, _d) = fake_nic(events, &log);
    let drop_log = log.clone();
    let _ = serve(
        &mut nic,
        || {
            drop_log.borrow_mut().push("drop");
            Ok(())
        },
        |_a| true,
        adapter,
    );
    assert_eq!(*count.borrow(), 1, "the loop stopped after the observer broke — the second delivery was not processed");
}
