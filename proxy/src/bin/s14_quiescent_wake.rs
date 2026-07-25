//! Sprint 14 — the live quiescent-wake liveness proof (Task 20 discharge), bin-sha self-declared.
//!
//! Proves, on the real bench 82599, that the reactor's EXISTING (unmodified, `idle_cap = None`)
//! quiescent-wake behavior already delivers a quiescent arrival to the box within its documented
//! ~5–10ms bound — a POSITIVE demonstration of a guarantee that was always there (the `DdWaitHost`
//! `~5ms` hang-safety re-check), not a proof that a fix closed a gap. Sprints 11–13 never verified this
//! live (every test drove `Idle` synthetically).
//!
//! THE VEHICLE — a head keystroke, injected OFF-NIC. The key to a causal proof WITHOUT a negative
//! control: the arrival is a write to the head SOCKET (never a NIC frame), so no MSI-X / DD-store wake
//! can exist for it. During proven box-silence the ~5ms `DdWaitHost` re-check is the ONLY thing that can
//! drain it. So proven-quiescence + off-NIC-arrival + ms-scale-timing together pin the delivery to the
//! periodic re-check — no before/after contrast needed.
//!
//! THREE-PART TELL: (1) every injected nonce is observed intact at the box; (2) PROVEN QUIESCENCE — zero
//! `Delivered` at the proxy in the window (only the pre-window setup + post-window teardown) AND
//! `next_deadline == None`: the proxy originates nothing reliable before a keystroke, so no retransmit
//! deadline is armed — established by construction and asserted empirically by (3); (3) THE MAX IS
//! LOAD-BEARING — N injections, the delivery-latency MAX stays bounded by the ~5ms hang-safety ceiling,
//! BELOW the ~10ms retransmit interval: that is what pins the mechanism to the ~5ms re-check (hence
//! `next_deadline == None`) rather than a ~10ms capped retransmit. The MEDIAN near mid-band (~ceiling/2)
//! confirms phase-random landing across the re-check cycle; zero-sub-us is structurally guaranteed by the
//! off-NIC vehicle (a socket write has no DD-store wake) and checked as corroboration.
//!
//! (Removing this team's counterproductive `idle_cap = 10ms` is a genuine 10ms→~5ms latency improvement,
//! not "identical behavior": the app cap was ABOVE the native ~5ms ceiling, so it only ever raised the
//! worst-case wake — dropping it restores the ~5ms the reactor already provided.)
//!
//! SAFETY: the ONLY devices touched are the two BENCH cards by EXACT BDF. NEVER device-id enumeration —
//! the production NIC at 0000:61:00.0 is ALSO a 0x10fb and would match. Two floors: the construct
//! allowlist = [41:00.0, 63:00.0], and gate() independently refuses the production identity (0x7a12).
//!
//! Build+run: `cargo run --features bench-capture --bin s14_quiescent_wake` (needs privilege + the bench
//! cards). The live privileged run is the held, capture-model step (operator paste-and-confirm first).

use std::io::{self, Write};
use std::ops::ControlFlow;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use directpath::harness::{hex, read_nonce, two_facade};
use directpath::{Bdf, Ctx, Event, SessionId, Subsystem};
use proxy::dispatch::Dispatcher;
use proxy::head::{HeadConnection, HeadFrame, decode_head_frame, encode_head_frame, run_adapter_composed};
use proxy::nic::{NicRun, RealNic};
use proxy::register::anthropic::VETTED_TOOLS;
use proxy::register::net::ROUTES;
use proxy::upstream::{UpstreamFrame, UpstreamHandle, UpstreamSpawn};

// ── SAFETY: exact bench BDFs only — never a device-id scan (61:00.0 production is also 0x10fb). ──────
const PROXY_BDF: &str = "0000:41:00.0"; // bench card A — the proxy role (Endpoint::Proxy, high half)
const BOX_BDF: &str = "0000:63:00.0"; // bench card B — the box role (Endpoint::Box, low half)
const PROXY_CORE: usize = 6; // match the proven nic RTT core pinning
const BOX_CORE: usize = 4;

