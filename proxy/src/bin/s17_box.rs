//! Sprint 17 Slice 2 — `s17_box`: the scripted BOX for the live end-to-end capture, bin-sha
//! self-declared.
//!
//! A **real, separate process** on the second bench card driving `Endpoint::Box` directly — deliberately
//! NOT a `two_facade` in-process role. `two_facade` hands a role closure an already-constructed
//! `Subsystem`, but the real proxy's `bring_up` constructs its own, so carrying the shipped composition
//! path inside `two_facade` would require hand-rolling `bring_up`'s body — a replica of the very thing
//! the capture is meant to exercise. Two processes is also what production *is*: the box is untrusted and
//! separate, not a thread sharing the proxy's address space.
//!
//! WHAT IT DRIVES, in phases, each paced stop-and-wait on quiescence:
//!   A. `open(net-address)` → the proxy's route gate admits the Anthropic destination.
//!   B. `send(sid, net-payload)` on the SAME session (the ruled two-call, one-session sequence) carrying
//!      a minimal Anthropic request with **nonce #1** in its content.
//!   C. Await the correlated response — the proxy relays the real upstream bytes back on a
//!      proxy-originated session, each frame stamped `in_reply_to` this box's own request session.
//!   D. Emit one `display` frame toward the head carrying a marker **derived from that response**
//!      (Anthropic's own `id`), which the proxy relays box→head over the real socket.
//!   E. `open(file-write)` → `send(file-chunk)` carrying **nonce #2**, which the proxy writes into the
//!      provisioned quarantine as the dropped uid.
//!
//! SCOPE HONESTY. The `display` payload is an **opaque marker**, not a rendered cell stream: the proxy
//! relays terminal frames without decoding them (`proxy-head.md` §1), so this proves the box→head
//! CHANNEL carries a frame traceable to the real upstream response — it does not exercise display
//! semantics, and the head client is not a renderer. Likewise the upstream call is a *vehicle* for the
//! round trip, never a re-test of the Anthropic client (Sprint 11 closed that).
//!
//! SAFETY: the ONLY device touched is the bench BOX card by EXACT BDF (`Bdf::parse` literal). NEVER
//! device-id enumeration — the production NIC at 0000:61:00.0 is ALSO a 0x10fb and would match a scan.
//!
//! Build+run (bench only, needs privilege): `cargo run --features bench-capture --bin s17_box`.

use std::io;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use directpath::harness::{hex, read_nonce};
use directpath::{Bdf, Ctx, Endpoint, Event, SessionId, Subsystem};
use proto::{
    FileWrite, Message, NetAddress, NetPayload, Scheme, decode_message, encode_message, scan_safe,
};

// ── SAFETY: exact bench BDF only — never a device-id scan (61:00.0 production is also 0x10fb). ──────
const BOX_BDF: &str = "0000:63:00.0"; // the bench BOX card — the ONLY device this binary touches
/// The PROXY card's MAC (`0000:41:00.0`), read from sysfs while ixgbe-bound — the mirror of the box MAC
/// `serve.rs` carries. Two separate processes cannot exchange MACs the way `two_facade` does in-process,
/// so each side is configured with the other's.
const PEER_PROXY_MAC: [u8; 6] = [0x90, 0xe2, 0xba, 0xc7, 0x69, 0xa0];

/// The route the proxy's register admits (`register::net::ROUTES`).
const NET_DESTINATION: &str = "api.anthropic.com/v1/messages";
/// The quarantine-relative destination for the write leg — a safe single segment.
const WRITE_DESTINATION: &str = "s17-capture.bin";

/// The upstream request: **minimal and cheap by construction** — one message, `max_tokens: 1`, the
/// cheapest current model. `stop_reason: "max_tokens"` in the reply is a COMPLETE, VALID response, not a
/// failure. The body is **synthetic**: a nonce plus fixed filler, carrying no repo, host, or path
/// information — a real third party receives these bytes.
fn anthropic_body(nonce: &str) -> Vec<u8> {
    format!(
        r#"{{"model":"claude-haiku-4-5-20251001","max_tokens":1,"messages":[{{"role":"user","content":"ping {nonce}"}}]}}"#
    )
    .into_bytes()
}

