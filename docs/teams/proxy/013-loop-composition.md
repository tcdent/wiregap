# Sprint 013 — Composing the Production Loop (Phase 4, closing)

> Proxy team, sprint 13, unifying Sprint 11's upstream loop and Sprint 12's head loop
> into the one production event loop, and standing up the composition-root binary
> that makes a runnable proxy visible for the first time. Drafted by
> zero.proxy-lead from planning input by zero.proxy-dev and zero.proxy-review
> (2026-07-23).

---

## Scope Boundary

This sprint owns unifying three previously-separate, sibling adapters
(`lifecycle::run_adapter`, `upstream::run_adapter_with_upstream`,
`head::run_adapter_with_head`) — each already accepted, each following the
identical `drain → route → maybe-kick → pump` shape but each owning its own
`Dispatcher` — into **one** production loop, and building the composition-root
binary that wires the real components together for the first time.

1. **The composed adapter** — one `Dispatcher`, one closure draining both the
   upstream ingress and the head ingress every tick, routing the single box
   event, kicking the upstream call / sending to the head exactly as each sprint
   already built, and one `pump_replies` call.
2. **The composition-root binary** — a real, runnable proxy: `RealNic` +
   `AnthropicClient` + a real `HeadConnection` + the credential + the existing
   fail-closed ladder, reusing the exact hardware-safety posture already
   established for `s5_capture`.

It explicitly does **not** own:

- **Task 20 (the reactor's bounded-park cap).** Still with the NIC team. This
  sprint's composed loop *logic* is fully provable without it (fake-driven
  ticks); the live wake is the sprint's one held item, and composing the two
  loops makes explicit that Task 20 now gates **both** ingresses' quiescent-
  arrival liveness, not upstream alone (below).
- **Clean worker/thread teardown on loop-exit.** Named as an explicit follow-on,
  consistent with Sprints 11 and 12 both deferring it — now scoped to the full
  set (upstream worker *and* head reader/writer threads), not one seam's.
- **The configuration DSL.** The composition-root binary's credential is a thin,
  clearly-marked interim env-var read passed straight to `Credential::new` — not
  a config loader, not a parser, not the DSL Sprint 8 deferred.
- **Any new fairness mechanism** (round-robin or N-per-tick draining across the
  two ingresses). The bounded cross-ingress-drain coupling this sprint
  introduces (below) is accepted as a named, bounded residual, not solved with
  new mechanism.

## Problem

Three adapters exist, all accepted, all following the same shape, each proven in
isolation against a fake: `run_adapter` (the original receive-then-reply loop,
Sprint 8), `run_adapter_with_upstream` (Sprint 11, adds the upstream ingress),
`run_adapter_with_head` (Sprint 12, adds the head's two channels). Each owns its
own `Dispatcher` — meaning a real proxy today can drive *one* of these loops, not
all three capabilities at once. Nothing has yet combined them, and nothing has
yet built a `main` that constructs the real components (`RealNic`,
`AnthropicClient`, a real `HeadConnection`, the credential, the register) and
serves.

Planning surfaced that the composition is mostly mechanical (both dev and
reviewer independently confirmed the core concurrency invariant — one
single-threaded `Dispatcher`, the two channels as the only thread boundaries, no
shared mutable state — is unchanged by combining the loops), but three things
need naming precisely, not glossed:

1. **The `replies`-map disjointness rests on a named invariant, not
   "separate by construction."** Upstream keys the map by the box's net-request
   session; head keys it by the display session `s_display`. These never
   collide only because one box `IdAllocator(Low)` mints monotonic-unique ids
   across *all* box services (net, file, display) within an epoch, and one
   proxy `IdAllocator(High)` does the same for reply sessions. The guarantee is
   the allocator's uniqueness, not an accident of the two ingresses being
   separate.
2. **A genuinely new fairness axis: ingress-drain order**, distinct from the
   already-named Sprint-10 reply-*pump* residual. Each ingress fully drains
   before the other and before the shared pump, each tick — so a large
   streaming upstream response or a head-input burst can delay the other
   ingress's processing within that tick. Both channels are bounded (head 256,
   upstream 64), so the effect is bounded, not unbounded — **accepted as a
   named residual this sprint, not solved with new draining mechanism** (see
   Non-Goals).
