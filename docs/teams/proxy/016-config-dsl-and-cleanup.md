# Sprint 016 — The Configuration DSL, and Pre-Operator Cleanup

> Proxy team, sprint 16. Closes the last placeholder authorities
> (`FilesystemRoots`, `KernelProtections`, `Credential`'s custody) with a
> compiled-in Rust DSL — the same pattern as the route register — and cleans
> up the five items the pre-operator completeness pass found. Reaching this
> sprint's milestone, the proxy becomes actually servable for the first time.
> Drafted by zero.proxy-lead from planning input by zero.proxy-dev and
> zero.proxy-review (2026-07-24).

---

## Scope Boundary

This sprint owns two things:

1. **Five cleanup items** from the completeness pass (Non-Goals lists what
   each is NOT): `serve.rs`'s held-boundary comment naming the real first
   blocker; `architecture.md` brought current for the reply path, upstream
   client, head connection, composed loop, and liveness proof; the
   credential-custody phrasing corrected in three places; the one
   `proxy-head.md` line Sprint 15's build superseded; `upstream::is_in_flight`
   made private.
2. **The configuration DSL** for the three still-placeholder authorities,
   compiled-in and structurally validated exactly like `proxy::register`
   (`Route::new`/`ToolIdentity::new`'s `const fn` + `assert!` pattern): the
   box cannot author it, an invalid entry is a compile error, there is no
   runtime loader.

The four startup authorities **decompose differently** — grounded against
`architecture.md` §8 and the existing `validate_authority` arms, and stated
explicitly so the DSL isn't built as if they were uniform:

- **`RouteRegister`** — a config *value* only (the compiled list); already
  done (Sprint 5/8). Untouched this sprint.
- **`FilesystemRoots`** — a config *value* (two paths) **plus validation**
  (resolve/stat/pin). The larger piece of this sprint (below).
- **`KernelProtections`** — **validation only, no config value at all**.
  `§8`'s configured set does not include it; the Sprint-8 `validate_authority`
  comment already distinguishes "resolve + stat *from cfg*" (roots) from
  "*probe*" (kernel protections). The required mount flags
  (`noexec`/`nosuid`/`nodev`) are **compile-time constants** — a configurable
  security floor is a weakenable one.
- **`Credential`** — `§8` states plainly that what's configurable is its
  **custody** (where the secret comes from), not the secret itself. The DSL
  compiles a minimal **descriptor** (an env-var name), never the value.

It explicitly does **not** own:

- **Any config *value* for `KernelProtections`.** No DSL surface exists for
  the required mount flags; they are hardcoded probe constants. Non-Goals.
- **A file-path (or any non-env) credential source variant.** `§8`
  constraint 2 ("configuration never lives in — and is never sourced from
  — the quarantine") is satisfied *structurally* by an env-var name having
  no path at all; a file variant would reopen exactly the
  quarantine-adjacency validation question this sprint avoids by
  construction. Named as a legitimate future operator widening (`§8`
  constraint 3), not pre-built.
- **A shape/format validator for the credential's secret value** (e.g. an
  expected vendor prefix). Considered and rejected during planning: it
  encodes a vendor format that can change under us, and the real proof of a
  good credential is the upstream call succeeding, not a local heuristic.
- **Reload-without-restart.** The register pattern's promotion model is
  edit + recompile + restart (`register.rs`'s own C4); this sprint doesn't
  change that, and `architecture.md` §11's reload-vs-restart ledger row
  stays explicitly open.
- **Symmetrizing the quarantine and readable root's validation.** They are
  asymmetric by design (write is dangerous, confined by a pinned fd; read is
  harmless, a plain resolved path) — imposing the write floor on the read
  path would be over-specification, the same class of error reviewer caught
  and corrected at Sprint 10's gate.

## Problem

Three authorities have sat as named, honest, fail-closed placeholders since
Sprint 8, deliberately not resolved then per the PM's ruling not to invent a
config system as a side effect of unrelated sprints. The operator has now
confirmed the DSL shape: a compiled-in Rust register, the same pattern
already proven for `proxy::register`.

Planning found the filesystem-roots piece is **larger than "add the
validation check"** — a real, verified API gap: `Dispatcher::with_readable_root`
and `Dispatcher::with_quarantine_root` are each independent constructors that
call `..Dispatcher::new()`, resetting the *other* field to its default. **No
existing constructor can set both roots on one `Dispatcher`** — every test
today exercises exactly one. Since a servable proxy needs *both* (readable
for file-read/list, quarantine for file-chunk writes), threading a compiled
path constant through `serve.rs` without first fixing this would silently
leave one file channel dead — and nothing in the current test suite would
catch it, since every existing test isolates one root.

Planning also found the completeness pass's `serve.rs` composes
`Dispatcher::new()` with **no roots at all** — the file-read, `list`, and
quarantine-write channels are genuinely **dead** in the real binary today,
not merely gated closed. Closing `FilesystemRoots` for real makes these
channels **live for the first time** — a behavior change this sprint must
gate on the real composition-root path, not only on hand-built test
dispatchers (Sprint 15's canary/nonce discipline applies: prove the confined
reader is rooted at the *configured* root, not merely *a* root).

One more concrete gap surfaced: `serve.rs`'s credential read
(`std::env::var(CREDENTIAL_ENV).ok()`) treats an env var that is **set but
empty** as `Some("")` — a present-but-empty credential currently satisfies
`Authority::Credential`'s `is_some()` check, which is not the fail-closed
behavior the authority is supposed to guarantee.

## Goal

Close `FilesystemRoots`, `KernelProtections`, and `Credential`'s custody with
a compiled-in DSL matching the register's established pattern, fix the
mutually-exclusive root-constructor gap so a servable `Dispatcher` can
actually hold both roots, wire the DSL values through `serve.rs` into a real,
live composition, and land the five cleanup items — reaching the milestone
where all four startup rungs are real and the binary can, for the first
time, actually reach `Readiness::Served`.

## Final State

1. **The five cleanup items land** (Scope Boundary #1), each a
   documentation/visibility fix, none changing behavior.
2. **A `CredentialSource` descriptor**, compiled-in and const-validated
   exactly like `Route::new` (a `const fn` + `assert!` over the env-var
   name's byte shape — non-empty, `[A-Za-z_][A-Za-z0-9_]*` — a compile
   error on anything else). Carries **only the name**, never a value; a
   literal secret cannot be expressed as a descriptor by construction
   (proven by a `trybuild` compile-fail, the Sprint-5 DSL negative-proof
   shape). `serve.rs`'s bare `CREDENTIAL_ENV` constant is replaced by this
   typed, centrally-registered descriptor.
3. **The empty-env-var gap is closed**: an env var that is set but empty is
   treated as absent, not as a valid (empty) credential — `Authority::
   Credential` fails closed on it, same as if the var were unset entirely.
4. **`Dispatcher` gains a way to hold both roots at once** (the verified API
   gap) — either a combined constructor or self-taking builder methods;
   whichever shape, proven by a test that exercises a file-read *and* a
   file-chunk write on the *same* `Dispatcher` instance, the both-live case
   no existing test exercises.
5. **Compiled-in path constants for the quarantine root and the readable
   root**, const-validated for shape (non-empty; **absolute-only**, sharpened
   during planning: a relative path resolves against the process's current
   directory, an implicit, mutable, ambient input unlike every other
   authority in this design, which is explicit — one `const` assert removes
   the question rather than documenting a resolution base). The **default
   quarantine location is `/run/zerotrust-proxy/quarantine`**, verified
   during planning against the actual bench host (`elsenor`): `/tmp` carries
   none of the required mount flags, `/run` carries all three
   (`noexec,nosuid,nodev`) — and `serve.rs` already uses
   `/run/zerotrust-proxy/head.sock`, so this runtime directory is already
   the binary's natural home. **Corrected during planning, empirically**:
   `/run` itself is `drwxr-xr-x root:root` — **not** unprivileged-writable
   (confirmed live: an unprivileged `mkdir` under it fails `EACCES`) — so
   the quarantine directory at this default path must be **provisioned**
   before the binary runs as its unprivileged worker (Final State #8 names
   the owner). **Named residual, the operator's call, not decided here**:
   `/run` is tmpfs — quarantine contents are volatile across a reboot,
   which may or may not be the desired custody model; a dedicated
   persistent mount carrying the three flags is the alternative if
   durability matters more than the default's simplicity.
6. **`FilesystemRoots` and `KernelProtections` validate the SAME pinned fd
   — bind-the-chain, not three separate resolutions** (structural finding
   from planning, superseding an earlier draft that would have re-resolved
   the quarantine path three times — once to pin it, once to `statfs` it,
   once to serve from it — a real TOCTOU window on the ground the box
   writes to, and the exact "validated == used" gap this team's bind-the-
   chain discipline exists to close elsewhere). The shape: `FilesystemRoots`
   pins the quarantine root **once** via the certified `pin_root`
   (`open_discipline`, `O_PATH|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC` — the
   already-accepted final-component-only symlink discipline, not
   re-litigated here) and the readable root via a plain resolve +
   directory-exists check; `KernelProtections` then runs `fstatfs` on
   **that same pinned fd** (not a fresh path-based `statfs`), and the
   **same fd** is what reaches the `Dispatcher` (Final State #4's
   both-roots API already takes an owned fd, so this needs no new
   mechanism — only threading the one fd through instead of re-deriving
   it). `fstatfs` lands **inside `open_discipline`** alongside `pin_root`
   — MB6's `{open_discipline, privilege}` set is preserved by construction,
   not by choosing between a new unsafe module and an unverified safe
   alternative. **Retired during planning, empirically, on the real bench
   host (kernel `6.12.74+deb12-amd64`)**: `fstatfs` on `pin_root`'s exact
   `O_PATH|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC` fd works and correctly reports
   the flags — confirmed twice (a standalone C probe against glibc, then
   Rust against the locked `libc = 0.2.186`) — this is no longer a
   build-time risk Slice 4 carries. **Two concrete implementation payloads
   from the same verification, for whoever builds Slice 4**: (a) the
   locked `libc`'s plain `statfs`/`fstatfs` struct has **no `f_flags`
   field at all** on this target (gnu) — use `libc::fstatfs64` /
   `libc::statfs64`, the LFS64 pair, which does carry it (on musl the
   non-64 pair already carries `f_flags` and would be used instead — a
   portability note, moot since this target is gnu); (b) `libc` exports
   `ST_NOEXEC`/`ST_NOSUID`/`ST_NODEV` directly on this target — the check
   is `f_flags & (ST_NOEXEC | ST_NOSUID | ST_NODEV)`, no hardcoded magic
   numbers, which matters for a security check under close review. Both
   calls are `extern`/`unsafe`, so `open_discipline` remains the correct
   home exactly as stated above. Both roots validated **at the authority
   rung**, before serving — never lazily at first use. The bad-root
   enumeration is proven fail-closed (nonexistent path, a file where a
   directory is expected, unreadable/unsearchable, a final-component
   symlink on the quarantine root) **and the success arm is proven too**
   — a good pair of roots, on a mount actually carrying the required flags,
   must actually reach validated, so the check is shown to derive from the
   configured value and the real probed state, not to be a hardcoded pass
   or fail. (An empty string is excluded by Final State #5's
   const-rejection — if tested, it exercises the validator function's own
   totality directly, not a DSL path that could actually carry one.)
   **Corrected during planning, empirically**: the production quarantine
   root's *content* directory (`/run/zerotrust-proxy/quarantine`) does not
   exist unprivileged (Final State #5/#8), so the **unprivileged success-
   arm test** pins an *existing* noexec tmpfs directory read-only — `/run`
   itself, live-verified — rather than the not-yet-provisioned production
   path; this is a test-fixture distinction only; production still
   validates the fd for the actual configured quarantine root once
   provisioning (Final State #8) has created it, and that fd carries the
   same flags because it lives on the same `/run` tmpfs. **The Slice-3
   both-roots-live test (Final State #4, exercising a real file-chunk
   write) needs a *writable* quarantine and so uses a plain temp dir
   instead** — verified no single directory on this host is both
   unprivileged-writable and `noexec` (`/run/user/1000` is writable but not
   `noexec`; `/run` is `noexec` but not unprivileged-writable), so this
   split is **forced by the host, not a convenience**: `noexec` is
   `KernelProtections`' concern, writability is `FilesystemRoots`'/
   `Dispatcher`'s, and no test should need one directory to satisfy both.
   **A second fixture worth keeping for Slice 4's enumeration**: `/run/
   user/1000` fails `noexec` alone (carries `nosuid`+`nodev` but not
   `noexec`) — a genuine partial-failure case that proves the check tests
   all three flags independently, not just one (the same
   disjointness-alone-proves-nothing rider from Sprint 15's
   `dispatch.rs:653` completeness finding: a check passing one good case
   and failing one all-bad case cannot show it isn't testing only a single
   flag).
7. **`KernelProtections`'s required flags stay compile-time constants** —
   `noexec`/`nosuid`/`nodev`, no DSL surface, no config knob (including a
   "strict/relaxed" mode) to weaken them; only the `openat2` `RESOLVE_*`
   probe (already using the certified `open_discipline` primitive) and the
   `fstatfs` check (Final State #6) constitute this authority — no config
   value of its own.
8. **`serve.rs` wires the DSL values through the real composition root**,
   **factored into one function the binary itself calls** (sharpened
   during planning, so "the real path" has one unambiguous, hardware-free-
   testable meaning): validated, pinned roots → `Dispatcher`. **The
   function takes the root paths (and the credential env-var name) as
   parameters, not read internally from the DSL consts** — caught during
   planning: `serve.rs`'s `main` passes the compiled DSL consts as the
   arguments, while Slice 3/4's tests pass fixtures (Final State #6's
   `/run`-pin success-arm case, the bad-root cases) through the exact same
   function body. This is still "one real assembly path, no replica" —
   same function, same code, only the inputs differ — it just means the
   assembly function's boundary is where the DSL consts get consumed,
   not inside it. The credential descriptor's named env var is read at
   startup, closing the last placeholder.
   **New, load-bearing finding from planning**: `/run/zerotrust-proxy`
   (Final State #5's parent directory, already used unmarked for
   `head.sock`) does not exist on a fresh boot and nothing in the current
   binary creates it — confirmed live (`drwxr-xr-x root:root`, an
   unprivileged `mkdir` under it fails `EACCES`) — so `UnixListener::bind`
   already fails `ENOENT` before the ladder is even reached, and after
   this sprint the same missing directory is *also* the quarantine root's
   parent, doubling the consequence. This sprint **names an owner** for
   creating it (a `systemd` `RuntimeDirectory=zerotrust-proxy` unit
   directive, or a privileged `mkdir` in `serve.rs` before the privilege
   drop) rather than leaving it unowned; **which mechanism is the
   operator's deployment choice**, stated as a residual in Non-Goals, not
   picked by this sprint. **Sharpened during planning, connecting to the
   pin-permission residual above**: whichever mechanism provisions the
   directory must also make it **writable by the drop-target uid**, not
   merely existing — `systemd`'s `RuntimeDirectory=` defaults to
   root-owned `0755`, which the dropped uid cannot write to, and (per the
   residual above) that failure would not surface until the first
   confined write, well after the proxy has already reported `Served`.
   The provisioning step must pair `RuntimeDirectoryMode=`/`User=`/
   `Group=` (or an explicit `chown` in the privileged-`mkdir` path) with
   the directory's creation — existence and writability are one
   requirement, not two independently-satisfiable ones.
9. **The milestone is two separable claims, proven differently — sharpened
   during planning** (the evidence-model gap both dev and reviewer caught
   independently): (i) **all four authority rungs' validators and the
   assembly path are real, exercised through Final State #8's real
   assembly function against real filesystem state** (`fstatfs`/`openat2`
   probes running on genuinely pinned fds, a real credential, `Served`
   reached at the end of the ladder) — this is the sprint's substantive
   new claim, and it is **hardware-independent**: it's provable with a
   fake `NicRun` driving the real assembly function, fully hardware-free
   and fresh-re-runnable, because the NIC leg itself was already
   discharged live in Sprints 8 and 14. **Precision, corrected during
   planning**: this proves the *validators and the assembly path* are
   real, not that *the compiled default configuration* (`/run/zerotrust-
   proxy/quarantine`) is itself already valid on an unprovisioned host —
   that constant's own validation is contingent on Final State #8's
   provisioning step having run, and stays unproven until it has. Do not
   state claim (i) as "the shipped configuration is proven valid"; state
   what was actually exercised. This is the **primary, required**
   milestone proof, gated at Slice 4. (ii) **the real binary serves live
   on the bench** — a genuine hardware capture (`Subsystem::construct`, a
   connected head, a real credential, `/run/zerotrust-proxy` actually
   provisioned) — is **not required by this sprint**; if pursued, it
   follows the full Sprint-8/14 capture-model discipline (self-declared
   bin-sha, exact-BDF allowlist, operator paste-and-confirm, the
   reviewer's coherence/run-outcome split) explicitly, not implicitly.
   `serve.rs`'s held-boundary comment is updated to reflect claim (i)'s
   completion and name claim (ii) as the remaining, optional, held item.
10. The sprint document carries role retrospectives; the sprint is
    committed with only its intentional files.

## In Scope — ordered slices

**Slice 1 — the cleanup items, isolated.**
The five items (Scope Boundary #1), each independently verifiable, no
design risk. Gate: each change is doc/visibility-only; the existing test
suite passes unchanged (behavior-preserving).

**Slice 2 — the credential descriptor.**
`CredentialSource`, const-validated, env-name-only (Final State #2); the
empty-env-var-as-absent fix (Final State #3). Gate: the `trybuild`
compile-fail proving a literal secret cannot be expressed as a descriptor;
const-validation rejects an empty or malformed env-var name; a set-but-empty
env var is proven to fail `Authority::Credential` closed, same as unset.

**Slice 3 — the filesystem-roots DSL, API fix, and wiring.**
The both-roots `Dispatcher` API fix (Final State #4); the compiled path
constants (Final State #5); real `FilesystemRoots` validation, asymmetric
by design (Final State #6); wiring through `serve.rs` (Final State #8, the
roots half). Gate: the both-roots-live test (a read and a write on one
`Dispatcher`); the bad-root enumeration **including the success arm**
(REJECT a validator that passes/fails unconditionally without deriving
from the configured value — the circular-shadow tell); the newly-live file
channels proven on the real composition-root path with Sprint 15's
canary/nonce discipline (a confined reader genuinely rooted at the
*configured* path, not merely *a* path); REJECT any symmetrizing of the
quarantine/readable validation.

**Slice 4 — `KernelProtections` validation and the milestone.**
The real probe on the pinned fd, `fstatfs64`/`ST_NOEXEC`|`ST_NOSUID`|
`ST_NODEV` (Final State #6/#7); the credential's env-var wiring into
`serve.rs` (Final State #8, the credential half); the
`/run/zerotrust-proxy` provisioning owner named (Final State #8); the
milestone's **primary, required** claim (Final State #9(i)) — all four
authority rungs validate for real and `serve()` reaches `Served`, proven
hardware-free via a fake `NicRun` driving the real assembly function from
Final State #8; `serve.rs`'s held-boundary comment updated to reflect
claim (i)'s completion. The milestone's second claim (Final State #9(ii),
a genuine hardware capture) is **optional and separately named** — pursue
only if the operator wants it this sprint, under the full Sprint-8/14
capture-model discipline, and does not gate Slice 4's close either way.
Gate: REJECT any DSL surface for the required mount flags (including a
"strict/relaxed" mode — the same weakening under a different name); REJECT
claiming the primary milestone from a mocked authority rung (each of the
four must be its real, sprint-built validator, only the NIC leg faked, and
only because it was already discharged live in Sprints 8/14); the
bad-flags enumeration includes the `/run/user/1000` partial-failure
fixture (`nosuid`+`nodev` present, `noexec` absent) alongside the full-bad
(`/tmp`) and full-good (`/run`) cases, proving all three flags are checked
independently and not just one; the updated `serve.rs` comment is checked
against whatever barrier (if any) remains after this slice, and correctly
distinguishes claim (i) (done) from claim (ii) (optional, held).

**Carried from Slice 1's routed finding, folded in here**: `architecture.md`
§5's config/route-register-loader row (line ~127) contradicts §11's L9
(line ~626, "RESOLVED by construction... no longer exists") — a genuine
multi-site staleness (also lines 81, 288) that Slice 4's compiled-in DSL
makes both sharper and fully fixable in one pass, rather than fixing it
twice across two slices. **Reviewer's added catch**: §8/L12 (around line
422) cites the config-loader row as L12's placement anchor — if Slice 4
removes or rewrites that row, this cross-reference must be updated in the
same edit, or it dangles at a deleted anchor.

**Second carried finding, found auditing Slice 1's own accepted change**:
nine `Task 20` references remain live in `proxy/src` — eight describing a
dependency Sprint 14 already discharged (`serve.rs:119` directly
contradicts the module doc `serve.rs:19` Slice 1 itself just wrote;
`head.rs:361,381`; `upstream.rs:19,216,344,872`), one substantively wrong
rather than merely stale (`upstream.rs:578`, which asserts "the deadlock
case Task 20 exists to fix" — the exact deadlock Sprint 14 proved never
existed against the real `DdWaitHost`), and one benign
(`s5_capture.rs:108`, correctly names Task 20 as a knob's origin with no
stale claim — leave as-is). All are comment-only, no behavior. Fold into
Slice 4's cleanup alongside the §5/L9 fix; `serve.rs:119`'s self-
contradiction and `upstream.rs:578`'s false-deadlock claim are the two
that must land, the rest ride along.

## Non-Goals

- Do not add a config *value* for `KernelProtections` — validation-only,
  hardcoded required flags, no DSL surface, no knob of any kind.
- Do not add a file-path or any other non-env credential source variant —
  named as a future, deliberate operator widening (`§8` constraint 3), not
  built now.
- Do not validate the credential's secret *shape* (an expected prefix or
  length) — the upstream call is the real proof of a good credential.
- Do not build reload-without-restart — the register's promotion pattern
  (edit + recompile + restart) is this sprint's model too; `§11`'s
  reload-vs-restart row stays open.
- Do not symmetrize the quarantine and readable root's validation — the
  write path's pinned-fd confinement and the read path's plain resolve are
  deliberately different floors for deliberately different risk profiles.
- Do not pick the *deployment mechanism* that provisions
  `/run/zerotrust-proxy` before the unprivileged worker starts (`systemd
  RuntimeDirectory=` vs. a privileged `mkdir` in `serve.rs` before the
  drop) — this sprint names that an owner is required (Final State #8) and
  leaves the specific mechanism to the operator's deployment choice.
- Do not treat this sprint as resolving `architecture.md` §11's
  configuration-mechanics ledger row (L12) as fully closed — the DSL
  *dissolves* several of its named sub-questions by construction (no
  format, no on-disk location, no parser to demand total-function
  discipline of), which the retrospective should state precisely, but
  reload-vs-restart remains a live, separate residual.
- Do not add a permission probe (`faccessat` on the pinned quarantine fd)
  to catch a mode-000 quarantine directory **at the authority rung**.
  Found during Slice 3: `pin_root`'s `O_PATH` performs no final-component
  permission check, so an unusable-permissions quarantine directory
  currently pins successfully and is only caught **fail-safe, at the
  first confined write** (`FileChunkDeny`) — a real but narrow gap in
  *when* the error surfaces, not a security hole (the kernel enforces at
  `openat2` regardless of any probe). **Verified during planning why a
  naive fix would be actively harmful, not merely unnecessary**:
  `assemble` (which calls `pin_root`) runs at `serve.rs:112`, strictly
  *before* `bring_up`'s privilege drop (`lifecycle.rs:363`) — so the pin
  happens as root. A `faccessat` placed beside `pin_root` would ask "can
  root write here", root bypasses DAC (`CAP_DAC_OVERRIDE`), and the probe
  would **pass on the exact mode-000 case it exists to catch** —
  converting "silently unusable" into "explicitly validated as usable,
  then still fails," which is worse than no check. **If ever wanted**, it
  must run *post-drop*, against the retained fd (`faccessat` with
  `AT_EMPTY_PATH`, checked as the dropped uid) — inside a validation step,
  not beside the pin. Not this sprint's trade to add on top of an already-
  loaded Slice 4; named as an accepted residual with its correct future
  placement recorded, so it is not implemented in the wrong place later
  by someone reading only "add a permission probe."

## Starting Surface

Audit first, in this order:

1. `proxy/src/register.rs` and `proxy/src/vocab.rs`'s `Route::new`/
   `ToolIdentity::new` — the exact `const fn` + `assert!` pattern this
   sprint's new descriptors and path constants must match.
2. `docs/proxy/architecture.md` §8 (Configuration) — the load-bearing
   citation for both the credential-custody-not-value framing and
   `KernelProtections`'s absence from the configured set.
3. `proxy/src/lifecycle.rs`'s `Authority`, `AUTHORITIES`, and
   `validate_authority` — the four arms this sprint makes real (three of
   four; `RouteRegister` untouched).
4. `proxy/src/dispatch.rs`'s `with_readable_root`/`with_quarantine_root` —
   the verified mutually-exclusive constructor gap.
5. `proxy/src/open_discipline.rs`'s `pin_root` — the exact seam the
   compiled quarantine-root constant feeds, and where the new `fstatfs`
   primitive (Final State #6) lands alongside it.
6. `proxy/src/bin/serve.rs` — `Dispatcher::new()` with no roots (the dead-
   channel finding), `CREDENTIAL_ENV`'s bare-constant shape (to be replaced),
   the existing `/run/zerotrust-proxy/head.sock` binding (the precedent
   the quarantine default follows, and the unmarked missing-directory gap
   it shares), and the held-boundary comment (Final State #9).

## Method

- **MB1–MB6** (defined in `docs/proxy/review-vocabulary.md` — cited, not restated
  here) apply to every new line. MB6's `{open_discipline, privilege}`
  set is **preserved, not widened** — the DSL itself is pure data, and the
  one new primitive this sprint needs (`fstatfs` on the pinned quarantine
  fd, Final State #6) lands **inside `open_discipline`** alongside
  `pin_root` rather than opening a third unsafe module. Any `unsafe` the
  `fstatfs` FFI call itself requires is confined there and reviewed at the
  same bar as `pin_root`.
- **The register pattern's properties (C1–C4) are the bar for every new DSL
  entry**: invalid is a compile error, no runtime loader, the box cannot
  author it, a change is edit + recompile + review.
- **The four authorities are gated as the four different things they are**
  — a config value, a value-plus-validation, validation-only, and a
  descriptor-plus-runtime-value — never uniformly. A reviewer treating them
  the same is a sign the decomposition wasn't respected.
- **Bad-root proofs require the success arm.** An enumeration of failure
  cases alone cannot distinguish "validates correctly" from "always
  returns false" — the same `checked=0`-proves-nothing discipline this team
  has applied throughout. Every negative proof in this sprint pairs with a
  positive one.
- **The newly-live file channels are proven on the real composition-root
  path**, not only against hand-built test dispatchers — this sprint turns
  dead code live, and Sprint 8's "prove the live path, not a candidate one"
  standard applies in full.

## Acceptance

- **Slice 1.** REJECT any behavior change disguised as cleanup.
- **Slice 2.** REJECT any credential source variant beyond env-var-name;
  REJECT a descriptor that can express a literal value; REJECT treating an
  empty-but-set env var as a valid credential.
- **Slice 3.** REJECT a `Dispatcher` API that still cannot hold both roots
  simultaneously; REJECT a bad-root enumeration without a proven success
  arm; REJECT symmetrized root validation; REJECT closing `FilesystemRoots`
  without proving the file channels live on the real composition-root path.
- **Slice 4.** REJECT any config surface (including a mode/knob) for the
  required mount flags; REJECT a flags check proven only by an all-good
  and an all-bad case (must include the `/run/user/1000` single-flag-
  absent fixture); REJECT claiming the primary milestone (Final State
  #9(i)) without all four rungs validating for real end-to-end on the
  real assembly function, hardware-free; REJECT presenting an optional
  hardware capture (#9(ii)) as required, or skipping the Sprint-8/14
  capture-model discipline if it is pursued; REJECT leaving
  `/run/zerotrust-proxy`'s provisioning unowned; REJECT leaving
  `serve.rs`'s held-boundary comment stale against whatever barrier
  remains after this sprint.
- **Sprint-wide, all reports and the retrospective.** REJECT any report,
  status line, or retrospective wording that flattens Final State #9(i)'s
  hedge back into "all four authorities validate" or "the proxy is
  servable" without the provisioning contingency attached — the compiled
  default quarantine constant stays unproven until Final State #8's
  provisioning step has actually run. The same class of guard Sprint 15
  needed to stop "transport built" from reading as "head connected."

## Review Questions

None outstanding. The credential-custody and `KernelProtections`
decompositions were settled during planning by direct citation of
`architecture.md` §8; the filesystem-roots API gap and the TOCTOU-via-
triple-resolution finding are verified facts of the current source, closed
by the single fd-bind-the-chain fix (Final State #6), not open design
forks. The milestone-evidence question (dev and reviewer independently
flagged the same gap: what does "proven on the real path" mean when the
NIC leg needs live hardware?) is resolved directly rather than escalated
— splitting it into a required hardware-free primary proof plus an
optional, separately-gated hardware capture matches the precedent already
set at Sprint 13's software/hardware slice split, so it is treated as an
in-group representation call, not a fresh architectural fork for the PM.

## Retrospectives

### zero.proxy-lead

The planning phase carried more real weight than
usual: dev's and reviewer's independent findings converged on the same three
fixes from different angles (the fd-bind-the-chain design closed both the
TOCTOU finding and the MB6/unsafe-placement contradiction at once; the
milestone split answered both the evidence-model gap and the scoping-fork
question at once) — recognizing that convergence early kept the contract
from carrying two redundant, differently-worded fixes for one problem.
Reviewer's post-close-of-planning catch (the assembly function had to be
*parameterized*, not just factored into one body, or the hardware-free
milestone proof would have been structurally unreachable) was the sharpest
single finding of the sprint — it would have surfaced mid-Slice-4 as a
forced refactor instead of a one-sentence contract fix, had it not been
caught before Slice 3 started. The discipline that paid off most: verifying
every load-bearing claim against source myself before routing a gate,
including running the actual test suite and clippy at every slice rather
than trusting reported counts — this caught one real arithmetic slip
(188 vs. dev's claimed +5) and confirmed, line-for-line, several claims
(the §8 normative sentence preserved verbatim, the exact grep count on the
Task-20 sweep, the ordering that made the faccessat-as-root finding
correct) that would otherwise have been taken on report alone. **Inherit**:
when dev and reviewer converge on a design fix from independent angles
without prompting each other (they never communicate laterally), that
convergence is strong evidence the fix is right — adopt it directly rather
than re-deriving it. Also inherit the now-standing residuals this sprint
named but did not close: the O_PATH permission-probe placement (post-drop
only, if ever wanted), reload-without-restart (L12), and the optional
hardware capture (claim (ii)) — none are blockers, all are legitimate
future openings.

### zero.proxy-dev

**What I built.** Four slices: the five cleanup items; the `CredentialSource` descriptor and the
empty-env-var fix; the filesystem-roots DSL with the both-roots `Dispatcher` API fix, the one
composition-root assembly, and real `FilesystemRoots` validation; then `KernelProtections`, the milestone,
and the cleanup backlog. The sprint's shape held, but three of the four slices were changed by evidence
found while building them, and one of my own proposals turned out to be actively wrong.

**Bind-the-chain: one design closed two independent findings.** Reading the drafted contract I flagged
that `serve()`'s validator returns `bool`, so no rung can yield the artifact it validated — which would
have forced the quarantine root to be resolved three separate times (pinned, `statfs`'d, then pinned
again for the `Dispatcher`). Three resolutions of a mutable path is a TOCTOU window on the exact ground
the box writes to. The fix was this team's own net-path discipline turned sideways: **validated == probed
== used** — pin once, `fstatfs` *that* fd, hand *that* fd (a `dup`, one open file description) to the
dispatcher. What made it worth the redraft is that reviewer had independently flagged an MB6 contradiction
in the same area — a path-based `statfs` would have wanted a second unsafe home — and the single fd-based
design dissolved both at once, because `fstatfs` naturally lands beside `pin_root` inside
`open_discipline`. Two findings from two directions, one structural answer. That is usually the signal a
design is right rather than merely adequate.

**I proposed a fix that would have been worse than no fix.** In Slice 3 I found that `pin_root`'s `O_PATH`
performs no permission check on the final component, so a mode-000 quarantine directory pins successfully
— and I suggested closing it with an `faccessat` probe beside `pin_root`. Reviewer showed why that is
wrong, and the lead verified it: `assemble` runs *before* the privilege drop, so the probe would execute
as root, and root bypasses ordinary permission checks — the check would have **passed on exactly the case
it existed to catch**, converting a known gap into a false assurance. I had reasoned about *where the
capability belongs* (the unsafe module) and not about *whose credentials the syscall runs with*. The
durable lesson: a permission probe is only meaningful when it runs as the identity that will later do the
work, and in a privilege-dropping process that makes probe placement a *timing* question before it is a
module question. The gap is now an accepted residual with the correct shape recorded for whoever picks it
up — post-drop, against the retained fd, as the dropped uid.

**Two API-shape choices where the stronger option removed the bug class instead of the bug.** The roots
constructors could not both be set (each was an associated constructor that defaulted the other away), and
the contract allowed either a combined constructor or self-taking builders. I took builders: a combined
constructor fixes the instance while leaving the hazardous pair sitting beside it, whereas a method that
consumes `self` has *nothing to reset* — the failure becomes unwriteable rather than merely unwritten. The
same instinct shaped `CredentialSource`: it carries a variable *name* in a private field with no
value-carrying constructor, so a compiled-in secret is not discouraged, it is **inexpressible**, proven by
a `trybuild` bypass audit rather than asserted in a comment.

**Evidence that changed the plan, and one thing that simply corrected me.** Verifying the contract's own
"confirm, don't assume" note on `fstatfs`-over-`O_PATH` turned up that `libc::statfs` has no `f_flags`
field on this target at all — only `statfs64` does — which would have cost a debugging cycle mid-Slice-4
had I taken the obvious call. Testing the empty-env-var rule was *impossible* as first imagined, because
`std::env::set_var` is `unsafe` under edition 2024 and the crate root is `deny(unsafe_code)`; that
constraint forced the resolution policy into a pure, total function with the env read as a thin shell —
a better design arrived at by prohibition rather than foresight. And in Slice 3 a test of mine failed
because **my assertion was wrong, not the code**: I expected a rootless dispatcher to refuse a
`file-write`, but that verdict is the root-independent tier-1 leaf check, with the confined open lazy at
the first chunk. I moved the proof to the chunk. Worth recording that the suite's job includes correcting
the person writing it.

**The hedge is the deliverable, not the caveat.** The milestone's honest claim is narrow — all four rungs
*validate*, the ladder reaches `Served` — and the two things it does not claim matter more than the claim:
the live round-trip stays held, and **quarantine writability is not an authority**. A root-owned `0755`
quarantine satisfies every rung, reports `Served`, and still fails at the first `file-chunk`, and no rung
can catch that for the same root-runs-the-pin reason above. Writing that up sharpened the lead's
provisioning finding into something more specific than we started with: `systemd`'s `RuntimeDirectory=`
*cannot* be made to solve it by setting `User=`, because it takes ownership **from** `User=`, which must
be `root` for the privileged NIC bring-up — so it always needs an explicit `chown` beside it. A
correct-sounding one-line deployment fix that does not work is exactly the kind of thing an operator
discovers at 2am, so it is documented in both places they will look.

**What the next sprint inherits.** Named and live: **reload-without-restart** (the whole of L12's
remaining residual — the compiled-in model dissolves format, location, parser, and take-effect, and
deliberately does not attempt reload); the **O_PATH permission gap**, with its correct post-drop shape
recorded; **quarantine volatility** on `tmpfs` and the **provisioning mechanism**, both operator calls;
the **readable-root default**, provisional and unowned by any requirement; and the **live round-trip**
(claim (ii)) under the standing capture-model discipline. Nothing here is a surprise — each was named as
it was found rather than discovered at the gate, which is the part of this sprint I would want repeated.

### zero.proxy-review

**What I checked.** Every claim in this sprint was re-derived from source and my own
build rather than from a report: the test counts summed from my own runs (179 → 182 →
188 → 191), the trybuild fixtures read rather than trusted (the bypass fixture attempts
an actual struct literal; the malformed fixture's expected stderr carries the exact
assert message, which is what proves the *validator* fired rather than an unrelated
const error), the live path traced end to end for each new mechanism, and the MB6
allow-list confirmed at exactly two modules. Two checks I committed to at contract time
and applied at the gate: that `lifecycle::assemble` is genuinely **one** function body
called by both `serve.rs` and the tests — the property the whole hardware-free milestone
claim rests on — and that the pinned quarantine fd is `try_clone`d rather than
re-derived, so `pinned == probed == used` is structural across both filesystem rungs
rather than conventional.

**What evidence changed the plan — the sprint's recurring shape.** Four times, a
measurement rather than an argument redirected the work, and in each case the reasoning
alone had pointed somewhere plausible and wrong. Reading the two root constructors
showed both build from `..Dispatcher::new()`, so **no `Dispatcher` could hold both roots
at once** — turning "thread a path constant through `serve.rs`" into a real API fix,
where the constant-threading alone would have left one file channel silently dead with
no existing test able to catch it. Running an actual `mkdir` under `/run` returned
`EACCES`, correcting a stated mechanism ("unprivileged-writable") to the one that works
(pin an *existing* protected directory; `pin_root`'s `O_PATH` needs only search
permission, never write). Tracing the call ordering showed `assemble` runs *before*
`bring_up`, so the pin happens as root while writes happen as the dropped uid — which
meant a proposed `faccessat` probe would have passed on exactly the case it was added to
catch, converting "silently unusable" into "explicitly validated, then still fails," a
worse outcome than no check. And a repo-wide grep found the stale `Task 20` claim in
**nine** places, not the one the diff showed.

**What I got wrong.** I accepted Slice 1's held-boundary cleanup after verifying the
replacement text was accurate — and the same file still contradicted itself a hundred
lines below, because for an item whose purpose is *retiring* a claim, accuracy of the
replacement is not the gate; **absence** of the claim is, and a diff-scoped read
structurally cannot establish absence. That check needs a file- or repo-scoped search,
and I ran it only after the fact. Separately, my proposed remedy for the provisioning gap
(`RuntimeDirectoryMode=` plus `User=`/`Group=`) would not have worked: `RuntimeDirectory=`
takes ownership from the unit's `User=`, which must be `root` here for the privileged NIC
bring-up, so the directory is root-owned whatever the mode — and a mode of `0777` would
"fix" writability by making it world-writable. The correction came from dev and is right;
my version would have sent an operator down a path that quietly does not work.

**What the next sprint inherits.** Three named residuals, none hidden: the quarantine
must be **provisioned writable by the drop-target uid** (fail-closed until then, denied at
the first `file-chunk`, and `RuntimeDirectory=` alone cannot satisfy it); the **live
hardware round-trip** remains the optional, unproven claim (ii); and **reload-without-
restart** stays L12's live residual. Beyond those, one reusable review lesson: this sprint
produced two individually-correct items — "name a provisioning owner" and "writability is
not an authority" — whose *defaults composed into a broken state*, a proxy that reaches
`Served` and then cannot write a single chunk. Per-item review has a structural blind spot
for that: each item passes its own gate, and the conflict lives only in their interaction.
Worth carrying forward as an explicit question at any gate that closes two related
residuals at once — not "is each correct?" but "what do their defaults do together?"
