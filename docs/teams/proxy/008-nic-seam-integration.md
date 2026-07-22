# Sprint 008 — Real NIC-Seam Integration (Phase 3)

> Proxy team, sprint 8, the first sprint of Phase 3. The team wires the proxy against the
> real, finalized `directpath` API for the first time, replacing Sprint 6's fake-wire
> harness, and resolves Task 14 (the Sprint-7 stub-envelope tripwire) as a blocking
> prerequisite. Drafted by zero.proxy-lead from planning input by zero.proxy-dev and
> zero.proxy-review (2026-07-21); green-lit by zero.proxy-pm.

---

## Scope Boundary

This sprint owns wiring the certified proxy against the real `directpath` crate
(`Subsystem::construct`/`run`, `Ctx`, `Event`, `Bdf`), the lifecycle ladder that governs
bring-up (`architecture.md` §9), the privilege-drop boundary (§10, L15), and Task 14's
resolution — scoped to the **receive path only**.

**Confirmed during planning, bounding this sprint's "done" precisely:** the transport is
uni-directional by design (the box is the sole session-originator); the finalized API's
only send mechanism, `Ctx::send`, explicitly *opens a new session* when called — exactly
what the crate's own documentation says the receiving side must never do ("that would
make this side originate, which the design forbids"). There is currently no visible
mechanism for the proxy to reply within a session the box already opened. **This is a
genuine, unresolved architectural question, routed to the PM/NIC team, not this sprint's
to guess at or work around.** Confirmed by the PM: Sprint 8 proves the **receive path**
only — a box-originated request reaching a recognizer verdict (admit/deny) on real
hardware. The proxy sending anything back to the box (a cloud response, file data) is
explicitly deferred until the reply-direction question resolves.

It explicitly does **not** own:

- **The proxy→box response/origination path.** Routed (R-c); out of scope until resolved.
- **The upstream (Anthropic TLS) forward.** The receive-path e2e proves recognition
  (admit/deny), not a full round-trip to Anthropic and back.
- **The D-3 quiescence gap** (`Ctx::is_quiescent`/`tx_dropped` are `cfg(bench)`-only; the
  shipped surface has only `Event::Idle`) — a fill-in gap routed to the NIC team, not
  fixed here.
- **The verb/betas envelope-sourcing question beyond the single Anthropic route.** This
  sprint fixes `verb="POST"` and `betas=&[]` as justified constants (the net wire conveys
  neither today); whether a future multi-method/beta net service needs wire-level fields
  is a deeper question, named and routed, not solved.
- **Re-litigating any already-accepted Sprint 1–7 design** (the taxonomy, the recognizer,
  the register, the reassembly mechanism) — this sprint integrates it against real
  hardware, it does not reopen it.

## Problem

The security core (Sprint 5), the relay layer (Sprint 6), and the reassembly seam
(Sprint 7) are all built and certified against a fake wire. `proxy` does not yet depend on
`directpath` at all — this sprint is literally first contact between five sprints of
certified logic and the real device. Three things must land together: the real
NIC-driven lifecycle (privileged bring-up, a fail-closed authority-validation rung, a
privilege drop that must be observed to actually happen — not merely asserted by design —
before any box byte is processed), the real event loop replacing the fake reactor drive,
and Task 14's resolution. **The live defect the real Route binding fixes is domain/path
misrecognition**: the stub hardcodes one envelope, so with more than one route it would
recognize a request to route B under route A's envelope; today there is exactly one route
(`api.anthropic.com`/`/v1/messages`), so the stub is *accidentally* correct — which is
precisely why it is dangerous (it looks like production). **The betas concern is real but
future-contingent, not a live mechanism today**: verified on disk, `NetAddress` carries only
`scheme` + `address` and `net-payload` is just the body — the wire conveys no beta channel
at all, so `betas=&[]` is not a dropped field but the literal truth of what the box can
express. It becomes a live bypass only if a future net-service extension wire-sources betas
while some path still vets them as empty — a recognize-vs-forward mismatch, not today's
risk. This premise (no beta channel exists on the wire) is load-bearing for Slice 2's gate
and is *verified there*, not assumed (see Acceptance).

