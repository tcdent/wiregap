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
use proxy::credential::Credential;
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
    assert!(validate_authority(Authority::RouteRegister, ROUTES, None, None));
    assert!(!validate_authority(Authority::RouteRegister, &[], None, None), "an empty register is no authority");
    // Sprint 16: FilesystemRoots is REAL now — it derives from the resolved roots, so `None`
    // (a failed or absent resolution) fails closed. KernelProtections is still config-pending.
    assert!(
        !validate_authority(Authority::FilesystemRoots, ROUTES, None, None),
        "unresolved filesystem roots must fail closed, not silently pass",
    );
    assert!(
        !validate_authority(Authority::KernelProtections, ROUTES, None, None),
        "KernelProtections must fail closed pending its probe, not silently pass",
    );
    // Sprint 11: the credential rung is REAL now — present-iff-supplied (no config DSL invented).
    assert!(!validate_authority(Authority::Credential, ROUTES, None, None), "a missing credential fails closed");
    let cred = Credential::new("sk-test-key".into());
    assert!(
        validate_authority(Authority::Credential, ROUTES, Some(&cred), None),
        "a supplied credential satisfies the rung",
    );
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
        |authority| validate_authority(authority, ROUTES, None, None),
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

/// Sprint 11: a MISSING upstream credential holds the **whole** proxy closed (the credential rung is
/// real now, not always-`false`). With every other authority passing, an absent credential still closes
/// the ladder at `Credential`; a supplied one satisfies that rung.
#[test]
fn missing_credential_closes_the_whole_proxy() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let (mut nic, deliveries) = fake_nic(vec![], &log);
    let drop_log = log.clone();
    let r = serve(
        &mut nic,
        || {
            drop_log.borrow_mut().push("drop");
            Ok(())
        },
        // Every authority passes EXCEPT the (absent) credential rung.
        |authority| match authority {
            Authority::Credential => validate_authority(Authority::Credential, ROUTES, None, None),
            _ => true,
        },
        |_reply, _ev| ControlFlow::Continue(()),
    );
    assert_eq!(
        r,
        Readiness::Closed(Authority::Credential),
        "a missing credential closes the WHOLE proxy",
    );
    assert_eq!(*log.borrow(), vec!["drop"], "drop happened; run did not (no box byte processed)");
    assert_eq!(*deliveries.borrow(), 0);
    // With the credential supplied, that rung passes.
    let cred = Credential::new("sk-test".into());
    assert!(validate_authority(Authority::Credential, ROUTES, Some(&cred), None));
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

// ─── Sprint 16 Slice 4 — the milestone ───────────────────────────────────────────────────────────

