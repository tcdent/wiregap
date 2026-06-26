# Sprint 003 — Data Path & the Copy Boundary (E3)

> The `nic` team's third build sprint (epic **E3**): the RX/TX descriptor rings and the copy boundary —
> the single piece of code standing between **adversary-influenced ring content** and the trusted host.
> All device-written content is treated as hostile (bound the length, treat status as hostile, never
> deref an unchecked field, drop-and-refill); each frame is copied out of DMA memory into app-owned
> memory at the earliest point so events carry the copy, never a ring pointer. It implements the **E4
> seams** (the owned-frame RX-drain and the `TransmitSink` TX-write) that Sprint 2 was built against. The
> data-path **logic** is built against a **simulated descriptor ring** and is fully provable in software,
> no NIC; the **`unsafe` DMA/MMIO crossing** is confined to one marked module behind that seam, and its
> **soundness under real device concurrency stays an open, bench-gated hazard** (`concerns-to-watch.md`)
> — this sprint builds the *shape*, it does **not** claim the crossing is proven sound. This document is
> the contract: the single source of truth for what this sprint owns, what it does not, and what evidence
> closes it.
>
> Team: `nic` (lead `zero.nic-lead`, dev `zero.nic-dev`, review `zero.nic-review`, pm `zero.nic-pm`).
> Authority: [`docs/nic/grounding/data-path-and-events.md`](../../nic/grounding/data-path-and-events.md)
> §1–3; [`docs/nic/architecture.md`](../../nic/architecture.md) §3.1; [`docs/nic/concerns-to-watch.md`](../../nic/concerns-to-watch.md).
> Builds on Sprint 1 (`transport`, `53bc323`) and Sprint 2 (`reactor`, `d745215`).

---

## Scope Boundary

**This sprint owns** the data path as logic + the copy boundary, behind a simulated-ring seam:

1. The **RX descriptor path**: poll the descriptor-done (`DD`) bit, read the frame, refill, advance the
   tail — against a **simulated ring** (the real volatile access is the confined-unsafe seam impl, §6).
2. **Adversarial-descriptor guards** (the security heart, `data-path-and-events.md` §1), **living in
   shared safe logic ABOVE the seam** so the real-ring impl is a thin guard-less volatile read: **bound the
   reported length** to the buffer size (never trust `desc.wb.upper.length`; `min(reported, BUF)` sizes
   every read/copy); **treat unexpected status fail-closed** (non-EOP, error flags, any unenumerated bit)
   as **drop-and-refill**; **never derive a memory access from an unchecked field**; single-buffer-per-
   frame, multi-segment unsupported (drop non-EOP).
3. The **copy boundary** (`architecture.md` §3.1): copy each frame **out of DMA memory into app-owned
   memory at the earliest point**; the emitted event carries the copy, **never a ring pointer** — this
   produces the owned frames the E4 RX-drain seam yields.
4. The **TX path**: descriptor write, **per-descriptor cleanup** (`RS`/`DD`, no batch-stranding), the
   `TransmitSink` TX-write seam copying app-owned bytes into a DMA buffer per emit.
5. The **reclamation split (R5, with the `data-path-and-events.md` §3 reconciliation)**: the **DMA TX
   buffer is TX-completion-bound** (freed on TX `DD`, a **ring-depth** pool); the **ACK-hold lifetime
   applies to the transport's app-owned buffer pool** (Sprint 1's `Rc` buffers, re-copied on
   retransmit), **not** the DMA mempool. Correct §3, which currently sizes the mempool by ACK-hold.
6. The **confined `unsafe` DMA/MMIO boundary**: the volatile/raw-pointer descriptor reads and the
   copy-from-DMA, in **one marked module** (`deny(unsafe_code)` crate-root + `allow` on that module),
   each `unsafe` sound with a SAFETY comment; the **DD→read ordering** (the device-done handshake).
7. **Honest grounding of the open hazards** (`concerns-to-watch.md` "unsafe DMA / memory boundary"):
   externally-mutated memory vs the language model, DD→read visibility ordering, copy soundness, no
   raw-pointer/device-reference leak into safe code, and *every* device-written field enumerated. These
   stay **open and bench-gated** — recorded as residuals, **never papered over with a confident fix**.

