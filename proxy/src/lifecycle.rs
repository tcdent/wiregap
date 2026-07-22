//! Sprint 8 — the startup lifecycle ladder (`architecture.md` §9, L1): a sequence of **fail-closed
//! rungs**, no rung climbed until the one below holds, all-or-nothing at readiness — a single broken
//! authority holds the **whole** proxy closed, named by channel. "Broken is broken; high safety, not
//! high availability" (operator, L1).
//!
//! The rungs, mapped to the real API:
//! - **Rungs 1–2 (privileged bring-up + transport up)** are [`directpath::Subsystem::construct`],
//!   which is atomic (returns `Ok` only if the link came up and the rings armed) — done on real
//!   hardware in [`bring_up`].
//! - **Privilege drop** ends rung 1 (L15): [`crate::privilege::drop_to`], before any box byte is
//!   *processed*. (RX rings are armed by `construct` while still privileged, but frames are only
//!   delivered inside `run`, after the drop — so drop-before-first-box-byte holds.)
//! - **Rung 3 (authority/environment validation)** — [`AUTHORITIES`], each validated post-drop
//!   (unprivileged); any failure holds everything closed, named.
//! - **Rung 4 (channels open — all or none)** — reached only when every authority validated; then and
//!   only then is [`NicRun::run`] entered.
//!
//! The [`serve`] core is **generic over [`NicRun`]** and takes the drop and the authority validator as
//! injected steps, so the *sequencing* (drop-before-run, all-or-none, fail-closed ordering) is
//! fresh-re-runnable against a fake — only the real `construct` bring-up and the real `run` round-trip
//! stay capture-model (the sprint's evidence split).

use std::io;
use std::ops::ControlFlow;

use directpath::{Bdf, Event, SessionId, Subsystem};

use crate::dispatch::{Dispatcher, Routed};
use crate::nic::{NicRun, RealNic};
use crate::vocab::{Route, ToolIdentity};

/// A named authority the proxy must validate before it serves — a channel of trust that, broken, holds
/// the **whole** proxy closed (all-or-none readiness, §9). Sprint 8 defines the set and the sequencing;
/// the per-authority *checks* are named fail-closed placeholders pending the config surface (see
/// [`validate_authority`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Authority {
    /// The compiled-in route register (U5). Its authority is the register itself (const-validated).
    RouteRegister,
    /// The filesystem roots — the quarantine write-root and the readable read-root — resolved and
    /// present on their expected mounts.
    FilesystemRoots,
    /// Kernel-enforced protections the tier-1 floor relies on: the quarantine mounted
    /// `noexec,nosuid,nodev`, and `openat2` RESOLVE confinement available.
    KernelProtections,
    /// The upstream credential, when the proxy holds it.
    Credential,
}

/// The ordered set of authorities validated at rung 3. **All-or-none**: readiness requires every one.
pub const AUTHORITIES: &[Authority] = &[
    Authority::RouteRegister,
    Authority::FilesystemRoots,
    Authority::KernelProtections,
    Authority::Credential,
];

/// The outcome of climbing the ladder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Readiness {
    /// Every rung held; the proxy served — `run` was entered and later returned.
    Served,
    /// An authority failed validation — the whole proxy held closed (named by channel); `run` was
    /// **never** entered, so not one box byte was processed.
    Closed(Authority),
    /// The privilege drop (or its post-drop self-observation) failed — fail closed; `run` never
    /// entered. Carries the failing `errno` (or `-1` when the failure was the self-observation, not a
    /// syscall).
    DropFailed(i32),
}

/// The generic ladder core (fresh-testable): drop privilege, validate every authority all-or-none, and
/// serve **iff** all held. `drop_privilege` and `validate` are injected so the sequencing can be driven
/// by a fake `NicRun` + fake authorities with no hardware and no real privilege change.
///
/// Ordering is load-bearing and enforced here: the drop happens **before** any authority check, and
/// `run` is entered **only after** every authority validates — a validation failure returns without
/// ever calling `run`, so a partial-authority serving state is unreachable.
pub fn serve<N, D, V, F>(nic: &mut N, drop_privilege: D, validate: V, on_event: F) -> Readiness
where
    N: NicRun,
    D: FnOnce() -> io::Result<()>,
    V: Fn(Authority) -> bool,
    F: FnMut(Event) -> ControlFlow<()>,
{
    // Rung 1 end (L15): drop privilege before any authority check or box byte. Fail closed if the drop
    // — or its self-observation — did not complete; never serve at elevated privilege.
    if let Err(e) = drop_privilege() {
        return Readiness::DropFailed(e.raw_os_error().unwrap_or(-1));
    }

    // Rung 3: validate every authority. All-or-none — the FIRST failure holds the WHOLE proxy closed,
    // named by channel, and returns before `run` is ever entered.
    for &authority in AUTHORITIES {
        if !validate(authority) {
            return Readiness::Closed(authority);
        }
    }

    // Rung 4: every authority validated → ready → serve.
    nic.run(on_event);
    Readiness::Served
}