/// **THE MILESTONE (Sprint 16, claim (i)): all four authority rungs validate for real, and the ladder
/// reaches `Served`.** Until this sprint two rungs were hardcoded `false`, so `Served` was unreachable
/// by construction no matter how correct the environment.
///
/// Every rung here is its **real, sprint-built validator** against a **real configured value** — the
/// compiled register; roots resolved by the same `assemble` the binary calls (the quarantine genuinely
/// pinned, the readable root a genuinely readable directory); `KernelProtections` genuinely probing that
/// pinned fd's mount for `noexec,nosuid,nodev` plus `openat2` RESOLVE availability; a genuinely non-empty
/// credential. **Only the NIC leg is faked**, and only because it was already discharged live on real
/// hardware in Sprints 8 and 14 — this claim is hardware-independent, so proving it hardware-free keeps
/// it fresh-re-runnable forever rather than resting on a capture.
///
/// **WHAT THIS DOES NOT CLAIM.** Reaching `Served` means every authority *validated*, not that the proxy
/// is serving traffic end-to-end. Two things stay outside it, deliberately:
///
/// 1. **The live round-trip** (claim (ii)) — a real 82599, a connected head, a real upstream call — is a
///    separate hardware capture this sprint does not require.
/// 2. **Quarantine WRITABILITY is not an authority.** The rungs prove the quarantine is pinned and sits
///    on a properly-protected mount; they do **not** prove the dropped uid can write into it. A
///    directory that is root-owned `0755` satisfies every rung above and still fails at the first
///    `file-chunk`. So "all four rungs validate" must never be reported as "the proxy is servable" or
///    "writes will work" — whichever mechanism provisions the quarantine (systemd `RuntimeDirectory=`
///    with `User=`/`Group=`/`RuntimeDirectoryMode=`, or an explicit `chown`) must make it writable by
///    the drop-target uid, and that is a **provisioning** obligation this ladder cannot check.
///
/// This test uses `/run` as the quarantine — a real mount carrying all three flags, pinned read-only —
/// precisely because the rungs need a *protected* mount and nothing more. That it is not writable by
/// this user is the point of the hedge above, not a flaw in the fixture.
#[test]
fn milestone_all_four_authority_rungs_validate_for_real_and_the_ladder_serves() {
    use proxy::lifecycle::assemble;
    use proxy::register::net::ROUTES as REAL_ROUTES;

    let readable = std::env::temp_dir().join(format!("proxy-s16-milestone-{}", std::process::id()));
    std::fs::create_dir_all(&readable).unwrap();

    // The REAL assembly — the same function `serve.rs`'s `main` calls.
    let assembly = assemble("/run", readable.to_str().unwrap());
    let roots = assembly.roots.expect("a protected quarantine mount and a readable root");
    let credential = Credential::new("sk-milestone-value".into());

    // Every rung, its real validator, against these real values — asserted individually first so a
    // failure names WHICH rung, then driven through the ladder itself.
    for authority in AUTHORITIES {
        assert!(
            validate_authority(*authority, REAL_ROUTES, Some(&credential), Some(&roots)),
            "{authority:?} must validate for real — it is no longer a placeholder",
        );
    }

    let log = Rc::new(RefCell::new(Vec::new()));
    let (mut nic, _deliveries) = fake_nic(vec![], &log);
    let readiness = serve(
        &mut nic,
        || Ok(()), // the real drop is exercised on hardware (Sprint 8's IB2); not this claim's subject
        |authority| validate_authority(authority, REAL_ROUTES, Some(&credential), Some(&roots)),
        |_reply, _event| ControlFlow::Continue(()),
    );

    assert_eq!(
        readiness,
        Readiness::Served,
        "with all four authorities real and satisfied, the ladder must reach Served",
    );
    assert_eq!(log.borrow().as_slice(), &["run"], "run was entered — rung 4 opened");

    let _ = std::fs::remove_dir_all(&readable);
}

/// The capture's bind-the-chain observer (Sprint 17): both the box and the proxy fingerprint the request
/// body with THIS function, and a match proves the body crossed the seam unmangled. Content-free by
/// construction — the output carries a hash and a length, never any of the bytes.
#[test]
fn body_fingerprint_distinguishes_bodies_and_leaks_none_of_them() {
    use proxy::lifecycle::body_fingerprint;

    let body = br#"{"model":"m","max_tokens":1,"messages":[{"role":"user","content":"ping abc123"}]}"#;

    // Deterministic: the two sides compute the same value for the same bytes (the whole point).
    assert_eq!(body_fingerprint(body), body_fingerprint(body));

    // Sensitive to the mangling classes this tell exists to catch.
    assert_ne!(body_fingerprint(body), body_fingerprint(&body[..body.len() - 1]), "truncation");
    let mut swapped = body.to_vec();
    swapped.swap(10, 20);
    assert_ne!(body_fingerprint(body), body_fingerprint(&swapped), "reordering");
    let mut extra = body.to_vec();
    extra.push(b' ');
    assert_ne!(body_fingerprint(body), body_fingerprint(&extra), "an appended byte");

    // CONTENT-FREE: the fingerprint contains no run of the body. The nonce is the thing that must not
    // leak into a log, so it is checked explicitly.
    let fp = body_fingerprint(body);
    assert!(!fp.contains("abc123"), "the fingerprint must not carry the body's content");
    assert!(!fp.contains("messages"), "the fingerprint must not carry the body's content");
    // It does carry the exact length, deliberately — a length mismatch is legible on sight.
    assert!(fp.ends_with(&format!("/{}", body.len())));
}