Planning also surfaced that the real `Subsystem` is **atomically opaque and cooperative in
a different way than the fake wire was**: `construct`/`run` are not generic or injectable
— there is no swap-in test double at that exact boundary — and the fake wire's own
cooperative behavior (clean ordering, infallible emit, only ever exercising the initial
ring arm) baked in silent assumptions the real device is not guaranteed to honor
(delivery ordering, emit fallibility on a full ring, partial/zero-length frames, spurious
wakes, and — the sharpest — steady-state ring re-arm, which a single-frame test can never
exercise and is exactly where a prior nic-team bug class hid).

## Goal

Prove the receive path end-to-end on real hardware — a box-originated request reaches a
recognizer verdict — while keeping every piece of *logic* that doesn't strictly require
the device fresh-verifiable by the reviewer, not degraded to a capture model by default.

## Final State

1. `contract/proxy-nic.md`'s fill-in half is pinned against the real, finalized API (the
   D-1–D-6 demands mapped to concrete `directpath` types/methods), closing that contract's
   deferred half. The one real gap found (D-3, quiescence) is named and routed to the NIC
   team, not silently accepted as satisfied.
2. Task 14 is resolved by wiring the **real per-session Route binding**: `Dispatcher`
   stores the matched `Route` on `net-address` admission (evicted alongside the existing
   binding/reassembler maps); `net-payload`'s recognizer envelope is built from that stored
   Route's real domain/path, with `verb`/`betas` as justified, named constants (not a
   hardcoded stand-in for the whole envelope). `stub_envelope_request` is deleted —
   grep-absent, not left reachable as a fallback. **The new fail-closed rung this
   introduces is precise**: `net-payload` on a session with **no** stored admitted Route
   (no prior `net-address`, or one that was denied) must **deny at recognition** — the old
   self-bind-then-recognize-with-a-stub path no longer happens. This is *not* a change to
   the separate verb-block session→service binding (a `net-payload` verb still maps to
   `Service::Net` by block, unchanged) — only the recognition step downstream of it now
   requires a real, stored Route.
3. A thin proxy-local trait (`NicRun` or similar) abstracts over `Subsystem::send`/`run`,
   implemented for the real `directpath::Subsystem` and, in tests, by a fake that
   synthesizes `Event::Delivered` sequences. This is the structural seam that keeps the
   lifecycle-sequencing and event-loop-adapter *logic* fresh-re-runnable by the reviewer,
   confining the capture-model evidence standard to only the irreducibly-hardware pieces
   (the real privileged bring-up, the real wire round-trip).
4. The lifecycle ladder (`architecture.md` §9) is wired as an explicit composition-root
   gate, fail-closed at every rung, all-or-nothing at readiness: privileged bring-up →
   privilege drop (net-new, in-house confined `unsafe`, never a third-party wrapper,
   per the standing house rule) → authority/environment validation → channels open only if
   *every* configured authority validated. A single broken authority (e.g. a bad route
   register) holds the **whole** proxy closed — proven, not just asserted.
5. Privilege-drop is proven to have *happened* in the running process, not merely designed
   to: the observation is the OS credential state after the drop (`getresuid`/`getresgid`,
   capability sets including bounding/ambient, all empty/unprivileged) plus a negative test
   — a privileged operation attempted from inside `run()` must fail. Sequencing is
   verified: the drop completes before `run()` processes a single box byte.
6. `Subsystem::run()` drives dispatch via the new adapter; the Sprint-6 `reactor_wiring`
   fake-`pump_all` integration test is replaced (by the `NicRun`-fake equivalent). The
   hardware-free direct `route()` logic tests remain the only hardware-free gate on
   routing/binding/reassembly/recognition and are not stranded behind bench hardware — but
   **"retained" does not mean "unchanged" wherever Slice 2's no-self-bind rung applies**.
   Confirmed during review: `net_payload_seam.rs`'s `through_seam` helper currently
   self-binds a fresh session directly to `net-payload` with no prior `net-address` (its
   own comment says so) — Slice 2 makes that path deny. The test is **updated**, not left
   as-is: it must establish a Route via a prior admitted `net-address` before sending
   `net-payload` chunks, which is *more* realistic (it exercises the real admission
   sequence this sprint wires) — the same check applies to any `net-payload` case in
   `tests/dispatch.rs`. What's preserved is the *discipline* (hardware-free, direct,
   independently re-derivable) and the corpus/fuzz content — not every line of the harness
   verbatim.