fn net_address_l2(joined: &str) -> Vec<u8> {
    encode_message(&Message::NetAddress(NetAddress {
        scheme: Scheme::Https,
        address: scan_safe(joined.replace('/', "\u{1f}").as_bytes()).expect("a safe destination"),
    }))
}
fn net_payload_l2(content: &[u8]) -> Vec<u8> {
    encode_message(&Message::NetPayload(NetPayload { more: false, content: content.to_vec() }))
}
fn file_write_l2(joined: &str) -> Vec<u8> {
    encode_message(&Message::FileWrite(FileWrite(
        scan_safe(joined.replace('/', "\u{1f}").as_bytes()).expect("a safe destination"),
    )))
}
/// A raw L2 frame `[verb u32 LE][opaque body]` — for the verbs the proxy relays without decoding.
fn raw_frame(verb: u32, body: &[u8]) -> Vec<u8> {
    let mut f = verb.to_le_bytes().to_vec();
    f.extend_from_slice(body);
    f
}

/// What a proxy→box delivery carried. Reply frames are `[8B in_reply_to][proto message]` (the Task-16
/// correlation stub), so the stamp is read before the body is decoded.
enum Reply {
    Response { in_reply_to: u64, bytes: Vec<u8> },
    Error { in_reply_to: u64, code: proto::ErrorCode },
    Undecodable,
}

fn classify(payload: &[u8]) -> Reply {
    if payload.len() < 8 {
        return Reply::Undecodable;
    }
    let in_reply_to = u64::from_be_bytes(payload[0..8].try_into().expect("len >= 8"));
    match decode_message(&payload[8..]) {
        Ok(Message::NetResponse(nr)) => Reply::Response { in_reply_to, bytes: nr.0 },
        Ok(Message::NetError(ne)) => Reply::Error { in_reply_to, code: ne.0.code },
        _ => Reply::Undecodable,
    }
}

/// Where the raw response body is written for the capture script to assert over (argv[1], with a
/// default). **The body is never printed by this binary**: an error body can in principle echo request
/// material, so it is recorded to a file and the script performs its checks BEFORE anything renders it.
fn response_body_path() -> String {
    std::env::args().nth(1).unwrap_or_else(|| "/tmp/s17-response-body.bin".to_string())
}

/// Locate the upstream's `request-id` header value in the RAW response bytes, case-insensitively.
///
/// **This is the box interpreting what it received, which it is entitled to do.** Only the *proxy* must
/// relay opaquely (`cloud/approach.md` §5); the box is the endpoint the response was sent to. Nothing
/// here is an HTTP parser — it searches for one byte pattern within the header block and takes the rest
/// of that line. Reachable at all only because Sprint 18 made the full response, headers included, arrive
/// at the box; under the superseded body-only relay there was nothing to look in.
fn upstream_request_id(raw: &[u8]) -> Option<Vec<u8>> {
    let head_end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(raw.len());
    let lowered = raw[..head_end].to_ascii_lowercase();
    const PAT: &[u8] = b"\r\nrequest-id:";
    let start = lowered.windows(PAT.len()).position(|w| w == PAT)? + PAT.len();
    let rest = &raw[start..head_end];
    let line_end = rest.windows(2).position(|w| w == b"\r\n").unwrap_or(rest.len());
    let value: Vec<u8> = rest[..line_end].to_vec();
    let trimmed = value.iter().position(|b| !b.is_ascii_whitespace()).map(|i| &value[i..])?;
    let end = trimmed.iter().rposition(|b| !b.is_ascii_whitespace()).map_or(0, |i| i + 1);
    Some(trimmed[..end].to_vec())
}

/// Is a located `request-id` safe to print verbatim in the capture output?
///
/// The same discipline the response body is held to: a value of unexpected shape is REPORTED but never
/// rendered, because response bytes can in principle echo request material and this binary must not be
/// the thing that puts such material on a terminal. A real Anthropic request id is a bounded
/// alphanumeric token, so anything else is treated as "present but not trustworthy to display".
fn is_safe_request_id(v: &[u8]) -> bool {
    !v.is_empty()
        && v.len() <= 128
        && v.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
}

