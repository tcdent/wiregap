# Sprint 011 — The In-Process Upstream Client to Anthropic (Phase 4, continued)

> Proxy team, sprint 11, the first of two sprints closing out the remaining Phase 4
> backlog the PM named (upstream client, then the real head connection). Builds the
> background-worker/channel concurrency foundation the box-facing reactor needs to
> receive asynchronous data, then the real TLS client to Anthropic on top of it.
> Drafted by zero.proxy-lead from planning input by zero.proxy-dev and
> zero.proxy-review (2026-07-22).

---

## Scope Boundary

This sprint owns two things that land together because the second needs the first:

1. **The concurrency/ingress foundation** — a background worker (thread) hands
   asynchronously-arriving frames to the box-facing loop over a bounded channel; the
   loop drains the channel every tick (reusing Sprint 10's `enqueue_net_response`) and
   pumps as usual. The `Dispatcher` stays single-threaded; the channel is the only
   thread boundary.
2. **The real upstream TLS client to Anthropic** — the worker that actually opens the
   connection, POSTs the recognizer-admitted body, and streams the response back
   through the foundation above.

It explicitly does **not** own:

- **The real head connection or its session-key mapping.** Named, sized, and
  sequenced as Sprint 12 — it reuses this sprint's foundation but is otherwise a
  separate, differently-shaped integration (transport-selection over security-design,
  per `proxy-head.md` §3).
- **The reactor's bounded-park / max-park capability.** A genuine liveness dependency
  on the `reactor` crate (not proxy's own), routed to the NIC team as **Task 20**
  (via the PM). This sprint builds and gates everything provable without it (the
  fake-ingress logic proof); the one thing it cannot close without Task 20 landing is
  named explicitly below (Acceptance) — **this sprint ships the client and the drain
  logic, not a working end-to-end upstream round-trip.** Do not read "the upstream
  client is built" as "an Anthropic response reaches the box" — the same
  receive-only-boundary discipline Sprint 8 held for hardware.
- **The configuration DSL.** The credential is a composition-root constructor
  parameter (the same shape as `quarantine_root`/`readable_root`) — a config *hook*,
  not the DSL itself, which stays a named, deferred Sprint-8 residual.
- **Any response inspection, sanitization, or neutralization.** The upstream response
  streams to the box verbatim, MB3 unchanged — hardening this direction would
  re-import the confidentiality worldview the spine already discards
  (`cloud/approach.md` §5).

## Problem

`cloud_relay`'s encoder exists (Sprint 6) but nothing drives it: no upstream
connection, no worker, and the box-facing reactor is single-threaded and synchronous
— it cannot block for the seconds an LLM response takes without stalling every other
box frame. Sprint 10 built the box-facing half of the reply path (the bounded
push-queue, `enqueue_net_response`) against a fake ingress specifically to defer this
gap; this sprint closes it for real.

Planning surfaced a genuine, independent liveness defect along the way: the
`reactor` crate's `park()` blocks **unbounded** when no retransmit deadline is armed
(`Arm::ParkForever` → `host.block(None)`, `transport::next_deadline()` is purely a
retransmit-timer filter). Concretely: the box sends a request and goes quiet awaiting
the response; with nothing in flight, the reactor parks forever on the NIC wake
alone; the upstream response arrives on the worker's channel seconds later, but
nothing wakes the loop to drain it — the box never receives its response. This is a
**liveness deadlock**, not a latency nicety, and it is independent of anything this
sprint's logic can fix (no pure-proxy workaround exists short of a rejected
keepalive-hack that would abuse the retransmit timer as a general-purpose timer).
Routed to the NIC team as Task 20: a small, additive cap on the *already-existing*
bounded-block path (`Host::block(Some(_))`, used today for retransmit deadlines),
not a new wait primitive.

## Goal

Build the worker/channel concurrency foundation and the real Anthropic TLS client on
top of it, with every piece of *logic* fresh-verifiable hardware-free via a fake that
correctly simulates quiescent-arrival — while stating honestly that the live,
end-to-end round-trip is held pending Task 20, the same evidence split Sprint 8 used
for hardware capture.

## Final State

1. **Scoped explicitly to one in-flight upstream call at a time** (resolved during
   review — dev caught that a single shared channel and "close = EOF" are only
   coherent for one call; this also gives review's concurrency-bound concern its
   strictest possible answer, bound = 1). A background worker (`std::thread`) owns
   *the* blocking upstream call; it communicates with the box-facing loop over a
   single **bounded** `sync_channel` — the only thread boundary in the system. No
   shared mutable state crosses it: the worker holds only a `Sender`, the loop only
   a `Receiver`; the `Dispatcher` is owned solely by the loop and the worker never
   touches it; data crossing is owned (`Vec<u8>` frames moved, never borrowed or
   shared). A second `NetPayloadAdmit` arriving while a call is already in flight is
   **DENIED** (`net-error`, reusing the existing JC-3 taxonomy and seam — not
   deferred into a queue): this keeps Slice 1's foundation to exactly one channel
   and no new buffering state, and it is honest to the box immediately rather than
   silently holding a request in an invisible queue. Never spawned as a second
   concurrent worker — this sprint does not build a worker pool or a per-call
   channel set. **Concurrent upstream calls are an explicit, named Sprint-12+
   follow-on**, not silently unhandled: the realistic box flow (one request, wait,
   next) is served, and serial scope keeps this sprint's foundation minimal and
   cleanly isolable, per
   the same "isolate the changed seam" discipline as Sprint 10's Slice 1. This
   also forecloses the cost/rate-limit amplification review flagged (a box cannot
   drive more than one concurrent upstream call against the proxy's own
   credential, by construction, not by a tunable limit).
2. The box-facing loop drains the channel every tick (`Event::Idle` and
   `Event::Delivered`) and hands each frame to the existing `enqueue_net_response`
   (Sprint 10) — reusing the proven bounded push-queue, WouldBlock pacing, and
   `in_reply_to` correlation without modification.
3. The worker has a **bounded timeout** on the upstream call (`ureq` supports this);
   on completion, error, or timeout, the worker exits and drops its `Sender`,
   closing the channel — the loop observes the disconnect as end-of-response. A hung
   or slow upstream never blocks the box-facing loop (the isolation is the whole
   point).
4. The real Anthropic client: `rustls` (pure-Rust TLS) + `ureq` (blocking, sync),
   added as dependencies wired **only** below the `NicRun`/reply seam (the worker),
   never linked into the box-reachable core. The worker POSTs the
   recognizer-admitted `NetPayloadAdmit` body **byte-verbatim** (no re-encode
   between vet and send — CB4/bind-the-chain finally realized on the real call) to
   `api.anthropic.com/v1/messages`, and streams the response back **verbatim**
   through `cloud_relay::relay_response` into the push-queue — never inspected,
   never neutralized (MB3).
5. Upstream failures (timeout, unreachable, TLS failure) map to `net-error` via the
   existing `relay_error`, stamped `in_reply_to` = the net-request session, over the
   same seam — no new error path invented.
6. The credential is a **constructor parameter** from the composition root (never
   hardcoded, never read via an invented loader) — Sprint 8's `Authority::Credential`
   placeholder made concrete: `validate_authority` now returns present-iff-supplied,
   so a missing credential holds the **whole** proxy closed (the ladder rung is real
   now, not always-`false`). The credential is trusted-side only: the box's request
   never carries or influences it, and it never leaks — not to the box (the response
   relay does not echo it) and not to a log or diagnostic. **Note (dev, planning):**
   supplying a credential makes this one rung real, but does not by itself open
   serving — `FilesystemRoots`/`KernelProtections` remain named, config-`false`
   placeholders (Sprint 8), so the ladder still holds closed on those. "The
   credential rung is real" is not "the proxy now serves."
7. The one thing this sprint does **not** prove: that the real reactor wakes to
   drain the channel when a response arrives during a real quiescent park. That
   proof is held, named explicitly, pending Task 20 (Non-Goals; Acceptance).
8. The sprint document carries role retrospectives; the sprint is committed with
   only its intentional files.

## In Scope — ordered slices

**Slice 1 — the concurrency/ingress foundation, isolated.**
Worker thread + bounded `sync_channel` + the box-facing loop's per-tick drain,
proven with a fake/timed worker (no real network yet). Gate, per the "isolate the
changed seam" discipline (Slice 1 of Sprint 10, the same shape): message-passing
only (no `Arc<Mutex<_>>`, no shared `Dispatcher`, verified — the compiler's
`Send`/`Sync` bounds enforce it, but the absence of any shared-state type is
checked explicitly); the channel is bounded and a full channel backpressures the
worker's send (proven by a fake worker producing faster than the loop drains);
worker cleanup is proven on completion, error, and timeout (no leaked thread, no
leaked channel, a hung worker does not block the loop). A second `NetPayloadAdmit`
arriving while a call is in flight is proven DENIED (net-error), never spawning a
second worker or channel (the serial scope, Final State #1). **The fake must
exercise the quiescent-arrival case specifically** — box gone silent, then the
response arrives, then a simulated wake — not merely "drain while the box is
still sending" (which proves nothing about the deadlock case Task 20 exists to
fix). MB6 holds (`std::thread`/`std::sync::mpsc`, no new `unsafe`).

**Slice 2 — the real Anthropic client.**
`rustls` + `ureq` wired below the seam; the worker POSTs the admitted body
verbatim and streams the response verbatim into the push-queue; error mapping to
`net-error`; the credential as a constructor parameter tied to the real
`Authority::Credential` rung (re-running Sprint 8's ladder tests with the rung now
real, confirming a missing credential closes the **whole** proxy, not just the net
channel). Gate: bind-the-chain on the POST body (byte-identical to the admitted
body, no re-encode); MB3 on the response (verbatim, never inspected, reusing
`cloud_relay::relay_response`); the credential's fail-closed authority and its
no-box-influence/no-leak properties; confirmation that `rustls`/`ureq` are wired
only below the seam (never linked into the box-reachable core) — tier-3, off the
L10 review budget, per the box-reachable-core precedent (third-party bulk is fine
off-surface, never in the reviewed core).

## Non-Goals

- Do not build the real head connection or its session-key mapping — Sprint 12,
  sequenced after this sprint, reusing its foundation.
- Do not build or extend the config DSL — the credential is a plain parameter, the
  same shape as `quarantine_root`/`readable_root`, not a loader.
- Do not weaken the response-streaming path with any inspection, sanitization, or
  content decision — MB3 holds; a bug here fails the feature or leaks already-
  undefended content, never grants box-reachable execution.
- Do not attempt a pure-proxy workaround for the reactor's unbounded park (the
  keepalive-hack is rejected: it abuses the retransmit timer as a general-purpose
  timer, a wrong-layer carry-not-rediscover violation) — the fix is Task 20's, on
  the NIC team's side.
