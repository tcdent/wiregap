//! Sprint 13 Slice 2 — the composition-root binary: the first RUNNABLE proxy.
//!
//! Wires the real components — the `RealNic` (82599, constructed inside `bring_up`) + an
//! `AnthropicClient` (upstream) + a real `HeadConnection` (accepted from a `UnixListener`) + the
//! credential + the register — through `bring_up` (the fail-closed ladder) and `run_adapter_composed`
//! (the one production loop, Slice 1). This makes a proxy **visible**, not yet **servable**.
//!
//! **ALL FOUR AUTHORITY RUNGS ARE REAL.** None is a placeholder any more: `RouteRegister` is the
//! compiled register; `FilesystemRoots` resolves the compiled root paths (the quarantine *pinned*, the
//! readable root proven a readable directory); `KernelProtections` probes **that same pinned fd** for
//! `noexec,nosuid,nodev` plus `openat2` RESOLVE availability; `Credential` requires a non-empty value
//! from the environment variable the compiled descriptor names. So `Readiness::Served` is now reachable
//! — and the ladder still closes, all-or-none and named, on whatever is missing.
//!
//! **WHAT THE ENVIRONMENT MUST PROVIDE** (each of these, or the proxy fails closed and says which):
//!   * the quarantine root exists, is a directory, and sits on a mount carrying `noexec,nosuid,nodev`;
//!   * **and is WRITABLE BY THE UID THIS PROCESS DROPS TO** — see the warning below;
//!   * the readable root exists and is a readable directory;
//!   * the credential variable is set to a non-empty value.
//!
//! **WRITABILITY IS NOT AN AUTHORITY — the one gap to know about.** The rungs prove the quarantine is
//! pinned and properly mounted; they do **not** prove the dropped uid can write into it, and no rung
//! can: `pin_root` runs *before* the privilege drop, as root, and root bypasses ordinary permission
//! checks — so a probe there would pass on exactly the case it exists to catch. A quarantine that is
//! root-owned `0755` therefore satisfies every rung, reports `Served`, and still fails at the **first
//! `file-chunk`** (fail-safe: denied, nothing written, the box told). Provisioning must close this:
//! whatever creates the runtime directory must also give it to the drop-target uid. Note `systemd`'s
//! `RuntimeDirectory=` alone does **not** suffice here — it takes ownership from `User=`, which must be
//! `root` for the privileged NIC bring-up, so the directory would land root-owned. Pair it with an
//! explicit ownership step (an `ExecStartPre=+/bin/chown <uid>:<gid> /run/zerotrust-proxy`, or the same
//! `chown` done by the operator's provisioning). Choosing and implementing that mechanism is the
//! operator's deployment call, deliberately not built into this binary.
//!
//! **STILL HELD:** the live end-to-end round-trip — a real 82599 on the bench, a connected head, a real
//! upstream call. Reaching `Served` means every authority *validated*, not that traffic has flowed.
//!
//! Beyond that, the live end-to-end round-trip additionally needs a real operator credential and a real
//! head — neither present in a normal environment. (The quiescent-wake liveness this once listed as held
//! on "Task 20" is **discharged**: Sprint 14 proved on real hardware that the unmodified reactor already
//! services a quiescent arrival, and the proxy-side `idle_cap` was dropped as counterproductive.)
//!
//! SAFETY (reuses `s5_capture`'s exact posture): the ONLY device touched is the bench proxy card by
//! EXACT BDF (`Bdf::parse` literal). NEVER device-id enumeration — the production NIC at 0000:61:00.0 is
//! ALSO a 0x10fb and would match a scan. Two floors: the construct allowlist = [41:00.0] (this card
//! only), and directpath's `gate()` independently refuses the production identity (0x7a12).
//! Feature-gated (`--features serve`) so `cargo build` (the production/TCB build) never compiles this
//! hardware-touching binary. Operator paste-and-confirm the BDF before any run.
//!
//! CREDENTIAL: read at startup from the environment variable named by the compiled, const-validated
//! `register::anthropic::CREDENTIAL_SOURCE` descriptor — the *custody* is configured, the secret itself
//! never enters source (`architecture.md` §8). The raw key is never printed, logged, or echoed to the
//! box (`Credential`'s `Debug` is redacted). A MISSING credential — unset, set-but-empty, or non-UTF-8,
//! all identically — holds the WHOLE proxy closed at the authority rung, never a silent open.
//!
//! Build (bench only): `cargo run --features serve --bin serve` (needs privilege + the bench card + a
//! head on the socket + the credential env var).