3. **The Sprint-10 reply-*pump* residual's blast radius widens.** One
   `pump_replies` now drains net-response, terminal-input, and file replies in
   one map-key-ordered pass (the `replies` map's key is the box-Low
   request/display session, not the lazily-opened proxy-High `reply_sid`), so
   its already-known bounded-starvation property (a lower box-session key can
   pace-starve a higher one until it completes) now applies *cross-service* —
   a net-response stream could pace-delay a terminal-input stream by key
   order. The mechanism is unchanged; where it bites is wider.

Also surfaced, and load-bearing for the held boundary: composing the two loops
makes the head-side liveness gap **unavoidable** in a way Sprint 12's isolated
tests never confronted. A quiet box (waiting on an operator's next keystroke)
parks the reactor unbounded absent Task 20; an arriving keystroke sits undrained
on the head ingress with nothing to wake the loop — the same deadlock shape as
the upstream's quiescent-response case, just previously masked because Sprint
12's tests drove `Idle` ticks synthetically. The production loop cannot do that;
it must actually wake.

## Goal

Build the one production loop that drives both the upstream and head
capabilities together, with every piece of *logic* fresh-verifiable against
fakes, and stand up a real, runnable composition-root binary — while stating
plainly that Task 20 now gates the live liveness of **both** ingresses, and the
binary makes a proxy *visible*, not *proven live*.

## Final State

1. One `Dispatcher`, one composed adapter closure: each tick, drain the upstream
   ingress (non-blocking) and the head ingress (non-blocking) — order is a free
   choice, not load-bearing (Non-Goals) — route the single box `Event`, kick the
   upstream call on `NetPayloadAdmit` / send to the head on a box→head display
   exactly as Sprints 11 and 12 already built unchanged, then one
   `pump_replies` call.
2. **No shared mutable state is introduced by the composition**: the two
   channels (upstream mpsc, head's two channels) remain the only thread
   boundaries; both ingress drains and the route/pump borrow `&mut Dispatcher`
   sequentially within the single-threaded closure, never concurrently — no
   `Arc`/`Mutex`, no second thread touching the `Dispatcher`.
3. `bring_up`'s growing argument list (already past clippy's too-many-args
   threshold before this sprint) is refactored to a config struct —
   **behavior-preserving**: every rung's semantics, ordering, and fail-closed
   behavior are identical; only the argument-passing shape changes.
4. The composition-root binary constructs the real `RealNic`, `AnthropicClient`,
   a real `HeadConnection` (accepted from a `UnixListener`), and the credential
   (a thin, clearly-marked interim env-var read — not a config loader), and
   serves through `bring_up` + the composed adapter. It reuses **`s5_capture`'s
   exact hardware-safety posture**: gated on the exact bench BDFs via
   `Bdf::parse` literals (never device-id enumeration), feature-gated so it is
   compile-absent from a normal build, operator paste-and-confirm before any
   hardware touch.
5. **Task 20 is the one unified held item**, gating the live quiescent-arrival
   liveness of **both** ingresses — a head keystroke and an upstream response
   deadlock identically absent it. Until Task 20 lands, the production loop
   deadlocks on a quiescent arrival from either direction; this is stated
   plainly, not softened by the binary's existence.
6. The sprint document carries role retrospectives; the sprint is committed
   with only its intentional files.

## In Scope — ordered slices

**Slice 1 — the composed adapter, isolated (pure software, fully provable).**
One `Dispatcher`, the union of the three adapters' bodies, the `bring_up`
config-struct cleanup, and **one integration test that drives both ingresses
active and interleaved within the same loop** — a fake upstream response *and*
a fake head input both flowing through the one composed adapter in the same
run, not each ingress re-run through the composed loop in isolation (which
would prove only that composition compiles, not that the two coexist). Gate:
disjoint routing (the head input lands on its `s_display` stream, the upstream
response on its net-session stream, neither misrouted to the other); no
interference (one does not corrupt, drop, or block the other); the head-side
injectable-frame fail-safety (Sprint 12) holds under composition too; the
`bring_up` cleanup proven behavior-preserving (existing lifecycle tests pass
unchanged); no shared mutable state introduced (Final State #2).

**Slice 2 — the composition-root binary.**
Wires the real components through `bring_up` and the composed adapter, reusing
`s5_capture`'s exact safety posture (Final State #4). Gate, the Sprint-8 IB
shape: the binary compiles and type-checks the wiring; the fail-closed ladder
holds with real authorities (re-run with a missing authority closing the whole
proxy, not just its channel); the credential's interim env-var source is
clearly marked, never silently mistaken for the eventual config DSL. Held:
the live end-to-end round-trip (no real head/hardware in the test
environment) — capture-model, same discipline as every prior hardware-touching
sprint. This slice makes a runnable proxy *visible*, not a *proven-live* one.

## Non-Goals

- Do not add a new fairness mechanism (round-robin, N-per-tick draining) for the
  ingress-drain-order coupling this sprint introduces — accepted as a named,
  bounded residual (the channels are bounded; realistic load — slow keystrokes,
  bounded upstream chunks — favors accepting it), revisited only if a real
  workload demonstrates it biting.
- Do not build clean worker/thread teardown on loop-exit — named as an explicit
  follow-on, now scoped to the **full** set of owned threads (the upstream
  worker *and* both head reader/writer threads), not one seam's.
- Do not invent a configuration system for the binary's credential, FS roots,
  or any other still-placeholder authority — the credential is a thin marked
  env-var read; the remaining config-pending authorities (Sprint 8) stay
  exactly as placeholder as they are today.
- Do not claim the runnable binary proves a live round-trip — it proves the
  wiring compiles and the ladder holds; the live proof needs Task 20, a real
  operator credential, and a real head, all still held.
- Do not re-litigate the Sprint-10 reply-pump drain-order residual as part of
  this sprint — its blast radius widens (cross-service) but its mechanism and
  disposition are unchanged.

## Starting Surface

Audit first, in this order:

1. `proxy/src/lifecycle.rs`'s `run_adapter`, `proxy/src/upstream.rs`'s
   `run_adapter_with_upstream`, `proxy/src/head.rs`'s `run_adapter_with_head` —
   the three sibling adapters this sprint unions into one.
2. `proxy/src/dispatch.rs`'s `replies` map and the `IdAllocator`
   (`transport/src/allocator.rs`) — the invariant the composed map's
   disjointness rests on (Problem #1).
3. `proxy/src/bin/s5_capture.rs` — the exact hardware-safety posture (exact-BDF
   gating, feature-gating, operator-confirm) Slice 2's binary reuses verbatim.
4. `proxy/src/lifecycle.rs`'s `bring_up` — the growing argument list this
   sprint refactors to a config struct.
5. `proxy/src/upstream.rs`'s `Credential` and `proxy/src/lifecycle.rs`'s
   `Authority::Credential` — what the binary's env-var-sourced credential feeds.

## Method

- **MB1–MB6** apply to every new line. MB6 stays `{open_discipline, privilege}` —
  the composition adds no new proxy-owned `unsafe`.
- **The core concurrency invariant is the thing to protect, not just check
  once**: one single-threaded `Dispatcher`, exactly the existing channels as
  thread boundaries. Any change that introduces `Arc`/`Mutex` or a second
  thread touching the `Dispatcher` to "share" state between the two ingress
  paths is a reject, not a style note.
- **The evidence split, Sprint-8/11/12-shaped**: Slice 1 is pure software, fully
  hardware-free and fresh-re-runnable — no held item at all. Slice 2's held
  item is exactly the hardware-touching capture-model boundary this team has
  used every time real hardware is involved, reusing `s5_capture`'s specific
  posture rather than inventing a new one.
- **Task 20's framing is unified, not restated per-ingress**: it is the single
  cross-team dependency gating the live liveness of both the upstream and head
  ingresses, named once at the sprint level.

## Acceptance

- **Slice 1.** REJECT an integration test that re-runs each ingress in
  isolation through the composed loop rather than proving both active and
  interleaved in the same run; REJECT any shared mutable state (`Arc`/`Mutex`,
  a second Dispatcher-touching thread) introduced to compose the loops; REJECT
  a `bring_up` cleanup that changes rung semantics, ordering, or fail-closed
  behavior rather than only the argument shape.
- **Slice 2.** REJECT a binary that does not reuse `s5_capture`'s exact
  hardware-safety posture (exact-BDF `Bdf::parse` literals, feature-gating,
  operator-confirm); REJECT a hardcoded or config-DSL-shaped credential source
  in place of the marked interim env-var read; REJECT any claim that the
  binary's existence proves a live round-trip. **The env-var read is a new
  credential ingress point, so Sprint 11's no-leak invariant carries to it
  explicitly** (added at review): REJECT the read itself printing or logging
  the credential value (no "read KEY=..." diagnostic), and confirm the value
  only ever reaches the redacted `Credential` type, never a box-facing frame
  (the box cannot read the proxy's environment regardless, but the invariant is
  stated at every new ingress point, not assumed).
- **Sprint-level.** REJECT any report or retrospective that frames Task 20 as
  gating only the upstream ingress now that the loops are composed — it gates
  both, stated explicitly.

## Review Questions

1. **Task 20's status** (with the PM, checking with the NIC team as of this
   draft): the sprint proceeds regardless — Slice 1 has no dependency on it,
   and Slice 2's held boundary is unaffected by its timing.
2. **The ingress-drain-order coupling** (resolved during planning, not
   pre-decided): accepted as a named, bounded residual rather than solved with
   new fairness mechanism — revisit only on evidence of real-workload impact.
3. **Clean worker/thread teardown** (named, deferred, consistent with Sprints
   11/12): scoped to the full set of owned threads once it is picked up, not
   one seam's.

## Retrospectives

### zero.proxy-dev

**What I built.** Three slices. Slice 1: `run_adapter_composed` — the union of the three sibling adapters
(`run_adapter`, `run_adapter_with_upstream`, `run_adapter_with_head`) onto ONE `Dispatcher`, plus the
`bring_up` config-struct cleanup and one integration test driving both ingresses active and interleaved.
Slice 1.5: the retain-on-full backpressure fix at both ingress push sites (below). Slice 2: the
composition-root binary — the first runnable proxy, feature-gated, reusing `s5_capture`'s exact safety
posture.

**"Mechanical" describes the code change, not the risk.** Both I and review independently framed this
sprint as mostly mechanical — union three functions that already share the `drain → route → kick → pump`
shape onto one dispatcher — and that framing was *correct about the code*: the core concurrency invariant
(one single-threaded `Dispatcher`, the channels as the only thread boundaries, no shared mutable state)
survived the union genuinely untouched, no `Arc`/`Mutex`, no second thread. But composing is exactly what
turned a latent Sprint-11 defect from theoretical to load-bearing: `drain` pulled a chunk off the mpsc and
handed it to a full reply queue that refused it, dropping it — silent loss on a reliable stream. The
composition didn't *introduce* the bug; it *promoted* it, by making that drain THE production path for the
first time. Review surfaced it (their Sprint-11 miss, owned cleanly); my job was to not let the
"mechanical" label lull me into trusting the combined path, and to context-switch from
combine-three-functions to a real data-loss fix without treating the fix as scope creep. The durable
lesson: a sprint can be mechanical in its diff and still be the moment a dormant single-component bug
becomes a live hazard — the wiring-up is precisely the risk event.

**Bounded ≠ lossless, from the implementer's side.** The fix is the wire-side `ReplyStream.pending`
pattern turned around onto the INGRESS side: on `Enqueue::Full`, retain the pulled frame in a one-slot
pending and STOP draining, so the mpsc (or head channel) fills and the producer — the worker's `send`, the
socket reader — backpressures. Real end-to-end flow control replaces a silent drop. The reliable-vs-
droppable distinction (`terminal.md` §4) is what decides drop-vs-backpressure per site: head EGRESS
correctly drops (display is droppable), but net-response and head-INPUT are reliable ordered streams, so
both get the retain-and-backpressure treatment. I applied it to both named sites and extended it to the
terminal-FAILURE frame in the same `drain` (an incomplete fix of one function would be its own defect) —
the lead approved the extension; review sharpened the one same-class site I left alone (`notify_deny`)
from "best-effort/tolerable" to structurally *unreachable*. The test lesson I'll carry: to gate "bounded"
on a reliable stream, the required shape is a backpressuring-consumer test (a fake sink that `WouldBlock`s
N frames then releases, asserting every item delivered in order none lost) — a fast fake consumer proves
the bound but leaves the drop path `checked=0`, which is exactly how this hid for two sprints.

**Five judgment calls in Slice 2 — surfacing over deciding, on a future-shaped surface.** The
composition-root binary had no anchor to reason *from* — a runnable proxy that didn't exist yet, every
config value a fork with no committed answer. The honest move at each was to make the in-posture choice,
mark it in the code, and route it up: (1) a new `serve` feature rather than reusing `bench-capture`
(reuse the *posture* — feature-gated, compile-absent — not the specific feature a composition-root needs
none of); (2) `allowlist=[local]` only, tighter than s5_capture's two-card list, correct for a single-role
binary; (3) marked placeholder config (zeroed `PEER_BOX_MAC` failing loud, uid/gid matching precedent)
pending the config DSL; (4) two forced consequences of the committed `bring_up` shape — head accepted
before the credential check, a never-invoked empty-key client when the credential is absent — noted, not
worked around; (5) the exit code on a `Closed` readiness. The lead ruled four "approved / correctly
flagged" and one CHANGE: exit non-zero on fail-closed, because a proxy that silently exits 0 without
serving is an operational hazard for systemd/monitoring (the library `Ok(Readiness::_)` return stays
right — it's a *binary-level* signal, not a Rust error). The pattern worth naming: four of my five reads
were right, but I could not have told *which four* from the inside — my confidence was highest on the
fifth (exit 0 "matches the library semantics"), which was the wrong one. Getting small at each fork
("here's my read, here's why, your call") is what let the one wrong call get caught cheaply by the person
who owns the operational picture, instead of shipping it.

**A named operational follow-on.** Review flagged (non-gating) that the `UnixListener` needs
unlink-before-bind eventually — a second run leaves a stale socket file that fails the bind. Out of scope
for this held/visible binary; captured here so it's tracked for whenever the binary graduates from
*visible* to *operationally run*.

### zero.proxy-review

Three things from this sprint are worth carrying forward.

**Owning a miss cleanly, and turning it into a shared guard.** Verifying my own
"the bound is real" check on the composed loop surfaced a net-response DATA-LOSS
bug: the upstream `drain` handed a chunk it had already pulled off the channel to a
full reply queue that refused it, dropping it — silent corruption of a reliable,
ordered stream under box-backpressure. It was latent since Sprint 11, and MY
Sprint-11 gate had certified the BOUND (buffered ≤ cap) but not LOSSLESS delivery,
because every fake box in the suite returns `Ok` from `send` and never
backpressures, so the drop path was `checked=0`. "Bounded" and "lossless under
backpressure" are different properties; a bounded buffer that DROPS on full needs a
SLOW/BACKPRESSURING-CONSUMER test (a fake sink whose `send` returns `WouldBlock`) to
exercise the drop. I owned that plainly, named the fix (retain-one-slot + stop-drain
so the mpsc fills and the worker blocks — real end-to-end backpressure), and the
lesson is now a shared memory note so the next reviewer catches this class earlier.
The fix landed (Slice 1.5) with exactly that regression test, both directions.

**Reachability, not "best-effort," as the axis.** When `notify_deny` was left
un-fixed as the same-class drop-on-full, the framing "it's best-effort/droppable"
was true but soft. The sharper — and correct — reason is REACHABILITY: `notify_deny`
pushes a single frame on the DENIED session's reply queue, and a session being denied
has no streaming replies queued, so that queue is empty and Full cannot arise. Net-
response fills its queue precisely BECAUSE it streams; a deny cannot. Same code
pattern, opposite disposition, decided by whether the overflow is reachable — with the
honest residual named (revisit if any future change makes a deny fire on a session
with a backed-up reply stream).

**Re-deriving already-approved rulings independently.** Five items were surfaced and
ruled during Slice 2, four already approved. Independence means re-deriving each from
source anyway, not taking the ruling: I verified that the never-invoked empty-key
client genuinely never reaches the wire (the ladder closes at the authority rung
BEFORE `run()` is entered, so `spawn` is never called), that the zeroed peer MAC is
unroutable (fails at the transport rather than mis-addressing), and that
`Readiness`/`Authority` structurally cannot carry the credential value into a log
line. A ruling being settled upstream is not a reason to skip the check — it is the
verdict's whole worth.

Sprint 13 closes with a runnable, VISIBLE proxy over the one LOSSLESS production
loop. Two items carry forward as named: Task 20 (the reactor bounded-park wake, the
NIC team's, gating both ingresses' live liveness) and clean worker/thread teardown
(now the full owned-thread set). The live real-head/hardware round-trip stays held,
same discipline as every prior hardware-touching sprint.

### zero.proxy-lead

**A sprint framed as mechanical still got the full evidence discipline, and that is
exactly why it stayed small instead of shipping a real defect.** Nothing about this
sprint's contract — "unify three already-accepted adapters" — signaled risk, and it
would have been easy to let the gate move faster because the diff looked like
plumbing. It didn't: reviewer's own light-check on the ingress-drain-order accept
("is the bound real") is what surfaced the net-response data-loss bug, not a
dedicated adversarial pass. **Rule reconfirmed, with a sharper edge this time: a
task's *apparent* size is not a signal to relax verification — the size of the
diff and the size of the risk are different axes, and this sprint is the clearest
instance yet of a supposedly-small diff carrying a real, common-case correctness
defect.**

**Ruling on Slice 1.5 (fix now, before Slice 2) was the easy half of the job; the
harder half was recognizing the two follow-on questions dev surfaced were already
answered by standing precedent.** Extending the fix to `Failed` frames and leaving
`notify_deny` untouched both resolved by citing decisions this team had already
made (a function shouldn't ship half-fixed; JC-3 notifications were already ruled
best-effort at Sprint 10's gate) — genuine judgment calls that turned out not to
need fresh judgment. The discipline that made this fast: check whether a lean
matches an existing ruling before treating it as a new fork, the same move that
resolved Sprint 9's file-write completion "fork" by simply reading `file.md` §1.

**What the next sprint inherits.** Task 20 is now unambiguously the single
cross-team dependency blocking a live proof of the whole composed proxy — not one
ingress's problem, the production loop's. The teardown follow-on is scoped to the
full thread set (upstream worker, head reader, head writer) for whoever picks it
up. The `bounded-not-lossless-backpressure-test` lesson is durable and already
shared; any future sprint adding a new bounded buffer over a reliable stream
should be gated against it explicitly, not rediscovered.