const N_INJECTS: usize = 20; // enough injections to show the latency DISTRIBUTION, not one sample
const INJECT_INTERVAL: Duration = Duration::from_millis(50); // >> the ~5ms ceiling → each delivers alone
// The delivery-latency MAX must stay below this to distinguish the ~5ms hang-safety ceiling
// (`next_deadline == None`) from the ~10ms retransmit interval — the load-bearing bound. Provisional,
// marked for review; the capture reveals the true ~5ms ceiling and this may tighten toward it.
const CEILING_BOUND: Duration = Duration::from_millis(8);
const SETUP_TIMEOUT: Duration = Duration::from_secs(10); // wait for the relayed setup display
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30); // box give-up guard (a real stall must not hang)

/// A raw terminal L2 frame: `[verb u32 LE][opaque body]` — the wire shape the dispatcher decodes for a
/// terminal verb (display/key), matching the composed-adapter routing tests.
fn raw_frame(verb: u32, body: &[u8]) -> Vec<u8> {
    let mut v = verb.to_le_bytes().to_vec();
    v.extend_from_slice(body);
    v
}

/// The proxy-role result: how many `Delivered` events the proxy saw across the whole run — MUST be
/// exactly 2 (the pre-window setup display + the post-window teardown), proving the window was silent.
struct ProxyResult {
    delivered: usize,
}

/// The box-role result: each received keystroke's raw payload + the instant it arrived, for nonce +
/// latency verification.
struct BoxResult {
    deliveries: Vec<(Vec<u8>, Instant)>,
}

/// A no-op upstream spawner — the head-keystroke vehicle never admits a net-payload, so this is never
/// called; it returns an immediately-closed handle defensively.
struct NoopSpawn;
impl UpstreamSpawn for NoopSpawn {
    fn spawn(&self, _session: SessionId, _destination: proxy::vocab::Route, _body: Vec<u8>) -> UpstreamHandle {
        let (_tx, rx) = std::sync::mpsc::sync_channel::<UpstreamFrame>(1); // _tx dropped → rx closed (EOF)
        UpstreamHandle { rx, join: None }
    }
}

