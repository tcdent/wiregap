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
use std::os::fd::{AsFd, OwnedFd};
use std::path::PathBuf;

use directpath::{Bdf, Event, SessionId, Subsystem};

use crate::dispatch::{Dispatcher, Routed};
use crate::nic::{NicRun, RealNic, Reply};
use crate::credential::Credential;
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
    F: FnMut(&mut dyn Reply, Event) -> ControlFlow<()>,
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
/// The receive-then-reply run-loop adapter. On each `Event::Delivered`, route the frame through the
/// [`Dispatcher`] (which may register an outbound reply stream, e.g. an admitted `file-read`), hand the
/// [`Routed`] verdict to `on_verdict`, then **pump the reply streams** via the [`Reply`] handle. On
/// `Event::Idle`, pump the reply streams (so a stalled reply makes progress the moment the wire frees).
///
/// **The reply happens through the Slice-1 seam** ([`Dispatcher::pump_replies`] takes the `Reply`
/// handle, live only for this callback invocation — never held across calls). Pacing/streaming/eviction
/// all live in the dispatcher; the adapter only drives the ticks. The adapter never breaks on its
/// **own**: the loop runs until the subsystem stops. `on_verdict` returns the [`ControlFlow`], so an
/// observer *can* stop the loop (the Slice-5 e2e stops after its proof; production returns `Continue`).
pub fn run_adapter<O>(
    mut dispatcher: Dispatcher,
    register: &'static [Route],
    vetted: &'static [ToolIdentity],
    mut on_verdict: O,
) -> impl FnMut(&mut dyn Reply, Event) -> ControlFlow<()>
where
    O: FnMut(SessionId, &Routed) -> ControlFlow<()>,
{
    move |reply, event| match event {
        Event::Delivered(session, payload) => {
            let routed = dispatcher.route(session, &payload, register, vetted);
            let cf = on_verdict(session, &routed);
            dispatcher.pump_replies(reply); // drive any outbound reply stream (WouldBlock-paced)
            cf
        }
        // Idle: no new frame, but push in-flight replies forward (a WouldBlock-stalled stream resumes).
        Event::Idle => {
            dispatcher.pump_replies(reply);
            ControlFlow::Continue(())
        }
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
pub fn validate_authority(
    authority: Authority,
    register: &[Route],
    credential: Option<&Credential>,
    roots: Option<&ValidatedRoots>,
) -> bool {
    match authority {
        // Real now — no external config needed; the register's own presence is its authority.
        Authority::RouteRegister => !register.is_empty(),
        // **REAL now (Sprint 16)** — the roots were resolved ONCE by `assemble` from the compiled
        // `RootPath` constants: the quarantine pinned via `pin_root`, the readable root proven a
        // readable directory. `Some` means that resolution succeeded and the pinned fd the proxy will
        // serve from is in hand; `None` means it failed (named by `RootFailure`) and the whole proxy
        // stays closed. The check derives entirely from the configured value — it cannot pass on a bad
        // root, and it cannot fail on a good one.
        Authority::FilesystemRoots => roots.is_some(),
        // **REAL now (Sprint 16)** — the kernel-enforced floor this design leans on is verified to be
        // actually in effect, rather than trusted unchecked (`architecture.md` §9's standing "verify at
        // boot" convention). Two probes, both against the SAME pinned quarantine fd `FilesystemRoots`
        // obtained (bind-the-chain: the mount probed is the mount written to):
        //   1. `fstatfs` on that fd → the mount carries `noexec`, `nosuid`, AND `nodev`.
        //   2. an `openat2` under the real RESOLVE mask → the confinement mechanism every confined
        //      write depends on is available on this kernel.
        // The required flags are compile-time constants with no config surface: a configurable security
        // floor is a weakenable one. No roots (a failed resolution) → no fd to probe → closed.
        Authority::KernelProtections => roots.is_some_and(|r| kernel_protections_hold(r.quarantine())),
        // **REAL** — the upstream credential, resolved from its compiled `CredentialSource` descriptor
        // (Sprint 16; the descriptor names the env var, never the value — `architecture.md` §8's
        // custody-is-the-configured-thing). A **missing** credential holds the **whole** proxy closed
        // (all-or-none ladder); a supplied one satisfies this rung (full serving still awaits the other
        // config-pending authorities above). Supplying a credential does NOT by itself open serving.
        //
        // **Present AND non-empty.** `Credential::from_env_value` already treats a set-but-empty
        // variable as absent, so this is the rung's own second floor: an empty credential fails closed
        // however it was constructed, not only when it came through that path. An authority that
        // accepted an empty secret would be open while reporting itself validated.
        Authority::Credential => credential.is_some_and(|c| !c.is_empty()),
    }
}

/// **The two filesystem roots, resolved once and validated** (Sprint 16) — the artifact the
/// `FilesystemRoots` authority rung reports on, and the one the serving proxy actually uses.
///
/// **Bind-the-chain: resolved ONCE.** The quarantine root is pinned a single time, here; the resulting
/// open file description is what the `Dispatcher` writes beneath and what `KernelProtections` probes.
/// Re-deriving it from the path at each step would mean the directory validated, the directory probed,
/// and the directory served from are three separate resolutions of a mutable path — a TOCTOU window on
/// the exact ground the box writes to. The same discipline as `recognized == reassembled == forwarded`
/// on the net path: **validated == probed == used**.
#[derive(Debug)]
pub struct ValidatedRoots {
    /// The pinned quarantine root — an `O_PATH|O_DIRECTORY|O_NOFOLLOW` fd from the certified
    /// [`pin_root`](crate::open_discipline::pin_root). Held so `KernelProtections` can probe *this* fd.
    quarantine: OwnedFd,
    /// The resolved readable root. A plain path by design (below).
    readable: PathBuf,
}

impl ValidatedRoots {
    /// A borrow of the pinned quarantine root, for probing **the same** fd that will be served from.
    #[must_use]
    pub fn quarantine(&self) -> std::os::fd::BorrowedFd<'_> {
        self.quarantine.as_fd()
    }

    /// The resolved readable root.
    #[must_use]
    pub fn readable(&self) -> &std::path::Path {
        &self.readable
    }
}

/// Why a filesystem root failed to validate — named, so the operator learns *which* root and *why*
/// rather than only that the proxy closed (`§7`, refusal-is-the-signal). The ladder still reports the
/// rung as `Closed(FilesystemRoots)`; this is the detail alongside it.
#[derive(Debug)]
pub enum RootFailure {
    /// The quarantine root could not be pinned: absent, not a directory, a symlink at the final
    /// component, or otherwise unopenable.
    Quarantine(io::Error),
    /// The readable root could not be resolved or read as a directory.
    Readable(io::Error),
}

impl core::fmt::Display for RootFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RootFailure::Quarantine(e) => write!(f, "quarantine root: {e}"),
            RootFailure::Readable(e) => write!(f, "readable root: {e}"),
        }
    }
}

