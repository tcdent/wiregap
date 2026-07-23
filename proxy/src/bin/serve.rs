//! Sprint 13 Slice 2 — the composition-root binary: the first RUNNABLE proxy.
//!
//! Wires the real components — the `RealNic` (82599, constructed inside `bring_up`) + an
//! `AnthropicClient` (upstream) + a real `HeadConnection` (accepted from a `UnixListener`) + the
//! credential + the register — through `bring_up` (the fail-closed ladder) and `run_adapter_composed`
//! (the one production loop, Slice 1). This makes a proxy **visible**; it does **not** prove a live
//! round-trip: that is HELD on Task 20 (the reactor's bounded park, still with the NIC team), a real
//! operator credential, and a real head — none present in a normal environment.
//!
//! SAFETY (reuses `s5_capture`'s exact posture): the ONLY device touched is the bench proxy card by
//! EXACT BDF (`Bdf::parse` literal). NEVER device-id enumeration — the production NIC at 0000:61:00.0 is
//! ALSO a 0x10fb and would match a scan. Two floors: the construct allowlist = [41:00.0] (this card
//! only), and directpath's `gate()` independently refuses the production identity (0x7a12).
//! Feature-gated (`--features serve`) so `cargo build` (the production/TCB build) never compiles this
//! hardware-touching binary. Operator paste-and-confirm the BDF before any run.
//!
//! CREDENTIAL: a thin, clearly-marked **interim** env-var read (`ANTHROPIC_API_KEY`) handed straight to
//! `Credential::new` — NOT a config loader/parser/DSL (Sprint 8's config surface is still queued). The
//! raw key is never printed, logged, or echoed to the box (`Credential`'s `Debug` is redacted). A
//! MISSING credential holds the WHOLE proxy closed at the authority rung (all-or-none ladder), never a
//! silent open.
//!
//! Build (bench only): `cargo run --features serve --bin serve` (needs privilege + the bench card + a
//! head on the socket + the credential env var).

use std::io;
use std::ops::ControlFlow;
use std::os::unix::net::UnixListener;

use directpath::Bdf;
use proxy::dispatch::Dispatcher;
use proxy::head::{HeadConnection, run_adapter_composed};
use proxy::lifecycle::{BringUpConfig, Readiness, bring_up};
use proxy::register::anthropic::VETTED_TOOLS;
use proxy::register::net::ROUTES;
use proxy::upstream::{AnthropicClient, Credential};

// ── SAFETY: exact bench BDF only — never a device-id scan (61:00.0 production is also 0x10fb). ──────
const PROXY_BDF: &str = "0000:41:00.0"; // the proxy's bench card — the ONLY device this binary touches

// ── PLACEHOLDER config (pending the Sprint-8 config DSL; no config system is invented here) ─────────
const DROP_UID: u32 = 65534; // nobody — the unprivileged drop target (config hook, same as s5_capture)
const DROP_GID: u32 = 65534;
// The box's L2 MAC the transport targets. No config DSL and no MAC-discovery for a standalone proxy yet,
// so a marked const the operator MUST set to the real box MAC before a LIVE run. Zeroed so a mis-run
// fails loudly at the transport rather than addressing a wrong peer. Never exercised this sprint (the
// live round-trip is HELD).
const PEER_BOX_MAC: [u8; 6] = [0u8; 6];
// The head-connection rendezvous socket (config hook) — the operator's head connects here.
const HEAD_SOCKET_PATH: &str = "/run/zerotrust-proxy/head.sock";
// The upstream endpoint (production Anthropic Messages API) and the interim credential env var.
const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const CREDENTIAL_ENV: &str = "ANTHROPIC_API_KEY";

fn main() -> io::Result<()> {
    // Exact-BDF gate (never enumeration): parse the literal; the allowlist is THIS card only.
    let local = Bdf::parse(PROXY_BDF).expect("valid bench proxy BDF");
    let allowlist: [Bdf; 1] = [local];
    eprintln!("proxy: local={PROXY_BDF} (exact BDF; production 61:00.0 NEVER touched)");

    // Thin, INTERIM credential read — env var only, never a config loader/DSL. The raw key is handed
    // straight to `Credential::new` and never printed/logged/echoed (redacted `Debug`). Absent → `None`
    // → the ladder holds the WHOLE proxy closed at the authority rung (all-or-none, fail-closed).
    let key = std::env::var(CREDENTIAL_ENV).ok();
    let ladder_credential = key.as_deref().map(|k| Credential::new(k.to_string()));
    eprintln!(
        "proxy: credential {}",
        if ladder_credential.is_some() { "present (redacted)" } else { "ABSENT — proxy will fail closed" },
    );

    // The real upstream client, built from the same interim key. When the key is absent this client is
    // unusable — but `run()` is never entered (the ladder closes the proxy at the authority rung first),
    // so its `spawn` is never called and the empty key is never used.
    let client = AnthropicClient::new(Credential::new(key.unwrap_or_default()), ANTHROPIC_ENDPOINT.to_string());

    // The real head connection: bind the rendezvous socket and accept ONE head (the operator's terminal).
    // Blocks until a head connects — a real proxy waits for its head. The composed adapter owns it. (The
    // accept precedes the ladder's authority check because the committed `bring_up` takes the adapter — and
    // thus the head — up front; a missing credential still closes the whole proxy, just after a head lands.)
    eprintln!("proxy: awaiting head on {HEAD_SOCKET_PATH} …");
    let listener = UnixListener::bind(HEAD_SOCKET_PATH)?;
    let (stream, _addr) = listener.accept()?;
    let head = HeadConnection::spawn(stream)?;
    eprintln!("proxy: head connected");

    // The one production loop (Slice 1): one Dispatcher, the upstream + head ingresses, one pump.
    // `on_verdict` returns `Continue` — production runs until the subsystem stops (no e2e break).
    let adapter = run_adapter_composed(
        Dispatcher::new(),
        ROUTES,
        VETTED_TOOLS,
        client,
        head,
        |_session, _routed| ControlFlow::Continue(()),
    );

    // `bring_up`: construct (privileged, exact-BDF) → real privilege drop (+ IB2 self-check) → the
    // authority ladder (a missing credential closes the WHOLE proxy, all-or-none) → run the composed
    // loop. HELD: the live round-trip needs Task 20 (the reactor wake), a real credential, a real head.
    let config = BringUpConfig {
        local,
        allowlist: &allowlist,
        peer_mac: PEER_BOX_MAC,
        uid: DROP_UID,
        gid: DROP_GID,
        register: ROUTES,
        credential: ladder_credential.as_ref(),
    };
    let readiness = bring_up(config, adapter)?;
    eprintln!("proxy: readiness={readiness:?}");

    // Exit code is a BINARY-level operational signal (NOT a Rust error — the library return stays
    // `Ok(Readiness::_)`, a reported outcome per the committed lifecycle design). A proxy that was
    // supposed to serve but held closed (a broken authority) or failed the privilege drop must exit
    // NON-ZERO so systemd/monitoring sees startup failed; exit 0 only when it actually served (and the
    // loop later returned cleanly, i.e. the subsystem stopped).
    match readiness {
        Readiness::Served => Ok(()),
        Readiness::Closed(_) | Readiness::DropFailed(_) => std::process::exit(1),
    }
}