**This sprint does NOT own** (named, with where it belongs):

- **Device bring-up / link (E1)** and **VFIO/IOMMU setup (E2)** — no real device binding here; the real
  ring lives behind the seam, exercised on the prototype cards (bench-gated).
- **Closing the §3.1 crossing-soundness hazards** — the architecture deliberately leaves these open
  (`concerns-to-watch.md`); they need real silicon and a verified account. This sprint confines and marks
  the unsafe and grounds the hazards honestly; it does **not** resolve them. The IOMMU-airtightness half
  (interrupt remapping, ATS, peer-to-peer) is **E2**'s, not here.
- **The reactor loop (E4)** and **the session protocol (E5)** — already built; E3 implements the seams
  they consume and is driven by the reactor, it does not reimplement them.
- **The register bring-up writes** (`RXPBSIZE`/`SRRCTL`/`HLREG0`/`RDBAL…`/`TXPBSIZE`/`DMATXCTL.TE` etc.)
  as *live device config* — those are applied at E1/E2 bring-up on real silicon. E3 owns the data-path
  *interpretation* and the *which-bits-and-why* record (CRC strip, `DROP_EN`, `SBP`-left-default, the
  §4.6.7-tail quirk bits), not their live application to a device that does not exist this sprint.

## Problem

The E4 reactor was built against two seams with **no implementation**: the RX-drain (must yield
all-available **owned** frames) and the TX-write `TransmitSink` (must copy app-owned bytes into DMA). The
descriptor ring is **adversary-influenced** (firmware writes it from box-controlled wire bytes; the IOMMU
bounds *where* the device DMAs, not *what* it writes into our rings — `kernel-bypass-and-dma.md` §2,
vector 2), and **nothing yet guards it**: no length-bounding, no hostile-status drop-and-refill, no
copy-out. And `data-path-and-events.md` §3 records a reclamation model the as-built transport contradicts
(R5). E3 is the one piece of code between adversarial ring content and the trusted host; until it exists
and is held to reviewed-by-reading, the reactor has nothing real to drain or to write.

## Goal

The data-path logic and the copy boundary, implemented and **proven adversarial-input-safe in software**
against a simulated descriptor ring (hostile descriptor content yields bound/drop/no-over-read, never a
panic or a read past the buffer), satisfying the E4 RX-drain/TX-write seam contracts; the `unsafe`
DMA/MMIO touches **confined to one marked module** and each shown locally sound; the reclamation split
corrected (R5); and the crossing-soundness hazards **grounded honestly as open**, bench-gated for the
prototype cards — never papered over.

> "Bound the reported length to the buffer size rather than trust `desc.wb.upper.length`; treat
> unexpected status bits as a drop-and-refill, never a read past the buffer; never derive a memory access
> from an unchecked descriptor field." — `data-path-and-events.md` §1.

## Final State

- A crate (see R1) that compiles `deny(unsafe_code)` + one marked `unsafe` module, and runs its
  self-validation on any Linux, no NIC, against a simulated ring.
- **RX:** `DD`-poll → bound-length → hostile-status drop-and-refill → copy-out to app-owned → refill →
  advance tail; the RX-drain seam yields owned frames, all-available-then-empty.
- **Adversarial safety (structural + tested):** an arbitrary descriptor (any length, any status bits)
  yields a bounded read or a drop; never a panic, never a read past the buffer; no memory access derived
  from an unchecked field.
- **TX:** descriptor write, per-descriptor `RS`/`DD` cleanup (no stranding), the `TransmitSink` copies
  app-owned bytes into a DMA buffer per emit.
- **Reclamation:** DMA TX buffer freed on TX `DD` (ring-depth pool); ACK-hold is the transport's
  app-pool, re-copied on retransmit. `data-path-and-events.md` §3 corrected to match.
- **`unsafe` confinement:** the volatile/raw-pointer DMA touches live in one module, marked, each with a
  SAFETY comment; no device-backed reference escapes into safe code.
- **Open hazards recorded honestly:** the `concerns-to-watch.md` DMA-crossing residuals remain open and
  bench-gated, with the field-enumeration done (every device-written field we read is listed).