- Do not let an unbounded channel or an untimed worker call ship — both are the
  outbound analogs of every unbounded-buffer defect this team has already caught
  (the read-slurp, the reassembly cap, the push-queue).
- Do not claim a working upstream round-trip as this sprint's deliverable — the
  drain logic and the real client are proven; the live wake is held pending Task 20.

## Starting Surface

Audit first, in this order:

1. `reactor/src/reactor.rs`'s `park()`/`arm_from_deadline`/`WakeReason`, and
   `transport/src/table.rs`'s `next_deadline()` — the liveness gap Task 20 exists to
   close, and why no pure-proxy lever reaches it.
2. `proxy/src/dispatch.rs`'s `enqueue_net_response`/`push_reply_frame`/`ReplyStream`
   (Sprint 10) — the seam this sprint's worker feeds, unchanged.
3. `proxy/src/cloud_relay.rs` (`relay_response`/`relay_error`, Sprint 6) — the
   encoders this sprint finally drives.
4. `proxy/src/lifecycle.rs`'s `Authority::Credential`/`validate_authority` (Sprint 8)
   — the placeholder this sprint makes concrete.
5. `docs/proxy/architecture.md` §10/L14 (the box-facing relay "opens outbound
   upstream sockets", in-process, unprivileged) and §11 L10 (TLS off the
   box-reachable review budget) — why this sprint's dependency choice needs no
   adversarial-closure gate.
