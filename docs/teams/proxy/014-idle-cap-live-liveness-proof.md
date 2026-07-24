# Sprint 014 — The Live Quiescent-Wake Liveness Proof (Task 20 discharge)

> Proxy team, sprint 14, a single-capture sprint discharging the one held item
> carried since Sprints 11–13. **Redrawn 2026-07-23** after the NIC designer's
> ruling reversed the sprint's original premise (below): the idle park was
> never actually unbounded, and the `idle_cap` this team added is dropped as
> counterproductive. This sprint now proves, on real hardware, that the
> reactor's existing, unmodified quiescent-wake behavior already delivers a
> quiescent arrival within its documented bound. Drafted by zero.proxy-lead
> from planning input by zero.proxy-dev and zero.proxy-review, redrawn
> following the NIC designer's ruling via the PM (2026-07-23).

---

## Scope Boundary

This sprint owns exactly one thing: a bench capture proving that a quiescent
arrival (a head keystroke, and/or an upstream response chunk) reaches the box
within the reactor's existing bound, on the real production path, with no
proxy-added mechanism involved.

It explicitly does **not** own:

- **Any change to the reactor, transport, or directpath crates.** The
  underlying liveness guarantee is already there and correct as shipped
  (designer's ruling, below).
- **Reinstating `idle_cap`/`IDLE_PARK_CAP`.** Confirmed counterproductive —
  removed from `bring_up` entirely (Final State #1); `construct` is called
  with `idle_cap = None` throughout, matching what `s5_capture`/`two_facade`
  already correctly did.
- **A live round-trip to the real Anthropic API or a real operator head.** The
  head-keystroke vehicle is a controlled, nonce'd write to the head socket at
  a known instant — not a real terminal session; a scripted upstream spawn (a
  fake worker on the real reactor) is the upstream-side equivalent if that
  ingress is also covered. Neither requires a live credential or a live head.
- **A negative control.** Unlike the sprint's original shape, there is no
  before/after to contrast — the guarantee this sprint proves was never
  actually broken (below), so there is no "without the fix" arm to compare
  against.

## Problem (redrawn — the ruling)

**The sprint's original premise was reversed.** Sprints 11–13 held a
"quiescent-arrival deadlock" against Task 20's landing, reasoning from the
reactor's documented `Host::block` contract ("park until a NIC wake") read
literally as an unbounded block. Planning this sprint's negative control
required verifying that premise against the actual implementation, and
reviewer found — independently verified against source by the lead — that
`DdWaitHost::block` does **not** park unboundedly: it already returns after
one bounded `~5ms` wait (the `max_ebx` hang-safety ceiling) even with no
retransmit deadline armed, and `Subsystem::run` fires `Event::Idle`
unconditionally every loop iteration before parking again. Since the
production loop drains both ingress channels on every `Idle` tick regardless
of why it woke, a quiescent arrival was **already** being delivered within
~5ms (nothing in flight) or ~10ms (mid-send, the retransmit interval) —
**before this team added anything.** The team's own `IDLE_PARK_CAP = 10ms`
was larger than that existing ~5ms ceiling, making it actively
counterproductive rather than merely inert.

**The NIC designer's ruling** (via the PM, 2026-07-23): confirmed the above.
The three sprints' held-item language was an honest, reasonable reading of
the documented interface (which does read as unbounded) — the gap was between
the documented contract and the actual implementation, not a defect in the
team's reasoning at the time. The designer owns closing that documentation
gap on their side. This team's job now: drop the counterproductive addition,
and prove the *real, already-shipped* guarantee holds on hardware — something
Sprints 11–13 never actually verified live, since every test drove `Idle`
synthetically.

## Goal

Prove, on real bench hardware, that the reactor's existing (unmodified,
pre-Task-20) `block(None)` re-poll delivers a quiescent arrival to the box, on
the real production path — a positive liveness demonstration of a guarantee
that was always there, not a proof that a fix closed a gap. **The `~5ms`
figure is the mechanism's guaranteed worst-case ceiling (the `max_ebx` EBX
timer), not an expected delivery cadence** (corrected post-capture, below) —
the guarantee is "at most ~5ms even with zero ambient wake sources," not
"paced by a ~5ms tick."