/// The display marker for the **genuine-transport-failure** branch only — the case where no upstream
/// response reached the box at all (timeout, unreachable, TLS refusal), so the sole thing the box
/// observed is the proxy's own `net-error` category. Under Sprint 18's verbatim pipe this is the
/// **unexpected** path: an invalid credential now arrives as the upstream's own 401 *response*, not as a
/// proxy-invented error, so the expected run takes the `resp-fp:` branch at the call site instead and
/// fingerprints the real response bytes.
///
/// Kept because the branch is real and must stay distinguishable in the capture: a marker beginning
/// `net-error:` means *nothing came back*, and a marker beginning `resp-fp:` means the upstream answered
/// and its bytes were relayed. Confusing the two would let a transport failure masquerade as a successful
/// round trip in the write-up.
///
/// (This doc previously described the marker as deriving from "the box's own observation of the upstream
/// rejection", which described the superseded net-error-based design — where a 401 *was* the net-error.
/// The call site was always correct; only this comment lagged.)
///
/// Opaque to the proxy either way, which relays it undecoded; the head then proves the socket carried a
/// frame traceable to it.
fn alloc_marker(code: proto::ErrorCode, in_reply_to: u64) -> String {
    format!("net-error:{code:?}:irt={in_reply_to:#x}")
}

/// The phases, advanced on quiescence (each frame ACKed before the next is sent).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    AwaitAddressAck,
    AwaitPayloadAck,
    AwaitResponse,
    AwaitDisplayAck,
    AwaitFileWriteAck,
    AwaitFileChunkAck,
    Done,
}

const STALL: Duration = Duration::from_secs(20); // generous: a real upstream call is in the middle
const RESPONSE_QUIET: Duration = Duration::from_secs(3); // no new frame for this long ⇒ response complete