## In Scope — the ordered slices

**Slice 1 — Simulated-ring seam + RX descriptor path + adversarial guards.** Define the descriptor-ring
seam **drawn ABOVE the guards** (the load-bearing structural decision): the seam exposes the **raw
untrusted fields UP** (reported length `u32`, status `u16`, buffer access) to **shared safe guard logic**;
only a **bounded-length copy primitive goes DOWN** into the (later) unsafe real-ring impl. All adversarial
guards — `min(reported, BUF)`, status checks, never-deref — live in the shared logic **above** the seam;
the real-ring impl is a **thin volatile read with ZERO security logic**. *Why:* on the simulated ring
(safe slices) an unchecked length cannot over-read (Rust panics, caught), so the sim can only prove "the
bounded length reached the read," never "the real raw read is safe" — the guard must be above the seam, or
it is tested-by-sim but absent-on-real (the dead-path trap). Build the RX path: `DD`-poll, bound length,
**hostile-status drop-and-refill** (non-EOP, error flags, **fail-closed**: unenumerated bit → drop),
`next_frame` **skips** a dropped descriptor and continues (`None` only on DD-clear). *Method: feed hostile
descriptor content (lengths 0..2×buffer, every status combination) and assert the guard FIRED — over-length
→ frame exactly `BUF_SIZE`; hostile status → not delivered AND refilled — never panic/over-read. The
security heart, tested first.*

**Slice 2 — The copy boundary.** Copy each frame out of the (simulated) DMA buffer into app-owned memory
at the earliest point; the event carries the copy, never a ring pointer; refill the descriptor and
advance the tail. The RX-drain seam yields owned frames (all-available-then-empty) — the E4 contract.
*Method: assert the emitted frame is owned (outlives the ring slot; the ring slot is refilled/overwritten
and the copy is unaffected); drop-and-refill keeps the ring full.*

**Slice 3 — TX path + per-descriptor cleanup + the reclamation split (R5).** TX descriptor write; the
`TransmitSink` copies app-owned bytes into a DMA buffer per emit; per-descriptor `RS`/`DD` cleanup (no
batch-stranding). **`emit` is infallible-by-construction** (the committed signature returns `()`): the DMA
TX pool is **bounded to ring-depth ≥ peak-in-flight K** (never unbounded), and a **full ring drops and
relies on retransmit** (the absent ACK is the back-pressure — the only non-failing option). The
reclamation split: DMA buffer freed on TX `DD` (ring-depth pool); ACK-hold is the transport's app-pool.
**Reconcile `data-path-and-events.md` §3** to this, preserving the old model as a "how this went wrong"
note (the doc change is surfaced for human ratification before commit — R3). *Method: assert DMA buffer
freed on TX `DD` (not on ACK); a retransmit re-copies from the app-pool into a fresh DMA buffer; a full
ring drops without error/panic; no stranding (each descriptor cleaned independently).*

**Slice 4 — The confined `unsafe` DMA/MMIO boundary + E4 integration + honest hazard grounding.** Make the
real volatile/raw-pointer descriptor reads and the copy-from-DMA real, in one `unsafe`-marked module
(`deny`+`allow`) — **a THIN volatile read carrying ZERO guard/security logic** (all guards stay above the
seam, Slice 1), so the sim tests the production guards. Each `unsafe` block discharges its **local**
preconditions with a SAFETY comment. Split the **DD→read** into: the **testable control-flow order** (poll
DD, only then read+copy; a DD-clear descriptor is never read/delivered — sim-testable, gates 2/3) versus
the **silicon-gated memory barrier/visibility** (`concerns` #2 — the SAFETY comment **names it
undischarged**, does not claim it sound). Wire the seam into the E4 reactor (the RX-drain and
`TransmitSink` it was built against). Then **ground the `concerns-to-watch.md` hazards honestly**:
enumerate every device-written field read; record the crossing-soundness residuals as **open,
bench-gated** (do not claim resolution). *Method: the simulated ring remains the test seam; the
real-volatile impl is exercised only on the prototype cards (bench-gated); the unsafe surface is shown
confined — re-derive no-leak from the module's public signatures (zero `pub` raw-pointer / device-borrow
returns).*