### Capture result (2026-07-24) — liveness discharged, cadence corrected

The privileged capture (bin-sha `555ff7c6…`, host `elsenor`, git `a046ece`)
delivered all 20 nonce'd keystrokes intact, under proven quiescence
(`Delivered == 2`, setup + teardown only), at **min=75.8µs, median=102.8µs,
max=146.8µs** — a tight cluster roughly 34× *below* the guaranteed ~5ms
ceiling, not a phase-random spread across it. **This strengthens the liveness
proof, not weakens it**: the reactor's own re-poll delivered an off-NIC
arrival with no coincidental box-frame wake possible, well inside its
guarantee. What it corrects is the *cadence* claim this sprint was originally
drafted around — the `~5ms` EBX is a **worst-case ceiling** the ambient
interrupt/scheduler activity on this non-isolated bench core never lets the
re-poll approach; the *measured typical* cadence here is ~100µs. Both facts
are consistent (100µs is well within a 5ms guarantee); the sprint's original
"paced by ~5ms" framing was wrong and is corrected throughout this document.
The **why** of the ~100µs cadence (plausibly ambient-interrupt-driven MWAITX
exits, per dev's inference) is a NIC/reactor-team characterization question,
named and routed as a non-blocking follow-up (Review Questions) — it does not
gate this sprint's liveness discharge.

## Final State

1. `IDLE_PARK_CAP`/`idle_cap` threading is **removed from `bring_up`**;
   `Subsystem::construct` is called with `idle_cap = None` throughout the
   proxy, matching `s5_capture`/`two_facade`'s already-correct usage. No
   proxy-side configuration of the idle ceiling remains. **This is a latency
   change, not a behavior-preserving no-op** (sharpened at review): before,
   `Some(10ms)` parked via `block(Some(retransmit-or-10ms))`; after, `None`
   parks via `block(None)`'s ~5ms hang-safety re-poll — removing the
   counterproductive cap genuinely restores the faster re-poll, it does not
   merely leave delivery "identical."
2. A capture harness drives the **real production path**
   (`RealNic::run` → `run_adapter_composed`), never a bespoke `park()`/
   `pump_all` loop — the live proof targets the path that actually ships.
3. The box is driven to **genuine quiescence — two conditions, both required**
   (sharpened at review; dropping either mislabels which mechanism is being
   measured): **zero `Delivered` events** observed at the proxy (directly
   countable via `on_verdict`) across the observation window, ruling out a
   coincidental box-frame wake; **and no retransmit deadline pending**
   (`next_deadline` is `None`) — naturally satisfied here since the only
   proxy activity before the window is receiving the box's setup display and
   sending to the head socket, neither a proxy→box reliable send, so nothing
   is ever awaiting an ACK. The harness **asserts** this second condition
   explicitly (not merely relies on it being naturally true), because without
   it the reactor would instead park via `block(Some(retransmit))` — a
   *different* mechanism (~10ms, the §7 retransmit interval), and a proof
   that measured that path while claiming to characterize the `block(None)`
   re-poll would mislabel the mechanism.
4. A **nonce'd arrival** (a head keystroke, and/or a scripted upstream chunk)
   is injected during that quiescent window, at a known instant.
5. Delivery to the box is observed **well within the guaranteed `block(None)`
   ceiling (~5ms)** — corrected post-capture (see Goal): the captured typical
   was ~100µs, a tight cluster far *below* the ceiling, not paced by it.
   **Measured as a distribution over N injections, not one sample** (dev's
   strengthening) — a single injection's latency could in principle land
   anywhere up to the ceiling, so one sample alone wouldn't demonstrate the
   bound. What the distribution actually needs to show, **corrected from the
   original "phase-random 0–5ms spread" framing** (which assumed the re-poll
   itself paces delivery at ~5ms — contradicted by the capture): **max must
   stay bounded by the guaranteed ceiling** (~5ms; an outlier *above* it would
   signal a retransmit path or a missed re-check, i.e. broken quiescence —
   this is the one statistic that remains load-bearing regardless of the
   actual cadence); median and the tight-cluster shape corroborate a genuine
   periodic re-poll rather than one-off luck; **no sub-microsecond samples**
   (structurally guaranteed by the off-NIC vehicle — a sub-µs sample would
   itself signal a harness bug, not a real coincidental NIC wake). The
   ~5ms-vs-~10ms discrimination this bullet originally leaned on (tell #3, as
   drafted) turned out to be moot at the captured scale (both ceilings sit
   34–68× above every sample) — harmless, since the no-retransmit condition
   (Final State #3) already excludes the retransmit path *structurally*, not
   by timing; the max-bounded-by-ceiling check stands on its own.
6. The sprint document carries role retrospectives; the sprint is committed
   with only its intentional files.

## In Scope — one slice

**Slice 1 — the capture.**
Build the harness (bin-sha self-declared, per the standing silicon-evidence
discipline) driving the real `run()` path with a nonce'd head-keystroke
arrival (the simplest, cleanest vehicle — no credential, no network
dependency, a fully controlled delay) during a proven-quiescent window;
capture delivery at ms-scale with the nonce intact. Optionally, repeat the
vehicle with a scripted upstream chunk to cover that ingress too. Gate: the
three-part tell (nonce, proven box-silence, ms-scale-not-sub-us timing)
present, driven on the real production path, exact-BDF bench safety observed
throughout.

## Non-Goals

- Do not reinstate `idle_cap`/`IDLE_PARK_CAP` in any form — confirmed
  counterproductive against the existing ceiling.
- Do not build or require a negative control — there is no "without the fix"
  arm; the guarantee predates this sprint entirely.
- Do not let the capture window carry anything awaiting a proxy→box ACK — a
  retransmit deadline pending would route the wake through `block(Some(_))`
  (~10ms), a different mechanism than the `block(None)` re-poll (~5ms) this
  sprint characterizes; the harness asserts `next_deadline` is `None`, not
  merely assumes it.
- Do not test via a bespoke `park()`/`pump_all` loop as the live proof — that
  exercises a different path than production.
- Do not treat this capture as a live round-trip to real Anthropic or a real
  operator terminal — the vehicle is a controlled, scripted arrival.
- Do not relitigate or reopen Sprints 11–13's closed contracts — their
  held-item language was an honest reading of the documented interface at the
  time; this sprint updates the record going forward, not the past.

## Starting Surface

Audit first, in this order:

1. `directpath/src/ddwaithost.rs`'s `block` — the existing `~5ms` hang-safety
   ceiling this sprint proves, and why it already bounds a quiescent park.
2. `directpath/src/lib.rs`'s `Subsystem::run` — confirming `Event::Idle` fires
   unconditionally every loop iteration before parking, so the composed
   adapter's per-tick drain already runs on this cadence.
3. `proxy/src/lifecycle.rs`'s `bring_up` — where `idle_cap`/`IDLE_PARK_CAP`
   threading is removed.
4. `proxy/src/bin/s5_capture.rs` — the bin-sha self-declare, exact-BDF, and
   capture-model discipline this harness reuses verbatim.
5. `[[silicon-evidence-anchor-degrades]]` — the standing evidence-model
   precedent for a reviewer who cannot freely re-run privileged hardware.

## Method

- **Capture-model, same discipline as every prior hardware sprint**: the
  harness self-declares its running bin-sha (`sha256` of `current_exe`, never
  a shell-computed hash); the reviewer's verdict splits binary-coherence
  (theirs, verified fresh) from run-outcome (taken from the dev's self-pinned
  capture, flagged as such).
- **Exact-BDF bench safety** (the standing hard constraint): gate on the exact
  bench BDFs via `Bdf::parse` literals, never device-id enumeration; the
  production NIC must never be touched; operator paste-and-confirm before any
  hardware run.
- **The head-keystroke vehicle needs an active display session first** —
  `route_head_input` fail-safe-drops a keystroke for an unknown/stale
  display. The harness sequences a box→head display as setup *before* the
  observation window opens (a `Delivered` event that precedes, not violates,
  the "zero `Delivered` across the window" claim), establishing `s_display`
  as active before injecting the nonce'd keystroke into the now-open
  quiescent window.
- **The tell is external and three-part** — nonce, proven quiescence, and
  ms-scale-not-sub-us timing — a positive demonstration, not a causal
  before/after contrast (no negative control this time; see Problem).
- **Why positive-only is causally complete without a negative control — the
  load-bearing argument** (dev, endorsed at review): the arrival vehicle is
  **off-NIC** (a head-socket write, or a scripted spawn's channel send), so
  there is *structurally* no NIC-event wake that could deliver it. Under
  proven quiescence (both conditions, Final State #3), the `block(None)`
  hang-safety re-poll is the *only* mechanism that can drain an off-NIC
  arrival — so delivery-at-all, at the right timescale, already pins the
  cause. This is what makes a positive proof sufficient here, not a
  simplification of a causal argument a negative control would otherwise be
  needed for.

## Acceptance

- REJECT a capture that doesn't drive the real `run()` → `run_adapter_composed`
  production path.
- REJECT a capture missing any of the three tell parts (nonce, proven
  box-silence, ms-scale timing).
- REJECT any reinstated `idle_cap` configuration in `bring_up` or elsewhere in
  the production wiring.
- REJECT any device-id-based enumeration or a missed exact-BDF/operator-confirm
  step.

## Review Questions

1. **The ~100µs measured cadence vs. the ~5ms guaranteed ceiling — RESOLVED**
   (designer's explanation via the PM, 2026-07-24, confirming dev's inference
   exactly): `MWAITX` returns on the earliest of a NIC frame (~4µs), the EBX
   timer (~5ms), or *any* interrupt delivered to the core — and on a live
   multi-core Linux host, ambient interrupts (scheduler ticks, IPIs, RCU work,
   other device interrupts) land roughly every ~100µs. So `block(None)`
   almost always returns on an ambient interrupt in practice; the ~5ms EBX
   only governs the pathological case of a core receiving *no* interrupts at
   all, which doesn't occur on a running system. Both numbers are correct,
   describing different things (a worst-case guarantee vs. an always-met
   practical cadence); spurious ambient-interrupt wakes are benign (no frame,
   a harmless re-poll), and the CPU is genuinely idle in `MWAITX` between
   them, not spinning. The liveness proof stood on the ceiling regardless,
   and still does — this closes the question, not the discharge.

## Retrospectives

### zero.proxy-dev

**What I built — and unbuilt, and rebuilt.** The currency fix threaded `Some(IDLE_PARK_CAP = 10ms)`
through `bring_up` — the "Task 20 wake" we'd held three sprints against. Days later I reverted it to
`None` when the deadlock premise collapsed. Then I built a ~250-line capture harness around the `~5ms`
re-poll (box + proxy on the bench via `two_facade`, a third operator thread injecting nonce'd keystrokes
on the fake head, the off-NIC causal pin, the N-sample timing tell). Then the hardware said `~100µs`.
Three different pictures of one mechanism in a single sprint, from the implementer's seat: a value I
added, removed, and then out-measured.

**A scalar I add is a guess about the world until hardware measures it.** `idle_cap = 10ms` felt
principled — a fixed proxy characteristic, aligned to the §7 window, marked provisional. It was
*counterproductive*: above a native `~5ms` ceiling I didn't know existed. Then `~5ms` itself — which the
whole redraw and my tell #3 were built around — turned out to be a worst-case *ceiling* the ambient
system never approaches; the real wake is `~100µs`. The reactor was livelier than the model at every
step, and every step I learned that only by measuring, never by reasoning from the code constant. A
constant in the source is a *bound*, not the behavior — treating my own added scalar as if it described
reality is the trap.

**Holding the green capture is the whole job.** The capture came back all-`[PASS]` — 20/20 nonces,
silent window, max under the bound. I could have forwarded "CAPTURE PASS, matches ~5ms" to review; the
coded gate said so. But the numbers — `min=76µs median=103µs max=147µs` — were a tight cluster `~50×`
below the `~5ms` my narrative claimed, and a tight cluster is not the `0–5ms` phase-random spread a
`~5ms` re-poll would produce. The green flag and the measured data disagreed, and the data is the truth.
So I held it for a joint read instead of shipping it. That is the discipline in one move: **a passing
coded gate is necessary and never sufficient — read the measured values, not the status flag** — the
same "evidence over assertion, from the live system, never a summary" rule that governs every build
report, turned on my own green capture.

**I wrote the tell that passed for the wrong reason.** Tell #3 — "max bounded by `~5ms`, not `~10ms`,
proves `next_deadline == None`" — was mine, and it *passed* (`147µs ≤ 8ms`), but trivially, for a reason
unrelated to the distinction it was built to draw: at `~100µs` both ceilings sit `50–100×` above the
data, so it distinguishes nothing. It was belt-and-suspenders over the structural `next_deadline == None`
exclusion (the proxy originates nothing pre-keystroke), so no load-bearing leg was lost — but I own that
I encoded the `~5ms`-as-cadence assumption into a gate, and a green on that gate is exactly what would
have hidden the wrong assumption had I trusted it over the numbers.

**Coherence is the dev's to pre-verify, not the reviewer's to discover.** Before sending the capture I
rebuilt it clean and hash-checked — and my first verify, in a `/tmp` target dir, did *not* match. Rather
than ship that as a scare or wave it off, I isolated it (the target-dir path leaks in un-remapped; a
repo-tree rebuild reproduces `555ff7c…` to the byte) and handed review a verified recipe plus the exact
gotcha. The self-declared bin-sha binds numbers to bytes only if the recipe actually reproduces;
verifying that myself, failure mode included, is what lets "run-outcome from the dev's self-pinned
capture" be trusted without re-running privileged hardware. (The harness itself was un-runnable until the
one privileged run — the `two_facade` `Endpoint` mapping (box=low/proxy=high, *not* s5_capture's
harmless receive-only inversion, since both sides originate here), the teardown-wake of a parked proxy,
the off-NIC injection — all compile-verified, all proven only live, and all held on the first run. The
capture-model working as designed: reason it correct, compile it, prove it live, once.)

### zero.proxy-review

This one sprint reversed its own premise TWICE, and both reversals came from the
same discipline: verify against the implementation and the measurement, not the
documentation or the model, at every load-bearing step.

**Reversal one — the deadlock that never existed.** The sprint began as a proof
that Task 20's `idle_cap` fixed a quiescent-arrival deadlock, reasoned from the
reactor's `Host::block` doc contract ("park until a NIC wake") read as an
unbounded block. Designing the negative control forced that premise against the
actual `DdWaitHost::block` impl — which returns after a bounded `~5ms max_ebx`
re-poll and, with `Subsystem::run` firing `Idle` every iteration, was already
servicing a quiescent arrival before this team added anything. There was no
deadlock; the `idle_cap` was counterproductive and was dropped. The held-item
language across Sprints 11–13 had been an honest reading of the documented
interface — the gap was documentation vs implementation, which only a
skeptical re-verification (sharpened by a *negative* control's bar) surfaces.

**Reversal two — the mechanism that measured different from the model.** Redrawn
as a positive proof of the `~5ms` re-poll, the capture then delivered at `~100µs`
— a tight cluster 34× below the modeled ceiling. `~5ms` turned out to be the
*guaranteed worst-case ceiling* (the `max_ebx` bound), not the *cadence*; the
real wake is far more frequent (ambient-driven on this non-isolated core). A code
constant is a bound, not the observed behavior — and only the measurement reveals
which. My own sharpened tell #3 (max distinguishing `~5ms` from `~10ms`) rested on
that same `~5ms`-as-cadence assumption and went moot at this scale; it was
belt-and-suspenders over the structural `next_deadline == None` exclusion, so no
load-bearing leg was lost, but I own that it inherited the premise the data
corrected.

The through-line: a green coded gate is necessary, never sufficient. Here the gate
passed while the un-fakeable timing data simultaneously *proved* the liveness and
*corrected* the mechanism the green would have hidden — and the developer holding
the green capture for a joint read rather than shipping it is exactly what makes
the capture-model trustworthy.

**The remap-recipe coherence discipline worked exactly as designed.** My first
naive rebuilds did not hash-match — correctly, because they lacked the
`--remap-path-prefix` flags. With the capture's self-declared Provenance recipe
(rustc + debug + the exact remap rustflags, in the repo working tree), the rebuild
reproduced the bin-sha `555ff7c…` to the byte, binding the run-outcome to the exact
reviewed source. The self-documenting header is what let a reviewer close
binary-coherence independently without re-running privileged hardware — the split
(coherence mine, run-outcome from the self-pinned capture) held cleanly.

This closes the Phase-4 arc. The proxy is a runnable, visible, lossless production
loop whose quiescent-wake liveness is now proven live on real hardware; the wake-
source cadence (RQ1) is the one named, non-blocking follow-up to the NIC/reactor
team.

### zero.proxy-lead

**Holding a green result for interpretation, twice in one sprint, is the clearest
evidence this loop's discipline generalizes beyond any single check.** Dev held
the software-gate-passing capture because the numbers didn't match the story the
contract told; reviewer, brought in for a joint read rather than a formal gate,
independently re-derived the same ceiling-vs-cadence correction and owned that
their own tell #3 rested on the premise the data corrected. Neither of those
moments needed my arbitration to happen — they needed only that the norm ("a
green gate is necessary, not sufficient; read the numbers") was already load-
bearing enough that stopping to interpret was the obvious move, not a deviation
requiring permission. **Rule: the measure of whether an evidentiary discipline has
actually taken hold is whether it fires without the lead prompting it — this
sprint is the cleanest instance of that yet.**

**Two premise reversals in one sprint, both caught before they shipped, both
routed to their actual owners rather than resolved by guessing.** The
"unbounded park" premise and the "~5ms cadence" premise were each wrong in the
same shape: reasoned from a documented interface or a code constant rather than
verified against the implementation or the measurement. Both times the team's
job was to verify precisely enough to know *what* was wrong, characterize it
honestly, and route the *why* (ambient interrupt cadence) to the team that
actually owns the mechanism — never inventing an answer to close the sprint
faster. That the same shape of error recurred within one sprint, and was caught
identically both times, suggests the lesson has actually generalized rather than
being a one-off catch.

**What this sprint — and the arc — leaves behind.** RQ1 (the ~100µs wake-source
cadence) is named and routed, non-blocking. The Phase-4 arc that started with
Sprint 10's reply seam now ends with a runnable, lossless, live-proven proxy;
every held item across five sprints (F1, F2, the data-loss defect, Task 20, and
now this) closed with verified evidence rather than assumption, and every
cross-team fork (the session-continuation gap, the PTY directionality, the two
premise reversals here) was routed to its actual owner rather than guessed at
by the team that couldn't settle it alone.