6. `docs/proxy/cloud/anthropic-api.md` (the exact upstream call shape: `POST
   api.anthropic.com/v1/messages`, the already-vetted body).

## Method

- **MB1–MB6** apply to every new line. MB6 stays `{open_discipline, privilege}` —
  neither the worker/channel nor `rustls`/`ureq` add proxy-owned `unsafe`
  (third-party `unsafe`, if any, is off-surface tier-3, not re-certified here).
- **The evidence split is the sprint's central discipline, Sprint-8-shaped**:
  hardware-free (here: network-free) logic — the drain/enqueue/pump/deliver chain
  given a wake, proven via a fake that specifically simulates quiescent-arrival —
  is fresh-re-run by the reviewer, normal discipline. The **one held item** is the
  live proof that a real reactor park, with a real response arriving mid-park,
  actually wakes and drains — gated on Task 20's landing, not on anything this
  sprint's own logic can supply.
- **Bind-the-chain, finally realized on a real network call**: the POST body must
  be the exact bytes the tier-1 recognizer vetted (`NetPayloadAdmit`), not a
  re-derivation. This is CB4 (Sprint 5/7) extended to the real upstream leg.
- **The concurrency model is message-passing, not shared-memory**: the only
  crossing between the worker and the box-facing loop is owned data through a
  bounded channel. Any shared mutable state (a mutex, a shared `Dispatcher`) is a
  reject, not a style preference — it reintroduces exactly the class of hazard the
  channel design exists to avoid.