## Non-Goals

- No real device binding, no live register writes, no VFIO/IOMMU (E1/E2); the real ring is bench-gated.
- **Do not claim the §3.1 DMA-crossing is sound** — it is an open hazard (`concerns-to-watch.md`); ground
  it honestly, never paper over with a confident fix (the file's own rule).
- No multi-segment / jumbo RX (drop non-EOP; single-buffer-per-frame).
- No software CRC/integrity check (inherited from hardware CRC + `DROP_EN`; `SBP` left at default).
- No reactor or protocol logic (E4/E5); E3 implements the seams they consume.
- **Do not delete a guard or a fallback because the simulated ring does not exercise it** — the hostile
  -status drop-and-refill and the length-bound must exist and be tested even though a friendly simulated
  ring never trips them; trace `data-path-and-events.md` §1 before trimming.

## Starting Surface

Audit first the **E4 seam contracts this sprint must satisfy** (the RX-drain yields all-available owned
frames then empty; the TX-write *is* `transport::TransmitSink`, app-owned bytes), and the **adversary
model** (`data-path-and-events.md` §1 + `kernel-bypass-and-dma.md` §2 vector 2: ring content is hostile,
the IOMMU does not guard it). Inventory shape the dev produces before editing:

```
field/path read from device memory (§ref) -> adversarial? -> guard (bound | drop | never-deref) -> owned-copy point -> test (hostile input) that proves it
```

Cross-check the siblings: `transport` (no_std) and `reactor` (std, `deny`+`allow` confined unsafe) — E3
matches the `deny`+`allow` confinement and the simulated-seam / injected-impl convention (no `cfg(test)`
forks in the data path).

## Method

- **Logic in software against a simulated ring; the unsafe crossing behind the seam.** The data-path
  logic and every adversarial guard run against a simulated descriptor ring (normal memory) and are fully
  software-provable, no NIC. The real volatile/raw-pointer DMA access is the seam's real impl, confined
  and marked, exercised on the prototype cards (bench-gated).
- **Adversarial-first testing.** The descriptor-interpreting code is the guard for vector 2; test it with
  *hostile* content first (out-of-range lengths, every status combination), asserting bound/drop/no-over-
  read — structural where possible (the type/bound makes over-read unsayable), behavioral otherwise.
- **`unsafe` is THE review target** (`architecture.md` §2.1): confined to one module, `deny`+`allow`
  compile-enforced, each block sound with a SAFETY comment; no device-backed reference escapes to safe
  code (re-derivable).
- **Honest open hazards** (`concerns-to-watch.md` rule): the crossing-soundness residuals stay visibly
  open and bench-gated; if an entry ever reads like a confident answer, something has gone wrong.
- **No dead path / no `cfg(test)` fork**: the simulated and (later) real ring are injected seam impls; the
  audited data-path code is the code that runs on the real ring.
- **Evidence is concrete**: cite the field, the guard, the hostile input, and the observed bound/drop —
  never "looks safe."

## Acceptance — the gates that close the sprint

1. The crate builds clean (`deny(unsafe_code)` + one marked module) and the self-validation runs green in
   software against the simulated ring; evidence carries a build-coherence marker.
2. **Adversarial RX safety — POSITIVE guard-fired observables (not just absence of panic).** "Never
   panic / never over-read" is **circular** on the sim (safe slices can't over-read anyway), so require
   the guard to be shown *firing*: an over-length descriptor (`len = 2×BUF`) → the copied frame is
   **exactly `BUF_SIZE`** (the bound fired, not the raw `2×`); a hostile status (non-EOP / error /
   DD-clear) → **not delivered AND the descriptor refilled** (drop-and-refill fired; the ring stays full).
   Status is **fail-closed**: any unexpected/unenumerated bit → **DROP** (sweep the guard-relevant bits,
   default-drop the rest — `concerns-to-watch.md` #5). The length-bound `min(reported, BUF)` is computed in
   **shared logic above the seam** and only the bounded value sizes any read/copy — the raw `desc.length`
   never reaches a read (so over-read is unsayable on the *real* ring, not just caught on the sim).
3. **Copy boundary — disjoint, copy-before-refill.** The emitted RX frame is a **real byte-copy into a
   fresh, disjoint allocation** (`to_vec`/`Vec::from`), **never a transmute/alias of the DMA region**
   (`concerns` #3); re-derived by overwriting the ring slot **after** the copy and asserting the frame is
   unchanged (disjointness) — no ring pointer escapes. The copy-out **precedes** refill/advance (§3.1
   "earliest point" — testable control-flow order on the sim). The RX-drain seam yields
   all-available-then-empty, **skipping** dropped descriptors (a hostile drop must not stall the drain;
   `next_frame` returns `None` only on DD-clear).
4. **TX + reclamation (R5) — infallible emit, bounded pool.** `TransmitSink::emit` is **infallible**
   (returns `()`), so emit is infallible-by-construction: the DMA TX pool is **bounded to ring-depth**
   (≥ peak-in-flight **K** = concurrent sessions, stop-and-wait caps each at ≤1; K provisional,
   bench-gated, `data-path` §6), **never unbounded**; on a full ring emit **drops and relies on
   retransmit** (the absent ACK is the back-pressure — reuses the RX-overrun→retransmit machinery; it
   cannot error/block/panic). The DMA buffer is freed on **TX `DD`** (ring-depth pool), **not** on ACK; a
   retransmit re-copies from the transport's app-pool; per-descriptor cleanup, no stranding.
   `data-path-and-events.md` §3 reconciled (R3).
5. **`unsafe` confinement — LOCAL soundness only + no-leak from public signatures.** `deny`+`allow` in one
   module (compile-enforced, re-derived from the module attribute). Each `unsafe` block discharges its
   **local/syntactic** preconditions with a SAFETY comment (non-null aligned pointer into a mapping we own;
   read sized ≤ buffer; no aliasing a live `&mut`). **No-leak** is re-derived from the module's **public
   signatures**: every `pub fn` returns **owned** (`Vec<u8>`) or plain — **zero** `pub` returns of
   `*const`/`*mut`/a reference tied to device memory (`concerns` #4).
6. **Honest open hazards — gate 5 vs gate 6, policed in the SAFETY comments.** The **device-concurrency
   crossing** (volatile-visibility, the DD→read memory barrier, no-tearing — `concerns` #1/#2/#3) stays
   **OPEN and bench-gated**. Each crossing `unsafe`'s SAFETY comment discharges only its *local*
   preconditions (gate 5) and **names the device-concurrency soundness as an UNDISCHARGED residual**
   deferred to `concerns-to-watch.md`. A SAFETY comment (or any gate) that reads as "the crossing is
   sound / we observe the device's writes correctly" is the **over-claim this gate forbids — REJECT it.**
   The device-written-field enumeration is done (every field we read is listed).
7. Independent review re-derives every gate from current source at cited lines + a settled run — not the
   dev's summary — and confirms: **every guard lives in shared logic above the seam and the real-ring impl
   carries ZERO guard/security logic** (a thin volatile read — so the sim tests the *production* guards,
   not a dead path); no `cfg(test)` fork in the data path (sim + real rings are injected trait impls, the
   audited interpret/bound/copy code is byte-identical sim-vs-real); no device reference escapes; and the
   open hazards are honestly open (not papered over).

## Review Questions

- **R1 (PM/human) — crate identity.** Lean: a separate **`driver`** (or `datapath`) crate, std,
  `deny(unsafe_code)` + one marked module, implementing the E4 seams and depending on **both `transport`
  (for `TransmitSink`/frame types) AND `reactor` (for `RxDrain` — the RX-drain trait is reactor-owned)**.
  Confirm the name and that it is a separate crate (sibling to `reactor`), or set the convention you want.
- **R2 (PM/human) — the software-provable vs silicon-gated split (the load-bearing scope call).** Both
  teammates confirm it is **sound** under the conditions now gated: every guard lives **above the seam**
  and the real-ring impl is **thin/guard-less** (gate 7), and the crossing-soundness stays honestly open
  **in the SAFETY comments** (gate 6). The key distinction (dev): the **vector-2 security guards
  (bound/drop/never-deref) are FULLY CLOSED NOW in software** — that is what closes the sprint; **only the
  crossing soundness** (UB/ordering/tearing) is silicon-gated. Holding E3 for silicon would block the
  whole stack for a hazard that is bench-gated regardless. Confirm this close basis (logic+guards proven
  now; crossing open), or say if you want E3 held for the prototype cards. My lean: build now, gate the
  crossing — matching how E4/E5 seamed hardware.
- **R3 (PM) — the `data-path-and-events.md` §3 reconciliation (R5) + the emit/sizing policy (A).** This
  sprint corrects §3: the DMA TX buffer is **TX-completion-bound (ring-depth)**; the ACK-hold sizing
  applies to the transport's app-owned pool. **Bundled with it (the dev's headline A):** `TransmitSink::
  emit` is **infallible** (returns `()`), so the DMA pool is **bounded to ring-depth ≥ peak-in-flight K**
  (never unbounded), and a full ring **drops-and-relies-on-retransmit** (the only non-failing option; K
  bench-gated). Both are **grounding/behavioral changes I will surface for HUMAN RATIFICATION before the
  commit** (per the standing doc-review policy), and the **old §3 model is preserved as a "how this went
  wrong" note** (it predated the as-built E5 transport — don't silently overwrite). Confirm the
  reconciliation direction + the drop-on-full policy, or flag if you read the intent differently.
- **R4 (reviewer/PM) — the open hazards stay open.** The `concerns-to-watch.md` DMA-crossing residuals are
  **not** resolved this sprint (they need silicon + a verified account). Confirm the acceptance bar is
  "confined + marked + honestly tracked open," and that a gate claiming the crossing *sound* would be a
  defect — so the reviewer rejects any over-claim.

---

## Outcome & Postmortem

**Verdict: ACCEPTED, sprint closed** (pending the §3 human-ratification gating only the commit).
Independently re-derived by `zero.nic-review` from the working-tree source at cited lines + its own
settled run; security-boundary properties proven structurally-first, tests as corroboration, every
SAFETY comment scanned (not skimmed).

**Delivered:** the `driver` crate (E3) — std, `#![deny(unsafe_code)]` + one marked `dma` module (11
`unsafe` blocks, all SAFETY-commented), depending on `transport` and `reactor`. The data path + copy
boundary, **the one piece of code between adversary-influenced ring content and the trusted host**:
RX adversarial guards (bound length, fail-closed status, drop-and-refill, never-deref), the copy
boundary (disjoint app-owned copy before refill), the TX path (per-descriptor cleanup, infallible
`emit`, the reclamation split), the confined real-volatile DMA rings. **17 tests** (16 unit incl. the
full adversarial sweep + 1 no-NIC full-stack E3↔E4↔E5 integration); the logic proven in software against
a simulated ring, the unsafe crossing confined and its soundness honestly open/bench-gated.

**Gates:** (1) clean build + coherence; (2) adversarial RX — **guards above the seam** (raw fields up,
`min(reported, BUF)` before any read — over-read unsayable on the real ring), positive guard-fired
(over-length→exactly BUF; hostile→dropped+refilled), fail-closed over the full **u32** status; (3) copy
boundary — disjoint owned copy, copy-before-refill, no ring pointer escapes; (4) TX/reclamation (R5) —
infallible `emit`, bounded ring-depth pool, full-ring-drops-and-retransmits, DMA freed on TX `DD`,
ACK-hold = the transport's app pool; (5) confinement — `deny`+`allow`, 11 unsafe all in `dma`, local
soundness only, no-leak from public signatures; (6) honest open hazards — every crossing SAFETY comment
names the device-concurrency soundness undischarged, **no claim the crossing is sound**, field
enumeration done; (7) no dead path — the real ring is a thin guard-less impl behind the same seam, no
`cfg(test)` fork.

**The two corrections this sprint forced (validate against the real artifact):** the **u16→u32 status**
fix — completing the real 82599 `status_error` map caught that a u16 seam would truncate the [31:20]
error field and *deliver error frames* (a real fail-closed defect, surfaced by the field-enumeration);
and the **§3 reclamation reconciliation** (R5) — which took a **reverse-then-re-reverse** arc worth
recording. The team found the as-built (app-owned ACK-bound `Rc` + a ring-depth, TX-completion-bound DMA
pool) did not match `data-path-and-events.md` §3, and proposed correcting §3 to match (R5/R3), routing the
**direction** to the operator rather than self-ruling it. The operator first **reversed** — "§3 as
written is right; the as-built is the defect; make the DMA buffer ACK-bound" — and the team conceded and
*scoped* (did not build) a 3-crate rewrite. While scoping, the dev surfaced that a **bare ACK has no ACK
to free on** (it cannot be ACK-bound). The operator then **re-reversed** to the final ruling: the §3
sentence was **over-prescribing an implementation detail** (the DMA mempool sizing — "ACK-hold × max
sessions, not ring depth") that a grounding doc has no business dictating; the **design intent** ("retain
a sent frame until its ACK") is what's normative, the TX memory layout is the implementation's choice.
The **designer removed the over-specifying clause from §3 and §6**; the as-built transport is a **valid
implementation**, **no code changed**, and the §3/§6 edit (the designer's minimal deletion, not the dev's
verbose split-diff) commits with this sprint. **Lesson:** a design-of-record can over-prescribe an
implementation detail; the resolution is to separate design *intent* from *prescription* and delete the
over-specification — and because the team deferred the reconciliation *direction* to the human (rather
than self-ruling R3), the operator's reverse-then-re-reverse cost **zero committed wrong code**.

**Next sprints inherit:** the reactor/transport **seams are now implemented** (owned-frame `RxDrain`,
`TransmitSink`) — E1/E2 wire the real rings behind the **same** seam with no driver-logic change, guards
stay above it. **E2 (VFIO/IOMMU)** gates two things: the real-volatile crossing soundness (the
`Volatile{Rx,Tx}Ring` are written/confined/commented but **bench-gated** — E2 maps the DMA/MMIO supplying
the validity the constructors' SAFETY preconditions assume; the prototype cards ground the
volatile-visibility / DD→read-barrier / no-tearing residuals) **and** the IOMMU-airtightness half
(interrupt remapping on, ATS off, no peer-to-peer). **E1** provides the device the rings run against
(Sprint-0 anchors). Until E2 grounds both, the firmware is **untrusted-but-contained in posture, not yet
fact**. The `concerns-to-watch.md` DMA-crossing residuals stay **open** until discharged on silicon with
a verified account — never asserted.

### Retrospective — nic-lead (zero.nic-lead)
- **Approach:** framed E3 as the security-critical guard, with an honest scope split — the logic +
  adversarial guards + copy boundary proven *now* in software against a simulated ring, the `unsafe`
  crossing confined and its soundness *open and bench-gated*. Formed as a team; routed one slice at a
  time, security-heart-first; deferred the sizing/§3 forks to the PM and the §3 grounding-doc edit to
  human ratification rather than applying it myself.
- **Evidence that shaped the plan:** both teammates **converged** on the load-bearing structural
  decision — guards *above* the seam, the real-ring impl thin and guard-less — which is what makes the
  sim test the *production* guards rather than a dead path (the trap that would let the central security
  gate pass on weak evidence). The dev's `emit`-infallible find (the committed signature *forced* the
  bounded-pool + drop-on-full design) and the u16→u32 datasheet catch, and the reviewer's circular-
  anti-trim catch (positive guard-fired observables) and "police the SAFETY comments" gate (the over-claim
  hides in a comment, not a gate), all sharpened the contract before a line was built.
- **Held the line on:** the honest-open-hazard discipline — no gate or SAFETY comment claims the crossing
  sound, `concerns-to-watch.md` stays open; and routing the §3 edit to human ratification with the old
  model preserved, not silently overwritten.
- **Next sprint inherits (process):** enumerating every device-written field is not busywork — it caught a
  truncation defect; and when the as-built disagrees with a grounding doc, route the **reconciliation
  direction to the human, do not self-rule it** — that discipline is exactly what made the operator's
  reverse-then-re-reverse on the §3 reclamation cost **zero committed wrong code** (the team scoped, never
  built, the rewrite). A grounding doc can over-prescribe an implementation detail; the fix is to delete
  the over-specification (separate intent from prescription), not to force the code to match it. The
  gate-split corollary (hold a gate whose correctness is downstream of an *unratified* fork) is sound in
  general — robust to either ruling — even though the final ruling vindicated the accepted direction.

### Retrospective — nic-dev (zero.nic-dev)
*"Guards above the seam; the unsafe crossing confined, its soundness honestly open."*
- **Built:** the driver crate (E3) — RX guards (bound length, fail-closed status, drop-and-refill,
  never-deref), the copy boundary (disjoint app-owned copy before refill), the TX path (per-descriptor
  cleanup, infallible `emit`, the reclamation split), the confined real-volatile DMA rings. Logic proven
  in software against a sim ring (incl. the full adversarial sweep); the unsafe crossing confined to one
  module, soundness honestly open/bench-gated.
- **Evidence that shaped the plan:** seam-above-the-guards (raw fields up + bounded copy down → guards
  hostile-testable in software, over-read unsayable even in the real impl); `emit` infallible-by-
  construction (the committed `()` signature forced the bounded pool + drop-on-full); the u16→u32 status
  correction (completing the real `status_error` caught that a u16 would truncate the error bits and
  deliver error frames — the field-enumeration doing its job).
- **Surprised:** "validate against the committed/real artifact, not the doc" recurred again (u16→u32),
  surfaced *by* the field-enumeration; and the R5 reclamation split propagated into the §3 doc correction
  (the as-built app-owned ACK-bound buffer made §3 wrong) — the build forced the grounding doc to reconcile.
- **Next sprints inherit:** E2 gates the real-volatile crossing soundness (DMA/MMIO mapping + the
  prototype cards) **and** the IOMMU-airtightness half; E1 provides the device; the `concerns-to-watch`
  DMA residuals stay open until silicon; the reactor/transport seams are implemented so E1/E2 wire the
  real rings behind the same seam with no driver-logic change (guards stay above it).

### Retrospective — nic-review (zero.nic-review)
- **Acceptance approach:** re-derive every gate from working-tree source at cited lines + my own settled
  run (clippy `-D warnings`, the full adversarial sweep, debug where a `debug_assert` is load-bearing),
  coherence-pinned each time. For the security boundary the discipline was **structural-first** — prove
  the property from the type/control-flow, tests as corroboration — and **scan, not skim, the unsafe**.
- **Evidence that sharpened a verdict:** (1) the **seam-above-guards [A]** condition, set at plan time —
  on a simulated ring an unchecked length cannot over-read (safe slices panic, caught), so the sim can
  only prove the bound was applied, never that the real raw read is safe; the guards had to live above
  the seam, re-derived at acceptance (`min(reported,BUF)` before any read; raw length never sizes a
  read). (2) The **circular anti-trim** catch (same shape as the E5 give-up gate) → positive guard-fired
  observables. (3) Gate 6 — scanning **every** SAFETY comment for the crossing over-claim ("we observe
  the device writes correctly" is the defect, because the over-claim hides in a comment that reads as
  resolved).
- **Held the line on:** the crossing-soundness stays honestly open (gate 5 = local preconditions only;
  device-concurrency soundness undischarged, deferred to concerns-to-watch); and the fail-closed policy
  over the full u32 status (the u16→u32 catch would otherwise have delivered error frames) — any bit
  outside the enumerated set drops, which is why a non-exhaustive u32 sweep is sound.
- **Next sprints (E1/E2) inherit:** the real-volatile crossing soundness must be re-verified **on
  silicon** (the `Volatile{Rx,Tx}Ring` are written/confined/commented but bench-gated, exercised by no
  test); the five concerns-to-watch DMA residuals stay open and must be discharged with a verified
  account, not asserted; the seam-above-guards boundary is the contract E1/E2 must preserve; the
  IOMMU-airtightness half is E2's, separate from the crossing residuals; carry `deny`+`allow`,
  no-leak-from-public-signatures, build-coherence, and live-path-not-dead-helper as standing gates.
