# Sprint 007 — Final Integration Validation (E6): the operational stack + honest residual grounding

> The `nic` team's seventh sprint, and the one that makes the subsystem **operational**. Sprint 6 proved a
> single raw frame crosses the DAC on live silicon (pure-direct, no IOMMU). Sprint 7 assembles the pieces
> into the running heartbeat the whole upstream stack rests on: the **reactor-driven bounded-poll loop**
> against the live 82599, **steady-state ring management**, and the **transport sessions** carried over the
> live link — then takes the **cross-cutting integration-validation measurements** that only exist once the
> stack is assembled (E6), and **grounds the honest-open residuals** (or keeps them visibly open). The
> organizing intent is unchanged: *"The goal is not maximum throughput. The goal is UART simplicity with a
> higher bit rate."* (`architecture.md`).
>
> **Standing hard constraint (carried from Sprint 6):** the operation runs on the **live production host**;
> only the DAC-paired bench cards (`41:00.0`/`63:00.0`) are driven, via the compile-enforced `Allowed(Bdf)`
> token + gate 2; the production NIC (`61:00.0`) is never touched. No IOMMU — the owned-DMA posture (§2) is
> in force, and now **operationally exercised at sustained scale**, not just one frame.
>
> Team: `nic` (lead `zero.nic-lead`, dev `zero.nic-dev`, review `zero.nic-review`, pm `zero.nic-pm`).
> Authority: [`architecture.md`](../../nic/architecture.md) (§3 reactor, §3.1 copy boundary, §4 sessions,
> §5 events, §6–7 protocol/reliability), [`grounding/kernel-bypass-and-dma.md`](../../nic/grounding/kernel-bypass-and-dma.md)
> (§4 bounded-poll idle-wakeup), [`grounding/data-path-and-events.md`](../../nic/grounding/data-path-and-events.md)
> (§3 ACK-bound reclamation, §4 idle-wakeup, §6 pool sizing), [`concerns-to-watch.md`](../../nic/concerns-to-watch.md),
> [`epics.md`](../../nic/epics.md) (E6), [`docs/teams/nic/006`](006-pure-direct-bringup-and-transmission.md)
> (the Sprint-6 retros + the two exercised residuals). Builds on Sprint 6 (`73e8e92`). Salvage in hand: the
> moved reactor (`reactor/`) with the bounded-poll `Host` seam; the built transport (`transport/`, S1); the
> driver copy-boundary + guard (`driver/`, byte-frozen); the `directpath` control module.

---

## Scope Boundary

**This sprint owns:**

1. **The reactor-driven live loop (E4 realized on silicon).** Wire the moved reactor — its poll-then-block
   algorithm with the **bounded-poll `Host`** impl (`kernel-bypass-and-dma.md §4`: no interrupt delivery on
   the pure-direct path, so "block" is a bounded poll / short sleep) — to the real 82599: **busy-poll the
   RX ring → demux by session ID → the session reacts inline; drain output to the TX ring (sole writer);
   when idle, bounded-poll on the retransmit deadline** (`architecture.md §3`). The copy boundary
   (`§3.1` — copy each frame out of the DMA region into app-owned memory at the earliest point) holds.
2. **Steady-state ring management.** Beyond the single-frame PoC: continuous RX **refill** (copy-out-early
   then hand the descriptor back), TX **reclaim** on the ACK-bound buffer lifecycle
   (`data-path-and-events.md §3` — buffers held until acked), back-pressure via the ready-set of sessions,
   and the **retransmit loop** (`§7`: ~10 ms interval + ~1–5 s give-up, silent internal reclaim). The rings
   stay full under sustained traffic — the overrun-avoidance the whole reliability story exists for.
3. **Transport sessions over the live link (the Sprint-6 R4 follow-on).** Carry the built transport (S1) —
   `[session ID][payload]` frames, bare-ACK by direction, strict stop-and-wait, multiplexing-by-instantiation,
   lifecycle by emission/silence (`§4, §6, §8`) — over the live DAC, `41:00.0 ↔ 63:00.0`. Real multiplexed
   sessions, not a raw frame.