## Acceptance

- **Slice 1.** REJECT any shared mutable state beyond the channel (no
  `Arc<Mutex<_>>`, no shared `Dispatcher`); REJECT an unbounded channel; REJECT a
  worker with no call timeout, or one whose failure/timeout does not close the
  channel; REJECT spawning a second concurrent worker/channel for a second
  in-flight `NetPayloadAdmit` (serial scope is by construction, not a tunable
  limit); REJECT a fake-ingress test suite that never exercises the specific
  quiescent-then-arrival-then-wake sequence (draining while the box is still
  active proves nothing about the deadlock).
- **Slice 2.** REJECT any re-encoding of the admitted body before it is POSTed
  (bind-the-chain break); REJECT any inspection, logging, or transformation of the
  streamed response (MB3 violation); REJECT a credential that is hardcoded, is
  read via a newly-invented loader, or whose absence does not hold the **whole**
  ladder closed; REJECT any path where the credential reaches a box-facing frame
  or a log line; REJECT `rustls`/`ureq` linked anywhere in the box-reachable core
  rather than strictly below the worker seam.
- **Sprint-level, the honest boundary.** REJECT any report or retrospective that
  reads "the upstream client is built" as "an Anthropic response reaches the box."
  Until Task 20 lands and the live wake is proven, the production flow deadlocks on
  a real quiescent arrival — stated as such, not softened.

## Review Questions

1. **Task 20 — the reactor's bounded-park/max-park cap** (routed to the NIC team via
   the PM, tracked here until it lands): the one held gate item; this sprint's
   Slice-1 foundation is provably robust to *how* the wake eventually arrives (a
   max-park poll or, later, a multiplexed wait) — only the live proof waits.