/// The receive-path run-loop adapter (Slice 4): the `on_event` closure that drives dispatch. Each
/// `Event::Delivered(session, payload)` is routed through the [`Dispatcher`] (which holds the
/// per-session binding/route/reassembler state); the resulting [`Routed`] verdict is handed to
/// `on_verdict` for observation. `Event::Idle` continues.
///
/// **No `Ctx`.** The proxy never originates in response (R-c), so nothing is sent back this sprint —
/// the adapter routes the receive path only. It never breaks on its **own**: the loop runs until the
/// subsystem stops. `on_verdict` returns the [`ControlFlow`], so an observer *can* stop the loop (the
/// Slice-5 e2e stops after its proof; production returns `Continue` and runs indefinitely).
pub fn run_adapter<O>(
    mut dispatcher: Dispatcher,
    register: &'static [Route],
    vetted: &'static [ToolIdentity],
    mut on_verdict: O,
) -> impl FnMut(Event) -> ControlFlow<()>
where
    O: FnMut(SessionId, &Routed) -> ControlFlow<()>,
{
    move |event| match event {
        Event::Delivered(session, payload) => {
            let routed = dispatcher.route(session, &payload, register, vetted);
            on_verdict(session, &routed)
        }
        Event::Idle => ControlFlow::Continue(()),
    }
}

/// The production authority validator — Sprint 8's **named fail-closed placeholders**.
///
/// The ladder and its all-or-none sequencing are real and complete; the per-authority *checks* mostly
/// await the config surface (a future Rust DSL, U5-register style — deliberately **not** built here, and
/// **not** invented as a side effect of this sprint). Until that config lands, an authority with nothing
/// yet to validate against fails **closed** — the posture's default (no proven authority = no serving,
/// §9), never a silent pass. The one authority already grounded is the route register, whose authority
/// *is* the compiled-in U5 register (`Route::new` const-validated): "validated" = present and non-empty.
///
/// Each arm names the exact point where a future `validate_from_config(cfg.<authority>)` slots in, so
/// the config DSL lands cleanly without reshaping the ladder. Returns `true` iff the authority is
/// validated (the *which* of a failure is the [`Authority`] itself, named by the ladder).
#[must_use]
pub fn validate_authority(authority: Authority, register: &[Route]) -> bool {
    match authority {
        // Real now — no external config needed; the register's own presence is its authority.
        Authority::RouteRegister => !register.is_empty(),
        // Pending the config DSL — fail closed (named placeholders). Each is where the config check lands:
        //   FilesystemRoots  → resolve + stat the quarantine/readable roots from cfg.
        //   KernelProtections→ statfs the quarantine (noexec/nosuid/nodev) + probe openat2 RESOLVE.
        //   Credential       → load the upstream credential from cfg, when proxy-held.
        Authority::FilesystemRoots => false,
        Authority::KernelProtections => false,
        Authority::Credential => false,
    }
}

/// The production bring-up (capture-model — needs the real 82599). Climbs the ladder end to end:
/// `construct` (rungs 1–2, privileged, atomic) → real privilege drop + IB2 self-observation → the
/// authority validators → readiness → `run`. `uid`/`gid` are the target unprivileged identity to drop
/// to — supplied by the caller (a config hook, pending the config DSL); this function does not choose
/// them. `on_event` is the run-loop adapter (Slice 4 supplies the dispatch adapter).
///
/// Returns `Err` only if the privileged bring-up itself failed (no subsystem); a dropped/closed ladder
/// is an `Ok(Readiness::{DropFailed,Closed})` — a *reported* fail-closed outcome, not an I/O error.
pub fn bring_up<F>(
    local: Bdf,
    allowlist: &[Bdf],
    peer_mac: [u8; 6],
    uid: u32,
    gid: u32,
    register: &[Route],
    on_event: F,
) -> io::Result<Readiness>
where
    F: FnMut(Event) -> ControlFlow<()>,
{
    // Rungs 1–2: privileged bring-up + transport up (real hardware). Atomic — `?` yields no subsystem.
    // `Endpoint::Proxy` is the proxy's FIXED identity (it originates response sessions from the high
    // half of the id space; directpath maps it to `Originator::High`) — a constant of what this binary
    // is, not a runtime/config choice, so it is hardcoded here rather than threaded as a parameter.
    let subsystem =
        Subsystem::construct(local, allowlist, directpath::Endpoint::Proxy, |_own_mac| peer_mac)?;
    let mut nic = RealNic(subsystem);

    // The real privilege drop, with the IB2 self-check folded in: after the drop, read the credential
    // state back and refuse to proceed unless it is fully dropped (a "successful" syscall that left a
    // privileged saved-uid or a non-empty bounding set is caught here and fails closed).
    let drop_privilege = || -> io::Result<()> {
        crate::privilege::drop_to(uid, gid)?;
        let creds = crate::privilege::observe();
        if creds.is_fully_dropped_to(uid, gid) {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "privilege drop incomplete: {creds:?} is not fully dropped to uid={uid} gid={gid}"
            )))
        }
    };

    Ok(serve(
        &mut nic,
        drop_privilege,
        |authority| validate_authority(authority, register),
        on_event,
    ))
}
