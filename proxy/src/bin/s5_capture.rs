//! Sprint 8 Slice 5 — the receive-path e2e capture on the real bench 82599 (bin-sha-declared).
//!
//! ONE privileged run on elsenor produces all three reviewer-required proofs:
//!   IB2 (both halves) — the proxy observes its credential state BEFORE the drop (privileged) and AFTER
//!     (fully dropped incl. no_new_privs), plus a NEGATIVE test: a privileged op (`setuid(0)`) attempted
//!     from INSIDE the dropped `run()` and asserted to FAIL (EPERM) — the un-fakeable half.
//!   IB3 — the box drives a sustained sequence of requests (2 frames each) through MULTIPLE RX-ring
//!     wraps (>> depth 64); the device RX counters (RXDGPC/GPRC) confirm wire-level receipt through the
//!     wrap/re-arm path.
//!   Receive-path e2e — a box-originated `open(net_address) -> sid` then `send(sid, net_payload)` (the
//!     ruled two-call, ONE-session sequence) reaches a recognizer ADMIT verdict; each net-payload body
//!     carries a fresh random NONCE which the proxy observes inside the admitted body — the un-fakeable
//!     content tell, cross-checked against the box's sent nonces.
//!
//! RECEIVE-PATH ONLY. This capture does NOT prove a response reached the box or that any upstream
//! (Anthropic) forward happened — the proxy is receive-only this sprint (R-c).
//!
//! SAFETY: the ONLY devices touched are the two BENCH cards by EXACT BDF. NEVER device-id enumeration —
//! the production NIC at 0000:61:00.0 is ALSO a 0x10fb and would match. Two floors: the construct
//! allowlist = [41:00.0, 63:00.0], and gate() independently refuses the production identity (0x7a12).
//!
//! Build+run: `cargo run --features bench-capture --bin s5_capture` (needs privilege + the bench cards).

use std::cell::RefCell;
use std::io;
use std::ops::ControlFlow;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use directpath::harness::{hex, read_nonce, two_facade};
use directpath::internals::datapath::{GPRC, GPTC, RXDGPC, TPR};
use directpath::{Bdf, Ctx, Event, SessionId, Subsystem};
use proto::{Message, NetAddress, NetPayload, Scheme, encode_message, scan_safe};
use proxy::dispatch::{Dispatcher, Routed};
use proxy::lifecycle::{Readiness, run_adapter, serve};
use proxy::nic::RealNic;
use proxy::privilege::{self, Credentials};
use proxy::register::anthropic::VETTED_TOOLS;
use proxy::register::net::ROUTES;

// ── SAFETY: exact bench BDFs only — never a device-id scan (61:00.0 production is also 0x10fb). ──────
const PROXY_BDF: &str = "0000:41:00.0"; // bench card A — the proxy role
const BOX_BDF: &str = "0000:63:00.0"; // bench card B — the box role
const PROXY_CORE: usize = 6; // match the proven nic RTT core pinning
const BOX_CORE: usize = 4;

// The unprivileged target for the drop (config hook — production sources this from the config DSL).
const DROP_UID: u32 = 65534; // nobody
const DROP_GID: u32 = 65534;

// 2 frames per request; > depth 64 × several → multiple RX-ring wraps (IB3).
const N_REQUESTS: usize = 200;
const STALL: Duration = Duration::from_secs(8);

/// What the proxy role returns for the capture verdict.
struct ProxyResult {
    before: Credentials,          // privileged, pre-drop
    after: Option<Credentials>,   // post-drop (should be fully dropped)
    neg_errno: Option<i32>,       // setuid(0) errno from inside run() — should be EPERM
    admitted_bodies: Vec<String>, // each admitted net-payload body (carries a nonce)
    rxdgpc: u32,                  // device: DMA-good Rx packets posted to host
    gprc: u32,                    // device: good Rx packets (L2-filtered)
    gptc: u32,                    // device: good Tx packets (the proxy's ACKs)
    // Diagnostics: what dispatch verdicts the proxy actually saw.
    n_route_admit: usize,
    n_payload_admit: usize,
    n_payload_deny: usize,
    n_no_route: usize,
    n_other: usize,
    stalled: bool,
    readiness: Readiness,
}