/// Resolve and validate both filesystem roots — **asymmetrically, by design** (Sprint 16).
///
/// The two roots carry different risk and so get different floors; symmetrizing them would be
/// over-specification, not extra safety:
/// - **The quarantine root is PINNED** via the certified `pin_root`
///   (`O_PATH|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC`). This is the ground the box's bytes land on, so its
///   validation *is* the confinement anchor: the flags reject a missing path, a non-directory, and a
///   symlink at the final component in one syscall, and the surviving fd is what every confined write
///   resolves beneath.
/// - **The readable root is plainly resolved** and checked to be a readable directory. Reading is
///   harmless (`filesystem.md` §4–§5: its breadth is "sanity, not safety"), so it needs no pinned-fd
///   confinement — a `read_dir` both proves directory-ness and catches the unreadable/unsearchable case
///   that a bare `stat` would miss.
///
/// Fail-closed and total: any failure returns [`RootFailure`], never a partially-validated pair.
pub fn resolve_roots(quarantine: &str, readable: &str) -> Result<ValidatedRoots, RootFailure> {
    // Pin the quarantine ONCE — this fd is the one that gets probed and served from.
    let quarantine = crate::open_discipline::pin_root(quarantine).map_err(RootFailure::Quarantine)?;
    // The readable root: resolve + prove it is a directory we can actually read.
    let readable_path = PathBuf::from(readable);
    std::fs::read_dir(&readable_path).map_err(RootFailure::Readable)?;
    Ok(ValidatedRoots { quarantine, readable: readable_path })
}