4. **The integration-validation measurements (E6's genuinely-final pass, `architecture.md §9`).** The
   cross-cutting end-to-end numbers that only exist assembled: **single-ring throughput at the real RTT**;
   the **reactor keeping up on one core under full load**; the **~10 ms retransmit window under a deliberate
   redraw storm**; **DMA buffer-pool sizing against ACK-hold** (`data-path-and-events.md §6`). Measured
   against the design's stated expectations (§2 headroom, §7 sizing), self-pinned.
5. **Ground the honest-open residuals — or keep them visibly open (`concerns-to-watch.md` rule).** Attempt
   a **verified account of the `dma.rs` crossing-soundness** (the volatile/visibility/`DD`-ordering/no-tearing
   crossing — the project's longest-standing open hazard); **ground what sustained operation exercised** of
   the owned-DMA posture; and **surface** the design-of-record reconciliation (below) with that grounding.

**This sprint does NOT own** (named, with where it belongs):

- **The design-of-record rewrite of `architecture.md`** — the **untrusted-but-contained posture (§2.1)
  stands**; the NIC is still untrusted-but-contained, we just **handle the containment ourselves**. What
  reconciles is the containment *mechanism*: the read-verified driver guarding adversarial descriptor/buffer
  content (vector 2) + owning the device, in place of a kernel-configured IOMMU — so §2.1's *"structural
  hardware: the IOMMU"* trusted-base bullet and the *(B) IOMMU-airtight* requirement update to
  *we-contain-it-ourselves*, with the no-IOMMU DMA-reach (vector 1, a compromised firmware) as the honest
  **owned residual** the policy audit weighs. Plus §3/§5 (the `epoll`/`eventfd` reactor → bounded-poll).
  These are the **designer's/operator's** to ratify; the nic team **grounds** (what the PoC/operation
  exercised) and **surfaces** the drift with evidence — it does not unilaterally rewrite the design
  authority (`[[normative-spec-is-the-authority]]`).
- **The policy-layer audit** of the owned-DMA posture — a cross-team activity "with the policy layer"
  (`kernel-bypass-and-dma.md §2`); the nic team provides the grounding of what sustained operation
  exercised, not the audit verdict.
- **The application layer** (PTY / file / request bindings) — rides on sessions, above the transport
  (`architecture.md §4, §8`; `epics.md` scope-out). Sprint 7 carries opaque session bytes, not their meaning.
- **New protocol/reactor features** — the transport and reactor algorithms are built and frozen; wire them
  to the live device, do not re-author (the guard/copy-boundary + reactor algorithm stay byte-frozen —
  only the backing changes, the Sprint-6 seam discipline).

## Problem

Sprint 6 proved one raw nonce frame crosses the DAC. The subsystem is **not yet operational**: nothing
runs the continuous reactor loop against the live device, the rings are single-frame not steady-state, no
transport sessions ride the live link, and none of the integration-validation measurements (throughput,
one-core load, the retransmit window, pool sizing) exist. And two hazards are open + now *exercised* — the
`dma.rs` crossing-soundness is undischarged, the owned-DMA posture ran unconfined on production — while the
design-of-record (`architecture.md §2.1` IOMMU-containment, §3/§5 epoll) still describes the **pre-pivot**
model.

## Goal

The subsystem is **operational** end-to-end: the reactor drives the live 82599 in a steady-state
bounded-poll loop carrying multiplexed transport sessions `41 ↔ 63`, the integration-validation
measurements are taken against the design's expectations, and the honest-open residuals are grounded (or
kept visibly open) with the design-of-record reconciliation surfaced to the designer.

> "It is the heartbeat of the whole upstream stack: the concurrency model and event model that drive
> everything above rest on this loop." — `architecture.md §1`.

## Final State

- The bounded-poll `Host` reactor drives the real RX/TX rings: RX-drain → demux → react; TX-drain (sole
  writer); bounded-poll idle on the retransmit deadline; copy-out at the edge (§3.1).
- Steady-state: rings stay full under sustained traffic; buffers ACK-bound reclaimed; retransmit on
  silence (~10 ms) + silent give-up (~1–5 s); back-pressure by ready-set.
- Multiplexed transport sessions carry data `41 ↔ 63` (stop-and-wait, ACKed, open-by-emission, dormant≠dead).
- The measurements exist: throughput at real RTT, reactor-on-one-core under load, the window under a redraw
  storm, pool sizing vs ACK-hold — vs §2/§7 expectations, self-pinned, bin-sha-bound.
- The `dma.rs` crossing-soundness is grounded (a verified account) or kept honestly open; the owned-DMA
  operational grounding is recorded; the `architecture.md §2.1/§3/§5` + spine reconciliation is surfaced to
  the designer with the evidence. Production `61:00.0` never touched throughout.

## In Scope — the ordered slices

**Slice 1 — The reactor-driven live loop (single session).** Wire the bounded-poll `Host` to the live
82599 RX/TX: the poll-then-drain loop, one session end-to-end (RX frame → demux → react → TX ACK), the
retransmit deadline composed into the bounded-poll idle, the copy boundary at the edge. **The bounded-poll
`Host` reinterprets `block(None)` as a CAPPED poll** (a short-sleep poll loop), not an infinite park — the
pure-direct path has no interrupt to wake an infinite park, so an idle-with-no-deadline wait must keep
polling the RX ring or it misses frames (`kernel-bypass-and-dma.md §4`). *Method: the reactor algorithm
frozen (empty-diff vs HEAD `73e8e92`) — but this is the FIRST live-steady-state exercise of that algorithm
(Sprint 6 ran it against a stub + one frame), so the empty-diff proves "not re-authored," NOT "works live";
the `block(deadline)→poll-interval` logic gets pure host tests (the `compute_timeout` analog), and the live
single-session round-trip is bench-run + bin-sha-bound.*

**Slice 2 — Steady-state ring management.** Continuous RX refill + TX ACK-bound reclaim + back-pressure +
the retransmit loop (interval + give-up), rings full under sustained single-session traffic. **Fixes a real
steady-state backing bug the dev grounded** (sim-hidden, like the Sprint-6 tail bug): `TxPath::emit` uses
`first_free()` (lowest free slot) while `VolatileTxRing` sets `TDT=(slot+1)`, so once reclaim frees slots
**out of order**, TDT is written *backwards* of TDH and the device stops transmitting — needs a **monotonic
produce cursor** (a `tx.rs` **backing** fix, below the seam; the reactor algorithm + `TransmitSink` contract
stay frozen). *Method: the ACK-bound buffer lifecycle (`data-path-and-events.md §3`); the monotonic-cursor
fix by NEW tests exercising out-of-order reclaim (the committed `dma_buffer_freed_on_tx_dd` shape); prove
the ring stays full + no overrun + TDT-monotonic under a sustained stream; the give-up reclaims (no leak).*

**Slice 3 — Multiplexed transport sessions over the link.** K concurrent sessions interleaved over the one
wire (the reactor's ready-set), each stop-and-wait + ACK, lifecycle by emission/silence. *Method: carry K
sessions `41 ↔ 63`; demux-by-ID correct; interleaving trivial (no scheduler); dormant≠dead honored.*

**Slice 4 — Integration-validation measurements + residual grounding + reconciliation surfacing.** The E6
measurements (throughput / one-core load / the window under a redraw storm / pool sizing) vs §2/§7 — with
the **three premise-gates** separated from characterization (Acceptance 4). The crossing-soundness splits:
the **fence CODE** (explicit acquire between the `DD` load and the payload load, replacing the implicit
x86-TSO reliance) is **slice-sized and in-scope**; the full **verified-soundness ACCOUNT** (the cited
reasoning artifact — device/PCIe write-ordering + x86-TSO + no-tearing + the platform residual) is
**larger-than-a-slice → 7b or kept honestly open**, never papered to fit. Plus the owned-DMA operational
grounding, and the `architecture.md §2.1/§3/§5` + spine reconciliation **surfaced to the designer** as a
proposed DESCRIPTIVE diff (the posture stands — §2.1's containment MECHANISM reconciles to
we-contain-it-ourselves for vector 2, the no-IOMMU DMA-reach vector 1 the owned residual). *Method:
self-pinned measurements bin-sha-bound + a SOURCE-verified methodology (how each number is computed, so it
can't be fabricated); the account is cited-reasoning checked against its specs, not "it ran"; the doc
reconciliation is proposed-not-self-applied, descriptive-not-prescriptive of the new §2.1 text.*

## Non-Goals

- **No design-of-record rewrite** by the nic team — ground + surface; the designer/operator ratifies
  `architecture.md`/the spine.
- **No policy-layer audit verdict** — the nic team grounds what operation exercised; the audit is the
  policy layer's.
- **No application-layer semantics** (PTY/file/request) — opaque session bytes only.
- **No new protocol/reactor features** — the algorithms are frozen; wire, don't re-author. The
  guard/copy-boundary + reactor algorithm stay byte-frozen (only the backing changes).
- **No maximum-throughput tuning** — "not maximum throughput; UART simplicity at a higher bit rate"
  (`architecture.md §2`); the measurements characterize, they don't drive optimization.
- **Only the bench cards** — production `61:00.0` never driven; the `Allowed(Bdf)` token + gate 2 stay.

## Method

- **Seam discipline (proven twice), but the empty-diff is necessary-NOT-sufficient here.** The reactor
  algorithm + the transport + the guard/copy-boundary are built and frozen; Sprint 7 wires the
  **bounded-poll `Host`** + the **live rings** behind the existing seams (`Host`/`RxSource`/`TransmitSink`).
  But Sprint 7 is the **first live-STEADY-STATE exercise** of the frozen reactor algorithm — Sprint 6 ran
  it against a stub RX + one frame — so a latent bug under continuous live timing is **invisible to the
  diff** (the TX-ring `TDT`-backwards-on-out-of-order-reclaim bug the dev grounded is the exemplar: a
  `tx.rs` backing fix, the seam contract unchanged). Certification = **empty-diff (not re-authored) AND the
  bench steady-state run (works live) AND the new `Host`/loop/backing tests** — a green diff is necessary,
  not sufficient.
- **Source-proven, not run-observed; bin-sha per hardware run.** The Sprint-6 coherence standard carries:
  every live-device run is bin-sha hash-bound to the committed source; the reviewer certifies source halves
  at zero hardware risk and re-derives run-outcomes from the dev's self-pinning captures (never becoming a
  second unconfined-DMA risk on the live host).
- **The safety spine stays.** The `Allowed(Bdf)` token is the sole state-change choke-point; gate 2 (i)
  validates every DMA physaddr; production `61:00.0` never touched — source-proven + capture-corroborated.
- **Honest-open discipline (`concerns-to-watch.md`).** Ground a residual only when actually shown; else
  keep it visibly open — a confident-sounding fix on the crossing-soundness would be the exact failure the
  file exists to prevent. The design-of-record reconciliation is **surfaced with evidence**, not
  self-applied.

## Acceptance — the gates

1. **The reactor loop drives the live device** — the bounded-poll `Host` reactor runs a single session
   end-to-end on the real 82599 (RX-drain → demux → react → TX-ACK, retransmit deadline in the idle wait),
   copy boundary at the edge; the frozen reactor algorithm proven empty-diff, the live round-trip bench-run
   + bin-sha-bound.
2. **Steady-state holds** — under a sustained single-session stream the ring stays full (no overrun, no
   starvation), buffers are ACK-bound reclaimed (no leak), retransmit fires on genuine silence and the
   give-up reclaims — observed, self-pinned.
3. **Multiplexed sessions operate** — K concurrent sessions carry data `41 ↔ 63`, demux-by-ID correct,
   interleaving trivial, dormant≠dead — observed.
4. **The measurements exist and are honest — three premise-GATES + characterization, not one gate.**
   *Characterization* (a surprise surfaces, does not fail the sprint): single-ring throughput at real RTT
   (no minimum — "not maximum throughput"), and the raw one-core headroom number. *Premise-gates* (a
   violation breaks a design premise — surface + resolve, never tune away): (a) **reactor keeps up** on one
   core at the operational demand (no unbounded backlog/starvation — the single-threaded model fails else);
   (b) **the ~10 ms window ≥ the worst observed ACK latency** under the redraw storm (else spurious
   retransmits/duplicates — §7 correctness; a violation is a real finding **routed to the designer**, 10 ms
   is a §7 design constant, re-sized not silently tuned); (c) **pool sizing ≥ the peak observed ACK-hold**
   (else the ring can't stay full — overrun). Each number bin-sha-bound with a source-verified methodology
   (so it can't be fabricated).
5. **Residuals grounded-or-open + reconciliation surfaced** — the `dma.rs` crossing-soundness is either a
   verified account or explicitly kept open (no papering); the owned-DMA operational grounding recorded;
   the `architecture.md §2.1/§3/§5` + spine drift surfaced to the designer with the PoC/operation evidence.
6. Independent review re-derives 1–5 from source + the self-pinning captures at zero hardware risk;
   **rejects** any over-claim (a papered crossing-soundness, a tuned-away measurement surprise, a frozen
   algorithm altered under "wiring," a self-applied design-of-record rewrite, production touched).

## Review Questions

- **R1 (PM/operator — the size/split call; the load-bearing one; dev + reviewer both endorse a split).**
  Large sprint — assembly (Slices 1–3) + the E6 measurements + the crossing-soundness. *Lean (dev +
  reviewer agree): one sprint through **Slice 3 = operational**, with a **hard checkpoint** before the
  Slice-4 residual work; default to **7a = Slices 1–3 + the measurements** (they measure the assembled
  stack, and are quick — E6's "small" pass) / **7b = the crossing-soundness verified-account + the
  design-of-record reconciliation** (a different KIND of work — cited reasoning, not wiring — and the
  project's longest-open hazard, the most likely to overrun).* The crossing-soundness **defaults to its own
  effort rather than being papered to fit a slice.* Confirm the shape.
- **R2 (PM/designer — the `dma.rs` crossing-soundness: SPLIT the code from the account).** The dev + reviewer
  converge: the **fence CODE** (an explicit acquire between the `DD` load and the payload load, replacing
  the implicit x86-TSO reliance) is **slice-sized, in-scope** (a concrete strengthening). The full
  **verified-soundness ACCOUNT** — the cited reasoning artifact (the 82599/PCIe payload-before-`DD`
  write-ordering the *must-cite* crux, x86-TSO LOAD-LOAD, no-tearing, the stated platform residual) — is
  **larger-than-a-slice → 7b or kept honestly open**, never a papered partial. It is the E3 unsafe-DMA
  soundness (`architecture.md §3.1` "not analyzed to resolution"). Confirm: fence-code in, full-account
  scoped to 7b/open.
- **R3 (PM/designer — the design-of-record reconciliation boundary).** `architecture.md §2.1` (IOMMU-
  containment → owned), §3/§5 (epoll → bounded-poll), and the spine describe the pre-pivot model. The nic
  team **grounds + surfaces** with the PoC/operation evidence; the **designer** reconciles the design
  authority. Confirm that boundary (nic grounds, designer rewrites) — the nic team should not rewrite
  `architecture.md` itself.
- **R4 (PM/operator — the "operational" bar; gates the dev's Slice 1).** What does *operational* require —
  single session (Slice 1), steady-state (Slice 2), or full multiplexing (Slice 3)? *Lean (dev + reviewer
  agree): **Slice 3 = multiplexed steady-state = operational** — the upstream stack rests on
  K-sessions-demuxed-on-one-wire (the whole transport/reactor design), so a single session is a necessary
  step, not the consumer's "operational."* And the measurement bar is **three premise-gates** (reactor
  keeps-up, window ≥ worst-ACK-latency, pool ≥ peak-hold) + throughput/headroom characterization (Acceptance
  4), not one gate. Sub-question the dev needs to start Slice 1: **pin exactly what the single-session
  end-to-end delivers** (the Slice-1 acceptance bar). Confirm.

---

## Outcome & Retrospectives — 7a close (2026-07-17)

**Validated. The subsystem is OPERATIONAL and MEASURED.** All 7a slices independently ACCEPTED by
`zero.nic-review`, each frozen half proven empty-diff and each live-device run bin-sha hash-bound to the
committed source. Over the live DAC `41:00.0 ↔ 63:00.0`, pure-direct / no-IOMMU: the reactor-driven
bounded-poll loop (single-session round-trip), steady-state ring management (128 round-trips across 2
depth-64 wraps, retransmit + give-up reclaim), **K=32 multiplexed concurrent sessions** (demux-by-ID,
byte-equal, no leak — the operational milestone, needing *no new backing* because the frozen reactor +
transport already do multiplexing-by-instantiation, §4), and the E6 integration measurements. Production
`61:00.0` was never touched. The gate-b **§7 window-margin finding** (the K=32 drain latency ~11.5 ms
reaches the 10 ms retransmit window — masked to 0 spurious retransmits by drain-before-deadline, but a thin
margin) was **surfaced-not-tuned** and routed to the designer for 7b.

**Sprint split:** 7a (this — operational + measurements) is complete and committed. **7b** (its own next
effort) = the `dma.rs` crossing-soundness verified account (the project's longest-open hazard) + the
`architecture.md` reconciliation surface. On that reconciliation, the operator-clarified framing governs:
**the untrusted-but-contained posture STANDS** — the NIC is still untrusted-but-contained, the driver guard
containing adversarial content (vector 2, unchanged); only the vector-1 DMA-reach became the owned residual
with no IOMMU. §2.1's containment *mechanism* reconciles to *we-contain-it-ourselves*; it is **not
"containment invalidated."** These items are honest-open, not discharged by the operational milestone:
**operational ≠ crossing-sound ≠ (fully) grounded-contained.**

### Lead retrospective — zero.nic-lead (Sprint 7a, 2026-07-17)

**The seam discipline held for a third consecutive sprint — the whole point, proven.** After containing two
architectural pivots (bare-metal, then pure-direct), the frozen reactor + transport + guard/copy-boundary
reached *operational + measured* on live silicon **without re-authoring the accepted logic** — every fix was
backing below the seams, git-verified byte-identical throughout. The K=32 multiplexing milestone needed *no
new code beyond the runner*, because multiplexing-by-instantiation was already built and frozen. When the
seams are right, the integration is a wiring exercise.

**"Necessary-not-sufficient" earned its keep, on both sides of the loop.** The empty-diff proves *not
re-authored*, never *works live* — the first live-steady-state exercise is load-bearing, and it caught two
single-frame-hidden backing bugs at Slice 2 (the TX monotonic-cursor and the reviewer's *own* Sprint-6
RX-refill miss). The reviewer owned its miss precisely and extracted the durable lesson (the discipline
applies to the ring *backing* — the re-arm doorbell semantics — not only the reactor algorithm); the dev
surfaced its own bug rather than quietly fixing it. That honesty on both sides is what makes the milestone
trustworthy.

**Honest-open held at the finish.** The gate-b window-margin finding was surfaced to the designer with the
§7 constant and the poll interval both untouched — §7 anticipated exactly this ("dial the window up if a
measurement surprises us"). The crossing-soundness stays a 7b cited-reasoning account (reject "it ran"). And
I conceded cleanly where the operator punctured my framing: I over-stated §2.1 as "invalidated" — the
posture *stands*, we handle the containment ourselves.

**A lead-conduct lesson I recorded.** I dressed team-converged answers (the 7a/7b split, the fence-code
split, the operational bar) up as forks for the operator, who correctly flagged it — *"feels like you
already answered your own questions."* When the dev and reviewer converge on a recommendation, adopt it and
proceed; surface to the operator only what is genuinely theirs and still open. Captured as durable feedback.

**What 7b inherits (process):** the crossing-soundness held to a cited account checked against specs; the
design-of-record reconciliation *surfaced, not self-applied* (nic grounds, designer rewrites); the
posture-stands framing; and the poll-interval / ring-depth / pool design parameters to revisit against the
7a numbers.

### Developer retrospective — zero.nic-dev (Sprint 7a, 2026-07-17)

**Built + proven across 7a** (the operational subsystem, live on the DAC `41 ↔ 63`, pure-direct, no IOMMU):
Slice 1 — the reactor-driven live loop (the frozen reactor + bounded-poll Host drives a single-session
round-trip through the real `copy_out`/`RxDrain` seam; new backing only: the bounded-poll Host
`block(None)`=capped poll + the Ethernet encap/decap). Slice 2 — steady-state rings (128 round-trips across
2 depth-64 wraps, RXMPC=0, TX-tail monotonic; retransmit + give-up reclaim, no leak). Slice 3 — multiplexed
K=32 (demux-by-ID, byte-equal, no foreign/loss/leak, dead-peer reclaim — **no new backing**,
multiplexing-by-instantiation paying off). Slice 4a — E6 measurements + the dormant-revive live demo. All
hash-bound, fresh-nonce-listed, reversible; `61:00.0` never touched.

**Evidence that changed the plan (freeze-diff is necessary-NOT-sufficient — proven repeatedly):** two
single-frame-hidden backing bugs caught + fixed at the first steady-state exercise, both below the frozen
guard — the TX monotonic-produce-cursor (first_free posted TDT backwards on out-of-order reclaim) and my own
Sprint-6 RX-refill RDT (`slot+1` is TX semantics, wrong for RX → `RDT=last_rx_index` + arm-all-depth); the
Ethernet-padding 802.3-length encap (the min-frame pad breaks the implicit-length L1 frame; fixed below the
transport, below-§6); the gate-b window-margin finding (the K=32 drain latency reaches the 10 ms §7 window —
bounded-poll granularity × back-pressure, not wire latency — surfaced to the designer, the 10 ms constant
and the poll interval left untouched).

**What 7b inherits:** the crossing-soundness verified account (the `dma.rs` volatile/DD-ordering/no-tearing
residual — the longest-open hazard; fence-code slice-sized, the full account larger, keep honestly open);
the `architecture.md` §2.1/§3/§5/§6/§7 reconciliation surface (nic grounds + surfaces the live evidence, the
designer ratifies); and the poll-interval / ring-depth / DMA-pool design parameters to revisit against the
7a numbers. The seam discipline held for the whole sprint — every fix was backing below the git-verified
byte-identical reactor + transport + guard/copy-boundary.

### Reviewer retrospective — zero.nic-review (Sprint 7a, 2026-07-17)

**Certified** (re-derived from source + my own host runs; live-device run-outcomes from the dev's
self-pinning bin-sha-bound captures, flagged — I never drove the live host): the subsystem is now
OPERATIONAL end-to-end over the live DAC, pure-direct/no-IOMMU — the reactor-driven bounded-poll loop,
steady-state ring management (128 round-trips across 2 wraps, retransmit + give-up live), K=32 multiplexed
concurrent sessions (the operational milestone), and the E6 measurements. Every hardware run bin-sha
hash-bound to the committed source (I re-derived each bind by a clean rebuild). The frozen halves — reactor
algorithm + transport + guard/copy-boundary — stayed byte-identical throughout; only the backing changed.

**Findings that changed the plan:** necessary-not-sufficient proven *twice* (an empty-diff proves "not
re-authored," never "works live" — the first live-steady-state exercise caught the TX monotonic-cursor AND
my own Sprint-6 RX-refill miss, `RDT=slot+1` certified on a single-frame that never re-armed; lesson: the
discipline applies to the ring backing, not only the reactor algorithm — the dev surfaced its own bug
honestly). The Ethernet encap/decap below §6 (the transport frame stays frozen, the length source moved
between below-transport sources, §6's adversary-length bound honored by the clamp — a 7b reconciliation
note, not a §6 mechanism-touch). The gate-b window margin-violation (11.51 ms reaches the 10 ms §7 window,
masked to 0 spurious by drain-before-deadline — an operational pass, honestly surfaced-not-tuned, routed to
the designer per §7's own dial-up anticipation). Dormant-revive observability (host-proven-sufficient at
Slice 3, live-demoed in 4a).

**7b's reviewer inherits:** (1) the `dma.rs` crossing-soundness held to a CITED reasoning account checked
against specs (the 82599/PCIe payload-before-`DD` write-ordering the must-cite crux; x86-TSO, coherent DMA,
volatile-is-not-a-fence, no-tearing split, the stated platform residual — reject "it ran" / volatile-as-fence
/ assumed-ordering / hand-waved-tearing / partial-as-full). (2) the `architecture.md` reconciliation
surfaced-not-self-applied — and, operator-clarified, the untrusted-but-contained **posture STANDS**: the
§2.1 containment *mechanism* reconciles (driver guard contains vector-2; only vector-1 DMA-reach is the
owned residual), **not** "containment invalidated." (3) the §7 window finding, into the §2/§7 reconciliation.
All three are owned/honest-open, not discharged by the operational milestone: **operational ≠ crossing-sound
≠ contained.**
