# Sprint 010 (9) — Library Legibility & API Polish

> The `nic` team's tenth sprint (the PM's **Sprint 9**). The subsystem is functionally complete and proven
> on silicon (Sprints 1–8): `transport` (session protocol), `reactor` (event reactor), `driver` (data path
> + copy boundary), `directpath` (pure-direct control + the event-driven `DdWaitHost`), `benchtools`
> (isolated verification tooling). This sprint changes **no behavior** — it makes the library **legible**:
> a layout and public surface easy to follow, a readable integration narrative, executable usage examples,
> sensible public APIs, and a coherent, readable testing strategy.
>
> **This is future-shaped work (load-bearing framing).** "Legible", "sensible", "readable", "easy to
> follow" are **taste and convention** — the operator's call, not the team's. The failure mode is the team
> restructuring the library to its *own* taste and the operator having to rip it out. So the sprint
> **audits and proposes first, and the human ratifies the target before any mechanical execution**
> (Slice 1) — manufacture the anchor (a concrete proposed shape), turning the taste decision into a lookup
> rather than barreling ahead. The objective layer is dev/reviewer-gated; the **legibility layer is the
> human's sign-off**.
>
> **The hard guardrail: no behavior change** — and the guardrails are themselves modified by the work.
> Every accepted algorithm stays (`MWAITX`-on-`DD`, `pump_all`, the transport sender/receiver, the driver
> guards + copy boundary, the gate token, the crossing-soundness code). Legibility touches **visibility,
> re-exports, module layout, docs, examples, test organization, dead-code removal** — never logic. But this
> sprint **restructures its own guardrails**: Slice 5 moves the tests (the suite is a guardrail), Slice 2
> restructures the surface the diff reads. So the discipline is **conservation of a PRE-PINNED anchor**, not
> "trust the diff" — pin the behavioral anchors *before* execution (the full **test-assertion set**; the
> **live-path make-or-break outcome**, self-hash-bound) and prove each is **conserved** after. This is the
> Sprint-8 move (empty-diff above the seam + bin byte-match), adapted.
>
> **The re-run boundary (load-bearing — the suite does NOT cover the silicon path).** `cargo test` does not
> exercise the live path (the `MWAITX`-on-`DD` wake, the DMA, the ring re-arm run on hardware in the s7/s8
> **bins**, which are *not* in `cargo test`). So:
> - **Pure-legibility** (docs, visibility that does **not** force a bin-assembly change, test moves that
>   conserve the assertion set, doctests, traced-clear bin retirement) = **diff-provable, no re-run.**
> - **Anything reaching a bin's assembly** (curation that changes a bin's imports; an integration-module
>   *extraction* the bins are refactored to call) = **outside the diff+suite guardrail** → needs a strict
>   byte-diff-of-the-assembly **or** one targeted self-hash-bound **make-or-break bench re-run** on the
>   *final* curated surface (after Slices 2+3 land, not per-slice). A **stripped-`.text` byte-compare** of
>   the live bins pre-vs-post is a **bonus** proof where codegen is untouched (identical `.text` =
>   proof-by-construction) — but legit renames/moves change mangling + debuginfo, so it **fails on a valid
>   rename**; use it as a gift when green, never as a gate.
>
> Team: `nic` (lead `zero.nic-lead`, dev `zero.nic-dev`, review `zero.nic-review`, pm `zero.nic-pm`,
> designer `zero.nic-design`). Authority for behavior: the committed code (unchanged this sprint). Builds on
> Sprint 8 (`74e85e7`).

---

## Scope Boundary

**This sprint owns (legibility only, across `transport`, `reactor`, `driver`, `directpath`, `benchtools`):**

1. **Factual doc corrections (accuracy, NOT taste — front-loaded, no ratification needed).** Stale crate
   docs describe superseded states and are simply **wrong**: `reactor`'s "bare-metal LAPIC/`hlt`/MSI-X" (the
   reality is pure-direct `MWAITX`-on-`DD`, `DdWaitHost`); `directpath`'s "Part-B (this slice), NON-DMA
   only" (Sprint-6 language; the crate is the full pure-direct control path); `driver`'s crossing-soundness
   "open bench-gated hazard" (7b established a verified account). Correcting a false doc is accuracy, not a
   taste decision — it needs correctness review, not legibility-ratification.