/// Do the kernel-enforced protections hold for this **pinned** quarantine directory? Both halves must:
/// the mount carries `noexec,nosuid,nodev`, and `openat2`'s RESOLVE confinement is available. Fail
/// closed on any probe error — an unreadable answer is not a satisfied authority.
#[must_use]
pub fn kernel_protections_hold(quarantine: std::os::fd::BorrowedFd<'_>) -> bool {
    let protections_ok = crate::open_discipline::mount_protections(quarantine)
        .map(|p| p.all_present())
        .unwrap_or(false);
    protections_ok && crate::open_discipline::openat2_resolve_available(quarantine)
}

/// A **content-free fingerprint** of a request body — the observer for the capture's bind-the-chain tell
/// (Sprint 17). The box prints the fingerprint of what it SENT; the proxy prints the fingerprint of what
/// it ADMITTED and forwarded; a match proves the body crossed the seam unmangled — while logging none of
/// its content. Both sides call THIS function, so the comparison cannot drift.
///
/// **Deliberately not a cryptographic hash, and the reason is the reviewability budget.** There is no
/// `sha2`/`digest` crate in this crate's dependency tree, so a literal SHA-256 here would mean adding a
/// crypto dependency to the TCB — spending the guarantee the whole project rests on (`principles.md`
/// §15) to print a diagnostic. It is not needed: the property required is that an *accidental* mangling
/// (a truncation, a re-encode, a reordered field) is detected, between two cooperating processes that are
/// both ours. There is no adversary crafting a collision between "what the box sent" and "what the proxy
/// forwarded". A 64-bit FNV-1a plus the exact length gives that with overwhelming probability, in eight
/// reviewable lines and no dependency.
///
/// Emits `<hash>/<len>`: a length mismatch is legible on sight, and length + hash is stronger than either.
#[must_use]
pub fn body_fingerprint(body: &[u8]) -> alloc::string::String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    for &b in body {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV prime
    }
    alloc::format!("{h:016x}/{}", body.len())
}

/// **The one composition-root assembly** (Sprint 16): compiled root paths → validated, pinned roots →
/// a `Dispatcher` holding both. `serve.rs`'s `main` calls this, and so do this sprint's tests — there is
/// no test replica of the wiring, so what the tests prove is what the binary runs.
///
/// Total by construction: a root failure does **not** abort here. It yields a rootless `Dispatcher`
/// (both file channels inert) plus the named failure, so the startup ladder can report
/// `Closed(FilesystemRoots)` in the ordinary way instead of the binary dying with an unnamed error and
/// losing the all-or-none ladder's whole point.
///
/// The `Dispatcher` receives a **duplicate descriptor of the same pinned directory** (`dup(2)`, one open
/// file description) — not a second `pin_root` of the path. Two descriptors, one resolution: the chain
/// stays bound.
#[must_use]
pub fn assemble(quarantine: &str, readable: &str) -> Assembly {
    match resolve_roots(quarantine, readable) {
        Ok(roots) => match roots.quarantine.try_clone() {
            Ok(for_dispatcher) => {
                let dispatcher = Dispatcher::new()
                    .with_quarantine_root(for_dispatcher)
                    .with_readable_root(roots.readable.clone());
                Assembly { dispatcher, roots: Ok(roots) }
            }
            // Duplicating an already-pinned fd essentially cannot fail (no path lookup, no permissions);
            // if the process is out of descriptors, fail closed like any other root failure.
            Err(e) => Assembly { dispatcher: Dispatcher::new(), roots: Err(RootFailure::Quarantine(e)) },
        },
        Err(why) => Assembly { dispatcher: Dispatcher::new(), roots: Err(why) },
    }
}