2. **A live end-to-end capture, once Task 20 lands** (team's call, not pre-decided):
   whether a real round-trip to Anthropic on real hardware is needed as a
   closeout gate for *this* sprint's retrospective, or whether it is folded into
   whichever sprint next touches the live NIC path. Not blocking this sprint's own
   slices.
3. **HTTP/1.1 sufficiency** (dev's diligence item, not a fork): `ureq` is
   HTTP/1.1-only; Anthropic's Messages API (including SSE streaming) is expected to
   work over it. Switch to a fallback (`reqwest`-blocking or `attohttpc`) only on
   concrete evidence it doesn't, not preemptively.

## Retrospectives

### zero.proxy-review

**A cross-crate claim load-bearing enough to justify a held gate gets re-derived, not cited.** The
whole sprint rests on one premise: the reactor parks unbounded when the box is quiet, so an upstream
response arriving mid-park never wakes the loop — a liveness deadlock. That premise justifies the
held gate, the honest-boundary acceptance clause, and Task 20's very existence. So I did not take the
contract's citation of it; I read `reactor::park` and `transport::next_deadline` myself and re-derived
it: `arm_from_deadline(next_deadline(), now)` yields `Arm::ParkForever` when nothing is armed →
`host.block(None)`, which parks until a NIC wake *only*, and `next_deadline()` is a pure
retransmit-timer filter that returns `None` when no sender is in flight — so a quiet box parks on a
NIC wake it never produces, and the mpsc response is not a NIC wake. Confirmed real. The discipline:
the more a decision *depends* on a claim (and a held gate + a whole cross-team task depend on this
one), the less acceptable it is to take the claim on citation — re-derive it against the primary
source, especially across a crate boundary the proxy team does not own.

**Two people finding the same defect from opposite ends is the strongest signal it's real.** My
concurrency-bound refinement (an unbounded worker count / request intake under a request-spam) and
the dev's channel-coherence finding (the singular channel + close-as-EOF is only coherent for one
in-flight call) turned out to be *the same issue from two angles* — I came at it from
resource-bounding, the dev from design-coherence, and they met at "the design is only sound for one
upstream call at a time." The resolution was the strongest possible: serial bound = 1 by
construction (`inflight: Option`, a flat deny of the second, no pending buffer at all), which answered
my MB4 concern and the dev's coherence issue in one stroke, with concurrent upstream named as a
Sprint-12+ follow-on. When an independent reviewer catch and an independent builder catch converge on
one fix, the fix is load-bearing, not speculative — and worth the strictest form.

**"The drain given a wake" vs "the wake itself" is the distinction that shaped the whole evidence
split.** The temptation with a deadlock this concrete is to treat the fake-ingress test as proving
the flow works. It does not — it proves only that *given* the loop is woken on a quiescent tick, the
drain/enqueue/pump/deliver chain correctly delivers the response verbatim, in order, correlated. It
does *not* prove the loop *wakes*, which is precisely the deadlock. So the gate accepts the fake for
the logic and *holds* the live real-wake-during-real-park — the sprint's one held item, pending Task
20 — exactly the Sprint-8 IB2/IB3 split (mechanism/logic certified now, live proof held). The honest
boundary followed from naming this precisely: the sprint ships the client and the drain logic, *not*
a working round-trip, and that is stated as a reject-if-softened acceptance clause — the code doc, the
test name ("simulated wake"), and the contract all carry it. A fake that proves "given a wake, the
logic works" is legitimate only when the wake it simulates is named as the unproven thing.

**What the next sprint inherits.**
- *The concurrency foundation + the real client* — message-passing (no shared mutable state beyond a
  bounded channel), serial bound = 1, hung-worker-safe, leak-free; the real rustls/ureq worker
  bind-the-chain-verbatim on the POST and MB3-verbatim on the response, off-surface (tier-3, confined
  to `upstream.rs`); the credential a fail-closed authority that never leaks.
- *Two held/named items, both honest, neither guessed at:* **Task 20** (the reactor's max-park cap —
  the live wake proof, NIC team's, the sprint's one held gate) and a **real end-to-end capture** once
  Task 20 and a real operator credential land (the live round-trip, currently a local-server stand-in).
- *Sprint 12* — the real head connection, reusing this sprint's worker/channel foundation but a
  differently-shaped integration (transport-selection over security-design).
- *The standing disciplines, reconfirmed:* re-derive a load-bearing cross-crate premise against its
  own source rather than take a citation; treat a reviewer-catch/builder-catch convergence as
  strong signal and reach for the strictest fix; and separate "the logic given the enabling event"
  from "the enabling event itself," holding the latter honestly when it cannot yet be proven. **CONFIRMED at build**: `ureq`
   3.3.0 (rustls 0.23, pure-Rust) POSTs + streams cleanly against a local server.

### zero.proxy-dev

**What I built.** Two slices. Slice 1: the concurrency/ingress foundation — a background worker owns
the one (serial) blocking upstream call and hands frames to the box-facing loop over a bounded
`sync_channel`; the loop drains it every tick into Sprint-10's push-queue. Message-passing only, no
shared mutable state, the `Dispatcher` never crosses the thread boundary. Slice 2: the real
`AnthropicClient` (ureq 3.3.0 + rustls 0.23) dropped into the seam unchanged — POSTs the admitted body
verbatim, streams the response verbatim, maps transport errors to `net-error` — plus the credential rung
made real (`Authority::Credential` present-iff-supplied; a missing credential closes the whole proxy).

**The reactor-deadlock finding, scoped as a held item rather than a blocker.** Grounding the upstream
client surfaced a genuine LIVENESS deadlock outside proxy's own crate: the reactor parks **unbounded**
(`Arm::ParkForever` → `host.block(None)`) when no retransmit deadline is armed, and `next_deadline()` is
purely a retransmit-timer filter. Concretely: the box sends a request and goes quiet; the proxy parks
forever on the NIC wake alone; the upstream response arrives on the worker's channel seconds later, but
nothing wakes the loop to drain it — the box never gets its response. I traced it against the real
`reactor.rs`/`table.rs`, confirmed no pure-proxy lever exists (the only one is a keepalive-send hack
that abuses the retransmit timer — rejected), and framed the fix as a *small additive cap on the
already-existing bounded-block path* (`Host::block(Some(_))`), not a new wait primitive — routed to the
NIC team as Task 20. The key move: this did NOT block the sprint. The fake ingress simulates the wake
(a test driving an `Idle` tick IS the wake), so all the *logic* is proven now, and the foundation is
robust to *how* the real wake eventually arrives — only the live proof waits, named explicitly. The
same Sprint-8 evidence-split discipline: prove everything provable off the held dependency, and state
the held piece honestly rather than softening it.

**Two independent catches converging on serial scope.** Reviewing the draft I caught an internal
inconsistency: a single shared channel plus "close = EOF" is only coherent for ONE in-flight call, yet
the transport allows concurrent net sessions — the contract was implicitly one-call-at-a-time without
saying so. Independently, review flagged that concurrent upstream calls must be *bounded*. Both
resolved to the same fix — serial, bound = 1 — which gives the bound its strictest possible answer (1)
and keeps Slice 1's foundation minimal (a single channel, the cleanest changed-seam to review in
isolation). The lesson: an ambiguity and a risk from two directions often name the same missing
decision; surfacing the inconsistency early let the lead fold both into one clean scope line.