2. **Library layout + cross-references** — module layout easy to follow; intra-doc `[`links`]` and
   crate-to-crate references accurate and untangled; **superseded bins retired** (after tracing clear —
   code imports *and* doc citations).
3. **An integration module — ADDITIVE by default** — a **readable narrative** (+ its doctest) of how the
   subsystem composes (gate → `pci`/`mem` → `link` → `datapath` → driver rings → reactor + `DdWaitHost` →
   transport), the "how it fits together" that today lives only implicitly inside the harness bins. The
   **additive** form (new narrative/example, **bins unchanged**) is truly behavior-preserving. An
   **extraction** form (the bins are refactored to *call* it, de-duplicating the assembly) is a
   behavior-preserving *bin refactor* — **not** trivially preserving; it is scoped explicitly, only if the
   operator wants de-dup, with shadow→prove→switch + a bench re-run. Default additive; do not smuggle a bin
   refactor under legibility.
4. **Public API review — `directpath` is a LEAF crate (grounded).** Nothing external depends on `directpath`;
   its ~80 `pub` items are consumed **only by its own bins** (~9/10 modules). So "curate the public API" has
   **no external-consumer benefit** — it is *internal* legibility, and a `pub use` facade over private
   modules would force the risky bin refactor (#3 extraction). High-value/low-risk: **fix the stale crate
   doc (#1) + prune genuinely-dead `pub` items, keep modules `pub`.** The facade is an operator-**taste**
   option only, carrying the bin-refactor + re-run cost. (Contrast: `reactor` is `pub mod` **and** `pub use`
   — redundant, and it *is* externally consumed by `directpath`, so any privatization there is gated by the
   consumer-import graph.) `transport`/`driver` already curate a private-`mod` + `pub use` surface.
5. **Testing strategy** — reviewed and rationalized to a **decision rule** (not a location default):
   **doctest** = public usage (how a consumer calls the API; doubles as docs); **in-module `#[cfg(test)]`**
   = a unit needing private access or an internal invariant/edge; **integration `tests/`** = black-box
   composition across the public surface. Applied uniformly (today uneven); test-name-as-spec-sentence,
   one-assertion-per-behavior; **conserving the existing assertion set**.

**This sprint does NOT own:**

- **Any behavior change.** No algorithm, wire format, or logic edit. **Do not smuggle a bin refactor**
  (the extraction integration module, or a facade that rewires bin assembly) **under "legibility"** — those
  are scoped explicitly, with a bench re-run, or not done.
- The two open **residuals** (no-IOMMU owned-DMA; platform-scoped crossing-soundness) — untouched.
- The **§7 max-K** ring-grow question (designer's, queued); `proto`/`proxy` (other teams').

## Ratified Design — the `directpath` facade (designer freeze, 2026-07-19; SUPERSEDES the Q2/Q3 taste framing above)

At Slice 1 the designer ruled Q2 not a taste call but a **security requirement**: `directpath` is the
**single integration point for BOTH the box and proxy sides** (symmetric, the same `transport` on both
ends), and the **proxy side is the trusted computing base** — so a curated **thin facade is MANDATORY**
to minimize the trusted surface. The team's earlier "leaf crate, no external benefit, keep-`pub`" reading
was working from an incomplete picture and is **withdrawn**. The frozen shape:

- **Public surface = the thin facade only:** `construct` / `run(handler) -> ControlFlow<()>` / session-open
  / `send` / `recv` (the handler + a `&mut Ctx` for send-in-response), **re-exporting `transport`'s session
  vocabulary**. Every hardware module (`pci`/`mem`/`gate`/`gate2`/`datapath`/`link`/`mwaitx`/`pagemap`/
  `ethframe`) is demoted to **`pub(crate)`**.
- **Interface-shape A (confirmed):** `directpath` **owns the reactor loop** and calls back per event —
  `run` is `{ pump_all(handler); park() }`, the handler *is* `pump_all`'s consume closure = the delivery-
  consumption + back-pressure-release point. This **encapsulates the stop-and-wait invariant** off the TCB
  surface (B — caller-owns-the-loop — would leak it in). *Recorded future boundary:* A cannot express a
  spontaneous external-triggered send while parked; the proven reactive request-response workload is fully
  covered — extend A or revisit B if that need ever arises.
- **`ControlFlow<()>`** stop condition (a completion payload only if a box exchange has a natural one).
- **The bench internals hatch** (harness coexistence): `#[cfg(feature = "bench")] #[doc(hidden)] pub mod
  internals { pub use crate::{hardware modules}; }` — the two-ended-loopback harness (`s8_slice2` drives
  *both* reactors; not a facade consumer) reaches raw internals **only under `--features bench`**. The
  **TCB / proxy build never enables `bench`**, so the hatch is **compile-absent** and the shipped surface
  is *exactly* the thin facade — mechanical, not merely reviewed. (Harness-logic-in-crate rejected: it
  pulls harness source into the production crate, breaking verification isolation.)
- **Load-bearing guard (designer):** confirm `bench` is **not a default feature** and is **unreachable from
  the proxy's build graph** (no direct or transitive dependency enables it) — verified via `cargo tree` /
  the resolved feature set on the TCB build.

**Consequences that supersede the framing above:** Q3 (integration module) **converges into the facade** —
the thin surface *is* the integration point + narrative (no separate additive module). The facade is now a
**bin-assembly-touching** change, so the **make-or-break bench re-run** applies (Slice 2): `s8_slice2`'s
source changes (import path + feature) so its bin-sha will **not** match `c30ae927` — the conserved anchor
is the **outcome** (128/128 byte-equal, 0-loss/0-spurious, `RXMPC0`=0, RTT/idle-wakeup in-band), self-hash-
bound to the new binary. The 5 integration tests (`pci`/`pagemap`/`gate2`/`gate`/`ethframe`) relocate
**in-crate** (`#[cfg(test)]` in the modules — the Slice-5 convention), the 289-assertion inventory conserved.

**Two designer additions before the signature locks (Slice-2, ruled 2026-07-19):**
1. **Module dissolution** (supersedes an interim `facade`→`api` rename) — `Subsystem`/`Ctx`/`Event` + the
   run loop move **directly into `lib.rs`** (Rust's default home for a small public API + composition root);
   a role-named collector module (`facade`/`api`) is the smell being removed. If the construct-wiring is
   bulky enough for its own file, it is **job-named** (e.g. `assembly.rs`) — never `build.rs` (the cargo
   build-script name) and never a pattern/role name. Public path stays `directpath::Subsystem` —
   behavior-free internal reorg; the one-file TCB-auditability now lives in `lib.rs`, the composition root.
2. **`Event::Delivered(SessionId, Vec<u8>)`** (symmetric with `send`'s return) — the designer ruled this a
   **genuine completeness defect** in the frozen reactor, not a gratuitous change: the reactor **drops** the
   transport-computed session id at `reactor.rs:121` (the `..` discard), so the facade **cannot express
   multiplexed delivery** (the app can't tell which session a delivery belongs to). This **unfreezes the
   reactor delivery path**: `pending` carries the session, `take_delivery` returns it, `pump_all`'s closure
   becomes `FnMut(SessionId, Vec<u8>)`, up through `Event::Delivered`. It is **additive-data** (the session
   is already computed by the transport; `step`/`park`/back-pressure/`Host`/`WakeReason` stay
   byte-identical) — the reactor diff is now the `quiescent()` accessor **plus** this delivery-session-carry,
   re-verified as no-algorithm-change at the re-gate. A **two-concurrent-session delivery test** is added
   (nothing exercised per-delivery session demux); existing reactor-test propositions are conserved (session
   handling is mechanics), the new test is a flagged addition (expected from real output).

**Slice-2 measurement discipline (learned at the re-gate):** the conservation baseline (`c30ae927`, RTT
138.8 µs / idle-wakeup 24.9 µs) is **DEBUG+remap**; a RELEASE re-run confounds the release-codegen speedup
with the two-thread-topology delta, so the make-or-break re-run is **DEBUG + `--remap-path-prefix` (host-
independent) + the benchtools Provenance header**, and only then are the RTT/idle medians interpretable as a
topology rebaseline.

**Scope addition — session-continuation API (operator-approved, 2026-07-22; folds into the facade before it
commits).** The facade under-exposes the transport: `transport` already has the origination/continuation
split (`open`/`send` in `table.rs`), and `architecture.md §4/§6/§8` already specify multi-frame-per-session
streaming — the code just never threaded it through the reactor + facade (no design-doc change; the design
is correct). Fold into the facade (Slice 2) so it commits once with the complete, correctly-named API:
- **`send(payload) -> SessionId` → `open(payload) -> SessionId`** — it is an origination; the name was a
  misnomer.
- **Add `send(id: SessionId, payload) -> Result<(), SendError>`** on both `Subsystem` and `Ctx` — continue
  an existing session (multi-frame).
- **Re-export `SendError`** from the facade.
- **Thread a reactor method** (`reactor.send_on_session`) to `transport.send`.
- **Rewrite the stale doc** at `directpath/src/lib.rs` ~141-160 (still "uni-directional, one side
  originates"). **Scoped to what the code can run TODAY** (Flag-A ruling): `allocator.rs` is a single
  monotonic counter (both endpoints mint from 0 — the Fork-1 collision is real), and the originator-partition
  (**Task 15**) is not yet built — so the doc describes the **origination/continuation split** (one
  originator's own ids never self-collide) and marks the `model.md §9` both-endpoints-originate reply model
  explicitly **COMING** (needs Task 15), *not* claimed as current. A both-originate doc over the shared
  allocator would describe the stall the silicon already found.
- Add a **multi-frame session-continuation test** (`open` then `send`-on-session, over the fake wire).
**Host-testable — no hardware re-run** (Flag-B ruling): there is **no reassembler** — a continuation frame is
delivered byte-identically to the silicon-proven single-frame path (the session-id is frame *content*, not
ring/DMA behavior; the ring/DMA re-arm/wrap/reclaim trap zone was already silicon-exercised by Slice 2), and
the new logic is host-side (sender `try_send`, receiver map no-op) — host-tested, with a narrow named
residual (the 2+-frames-on-one-session *scenario* wasn't silicon-run). The rename is behavior-neutral
(`open` == old `send`), so the Slice-2 silicon captures **stand** as the conservation evidence. Unblocks the
proxy team's Phase-3 (**receive-path only**) proof. ACCEPTED; sign-offs land on the complete open/send
surface.

**Scope addition — Task 15, the originator-partition (operator-approved, 2026-07-22; a FUNCTIONAL transport
change, commits AFTER the facade).** The gate for the proxy team's **Phase 4** (both-originate replies).
`transport/src/allocator.rs` becomes a **disjoint session-ID space**: top bit = originator (box 0 / proxy 1),
a 63-bit monotonic counter per side — so both endpoints originate in disjoint halves, resolving the Fork-1
collision **by construction** (designer/`model.md §9`-specified). Scope: the allocator partition + whatever
both-originate support the role logic (`§6` data-from-ACK-by-role) needs (a side originated a session iff its
top-bit matches its own side) + host tests (disjoint-space collision-safety, the role logic, both endpoints
originating without collision) + the facade doc-update (`both-originate = COMING` → **now**, collision-safe
via the partition). **Sequencing:** it modifies the transport, so building it before the facade commits would
recompile the facade's byte-matched bins — so it **builds after the facade commits** (facade first with the
one-directional doc, then Task 15 with the partition + the both-originate doc-update; each commit's doc
matches its transport state). **Host-testable** (the partition is host logic; both-originate is the same
proven TX/RX mechanisms) — no hardware run; the reviewer rules the residual at the gate. Gated as a
functional-change slice, distinct from the legibility work.

*Design rulings (designer, 2026-07-22 — all confirmed).* (1) **Side type:** a neutral `Originator::{Low,
High}` in the transport (no `Box`/`Proxy` naming — a layering violation); the **facade** owns the mapping
(`construct` takes `Endpoint::{Box, Proxy}`, maps `Box -> Originator::Low` / `Proxy -> Originator::High`,
hands the neutral value down). (2) **Partition-only, no runtime assertion — encoded structurally** so the
invariant holds by construction: `id = (side_bit << 63) | (counter & ((1 << 63) - 1))` — the top bit is
definitionally the side, the low 63 bits wrap **within** the half on overflow; the mask **is** the defense
(a `debug_assert` on the top bit would be vacuously true, dead weight). (3) **No side-less constructor at any
layer:** `SessionTable::new(side)` required (kill the defaulted `new()`); push the same to
`IdAllocator::new(side)` (drop the `Default` derive). Full chain: `construct(Endpoint) -> Originator ->
SessionTable::new(Originator) -> IdAllocator::new(Originator)`. **Doc:** `transport/src/allocator.rs`'s
"the box is the sole allocator" header is stale — update it with this change; no `architecture.md` edit
(`§1.1`/`§8` already carry the originator-partitioned model).

## Problem

The subsystem works but reads unevenly. `transport`/`driver` curate a private-`mod` + `pub use` surface
with strong docs; `reactor` carries both `pub mod` and `pub use` (redundant) and a **stale** bare-metal
crate doc; `directpath` is a **leaf** crate that exposes **everything** (`pub mod` × 10, ~80 `pub` items,
consumed only by its own bins) with a crate doc still describing the Sprint-6 "Part-B, NON-DMA only" slice,
and **how the primitives assemble lives only inside the harness bins** — no readable integration narrative.
There are **no executable examples** anywhere. The test layout is per-crate but uneven, with no rationalized
unit/integration split. Several **bins are superseded** (Sprint-6 `partb`/`slice2`/`slice4`) and add noise.

## Goal

The `nic` library reads cleanly end to end: accurate docs and cross-references (no stale-state
descriptions); a sensible, reviewed public surface per crate; a readable **additive** integration module
narrating the composition; executable usage examples on the public APIs; and a coherent, readable testing
strategy under the decision rule above — **with the proven behavior entirely unchanged**, verified by
**conservation of pre-pinned anchors**. The **operator has ratified the target shape** (Slice 1) before any
execution.

## Final State

- Stale crate docs corrected (accuracy); cross-references accurate; superseded bins retired (traced clear —
  code *and* docs).
- `directpath` legibility improved as ratified (doc-fix + dead-`pub` prune by default; facade only if the
  operator's taste chose it, then as a scoped extraction with a re-run).
- A readable **additive** integration module narrates the composition; **doctests** exist across the public
  APIs and pass `cargo test --doc`.
- Testing strategy rationalized to the decision rule; the **assertion set is conserved** (inventory-diffed,
  not just "still passes").
- **Behavior preserved by conservation**: the pinned test-assertion set + the pinned live-path make-or-break
  outcome are both conserved; any bin-assembly-touching change carried the byte-diff-or-re-run per the
  boundary.
- The **operator has signed off** on the legibility of the key surfaces.

## In Scope (ordered slices)

- **Slice 0 (front-loaded, no ratification) — factual doc corrections** [ask: accuracy]. Fix the stale
  crate docs (`reactor` bare-metal → `MWAITX`-on-`DD`; `directpath` Part-B → pure-direct + `DdWaitHost`;
  `driver` crossing "open" → 7b-verified account). Pure accuracy, no taste, no logic — correctness review
  only. Safe to land first.
- **Slice 1 — Audit + proposed target (the anchor; NO code changes).** The dev produces **(a)** a per-crate
  inventory — module layout; the public-API surface (every `pub` item) **with, per item, its consumer-side
  import graph** (every `use crate::item` from bins, tests, and other crates), so the curated surface is a
  provable **superset** of what live code imports (decided at audit, not compile-then-discover); the
  cross-ref/`[`link`]` graph (fresh vs stale); test + doctest coverage; and the **superseded-bin list traced
  clear against code imports AND doc citations**; and **(b)** a **proposed target** with the team's grounded
  leans: `directpath` doc-fix + dead-prune (keep `pub`) vs the facade; the **additive** integration module;
  the testing convention; the doctest plan. Also **pin the behavioral anchors** here: the full
  test-assertion inventory, and the current live-path make-or-break outcome (self-hash-bound). Routed to the
  **operator (via PM) to ratify the target** and to rule the **`s7_revive` judgment** (below). Inventory
  shape: `crate -> module -> {pub items + their importers, links (fresh/stale), tests, doctests} ->
  proposed action`.
- **Slice 2 — Layout, public surface, cross-references** [asks 2, 4]. Execute the ratified surface:
  `directpath` doc-fix + dead-`pub` prune (or the facade, iff ratified — then after Slice 3 + a re-run);
  fix stale cross-refs; retire the traced-clear superseded bins; every live consumer compiles against the
  curated surface. Behavior preserved (diff-provable unless a bin's assembly changed → the boundary re-run).
- **Slice 3 — The additive integration module** [ask 3]. Add the readable narrative + its doctest,
  composing the existing primitives, **bins unchanged**. (Extraction only if the operator chose de-dup —
  then scoped with shadow→prove→switch + a bench re-run.)
- **Slice 4 — Executable usage examples** [ask 5→doctests]. Doctests across the curated public APIs; they
  compile + run (`cargo test --doc`), use **real public types** (not stubs), read as genuine usage, and are
  **derived from the Slice-3 integration module** (example == real caller, carry-not-rediscover). Hardware-
  facing APIs are `no_run` (type-level only — flagged, sequence verified against the real bins by reading).
- **Slice 5 — Testing strategy** [ask 5]. Apply the decision rule; add/move feature-demonstrating tests to
  their right home; readable, test-name-as-spec-sentence. **Conserve the assertion set** (inventory-diff);
  any **added** test derives its expected from the **real current output**, never hand-computed; a genuine
  **assertion improvement** is **surfaced separately**, not folded under no-behavior-change.

## Non-Goals

- **No behavior change**; **no bin refactor smuggled under legibility** (extraction / facade rewiring are
  scoped explicitly with a re-run, or not done).
- No new features, no performance work, no residual-closing.
- Do not delete a bin/module/`pub` item because it "looks unused" — trace **code imports AND doc citations**
  first; retire only the traced-clear (and `s7_revive` is a *judgment*, not a trace-clear — below).
- Do not invent an unratified legibility convention; do not weaken the test suite (the assertion set is
  conserved).

## Starting Surface

Audit first (Slice 1), before any edit (Slice 0 doc-fixes excepted — they are pure accuracy):

- **Consumer import graph (the completeness check):** for every `pub` item, who imports it (bins, tests,
  other crates). The curated surface must be a provable superset of live imports.
- **`directpath/`** (LEAF; all `pub mod`, ~80 items, consumed only by its own bins; crate doc = Sprint-6
  Part-B) — internal legibility: doc-fix + dead-prune; facade is taste-only.
- **`reactor/`** (`pub mod` + `pub use`, redundant; **externally consumed by `directpath`** — gate any
  privatization on the consumer graph; **stale** bare-metal crate doc).
- **`driver/`** (curated; crate doc calls crossing-soundness "open" — 7b-verified now).
- **`transport/`** (curated, strong docs) — check for stale refs + doctest gaps.
- **`benchtools/`** (isolated; `affinity`/`msr` inline `pub mod` — fine; split reads better if touched).
- **Bins:** `partb`/`slice2`/`slice4` trace-clear superseded → retire. **`s7_revive`** is the **only bench
  demo of dormant-revive** (the property is host-tested in `transport`; `s7_slice3` notes revive is
  host-only there) — **a judgment for the operator** (retire the historical demo vs keep the sole bench
  proof), flagged separately, not a mechanical retire.

## Method (the discipline every slice follows)

- **Audit + propose + human-ratify before executing** (Slice 1). Do not execute a taste decision the
  operator has not seen; present grounded leans, not open menus.
- **Conservation of pre-pinned anchors, not "trust the diff."** Pin before execution: the full
  test-assertion **inventory** (count + the set of asserted propositions), and the live-path make-or-break
  outcome (self-hash-bound). Prove each **conserved** after. A **reduced** suite also passes — so the
  reviewer diffs the assertion *inventory*, never just "cargo test still passes" (subset-shadow).
- **The re-run boundary** (see framing): pure-legibility = diff-provable, no re-run; anything reaching a
  bin's assembly = byte-diff-of-assembly or one targeted make-or-break bench re-run on the *final* surface.
  Stripped-`.text` byte-match is a bonus proof (fails on legit renames — never a gate).
- **Objective gates, every slice:** `cargo build` + `clippy` clean; the existing `cargo test` suite passes
  (assertion inventory conserved); `cargo test --doc` passes; **no broken intra-doc links**
  (`-D rustdoc::broken_intra_doc_links`); every live consumer compiles against the curated surface.
- **Guardrail teeth (tests):** a readability edit is legibility **only if the asserted proposition** (the
  concrete inputs → expected literals/conditions) is proven **identical** — not merely "the assert line was
  untouched" (extract-a-helper / magic-number-to-const can silently flip a value). A genuine assertion
  **improvement** (`assert!(x>0)` → the true `x==4`) is **surfaced and routed separately**, not folded.
- **Added/doctest expected values derive from the REAL current output**, never hand-computed (a
  hand-authored "expected" can bless a bug as correct).
- **Doctest coherence:** read the **raw** doctest source (hidden `#` lines can conceal the load-bearing
  call); flag every `no_run`/`ignore` as type-level-only and verify the sequence against the real bins by
  reading; require **real public types**, not stubs; derive from the integration module (example == caller).
- **Legibility is the human's gate.** The reviewer verifies the objective gates + conservation + a
  legibility rubric; the **operator signs off** on the legibility of the key surfaces.

## Acceptance (the gates that close the sprint)

- **Slice 0:** stale docs corrected (accuracy, correctness-reviewed); no logic touched.
- **Slice 1:** the audit inventory (incl. the consumer import graph + doc-citation traces) + the proposed
  target are complete; the behavioral anchors are pinned; the **operator has ratified** the target and ruled
  `s7_revive`.
- **Per execution slice (2–5):** `cargo build`/`clippy` clean; the existing suite passes with the
  **assertion inventory conserved** (not just green); `cargo test --doc` passes; no broken intra-doc links;
  every live consumer compiles. Pure-legibility diffs carry no re-run; any bin-assembly-touching change
  carried the byte-diff-of-assembly or the final-surface bench re-run.
- **Behavior conserved:** the pinned test-assertion inventory and the pinned live-path make-or-break outcome
  are both conserved; the diff audit shows no accepted algorithm/wire/logic change.
- **Legibility sign-off:** the operator signs off on the curated surface, the integration narrative, the
  examples, and the testing strategy.

## Review Questions (for the reviewer + the operator/designer)

1. **The target shape (operator, Slice 1):** does the proposed shape match the operator's sense of
   "legible"? (ratified before execution)
2. **`directpath` surface (operator, with the team's lean):** `directpath` is a **leaf** (consumed only by
   its own bins), so a facade has **no external-consumer benefit** and forces the bin refactor — the team
   recommends **doc-fix + dead-`pub` prune, keep modules `pub`**. Facade only if the operator's taste wants
   it (then Slice 3 before Slice-2 privatization + a bench re-run).
3. **Integration module (operator, with the team's lean):** the team recommends the **additive** form (safe,
   no re-run). De-dup extraction (bins call it) only if the operator wants it, scoped as a bin refactor with
   a re-run.
4. **`s7_revive` (operator judgment):** retire the historical bin (dormant-revive is host-tested in
   `transport`) or keep it as the **sole bench demo** of dormant-revive? Not a mechanical trace-clear.
5. **Testing convention (reviewer + operator):** confirm the decision rule (doctest=public usage /
   in-module=private/internal / integration=composition), the assertion-set conservation, and the
   no-hand-computed-expected rule.
6. **Behavior-preservation confidence:** is conservation-of-pinned-anchors + the one final-surface re-run
   (for bin-assembly-touching changes) sufficient — or does any slice warrant more?