fn main() -> io::Result<()> {
    println!("{}", benchtools::Provenance::capture().header("proxy_s17_box"));
    println!("bench: box={BOX_BDF}  peer(proxy)={PEER_PROXY_MAC:02x?}  (exact BDF; production 61:00.0 NEVER touched)\n");

    let box_bdf = Bdf::parse(BOX_BDF).expect("valid bench box BDF");
    let allow: [Bdf; 1] = [box_bdf]; // this card only

    let request_nonce = hex(&read_nonce()?);
    let chunk_nonce = hex(&read_nonce()?);
    println!("nonce#1 (upstream request body): {request_nonce}");
    println!("nonce#2 (quarantine file-chunk): {chunk_nonce}\n");

    let mut sys = Subsystem::construct(box_bdf, &allow, Endpoint::Box, None, |_own_mac| PEER_PROXY_MAC)?;

    let addr = net_address_l2(NET_DESTINATION);
    let request_body = anthropic_body(&request_nonce);
    // TELL #1 (bind-the-chain): the fingerprint of the body this box SENDS. The proxy prints the
    // fingerprint of the body it ADMITTED and forwarded upstream, computed by the SAME function; the two
    // must match, proving the body crossed the seam unmangled without either side logging its content.
    println!("sent-body fingerprint  : {}   <- must equal the proxy's forwarded-body fingerprint", proxy::lifecycle::body_fingerprint(&request_body));
    let payload = net_payload_l2(&request_body);

    // Phase A begins: open the net-address session.
    let net_sid: SessionId = sys.open(&addr);
    let mut phase = Phase::AwaitAddressAck;
    let mut last_progress = Instant::now();

    let mut response = Vec::new();
    let mut last_response_frame: Option<Instant> = None;
    let mut net_error: Option<(u64, proto::ErrorCode)> = None;
    let mut response_in_reply_to: Option<u64> = None;
    let mut stalled = false;
    let mut send_err: Option<String> = None;
    let mut file_sid: Option<SessionId> = None;

    sys.run(|ctx: &mut Ctx, ev: Event| match ev {
        // A delivery is proxy-ORIGINATED: the relayed upstream response, or a net-error.
        Event::Delivered(_session, frame) => {
            match classify(&frame) {
                Reply::Response { in_reply_to, bytes } => {
                    response_in_reply_to.get_or_insert(in_reply_to);
                    response.extend_from_slice(&bytes);
                    last_response_frame = Some(Instant::now());
                }
                // Legible failure recording: an upstream failure must read as a NAMED failure, never
                // as a hang.
                Reply::Error { in_reply_to, code } => {
                    net_error = Some((in_reply_to, code));
                    last_response_frame = Some(Instant::now());
                }
                Reply::Undecodable => {}
            }
            ControlFlow::Continue(())
        }
        Event::Idle => {
            // Awaiting the upstream response is not a quiescence question — it is a wait for the
            // proxy to originate. Complete it once frames have stopped arriving for a quiet window.
            if phase == Phase::AwaitResponse {
                // ANY terminal reply advances the run to the display leg — a relayed response body
                // (the expected outcome: the real endpoint's own rejection bytes, per `cloud/approach.md`
                // §5) or a net-error (a genuine transport failure). Which one arrived is judged in the
                // final account below, not here: the remaining phases are worth driving either way, so
                // this deliberately does not stop early on the unexpected case.
                match last_response_frame {
                    Some(t) if t.elapsed() >= RESPONSE_QUIET => {
                        // Phase D: the marker is DERIVED FROM THE REAL RESPONSE CONTENT, as a
                        // FINGERPRINT rather than a parsed field. An upstream error body's shape is not
                        // known in advance — under the superseded behaviour it never reached the box at
                        // all, so nothing here has ever observed one — and keying the marker on an assumed
                        // JSON field would risk failing the run for a reason unrelated to the property
                        // being proven. A fingerprint is shape-agnostic, and content-free, so the marker
                        // stays safe to relay even if the body echoed request material.
                        let marker = if !response.is_empty() {
                            format!("resp-fp:{}", proxy::lifecycle::body_fingerprint(&response))
                        } else if let Some((irt, code)) = &net_error {
                            alloc_marker(*code, *irt) // a genuine transport failure — no response at all
                        } else {
                            "no-observation".into()
                        };
                        // Record the raw body for the capture script's assertions (the credential-echo
                        // check). Written, never printed — see `response_body_path`.
                        if !response.is_empty()
                            && let Err(e) = std::fs::write(response_body_path(), &response)
                        {
                            eprintln!("s17_box: could not record the response body: {e}");
                        }
                        let display = raw_frame(proto::verb::DISPLAY, marker.as_bytes());
                        let _ = ctx.open(&display);
                        phase = Phase::AwaitDisplayAck;
                        last_progress = Instant::now();
                        return ControlFlow::Continue(());
                    }
                    _ => {
                        if last_progress.elapsed() > STALL {
                            stalled = true;
                            return ControlFlow::Break(());
                        }
                        return ControlFlow::Continue(());
                    }
                }
            }

            if !ctx.is_quiescent() {
                if last_progress.elapsed() > STALL {
                    stalled = true;
                    return ControlFlow::Break(());
                }
                return ControlFlow::Continue(()); // current frame still in flight
            }
            last_progress = Instant::now();

            match phase {
                // Phase B: the address was ACKed → continue the SAME session with the payload.
                Phase::AwaitAddressAck => match ctx.send(net_sid, &payload) {
                    Ok(()) => {
                        phase = Phase::AwaitPayloadAck;
                        ControlFlow::Continue(())
                    }
                    Err(e) => {
                        send_err = Some(format!("net-payload: {e:?}"));
                        ControlFlow::Break(())
                    }
                },
                // Phase C: the request is on the wire; wait for the proxy to relay the response.
                Phase::AwaitPayloadAck => {
                    phase = Phase::AwaitResponse;
                    ControlFlow::Continue(())
                }
                Phase::AwaitResponse => ControlFlow::Continue(()), // handled above
                // Phase E: the display was ACKed → open the quarantine write on its OWN session (the
                // file-write binds that session to the File service and declares the destination).
                Phase::AwaitDisplayAck => {
                    file_sid = Some(ctx.open(&file_write_l2(WRITE_DESTINATION)));
                    phase = Phase::AwaitFileWriteAck;
                    ControlFlow::Continue(())
                }
                // The destination was ACKed → the chunk CONTINUES that same session, carrying nonce #2.
                Phase::AwaitFileWriteAck => {
                    let sid = file_sid.expect("opened in the previous phase");
                    match ctx.send(sid, &raw_frame(proto::verb::FILE_CHUNK, chunk_nonce.as_bytes())) {
                        Ok(()) => {
                            phase = Phase::AwaitFileChunkAck;
                            ControlFlow::Continue(())
                        }
                        Err(e) => {
                            send_err = Some(format!("file-chunk: {e:?}"));
                            ControlFlow::Break(())
                        }
                    }
                }
                Phase::AwaitFileChunkAck => {
                    phase = Phase::Done;
                    ControlFlow::Break(())
                }
                Phase::Done => ControlFlow::Break(()),
            }
        }
    });

    // ── The capture's own account ────────────────────────────────────────────────────────────────
    println!("phase reached: {phase:?}  stalled={stalled}  send_err={send_err:?}");
    println!("box request session     : {:#x}", net_sid.0);
    // THE EXPECTED OUTCOME is now a non-empty RESPONSE. Under the §5 ruling an HTTP status is a response,
    // not an error: the deliberately-invalid credential makes the real endpoint reject, and those real
    // rejection bytes reach this box VERBATIM. A net-error now means only a genuine TRANSPORT failure
    // (no response at all), which is NOT what this capture expects.
    if !response.is_empty() {
        println!("response bytes received : {}   <- EXPECTED: the upstream's own reply, relayed verbatim", response.len());
        println!("response fingerprint    : {}", proxy::lifecycle::body_fingerprint(&response));
        println!("response in_reply_to    : {response_in_reply_to:?}");
        println!(
            "  correlated to this box's request session ({:#x}): {}",
            net_sid.0,
            if response_in_reply_to == Some(net_sid.0) { "YES" } else { "NO — MISMATCH" },
        );
        println!("raw body written to     : {}", response_body_path());
        println!("  (NOT printed here — an error body can echo request material, so the capture script");
        println!("   asserts over the file before anything renders it)");

        // ---- THE RESPONSE-SIDE TELL, with its FS#7 narrowing fixed IN ADVANCE ----
        // Both branches are written here before the run, deliberately, so that whichever one the live
        // response takes, the strength of the claim is not a judgement made after seeing the result.
        match upstream_request_id(&response) {
            Some(id) if is_safe_request_id(&id) => {
                println!("response request-id     : {}", String::from_utf8_lossy(&id));
                println!("  FULL RESPONSE-SIDE CLAIM: this id is generated by the upstream for THIS call.");
                println!("  It carries PER-CALL UNIQUENESS — it cannot be replayed from a prior run or");
                println!("  fabricated locally, so it binds this capture to this specific round trip, and");
                println!("  is independently checkable against the upstream's own records.");
            }
            Some(_) => {
                println!("response request-id     : PRESENT but of unexpected shape — NOT printed");
                println!("  (same discipline as the body: unvalidated response material is never rendered");
                println!("   by this binary; read it from the recorded file instead)");
                println!("  FS#7 NARROWING APPLIES: an id that cannot be safely displayed is treated as");
                println!("  ABSENT for the purpose of the claim — see the narrowed wording below.");
                println!("  NARROWED CLAIM: reachability and correct real-path behavior ONLY. The upstream");
                println!("  was genuinely reached and its own bytes were relayed verbatim. NO per-call");
                println!("  uniqueness is claimed: nothing in the response distinguishes this call from");
                println!("  any other, so the response side does not bind this run to this moment.");
            }
            None => {
                println!("response request-id     : ABSENT");
                println!("  FS#7 NARROWING (fixed in advance, not decided after the fact):");
                println!("  NARROWED CLAIM: reachability and correct real-path behavior ONLY. The upstream");
                println!("  was genuinely reached and its own bytes were relayed verbatim. NO per-call");
                println!("  uniqueness is claimed: nothing in the response distinguishes this call from");
                println!("  any other, so the response side does not bind this run to this moment.");
                println!("  The REQUEST-side nonce (#1) remains the per-call-unique binding for this run.");
            }
        }
        // BILLING is NOT determinable at the box: the net-response verb carries no status field, so "a
        // response arrived" says nothing about whether it was metered. A rejected request is unmetered
        // regardless. Judging it is the script's/operator's job from the recorded body, not an inference
        // this binary is entitled to make.
        println!("billing                 : not determinable at the box (no status on the wire);");
        println!("                          a rejected request is unmetered regardless.");
    } else if let Some((irt, code)) = net_error {
        println!("UNEXPECTED: net-error code={code:?} in_reply_to={irt:#x}, and NO response body.");
        println!("  A net-error now means a genuine TRANSPORT failure (timeout / unreachable / TLS):");
        println!("  the upstream was never reached, so no real reply could be relayed. This capture");
        println!("  expects the real endpoint's own rejection bytes instead.");
        return Err(io::Error::other("transport failure — no upstream response to relay"));
    } else {
        println!("NO response and NO net-error — the upstream leg produced nothing.");
        return Err(io::Error::other("no upstream outcome observed"));
    }
    println!("request nonce (#1)      : {request_nonce}");
    println!("chunk nonce   (#2)      : {chunk_nonce}   <- must appear in the quarantine file");
    if stalled || send_err.is_some() {
        return Err(io::Error::other("box did not complete its phases"));
    }
    Ok(())
}