/// What [`assemble`] produces: the `Dispatcher` the proxy serves with, and the validated roots the
/// `FilesystemRoots` rung reports on (or the named failure that closes it).
pub struct Assembly {
    /// The dispatcher, holding **both** roots on success; rootless (file channels inert) on failure.
    pub dispatcher: Dispatcher,
    /// The validated roots, or why they failed.
    pub roots: Result<ValidatedRoots, RootFailure>,
}

/// The composition-root configuration for [`bring_up`] — every non-closure input the ladder needs,
/// grouped so the entry takes `(config, on_event)` rather than a wide argument list (already past
/// clippy's too-many-args threshold once the Sprint-11 credential landed, and Sprint 13 adds more).
/// **A pure data carrier**: no rung semantics, ordering, or fail-closed behavior lives here — grouping
/// the arguments changes only their shape, never the ladder's behavior.
pub struct BringUpConfig<'a> {
    /// The proxy's own bench card (exact BDF; the caller gates on it — never a device-id scan).
    pub local: Bdf,
    /// The exact-BDF allowlist the transport is permitted to touch.
    pub allowlist: &'a [Bdf],
    /// The peer (box) MAC the transport targets.
    pub peer_mac: [u8; 6],
    /// The unprivileged uid to drop to (supplied by the caller; this entry does not choose it).
    pub uid: u32,
    /// The unprivileged gid to drop to.
    pub gid: u32,
    /// The compiled-in route register (U5) — the one already-grounded authority.
    pub register: &'a [Route],
    /// The upstream credential (Sprint 11) — `None` holds the **whole** proxy closed (all-or-none ladder).
    pub credential: Option<&'a Credential>,
    /// The validated filesystem roots from [`assemble`] (Sprint 16) — `None` (a failed resolution) holds
    /// the **whole** proxy closed at the `FilesystemRoots` rung, same all-or-none rule.
    pub roots: Option<&'a ValidatedRoots>,
}

/// The production bring-up (capture-model — needs the real 82599). Climbs the ladder end to end:
/// `construct` (rungs 1–2, privileged, atomic) → real privilege drop + IB2 self-observation → the
/// authority validators → readiness → `run`. `uid`/`gid` are the target unprivileged identity to drop
/// to — supplied by the caller (a config hook, pending the config DSL); this function does not choose
/// them. `on_event` is the run-loop adapter (Slice 4 supplies the dispatch adapter).
///
/// Returns `Err` only if the privileged bring-up itself failed (no subsystem); a dropped/closed ladder
/// is an `Ok(Readiness::{DropFailed,Closed})` — a *reported* fail-closed outcome, not an I/O error.
pub fn bring_up<F>(config: BringUpConfig<'_>, on_event: F) -> io::Result<Readiness>
where
    F: FnMut(&mut dyn Reply, Event) -> ControlFlow<()>,
{
    let BringUpConfig { local, allowlist, peer_mac, uid, gid, register, credential, roots } = config;
    // Rungs 1–2: privileged bring-up + transport up (real hardware). Atomic — `?` yields no subsystem.
    // `Endpoint::Proxy` is the proxy's FIXED identity (it originates response sessions from the high
    // half of the id space; directpath maps it to `Originator::High`) — a constant of what this binary
    // is, not a runtime/config choice, so it is hardcoded here rather than threaded as a parameter.
    // `None` for the idle-park cap: the reactor's `DdWaitHost` already bounds its own idle park (a ~5ms
    // hang-safety ceiling), so a quiescent arrival is serviced within ~5–10ms with no app-supplied cap —
    // this capability is not something the proxy configures (NIC designer ruling 2026-07-23; the earlier
    // app-set 10ms cap was counterproductive, not inert, so it is dropped).
    let subsystem =
        Subsystem::construct(local, allowlist, directpath::Endpoint::Proxy, None, |_own_mac| peer_mac)?;
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
        |authority| validate_authority(authority, register, credential, roots),
        on_event,
    ))
}