**Deny-vs-defer: the minimal answer is the honest one.** For a second admit while a call is in flight,
the ruling was DENY (`net-error(Unavailable)`) over defer (a depth-1 queue). Deny keeps Slice 1 to
exactly one channel with no new buffering/queue state to build and gate, and it is honest to the box
immediately rather than silently holding a request invisibly — reusing the JC-3 net-error seam, no new
path. A defer queue would have been its own small residual against the sprint's stated minimalism.

**The quiescent-arrival-specific fake discipline.** The reviewer sharpened the gate: the fake must
exercise box-gone-silent → response-arrives → simulated-wake, NOT merely "drain while the box is still
sending" — because only the former exercises the deadlock case Task 20 exists to fix. Draining while
the box is active proves nothing about the quiescent park. Building the test to that exact sequence is
what makes the held item precisely one thing (the real wake mechanics) rather than a vague "hardware
later."

**What went cleanly.** The Slice-1 seam paid off exactly as intended: the real ureq/rustls worker
dropped in behind `UpstreamSpawn` with the ingress/adapter unmoved. rustls + ureq (pure-Rust, blocking,
sync) fit the worker-thread + sync-reactor model with zero async runtime, and stayed strictly
off-surface (grep-confirmed only in `upstream.rs`, tier-3, off the L10 budget). Bind-the-chain finally
landed on a real network call — the admitted bytes reach the wire byte-identical, proven against a
local server over the real ureq path.

### zero.proxy-lead

**A genuinely new-shaped finding (concurrency, a first for this team) still resolved into the same
pattern every prior cross-team surprise has.** Every sprint before this one surfaced its structural
finding *inside* proxy's own crates or at a boundary the team already had a name for (a facade gap, a
changed seam, a tier misclassification). The reactor's unbounded park was different in kind — a
liveness property of a crate this team doesn't own, discovered through reasoning about a feature
(streaming replies) rather than through a build failure. What held steady regardless: verify the claim
against the actual source before treating it as fact (I read `park`/`arm_from_deadline`/
`next_deadline` myself before routing anything), scope what's independently provable now versus what
genuinely waits on the other team, and route the *actual* dependency precisely enough that it becomes
a small, acceptable ask rather than an open-ended one. The shape of the finding was new; the discipline
for handling it wasn't, and didn't need to be.

**Two independent catches converging on the same fix is a stronger signal than either catch alone —
worth naming as a pattern, not just crediting once more.** Dev's channel-coherence inconsistency and
review's concurrency-bound requirement met at exactly one resolution (serial, bound=1, by
construction) from two completely different angles — one from "does this design cohere," one from
"is this resource bounded." Neither had seen the other's finding when they converged. When that
happens, the fix is close to force-derived rather than chosen, and it's a signal to adopt it directly
rather than search for alternatives — which is exactly what happened here, quickly.

**What the next sprint inherits.** Sprint 12 (the real head connection) reuses this sprint's
foundation nearly whole-cloth — the worker/channel/drain pattern, the message-passing MB bar, the
serial-vs-concurrent framing (proxy-head.md's own transport is deliberately flexible, so the design
question there is more transport-selection than a repeat of this sprint's concurrency question, per
dev's original read). Task 20 remains the standing cross-team dependency for any live capture — this
sprint's own retrospective and Sprint 12's both point at it, and it should close before either sprint's
retrospective claims a live round-trip.