fn net_address_l2(joined: &str) -> Vec<u8> {
    encode_message(&Message::NetAddress(NetAddress {
        scheme: Scheme::Https,
        address: scan_safe(joined.replace('/', "\u{1f}").as_bytes()).unwrap(),
    }))
}
fn net_payload_l2(content: &[u8]) -> Vec<u8> {
    encode_message(&Message::NetPayload(NetPayload { more: false, content: content.to_vec() }))
}
/// A valid Anthropic body (admits) embedding the nonce hex in the inert `content` field.
fn anthropic_body_with_nonce(nonce_hex: &str) -> Vec<u8> {
    format!(r#"{{"model":"claude-x","max_tokens":100,"messages":[{{"role":"user","content":"{nonce_hex}"}}]}}"#)
        .into_bytes()
}

fn main() -> io::Result<()> {
    println!("{}", benchtools::Provenance::capture().header("proxy_s5_capture"));
    println!("bench: proxy={PROXY_BDF}  box={BOX_BDF}  (exact BDFs; production 61:00.0 NEVER touched)\n");

    let proxy_bdf = Bdf::parse(PROXY_BDF).expect("valid bench proxy BDF");
    let box_bdf = Bdf::parse(BOX_BDF).expect("valid bench box BDF");
    let allow: [Bdf; 2] = [proxy_bdf, box_bdf];

    // Barrier: the box signals AFTER its construct so the proxy's process-wide privilege drop cannot
    // strip the box's still-in-progress privileged construct (both constructs complete pre-drop).
    let (box_ready_tx, box_ready_rx) = mpsc::channel::<()>();

    let (proxy_r, box_nonces) = two_facade(
        proxy_bdf,
        box_bdf,
        &allow,
        PROXY_CORE,
        BOX_CORE,
        // Idle-park cap (Task 20): None (uncapped) — this receive-only capture is BOX-DRIVEN (the box's
        // frames wake the reactor on NIC events), with no quiescent-arrival dependency, so the prior
        // uncapped behavior is preserved exactly. The composed production loop (bring_up) is the one that
        // needs Some(cap) for a quiescent wake.
        None,
        // ── PROXY role (41): bring_up's body — observe(before) → drop+observe(after)+negative test →
        //    serve(run_adapter) driving dispatch; capture verdicts + RX counters. ──
        move |sys: Subsystem| -> io::Result<ProxyResult> {
            box_ready_rx.recv().map_err(|_| io::Error::other("box-ready barrier dropped"))?;
            let before = privilege::observe(); // still privileged (root), pre-drop

            let mut nic = RealNic(sys);
            let after_creds: Rc<RefCell<Option<Credentials>>> = Rc::new(RefCell::new(None));
            let admitted: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
            let counts: Rc<RefCell<[usize; 5]>> = Rc::new(RefCell::new([0; 5])); // route/pay-admit/pay-deny/no-route/other
            let neg_errno: Rc<RefCell<Option<i32>>> = Rc::new(RefCell::new(None));
            let neg_done: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));

            let drop_after = after_creds.clone();
            let sink = admitted.clone();
            let cnt = counts.clone();
            let neg_r = neg_errno.clone();
            let neg_d = neg_done.clone();

            let readiness = serve(
                &mut nic,
                // The real drop, with the IB2 read-back self-check folded in (fail closed if incomplete).
                move || -> io::Result<()> {
                    if let Err(e) = privilege::drop_to(DROP_UID, DROP_GID) {
                        eprintln!("[proxy] drop_to FAILED: {e}");
                        return Err(e);
                    }
                    let after = privilege::observe();
                    *drop_after.borrow_mut() = Some(after);
                    if after.is_fully_dropped_to(DROP_UID, DROP_GID) {
                        Ok(())
                    } else {
                        Err(io::Error::other("privilege drop incomplete"))
                    }
                },
                // e2e validator: the bench run is a controlled valid environment; the RECEIVE PATH is
                // what is under test here (the config-pending authorities are fresh-tested in Slice 3).
                |_authority| true,
                // The production run_adapter (Slice 4): Event::Delivered -> Dispatcher::route; Idle ->
                // continue. The observer records verdicts + the un-fakeable nonce tell, runs the IB2
                // negative test once (post-drop, inside run()), and breaks after all N net-payloads are
                // recognized (the e2e stop pattern). A box stall is guarded by the outer run timeout.
                run_adapter(Dispatcher::new(), ROUTES, VETTED_TOOLS, move |_session, routed| {
                    // IB2 negative half — ONCE, inside run() (post-drop): a privileged op must FAIL.
                    if !*neg_d.borrow() {
                        *neg_d.borrow_mut() = true;
                        // SAFETY: setuid is a plain syscall with an integer arg; we only read its result.
                        let r = unsafe { libc::setuid(0) };
                        *neg_r.borrow_mut() =
                            Some(if r == -1 { io::Error::last_os_error().raw_os_error().unwrap_or(-1) } else { 0 });
                    }
                    match routed {
                        Routed::NetRouteAdmit(_) => cnt.borrow_mut()[0] += 1,
                        Routed::NetPayloadAdmit(body) => {
                            cnt.borrow_mut()[1] += 1;
                            if let Ok(s) = std::str::from_utf8(body) {
                                sink.borrow_mut().push(s.to_string());
                            }
                        }
                        Routed::NetPayloadDeny(_) => cnt.borrow_mut()[2] += 1,
                        Routed::NetPayloadNoRoute => cnt.borrow_mut()[3] += 1,
                        _ => cnt.borrow_mut()[4] += 1,
                    }
                    if cnt.borrow()[1] >= N_REQUESTS {
                        ControlFlow::Break(()) // proof complete
                    } else {
                        ControlFlow::Continue(())
                    }
                }),
            );

            // Post-run device counters (the un-fakeable device-level tell; sys retained — run took &mut).
            let rxdgpc = nic.0.counter(RXDGPC);
            let gprc = nic.0.counter(GPRC);
            let gptc = nic.0.counter(GPTC);
            let c = *counts.borrow();

            eprintln!(
                "[proxy diag] readiness={readiness:?} | verdicts: route_admit={} pay_admit={} pay_deny={} no_route={} other={} | RXDGPC={rxdgpc} GPRC={gprc} GPTC(tx-ack)={gptc} | before_uid={:?} after={:?} neg_errno={:?}",
                c[0], c[1], c[2], c[3], c[4], before.uid, *after_creds.borrow(), *neg_errno.borrow(),
            );

            Ok(ProxyResult {
                before,
                after: *after_creds.borrow(),
                neg_errno: *neg_errno.borrow(),
                admitted_bodies: admitted.borrow().clone(),
                rxdgpc,
                gprc,
                gptc,
                n_route_admit: c[0],
                n_payload_admit: c[1],
                n_payload_deny: c[2],
                n_no_route: c[3],
                n_other: c[4],
                stalled: false, // run_adapter has no Idle stall-guard; a stall is caught by the outer run timeout
                readiness,
            })
        },
        // ── BOX role (63): per request, open(net_address) -> sid, then send(sid, net_payload(nonce)) on
        //    the SAME session, paced stop-and-wait on quiescence. Returns the sent nonces. ──
        move |mut sys: Subsystem| -> io::Result<Vec<String>> {
            box_ready_tx.send(()).map_err(|_| io::Error::other("box-ready send failed"))?;

            // Pre-build the N requests: each a fresh random nonce in a valid Anthropic body.
            let mut nonces: Vec<String> = Vec::with_capacity(N_REQUESTS);
            let mut payloads: Vec<Vec<u8>> = Vec::with_capacity(N_REQUESTS);
            for _ in 0..N_REQUESTS {
                let h = hex(&read_nonce()?);
                payloads.push(net_payload_l2(&anthropic_body_with_nonce(&h)));
                nonces.push(h);
            }
            let addr = net_address_l2("api.anthropic.com/v1/messages");

            // Open request 0's address; the run loop paces the rest on quiescence (ACK-arrived).
            let mut sid: SessionId = sys.open(&addr);
            let mut idx = 0usize;
            let mut sent_payload = false; // false = address sent (awaiting ACK), true = payload sent
            let mut last_progress = Instant::now();
            let mut stalled = false;
            let mut send_err: Option<String> = None;

            sys.run(|ctx: &mut Ctx, ev: Event| match ev {
                // The box only ever receives (hidden) ACKs; a delivery would mean the proxy originated.
                Event::Delivered(..) => ControlFlow::Continue(()),
                Event::Idle => {
                    if !ctx.is_quiescent() {
                        if last_progress.elapsed() > STALL {
                            stalled = true;
                            return ControlFlow::Break(());
                        }
                        return ControlFlow::Continue(()); // current frame still in flight
                    }
                    last_progress = Instant::now();
                    if !sent_payload {
                        // Address ACKed → continue the SAME session with the net-payload.
                        match ctx.send(sid, &payloads[idx]) {
                            Ok(()) => {
                                sent_payload = true;
                                ControlFlow::Continue(())
                            }
                            Err(e) => {
                                send_err = Some(format!("{e:?}"));
                                ControlFlow::Break(())
                            }
                        }
                    } else {
                        // Payload ACKed → request done; next request or stop.
                        idx += 1;
                        if idx >= N_REQUESTS {
                            ControlFlow::Break(())
                        } else {
                            sid = ctx.open(&addr);
                            sent_payload = false;
                            ControlFlow::Continue(())
                        }
                    }
                }
            });

            // Diagnostics (stderr): the box's device counters — did it TX and get ACKs back?
            eprintln!(
                "[box diag] idx={idx}/{N_REQUESTS} sent_payload={sent_payload} sid={sid:?} stalled={stalled} send_err={send_err:?} | GPTC(tx)={} GPRC(rx)={} TPR(total-rx)={}",
                sys.counter(GPTC),
                sys.counter(GPRC),
                sys.counter(TPR),
            );

            if let Some(e) = send_err {
                return Err(io::Error::other(format!("box send(sid,...) failed at request {idx}: {e}")));
            }
            if stalled {
                return Err(io::Error::other(format!("box STALLED at request {idx}/{N_REQUESTS} (no ACK)")));
            }
            Ok(nonces)
        },
    )?;

    // ── Verify + print the three proofs ──────────────────────────────────────────────────────────
    let mut all_ok = true;
    let mut check = |name: &str, ok: bool, detail: String| {
        all_ok &= ok;
        println!("  [{}] {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    };

    println!(
        "proxy dispatch verdicts: route_admit={} pay_admit={} pay_deny={} no_route={} other={} | tx-ack(GPTC)={} | stalled={}\n",
        proxy_r.n_route_admit, proxy_r.n_payload_admit, proxy_r.n_payload_deny, proxy_r.n_no_route, proxy_r.n_other, proxy_r.gptc, proxy_r.stalled,
    );

    println!("IB2 — privilege drop (both halves):");
    let before_root = proxy_r.before.uid == (0, 0, 0);
    check("before-drop privileged", before_root, format!("uid={:?} gid={:?}", proxy_r.before.uid, proxy_r.before.gid));
    let dropped = proxy_r.after.map(|a| a.is_fully_dropped_to(DROP_UID, DROP_GID)).unwrap_or(false);
    check(
        "after-drop fully dropped (incl no_new_privs)",
        dropped,
        proxy_r.after.map(|a| format!("{a:?}")).unwrap_or_else(|| "none".into()),
    );
    let neg_ok = proxy_r.neg_errno == Some(libc::EPERM);
    check(
        "negative test: setuid(0) from inside run() FAILS",
        neg_ok,
        format!("errno={:?} (want EPERM={})", proxy_r.neg_errno, libc::EPERM),
    );

    println!("IB3 — sustained frames through RX-ring wraps:");
    let frames = (N_REQUESTS * 2) as u32; // net-address + net-payload per request
    let wraps = frames / directpath::internals::datapath::DEPTH as u32;
    check(
        "device RX counter confirms wire-level receipt through wraps",
        proxy_r.rxdgpc >= frames && wraps >= 2,
        format!("RXDGPC={} GPRC={} (>= {frames} frames, ~{wraps} ring wraps @ depth {})", proxy_r.rxdgpc, proxy_r.gprc, directpath::internals::datapath::DEPTH),
    );

    println!("Receive-path e2e (RECEIVE ONLY — no response, no upstream forward):");
    check("readiness served", proxy_r.readiness == Readiness::Served, format!("{:?}", proxy_r.readiness));
    let admitted_n = proxy_r.admitted_bodies.len();
    check("all N net-payloads recognized+admitted under their session route", admitted_n == N_REQUESTS, format!("{admitted_n}/{N_REQUESTS} admitted"));
    // The un-fakeable content tell: every box nonce appears in an admitted body (recognizer vetted the
    // exact random nonce bytes the box sent, on the correlated session).
    let matched = box_nonces.iter().filter(|n| proxy_r.admitted_bodies.iter().any(|b| b.contains(*n))).count();
    check(
        "every box nonce observed inside an admitted body (correlation tell)",
        matched == N_REQUESTS,
        format!("{matched}/{N_REQUESTS} nonces matched"),
    );

    println!("\n{}", if all_ok { "CAPTURE PASS — all three proofs hold." } else { "CAPTURE FAIL — see FAIL lines above." });
    if all_ok { Ok(()) } else { Err(io::Error::other("capture had failing proofs")) }
}