fn main() -> io::Result<()> {
    println!("{}", benchtools::Provenance::capture().header("proxy_s14_quiescent_wake"));
    println!("bench: proxy={PROXY_BDF}  box={BOX_BDF}  (exact BDFs; production 61:00.0 NEVER touched)\n");

    let proxy_bdf = Bdf::parse(PROXY_BDF).expect("valid bench proxy BDF");
    let box_bdf = Bdf::parse(BOX_BDF).expect("valid bench box BDF");
    let allow: [Bdf; 2] = [proxy_bdf, box_bdf];

    // The head socketpair: the proxy owns `worker_sock` (its HeadConnection); the operator owns
    // `fake_head` (reads the relayed setup display to learn s_display, then injects nonce'd keystrokes).
    let (worker_sock, fake_head) = UnixStream::pair()?;

    // Pre-generate N nonces (hex strings), injected one per keystroke — the un-fakeable content tell.
    let mut nonces: Vec<String> = Vec::with_capacity(N_INJECTS);
    for _ in 0..N_INJECTS {
        nonces.push(hex(&read_nonce()?));
    }

    // `stop` lets the box tell the proxy to leave its production loop (the proxy's on_verdict breaks on
    // the first Delivered seen with stop set — that Delivered is the box's teardown frame).
    let stop = Arc::new(AtomicBool::new(false));

    // ── The OPERATOR (a 3rd actor, concurrent with two_facade's box+proxy threads): read the relayed
    //    setup display off the fake head to learn s_display, then inject the nonce'd keystrokes, paced so
    //    each is delivered before the next. Returns each (nonce, inject-instant). ──
    let op_nonces = nonces.clone();
    let operator = thread::spawn(move || -> io::Result<Vec<(String, Instant)>> {
        let mut fh = fake_head;
        fh.set_read_timeout(Some(SETUP_TIMEOUT))?;
        // The proxy relays the box's setup display to the head as a HeadFrame; its session IS s_display.
        let setup = decode_head_frame(&mut fh)?;
        let s_display = setup.session;

        let mut injected = Vec::with_capacity(op_nonces.len());
        for nonce in &op_nonces {
            let keystroke = raw_frame(proto::verb::KEY, nonce.as_bytes());
            let frame = HeadFrame { session: s_display, payload: keystroke };
            let bytes = encode_head_frame(&frame).expect("keystroke frame in bounds");
            let at = Instant::now();
            fh.write_all(&bytes)?;
            injected.push((nonce.clone(), at));
            thread::sleep(INJECT_INTERVAL); // > the ceiling: one quiescent arrival at a time
        }
        Ok(injected)
    });

    // ── two_facade drives the two roles on real hardware. CRITICAL mapping: card `a` is constructed
    //    Endpoint::Box (low half) and card `b` Endpoint::Proxy (high half), so a=BOX, b=PROXY — the
    //    production originator-partition (box=low, proxy=high). (Unlike s5_capture, both sides originate
    //    here, so the halves MUST match production, not be inverted.) `idle_cap = None` throughout: the
    //    unmodified production behavior is exactly what is under proof. ──
    let stop_box = stop.clone();
    let stop_proxy = stop.clone();
    let (box_res, proxy_res) = two_facade(
        box_bdf,
        proxy_bdf,
        &allow,
        BOX_CORE,
        PROXY_CORE,
        None, // idle_cap — the unmodified production value (the whole point)
        // ── BOX role (Endpoint::Box, low half): send the setup display (marks s_display active at the
        //    proxy + relays it to the fake head), go quiescent, then RECEIVE the proxy-originated
        //    keystroke deliveries; on the last, wake the proxy for teardown and stop. ──
        move |mut sys: Subsystem| -> io::Result<BoxResult> {
            let _s_display = sys.open(&raw_frame(proto::verb::DISPLAY, b"pty-setup"));
            let mut deliveries: Vec<(Vec<u8>, Instant)> = Vec::new();
            let start = Instant::now();
            sys.run(|ctx: &mut Ctx, ev: Event| match ev {
                // A delivery here is a proxy-originated keystroke (the box only ever ACKs otherwise).
                Event::Delivered(_sess, payload) => {
                    deliveries.push((payload.to_vec(), Instant::now()));
                    if deliveries.len() >= N_INJECTS {
                        stop_box.store(true, Ordering::SeqCst);
                        let _ = ctx.open(&raw_frame(proto::verb::DISPLAY, b"teardown")); // wake the proxy
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                }
                Event::Idle => {
                    if start.elapsed() > TOTAL_TIMEOUT {
                        stop_box.store(true, Ordering::SeqCst);
                        let _ = ctx.open(&raw_frame(proto::verb::DISPLAY, b"teardown"));
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                }
            });
            Ok(BoxResult { deliveries })
        },
        // ── PROXY role (Endpoint::Proxy, high half): drive the REAL production path — RealNic::run over
        //    the composed adapter (head + a no-op upstream). Count Delivered (must be exactly 2: setup +
        //    teardown); break when the box sets `stop` (observed on the teardown Delivered). ──
        move |sys: Subsystem| -> io::Result<ProxyResult> {
            let head = HeadConnection::spawn(worker_sock)?;
            let delivered = Arc::new(AtomicUsize::new(0));
            let count = delivered.clone();
            let mut nic = RealNic(sys);
            let adapter = run_adapter_composed(
                Dispatcher::new(),
                ROUTES,
                VETTED_TOOLS,
                NoopSpawn,
                head,
                move |_session: SessionId, _routed| {
                    count.fetch_add(1, Ordering::SeqCst);
                    if stop_proxy.load(Ordering::SeqCst) {
                        ControlFlow::Break(()) // the teardown Delivered arrived with stop set → exit run()
                    } else {
                        ControlFlow::Continue(())
                    }
                },
            );
            nic.run(adapter);
            Ok(ProxyResult { delivered: delivered.load(Ordering::SeqCst) })
        },
    )?;

    let injected = operator.join().map_err(|_| io::Error::other("operator thread panicked"))??;

    // ── Verify the three-part tell ──────────────────────────────────────────────────────────────────
    let mut all_ok = true;
    let mut check = |name: &str, ok: bool, detail: String| {
        all_ok &= ok;
        println!("  [{}] {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    };

    // (1) NONCE: every injected nonce is observed intact in some delivered keystroke; pair each delivery
    //     to its nonce for the latency measurement.
    let mut latencies: Vec<Duration> = Vec::new();
    let mut matched = 0usize;
    for (nonce, t_inject) in &injected {
        let needle = nonce.as_bytes();
        if let Some((_, t_deliver)) =
            box_res.deliveries.iter().find(|(payload, _)| contains(payload, needle))
        {
            matched += 1;
            latencies.push(t_deliver.saturating_duration_since(*t_inject));
        }
    }
    println!("Quiescent-wake liveness (production path, idle_cap=None, unmodified reactor):");
    check(
        "every injected nonce observed intact at the box",
        matched == N_INJECTS,
        format!("{matched}/{N_INJECTS} nonces delivered"),
    );

    // (2) PROVEN QUIESCENCE (zero-Delivered + next_deadline=None): the proxy saw exactly 2 Delivered
    //     across the run — the pre-window setup display and the post-window teardown — so ZERO box frames
    //     arrived in the observation window (no coincidental NicMsix wake; the keystrokes are off-NIC).
    //     `next_deadline == None` holds by construction (the proxy originates nothing reliable before a
    //     keystroke → no retransmit deadline is armed) and is asserted empirically by tell (3)'s ~5ms
    //     bound (a ~10ms max would betray a pending retransmit deadline instead).
    check(
        "proxy window silent — zero Delivered besides setup + teardown",
        proxy_res.delivered == 2,
        format!("proxy saw {} Delivered (want 2: setup + teardown)", proxy_res.delivered),
    );

    // (3) THE MAX IS LOAD-BEARING — the ~5ms distinction. The delivery-latency MAX must stay bounded by
    //     the ~5ms hang-safety ceiling (below the ~10ms retransmit interval): that is the empirical
    //     assertion that next_deadline was None (the mechanism is the ~5ms re-check, not a ~10ms capped
    //     retransmit). The MEDIAN near mid-band (~ceiling/2) confirms phase-random landing across the
    //     re-check cycle. Zero-sub-us is structurally guaranteed by the off-NIC vehicle — checked as
    //     corroboration, never the primary tell.
    if latencies.is_empty() {
        check("delivery MAX bounded by the ~5ms ceiling", false, "no deliveries to measure".into());
    } else {
        let mut sorted = latencies.clone();
        sorted.sort_unstable();
        let median = sorted[sorted.len() / 2];
        let max = *sorted.last().unwrap();
        let min = *sorted.first().unwrap();
        let bounded_by_ceiling = max <= CEILING_BOUND; // LOAD-BEARING: ~5ms (None), not ~10ms (retransmit)
        let never_sub_us = min >= Duration::from_micros(1); // structural (off-NIC); corroboration only
        check(
            "delivery MAX bounded by the ~5ms ceiling (next_deadline=None, not the ~10ms retransmit)",
            bounded_by_ceiling && never_sub_us,
            format!(
                "min={min:?} median={median:?} (~mid-band ⇒ phase-random) max={max:?} bound={CEILING_BOUND:?} over {} samples",
                latencies.len(),
            ),
        );
    }

    println!(
        "\n{}",
        if all_ok {
            "CAPTURE PASS — the unmodified reactor delivers a quiescent arrival within its ~5ms ceiling."
        } else {
            "CAPTURE FAIL — see FAIL lines above."
        }
    );
    if all_ok { Ok(()) } else { Err(io::Error::other("capture had failing proofs")) }
}

/// `true` if `haystack` contains `needle` as a contiguous subslice (the nonce ascii inside a delivered
/// keystroke payload — `[8B s_display][KEY verb][nonce]`).
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|w| w == needle)
}