7. A receive-path e2e is proven on the real 82599: a box-originated `net-address` +
   `net-payload` sequence, delivered through real `construct`/`run`, reaches a recognizer
   admit/deny verdict — captured with a bin-sha-declared, un-fakeable tell (a unique-nonce
   payload plus the device's own RX counter), per the silicon-evidence discipline.
8. The sprint document carries role retrospectives; the sprint is committed with only its
   intentional files.

## In Scope — ordered slices

**Slice 1 — pin the `proxy-nic.md` fill-in against the real API.**
Doc-only. Maps each D-1–D-6 demand to its concrete `directpath` form; names the D-3
quiescence gap explicitly rather than papering over it. Gate: normal fresh re-derivation
(no hardware dependency).

**Slice 2 — resolve Task 14 (front-loaded; zero dependency on NIC hardware).**
Per-session `Route` storage on `net-address` admission; the recognizer envelope built from
the real, stored Route; the new no-prior-Route-denies rung; `stub_envelope_request`
deleted; `net_payload_seam.rs` (and any `net-payload` case in `tests/dispatch.rs`) updated
to establish a Route via a prior admitted `net-address` before sending chunks, since the
no-self-bind rung makes the old self-binding harness path deny. Pure dispatch logic,
testable with the existing direct `route()` tests. Gate: Sprint 5/6's standing
adversarial/MB discipline, fresh-verified — **plus** the specific check that the envelope
the recognizer vets includes the real domain/path (two different admitted routes must
recognize under two different, correctly-sourced envelopes — a shared envelope for both is
the stub in disguise), **and an explicit, verified (not assumed) check of the betas
premise**: confirm on disk that `NetAddress`/`net-payload` and the request body carry no
beta channel at all, and that the recognizer reads betas only from the `Request`
envelope's own field, never derived from the body — if that holds, `betas=&[]` is the
literal truth and not a drop; if any box-influenced path can set a beta the recognizer
gates on, `betas=&[]` *is* the bypass, and this gate must catch it.

**Slice 3 — the bring-up sequence, the lifecycle ladder, and privilege-drop.**
`construct()` (rungs 1–2, real hardware) → a net-new privilege-drop (confined in-house
`unsafe`, the standing house rule — never a third-party wrapper) → authority/environment
validation (rung 3, fail-closed, named-by-channel failure) → all-or-nothing readiness
(rung 4). Introduces the `NicRun` trait seam explicitly as structural, not incidental —
this is what keeps the lifecycle *sequencing* logic fresh-testable. Gate, split by
evidence model:
- Lifecycle sequencing (validate-before-run, all-or-none, fail-closed ordering): hardware-
  free via the trait fake, reviewer re-runs fresh.
- The privilege-drop mechanism itself: reviewer reads and reasons over the confined
  syscalls as a certified reasoning artifact (the crossing-soundness discipline), plus a
  privileged capture showing the credential state before/after.
- The real `construct()` bring-up: capture-model, bin-sha-declared.

**Slice 4 — wire `run()` as the driver via the adapter.**
The thin `on_event` closure: `Event::Delivered` → `Dispatcher::route`; `Event::Idle` →
continue. `Ctx` is deliberately unused/omitted from the proxy's own abstraction — the
proxy never originates in response, consistent with R-c. Gate: the adapter logic is
hardware-free via the trait fake, reviewer re-runs fresh; the real `run()` against the
82599 is capture-model.

**Slice 5 — the receive-path e2e proof on real hardware.**
A box-originated `net-address` + `net-payload` sequence through real `construct`/`run`
reaching a recognizer verdict. Capture-model throughout (bin-sha-declared, unique-nonce +
RX-counter tell). Explicitly does **not** prove a response reaches the box or that the
upstream forward happens — the honest boundary of a receive-only e2e, stated in the gate
so "e2e proof" isn't misread as a full round-trip.

## Non-Goals

- Do not attempt the proxy→box response path or call `Ctx::send` from the receive handler
  — the reply-direction question (R-c) is unresolved and routed; using `Ctx::send` here
  would violate the box-is-sole-originator invariant the crate's own docs warn against.
- Do not treat "construct/run need real hardware" as license to drop the reviewer's
  fresh-re-run discipline for everything this sprint touches — the `NicRun` trait seam
  exists precisely so only the irreducibly-hardware pieces degrade to capture-model.
- Do not delete the hardware-free direct `route()` logic tests when `run()` replaces the
  fake-wire integration test — only the Sprint-6 `reactor_wiring` fake-`pump_all` test is
  superseded; `tests/dispatch.rs` and `net_payload_seam.rs` stay as the standing
  hardware-free gate, **updated** wherever Slice 2's no-self-bind rung requires a prior
  admitted Route to reach the same admit/deny cases (Final State #6) — do not read
  "retained" as "untouched" where the two are in tension.
- Do not leave `stub_envelope_request` reachable as a fallback once the real Route binding
  is wired — a half-wired resolution that still falls back to the stub is worse than
  having kept the build-time guard; delete it, grep-absent.
- Do not let a `net-payload` on a session with no admitted Route silently self-bind and
  proceed — that path must now deny, closing the correctness gap the real binding exposes.
- Do not accept "the design says `run()` is unprivileged" as proof that the drop happened
  — the gate requires an observed OS credential state and a negative test (a privileged
  op fails from inside `run()`), never a reading of the design claim alone.
- Do not conflate the privilege-drop's scope with DMA containment — the drop confines the
  *process's* syscall privilege; the device's DMA reach on the committed pure-direct,
  no-IOMMU path is the separately-owned residual (`nic/architecture.md` §2.1), untouched
  by this sprint and not to be credited with more than it provides.

## Starting Surface

Audit first, in this order:

1. `directpath/src/lib.rs` in full — the finalized `Subsystem`/`Ctx`/`Event` surface this
   sprint wires against, including its own doc comments on origination (load-bearing for
   R-c).
2. `docs/proxy/architecture.md` §9 (lifecycle ladder, all-or-nothing readiness,
   crash-only-safe) and §10 (process shape, L14/L15 both settled).
3. `contract/proxy-nic.md` (the accepted shape-now half; §3's D-1–D-6 demands; §4's
   now-unblockable fill-in).
4. `proxy/src/dispatch.rs` (Sprint 6's fake-wire `reactor_wiring` test and Sprint 7's
   `stub_envelope_request`) and the existing hardware-free logic tests
   (`tests/dispatch.rs`, `net_payload_seam.rs`).
5. `reactor/src/reactor.rs`'s generic `Host`/`RxSource`/`TransmitSink` seam — retained and
   distinct from the new `NicRun` trait; useful context, not a double for `Subsystem`.
6. My memory note `[[transport-uni-directional-sole-allocator]]` and the standing
   `[[silicon-evidence-anchor-degrades]]`/`[[box-reachable-core-no-third-party-unsafe]]`
   disciplines this sprint's evidence model and privilege-drop implementation both draw on.

## Method

This sprint's evidence model is the sharpest split the team has run: most of it stays
under the reviewer's normal fresh-re-run discipline; only the irreducibly-hardware pieces
degrade to a capture model, and the split is **structural** (the `NicRun` trait seam), not
incidental or assumed.

- **MB1–MB6 (Sprint 6's bar) still applies to every new glue line, with MB6 widened.**
  The arm-by-arm conformance-walking discipline applies to any new dispatch/drive-loop
  routing. MB6 for this sprint reads "unsafe confined to `{open_discipline,
  privilege-drop}`, each its own allow-marked module, crate root still deny" — Slice 3
  adds a second confined-unsafe module, so "one module" no longer describes the whole
  crate, only each individual confinement. Separately: the proxy now depends on
  `directpath`'s own box-reachable `unsafe` (real DMA/FFI) — that is the NIC team's
  certified in-house surface, cited here, not re-owned or re-certified by this review; the
  proxy's *own* new unsafe is only the privilege-drop, which is the crossing-soundness
  reasoning artifact this sprint certifies.
- **IB1 — Lifecycle fail-closed.** Each rung either succeeds and advances, or fails and
  halts the *whole* bring-up with no serving — never log-and-skip, never degrade, never
  partial-serve. Proof enumerates each rung × {ok, fail} and shows readiness only on all-ok.
- **IB2 — Privilege-drop observed, not asserted (the sharpest gate).** The code-proof is a
  runtime assertion of OS credential state after the drop — real/effective/saved uid and
  gid, and capability sets including bounding/ambient, all at the unprivileged target. A
  drop leaving the saved uid or bounding set privileged is not a drop (the process could
  re-acquire). The un-fakeable tell is a **negative test**: a privileged operation attempted
  from inside `run()` must fail — proving impossibility, not just reading a non-zero uid.
  Every privilege-affecting syscall's return is checked; an unchecked failure that silently
  leaves `run()` at root is the classic bug this guards against. Ordering is load-bearing:
  the drop completes before `run()` processes one box byte — RX rings may be armed
  (privileged) before the drop, but frames are only *processed* after it.
- **IB3 — Real-API conformance, re-derived at the actual cited lines, split by layer.**
  Every assumption Sprint 6's fake wire baked in (delivery ordering and availability, emit
  fallibility on a full ring, partial/zero-length frames, spurious/coalesced wakes) is
  re-verified against the real API's actual guarantee, never against the fake's
  cooperative approximation. **The split that matters**: the `NicRun` fake synthesizes
  `Event::Delivered` sequences *above* the RX ring — it can freshly test the **adapter**
  under sustained synthetic load (many deliveries, ordering, idle interleaving,
  `ControlFlow`), but it *cannot* reach **steady-state ring re-arm** at all, because the
  ring lives inside `directpath` (`datapath.rs`), below `Subsystem::run`, unreachable from
  proxy code. Ring re-arm is verified only at the Slice-5 e2e capture (sustained frames and
  wrap) and its correctness is nic-team ground (the RX-RDT steady-state bug class,
  `[[ixgbe-82599-datapath-bringup-gotchas]]`) — cited, not a proxy unit-test target. A gap
  between what the *adapter* assumes and what the real event stream guarantees is closed
  with a new fake-driven test; ring re-arm itself is not re-derivable outside Slice 5.
- **The evidence model, per slice** (§ Final State's own split, restated as the
  standing rule): hardware-free logic (Slices 1, 2, and the *sequencing*/*adapter* halves
  of Slices 3–4) is fresh-re-run by the reviewer, normal discipline. The irreducibly-
  hardware pieces (the real `construct()` bring-up, the real `run()` wire round-trip, the
  privileged credential-state capture) degrade to the silicon-crossing capture model:
  bin-sha-declared, unique-nonce-and-device-counter tell, coherence verified by the
  reviewer even where the run itself cannot be freshly re-executed by them.
- **The directpath content-anchor pin is recurring, not one-time (added at the Slice-1
  gate).** The finalized `directpath` public API is genuinely uncommitted working-tree
  state (verified: `HEAD`'s last `directpath` commit predates the whole `Subsystem`/`Ctx`/
  `Event` surface). Slice 1 pinned `proxy-nic.md` §4 to a content hash of
  `directpath/src/lib.rs` rather than a commit sha, tracked as a residual (R-7) owed a
  proper re-pin once the NIC team commits the facade. Because Slices 3–5 build real code
  against this same uncommitted API, the working tree could drift *before* that commit
  lands — so the content-hash is **re-verified at every subsequent slice that depends on
  `directpath`** (3, 4, 5), not just checked once at Slice 1. A drifted hash re-triggers
  re-derivation of any wiring built against the stale content, the same "after any source
  change, re-capture" discipline the team's bin-sha coherence work established.

## Acceptance

- **MB1–MB6** apply to every new glue line, per Sprint 6's standing bar.
- **IB1 (lifecycle fail-closed).** REJECT on any rung that degrades, logs-and-skips, or
  serves partially. The load-bearing sub-case: a single bad authority (e.g. the route
  register) must close **every** channel (net, file, terminal), not just the one it feeds
  — proven by total system inertness (frames-processed == 0, the wire never polled,
  `recognize` never called, `run()` never entered), not merely "`construct()` returned
  `Err`."
- **IB2 (privilege-drop observed).** REJECT unless: the post-drop credential state is
  directly observed (not inferred); a negative test proves a privileged op fails from
  inside `run()`; the drop-before-first-box-byte ordering is verified explicitly; and the
  drop's scope is correctly characterized as process-privilege only, never credited with
  containing the DMA-reach residual.
- **IB3 (real-API conformance).** REJECT on any fake-wire assumption not re-verified
  against the real API's actual guarantee — especially steady-state ring re-arm, which
  must be exercised under sustained frames and wrap, not a single-frame happy path.
- **Task 14 (genuine closure, not cosmetic).** REJECT if: the same envelope is recognized
  for two different admitted routes (proving the stub in disguise); betas are dropped from
  what's actually sourced/forwarded while still vetted as empty; or
  `stub_envelope_request` remains reachable as a fallback rather than deleted.
- **The receive-path e2e (Slice 5).** REJECT on any claim that a response reached the box
  or that the upstream forward happened — the gate is admit/deny verdict on the receive
  side only, stated as such.

## Review Questions

1. **R-c — the proxy→box response/origination mechanism (routed to the PM/NIC team,
   confirmed non-blocking for this sprint's scope):** the finalized API's only send
   primitive opens a new session, which the crate's own docs say the receiving side must
   never do. How the proxy is meant to ever reply is a genuine, unresolved design question
   — the PM has confirmed Sprint 8 proceeds on the receive path only, with this question
   pursued separately.
2. **D-3 — quiescence signal is `cfg(bench)`-only on the shipped surface** (routed to the
   NIC team, non-blocking): if the proxy's shutdown/backpressure logic ever needs a real
   "TX fully ACKed" signal rather than `Event::Idle`'s "drained, about to park," that gap
   needs filling upstream.
3. **The verb/betas envelope-sourcing residual** (routed to design, non-blocking): fixing
   `verb="POST"`/`betas=&[]` as constants is correct for the single Anthropic route today;
   whether a future multi-method or beta-bearing net service needs these carried on the
   wire is a deeper question for whoever extends the net service beyond this one route.
4. **The `NicRun` trait seam's exact shape** (dev's call, confirmed in-scope for Slice
   3/4): a local trait over `Subsystem`, implemented for the real type and a
   test-only event-synthesizing fake — the structural lever that keeps most of this
   sprint's logic fresh-verifiable rather than capture-model by default.

## Retrospectives

### zero.proxy-review

**A reasoning certification of FFI cannot certify the calling convention — flag it
run-unverified.** At Slice 3 I certified the privilege-drop mechanism as a sound reasoning
artifact: I verified the drop *sequence* (gid-before-uid, saved-uid, bounding/ambient cleared,
no_new_privs last), the hand-declared cap ABI *structs* (v3 version, field order, the two-element
array), and that every syscall return was checked. All of that was correct. What I missed is that
`libc::prctl` is **variadic**: the two-argument call sites left args 3–5 uninitialized, and the
kernel rejects `PR_SET_NO_NEW_PRIVS` unless they are zero — so the drop would have `EINVAL`'d at step
8 on every run (fail-closed, never serving). I even *noted at Slice 3* that the ambient call passed
five args while the others passed two, and did not flag the inconsistency as a bug — it was
catchable, and I over-stated soundness on the call sites. The durable lesson: a reasoning
certification of syscall/FFI code covers the sequence and the struct-ABI, but the **calling
convention** (variadic argument passing, register/stack layout) is a property only a real run
surfaces — certify the parts reasoning can reach, and mark the calling convention explicitly as
**run-unverified**, never as sound. (Concretely: `libc::prctl` is variadic — always pass all five
args explicitly; the team recorded the gotcha to memory.)

**The capture-model evidence split worked exactly as designed — it is defense-in-depth, not a
compromise.** The sprint's split — reasoning-certify the mechanism now, *hold* the live privileged
capture (IB2's observed credential state **and** the negative test, IB3's real ring-wrap) for the
hardware slice — is precisely what caught the variadic bug my reasoning missed: the drop `EINVAL`'d
on the real run. The hold I placed at Slice 3 (do not close IB2 on the read-back alone; require a
privileged op to *actually fail* from inside `run()`) was the safety net. A split that degrades some
evidence to capture-model is easy to read as a weakening; here it was the opposite — the piece that
could only be proven on hardware is exactly the piece a subtle ABI bug hid in, and holding it for the
run is what found the bug. The structural `NicRun` seam kept everything else (sequencing, adapter,
all-or-none, the drop *sequence* and the `is_fully_dropped_to` predicate) fresh-re-runnable, so only
the irreducibly-hardware pieces degraded.

**The recurring content-hash check earned its keep.** I recommended at Slice 1 that a working-tree
content-anchor be re-verified at *every* dependent slice, not pinned once — because a working-tree
API can drift before it commits. It held at Slices 3 and 4 (no drift), then **fired at Slice 5**: the
`directpath` API split `send` into `open`/`send(id)`, the hash changed, and the contract re-pinned
R-7. Had the pin been one-time, the D-1..D-6 mapping would have silently gone stale against a moved
API. The re-derivation showed the change was in fact *correct* (the `open`/`send(id)` split is the
per-session correlation the box needs), but the point stands: an uncommitted-source dependency is a
moving target, and currency has to be re-checked per slice, not assumed from a first pin.

**What the next sprint inherits.**
- *The receive path proven on real hardware* — a box-originated `net-address`+`net-payload` sequence
  reaching a recognizer verdict on the 82599, IB2/IB3/e2e all captured (bin-sha-declared,
  self-coherent), receive-only. Everything hardware-free stays fresh-re-runnable via `NicRun`.
- *Two residuals carried, named:* the **deploy-story device-rebind** (the post-drop rebind is
  privileged and fails, leaving the card unbound on exit — host state that crash-only/epoch-reset
  does not cover; routed to the PM, distinct from R-1's steady-state device-reset), and **R-7's owed
  commit-pin** (the `directpath` facade is still uncommitted working-tree; re-pin to the nic team's
  commit hash once it lands).
- *The out-of-scope threads, named not guessed:* the **reply/response path** (R-c — the proxy
  originates a *new* proxy→box session, blocked on the transport originator-partition, the nic team's
  Phase 4); the **config DSL** (three of four authorities are named fail-closed placeholders — the
  ladder is real and holds closed today by design, the checks await the config surface); and the
  **verb/betas envelope-sourcing** beyond the single Anthropic route (RQ3).
- *The standing disciplines, reconfirmed and one added:* build-and-run-everything-yourself for the
  hardware-free half; the capture-model split with un-fakeable tells (device counters, nonce
  correlation, the EPERM negative test) for the hardware half; per-slice currency re-checks on an
  uncommitted dependency; and — the one this sprint added — **mark the calling convention of FFI as
  run-unverified in any reasoning certification**, and hold the live run that proves it.

### zero.proxy-dev

**What I built.** Five slices, in order: (1) pinned `contract/proxy-nic.md`'s fill-in against
the real API (D-1–D-6 → concrete methods; D-3 quiescence named as a routed gap); (2) resolved
Task 14 with the real per-session `Route` binding (the discriminator test proves it fails under a
stub); (3) the fail-closed lifecycle ladder (`serve`) + the confined-unsafe privilege-drop module +
the `NicRun` seam, sequencing fresh-tested against a fake; (4) the `run_adapter` run-loop driver; (5)
the bench capture on the real 82599 — all three proofs (IB2 both halves, IB3 sustained wrap, the
receive-path e2e) PASS, bin-sha-declared, exact-BDF-gated. When the NIC team landed `open`/`send(id)`,
I did the coherent `NicRun` migration + the contract re-pin (content-hash, `send(id)` into D-1, R-7)
in one pass.

**What evidence changed the plan.**
- *The session-correlation facade gap (the big one).* Building the box role, I found the proxy's
  per-session correlation (net-address stores the `Route`; net-payload on the SAME session reads it)
  was **not expressible over the real box facade**: `Subsystem::send` was `originate`-only
  (`transport.open` allocates a fresh id each call), with no send-on-existing-session surfaced —
  `transport.send(id,…)` existed underneath but was never exposed. Every prior test had fed dispatch
  *hand-chosen* same-session ids (unit tests call `route(SessionId(1), …)` twice; the Sprint-6 reactor
  test crafted ids; the nic RTT is one-message-per-session), so nothing had ever driven two
  same-session frames through the real reactor. First real integration surfaced it — exactly Sprint
  8's purpose. I surfaced it rather than hacking around it; the NIC designer ruled the **proxy
  correlation correct as-built** and the gap a pure NIC-side exposure, and the fix was the ruled
  `open`/`send(id)` API. No proxy rework.
- *Two policy/config forks I surfaced not guessed:* the privilege-drop target uid/gid, and how deep
  rung-3 authority validation goes. The PM confirmed my lean on both — parameterize uid/gid, build the
  ladder + `Authority` abstraction with **named fail-closed placeholders** pending a future config DSL
  (U5-register style), and invent no config system as a side effect.

**What failed / needed correcting.**
- *The `prctl` variadic bug — caught only by the live run.* The first captures failed:
  `drop_to` returned `DropFailed(EINVAL)` at step `PR_SET_NO_NEW_PRIVS`, so the proxy never served and
  the box gave up (surfacing downstream as `NoSuchSession`). Root cause: `libc::prctl` is **variadic**,
  and `prctl(PR_SET_NO_NEW_PRIVS, 1)` left arg3/4/5 as uninitialized stack garbage; the kernel requires
  them zero → EINVAL. Fixed by passing explicit trailing zeros on every short `prctl` call. **The
  reviewer had certified the drop sequence as a sound *reasoning artifact* (order, every-return-
  checked) — but reasoning over the logic cannot catch a C variadic-ABI detail; only the privileged run
  did.** This is precisely why IB2 requires the capture-model, not just a reviewed artifact.
- *The drop breaks the NIC's auto-rebind.* `Subsystem::drop`'s `ixgbe` rebind is a privileged sysfs
  write; after the privilege drop it fails, leaving both cards unbound on exit. I rebound them manually
  after every run. Consistent with the crash-only/epoch-reset posture, but a real operational residual.

**What the next sprint should inherit.**
- **R-c — the proxy→box response/origination path (still open).** This sprint proved the RECEIVE path
  only. The reply direction (net-response/file-data) needs the proxy to *originate* a proxy→box session
  (`open` + `send(id)` on a new session, `model.md` §9) — a design still to be settled before it is
  wired; see [[box-proxy-per-session-correlation-open-send]] and [[transport-uni-directional-sole-allocator]].
- **The config surface (a future Rust DSL, U5-register style).** Rung-3's authority checks (FS roots,
  quarantine `noexec/nosuid/nodev` mount, `openat2` availability, credential) and the privilege-drop
  target uid/gid are **named fail-closed placeholders** today — `validate_authority` marks the exact
  slot each config check lands. The production ladder correctly refuses to serve until they validate.
- **R-7 — the commit re-pin.** `directpath`'s `open`/`send(id)` surface is landed-and-accepted but
  still uncommitted working-tree state; §4 is content-anchored (`3c69e5dd…`), re-verified at each
  slice. Re-pin to the commit hash once the NIC team commits.
- **Named, routed residuals:** the D-3 shipped-surface quiescence gap; the verb/betas envelope-sourcing
  question beyond the single Anthropic route; the NIC-unbound-on-exit operational residual above.
- **A durable lesson, recorded:** `libc::prctl` is variadic — always pass all five args, zeroing the
  unused ones ([[libc-prctl-variadic-must-zero-trailing-args]]).

### zero.proxy-lead

**Routing, not implementing, through a real cross-team blocker.** The sprint's sharpest moment
wasn't a code defect — it was dev holding, correctly, at the box facade's session-continuation
gap rather than working around it. My job there was narrow: verify the finding myself against
`reactor.rs`/`transport/table.rs` before treating it as fact, then route the actual fork (extend
the facade vs. reconsider the correlation model) to the PM rather than rule on it, since it was
genuinely cross-team and above what either proxy role could settle alone. The PM routed it to the
NIC designer, who ruled the proxy design correct — confirming the hold was the right call, not an
excess of caution. Lesson reconfirmed: a "my correlation model might be wrong" finding from a
teammate should be verified on disk, then escalated by its true owner, not absorbed into a
guess dressed as a ruling.

**Independent verification stayed cheap and caught nothing false this time — that's fine.** I
checked dev's and review's claims against the repo at every load-bearing step (grep for the real
API signatures, `cargo build`/`cargo test` for the "green" claims, the memory index for the
recorded gotcha, the feature-gate for the bench bin) rather than accepting summaries. Nothing I
checked contradicted the reports this sprint — which is itself useful signal about the team's
reporting discipline, not a reason to check less next time.

**A cosmetic doc-structure slip, caught before commit.** Dev's and review's retrospectives briefly
landed under two separate `## Retrospectives` headers instead of one shared section (a transient
append-ordering artifact, self-corrected by the time I read the file). Worth naming only because
it's the kind of small thing a lead should catch before a commit, not because it needed escalation.

**What the next sprint inherits, from this seat.** The R-c reply-path design fork is still fully
open and will need the same treatment as this sprint's facade gap: verify what the transport
actually exposes before ruling, and expect the real answer to be an extension of the existing
design (as `open`/`send(id)` was) rather than a rework. The three routed residuals (R-7 commit-pin,
D-3 quiescence, verb/betas envelope-sourcing) and the new NIC-unbind deploy residual are all named,
owned, and outside this sprint's scope to close — track them into whichever sprint picks up Phase 4.