use std::io;
use std::ops::ControlFlow;
use std::os::unix::net::UnixListener;

use directpath::Bdf;
use proxy::head::{HeadConnection, run_adapter_composed};
use proxy::lifecycle::{Assembly, BringUpConfig, Readiness, assemble, bring_up};
use proxy::register::anthropic::{CREDENTIAL_SOURCE, VETTED_TOOLS};
use proxy::register::filesystem::{QUARANTINE_ROOT, READABLE_ROOT};
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
// The upstream endpoint (production Anthropic Messages API). The credential's SOURCE is no longer a
// bare constant here — it is `register::anthropic::CREDENTIAL_SOURCE`, a const-validated descriptor in
// the compiled register (Sprint 16), so *where* the secret comes from is reviewed with the rest of the
// operator's authority data rather than buried in the binary.
const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";

fn main() -> io::Result<()> {
    // Exact-BDF gate (never enumeration): parse the literal; the allowlist is THIS card only.
    let local = Bdf::parse(PROXY_BDF).expect("valid bench proxy BDF");
    let allowlist: [Bdf; 1] = [local];
    eprintln!("proxy: local={PROXY_BDF} (exact BDF; production 61:00.0 NEVER touched)");

    // Resolve the credential from its COMPILED DESCRIPTOR — the register names the variable, this reads
    // it. The raw key is never printed/logged/echoed (redacted `Debug`; the line below prints only
    // presence and the variable's NAME, never its contents). Absent — unset, set-but-empty, or non-UTF-8,
    // all identically — yields `None`, and the ladder then holds the WHOLE proxy closed at the authority
    // rung (all-or-none, fail-closed).
    let ladder_credential = Credential::from_source(&CREDENTIAL_SOURCE);
    eprintln!(
        "proxy: credential from ${} — {}",
        CREDENTIAL_SOURCE.var_name(),
        if ladder_credential.is_some() { "present (redacted)" } else { "ABSENT — proxy will fail closed" },
    );

    // The real upstream client. When the credential is absent this client holds an empty key and is
    // unusable — but `run()` is never entered (the ladder closes the proxy at the authority rung first,
    // and an empty credential fails that rung), so its `spawn` is never called and the empty key is
    // never used.
    let client = AnthropicClient::new(
        Credential::from_source(&CREDENTIAL_SOURCE).unwrap_or_else(|| Credential::new(String::new())),
        ANTHROPIC_ENDPOINT.to_string(),
    );

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
    // THE ONE ASSEMBLY (Sprint 16): compiled root paths → validated, pinned roots → the Dispatcher.
    // This is the same function the sprint's tests drive, so what they prove is what this binary runs.
    // A root failure does not abort here: it yields a rootless dispatcher plus the named reason, and the
    // ladder reports `Closed(FilesystemRoots)` in the ordinary all-or-none way.
    let Assembly { dispatcher, roots } = assemble(QUARANTINE_ROOT.as_str(), READABLE_ROOT.as_str());
    match &roots {
        Ok(_) => eprintln!(
            "proxy: filesystem roots validated (quarantine={}, readable={})",
            QUARANTINE_ROOT.as_str(),
            READABLE_ROOT.as_str(),
        ),
        Err(why) => eprintln!("proxy: filesystem roots FAILED — {why} (proxy will fail closed)"),
    }

    let adapter = run_adapter_composed(
        dispatcher,
        ROUTES,
        VETTED_TOOLS,
        client,
        head,
        |_session, _routed| ControlFlow::Continue(()),
    );

    // `bring_up`: construct (privileged, exact-BDF) → real privilege drop (+ IB2 self-check) → the
    // authority ladder (a missing credential closes the WHOLE proxy, all-or-none) → run the composed
    // loop. HELD: the live round-trip needs a real credential, a real head, and the bench card.
    let config = BringUpConfig {
        local,
        allowlist: &allowlist,
        peer_mac: PEER_BOX_MAC,
        uid: DROP_UID,
        gid: DROP_GID,
        register: ROUTES,
        credential: ladder_credential.as_ref(),
        roots: roots.as_ref().ok(),
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
