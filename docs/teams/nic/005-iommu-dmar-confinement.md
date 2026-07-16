# Sprint 005 — IOMMU / DMAR Confinement (we write and own the DMAR)

> The `nic` team's fifth build sprint, and **the security core**: on the bare-metal substrate we **write
> and own the VT-d DMAR tables ourselves** to confine the NIC's DMA to **exactly the Sprint-4 arena
> regions** — nothing else is reachable. When the IOMMU was kernel-configured it was something we *cited*;
> hand-written and inside the trusted base, **a DMAR bug is a containment hole** (`architecture.md §2.1`),
> so the DMAR configuration earns **E3-grade confinement *and* soundness scrutiny** — a three-level verdict
> (remapping **enabled** → out-of-range DMA **blocked** → setup **sound**), an **induced out-of-range-DMA
> attempt that must fault**, and hazards **kept honestly open until silicon**, never papered with a
> confident config. Grounding this is what turns *untrusted-but-contained* from a posture into a **fact**
> and deletes the firmware from the trusted base. Built + proven in **QEMU with `-device intel-iommu`**
> (the emulation vehicle, as e1000e was for ECAM); the real 82599 + real VT-d re-verify on the bench card
> (Sprint 6). Clean-build bar (the DMAR is a trusted-base member): auditable-in-one-sitting, no dead code,
> confined `unsafe`, pure table-construction host-tested. This document is the contract; acceptance is
> team-defined per the clean-build discipline.
>
> Team: `nic` (lead `zero.nic-lead`, dev `zero.nic-dev`, review `zero.nic-review`, pm `zero.nic-pm`).
> Authority: [`docs/nic/grounding/kernel-bypass-and-dma.md`](../../nic/grounding/kernel-bypass-and-dma.md)
> §2 (the confinement spec + the (A)+(B) convergence); [`docs/nic/grounding/bare-metal-substrate.md`](../../nic/grounding/bare-metal-substrate.md)
> §7 (the trusted-base DMAR posture). Builds on Sprint 4 (`substrate`, `355dd73` — the arena is already laid
> DMAR-confinable: one 2 MiB-aligned whole-DMAR-page extent). Sprint 6 = device bring-up; Sprint 7 = integration.

---

## Scope Boundary

**This sprint owns**, hand-written in the `substrate` crate and reviewed by reading:

1. **The VT-d DMAR tables** — the root-entry table, the context-entry table (for the NIC's BDF), and the
   **second-level page tables** that translate the NIC's DMA. Constructed to map **only** the Sprint-4
   arena extent (identity-mapped `iova == phys` over that extent, at the DMAR page granularity the arena
   was aligned to); every other address is **unmapped → faults**. **The tables themselves live in a
   separate NIC-UNREACHABLE region — NOT in the second-level map** (a root/context/SL table the NIC can
   DMA to would let a rogue NIC rewrite its own confinement — a containment hole; this is the sharpest
   trusted-base property). The table **geometry is CAP/SAGAW-dependent** (read the capability register for
   the address-width / level count — QEMU `intel-iommu` defaults 39-bit/3-level unless `aw-bits=48`); do
   **not** hardcode a level count. The **table construction is pure logic** — parameterized by table-region
   base + arena extent + BDF, no baked physical base (host-testable); the register programming (`RTADDR`,
   enable, invalidation) is confined `unsafe` MMIO.
2. **DMA remapping — reach confined to the arena** (`kernel-bypass-and-dma.md §2`, vector 1): the NIC can
   DMA to the arena and **nowhere else**. Enable translation; a DMA to an out-of-arena address raises a
   **DMAR fault** we observe.
3. **Interrupt remapping (the (B) airtightness) — scope per R4:** govern the NIC's MSI/MSI-X writes so a
   rogue NIC cannot craft writes to arbitrary addresses (an *escalation*, not a DoS). Either the interrupt
   remapping table (IRT) is written here as part of the DMAR setup, or it is a **named, honestly-open**
   (B) item scoped to a follow-up — R4 settles it. **ATS off** and **no peer-to-peer** are confirmed
   (config/topology; our single-NIC topology + our tables map no device-to-device).
4. **The three-level verdict + the induced test.** Each (B) item states its level: (i) **ENABLED** (our
   DMAR is programmed to remap — now **source-checkable**, our code reviewed-by-reading, not a kernel
   config we cite), (ii) **BLOCKED** (the **induced out-of-range-DMA attempt faults** — measurement-provable
   in QEMU), (iii) **SOUND** (the setup is verified). Two semantic sharpenings the *our-DMAR* shift forces:
   - **"Enabled" is three things, and G1 proves only one.** Enabling the DMAR is (1) table construction
     (pure, G1), (2) register programming (`RTADDR`/`TE`, unsafe), (3) cache invalidation (unsafe). **G1 is
     a property of the table BYTES, not the running IOMMU** — it can be green while (2)/(3) are wrong. So
     **G1-pure is necessary-not-sufficient and never reads as "enabled/blocked"**; it is the
     *table-construction* proof, and only the **induced fault** (G2) proves (2)+(3) actually enforce. (This
     is the *inverse* of the E2 config-citation risk: there config stood in for blocked; here pure-table-
     correct must not stand in for actually-enforcing.)
   - **(iii) SOUND is a silicon residual QEMU cannot discharge.** Soundness = the register programming
     *and* the cache invalidation are correct against the VT-d spec. A missed/mis-ordered invalidation, or
     an enable-vs-first-DMA race, is a real VT-d bug class QEMU's emulation may not model (it may read the
     tables fresh and "work" without a required invalidation). A QEMU-green induced fault proves (ii)
     **BLOCKED** (the mechanism) but **not (iii) SOUND** — a run corroborates soundness, it does not
     discharge it. (iii) stays **honest-open** (silicon + a verified account of the invalidation/enable
     ordering vs the VT-d spec) even after G2 passes.

   What cannot be verified in QEMU — the real VT-d confinement on the bench card, "rogue MSI blocked"
   (un-inducible: a well-behaved emulated device won't fire a rogue MSI, so IR-blocked stays open even with
   a source-correct IRT — see R4), and (iii) soundness above — stays **honestly open**.

**This sprint does NOT own** (named, with where it belongs):

- **Driving the 82599 (Sprint 6)** — its own registers, the link, real RX/TX rings, the real NIC MSI-X.
  This sprint may drive a **minimal** device DMA (a QEMU device's descriptor at an in- vs out-of-arena
  address) *solely* to induce the confinement test — not the full driver. R1 settles the split.
- **The real VT-d confinement on silicon (Sprint 6/7)** — QEMU `intel-iommu` proves the *mechanism* and
  the induced fault; the real 82599 + real platform VT-d re-verify on the bench card. Honest-open until then.
- **The E3 driver / E4 reactor / E5 transport logic** — unchanged; the DMAR is a new substrate member the
  arena feeds. The E3 real-volatile crossing soundness stays E3/bench-gated (separate from DMA-reach).

## Problem

The arena is laid out DMAR-confinable, but **nothing writes the DMAR** — the NIC's DMA is currently
unbounded (in the real design; in QEMU without an IOMMU, a device can DMA anywhere). The load-bearing
security claim — *the IOMMU contains a rogue NIC* — is unrealized: no DMA-remapping table confines the
device to the arena, no induced out-of-range attempt has been shown to fault, and the (B) airtightness
(interrupt remapping, ATS-off, no-p2p) is unaddressed. And now the DMAR is **ours to write** — so its
correctness is a trusted-base, reviewed-by-reading concern, not a platform default we cite.

## Goal

The substrate writes and owns the VT-d DMAR, confining the NIC's DMA to **exactly the arena** — proven by
an **induced out-of-range DMA that faults** while in-arena DMA succeeds, in QEMU `intel-iommu` — with the
DMAR configuration a small-and-ours trusted-base member (auditable, no dead code, confined `unsafe`, pure
table-construction host-tested), each (B) item carrying its three-level verdict, and the residuals that
need silicon kept honestly open. *Untrusted-but-contained* becomes a demonstrated fact for the mechanism.

> "A DMAR bug is a containment hole — so the DMAR configuration earns the same confinement-*and*-soundness
> scrutiny E3 gives the unsafe DMA boundary." — `bare-metal-substrate.md §7`.

## Final State

- The substrate constructs the DMAR root/context/second-level tables (pure logic, host-tested) mapping
  **only** the arena, programs the IOMMU (`RTADDR` + enable + invalidation), confined `unsafe`.
- **The DMAR tables live in NIC-unreachable memory** — a separate region, **not** in the second-level
  map; a DMA aimed at the table region faults like any other out-of-arena address.
- In QEMU `intel-iommu`: DMA remapping **enabled** (observed); the **pair** is the confinement proof — an
  **in-arena DMA succeeds** *and* an **out-of-arena DMA FAULTS** (a captured DMAR fault whose `FRCD` IOVA
  equals the out-of-arena address); the out-of-arena fault alone would be vacuous (translation-on-with-
  nothing-mapped faults everything), so in-arena-success is load-bearing, not decoration.
- The (B) items carry their three-level verdicts: DMA-reach **blocked** (ii, induced pair); interrupt
  remapping per R4 (IRT (i)-source if written, "rogue MSI blocked" (ii) honest-open — un-inducible in
  QEMU); ATS-off + no-p2p (i)-source with the bypass-blocked inferred; **(iii) soundness honest-open** (a
  silicon residual QEMU cannot discharge). Residuals needing silicon are honestly open.
- The DMAR is reviewed-by-reading: small-and-ours, no dead code, `unsafe` confined + SAFETY-noted, no
  leak; the table math is pure + host-proven.

## In Scope — the ordered slices

**Slice 1 — DMAR table construction (pure) + programming.** Build the root/context/second-level tables to
map the arena extent — as **pure functions** (arena extent + table-region base + BDF → table bytes) with a
pure `walk(tables, iova) → Option<phys>`, host-tested; then program the IOMMU (`RTADDR`+`SRTP`, cache
invalidation, then set `TE`) in confined `unsafe`. *Method: host `cargo-test` the construction as the
confinement in pure logic — an in-arena IOVA resolves to its identity phys, and the **negative space is
positively UNMAPPED**: a **hostile-IOVA sweep** (0, arena base±1, arena-end, 2×-arena, `u64` corners) all
resolve `None`, a **different BDF** is unmapped, and **the DMAR-table region itself is unmapped** (the
tables-not-in-the-map assertion). QEMU: only "DMA remapping enabled" observed (`GSTS.TES=1` after the
sequence) — G1-pure does NOT claim enforcement; that is Slice 2.*

**Slice 2 — Confine to the arena + the induced out-of-range test.** Map only the arena; induce a device
DMA (a minimal QEMU device DMA, R1) to an **in-arena** address (succeeds) and an **out-of-arena** address
(**faults**). *Method: QEMU `intel-iommu` — the **pair is the proof**: capture the in-arena success **and**
the out-of-arena DMAR fault (observed→expected, self-pinned); the out-of-arena fault alone is vacuous, so
in-arena-success is required. Confirm it is the **IOMMU** blocking — a real DMAR fault in `FSTS`/`FRCD`
whose recorded **IOVA equals the out-of-arena address**, not a device error or QEMU artifact.*

**Slice 3 — The (B) airtightness verdicts + honest-open.** Interrupt remapping (per R4: write the IRT +
its (i)-source verdict, or name it open) — but "rogue MSI to arbitrary address **blocked**" (ii) is
**un-inducible in QEMU** (a well-behaved emulated device won't fire a rogue MSI), so IR-blocked stays
**honest-open even with a source-correct IRT**; do not let "we wrote+verified the IRT" become "IR
airtight". ATS-off and no-peer-to-peer are **(i)-source and now stronger than E2's citation** (a
*disabled/absent* bypass is a legitimate (i) verdict — our config doesn't enable ATS, our tables map no
device-to-device), with the bypass-blocked **inferred** (the (ii) induced test unattempted), never a bare
"confirmed" reading as (ii); the 82599's ATS-advertise datasheet half is honest-open to Sprint 6. *Method:
per-item `expected → observed → verdict`, **level stated**; a (ii)/(iii) claim on (i) evidence is a defect.*

**Slice 4 — Trusted-base grounding + integration.** Ground the confinement honestly (`concerns-to-watch.md`):
what QEMU proved (the mechanism + the induced fault) vs what re-verifies on silicon (real VT-d, real
82599). Reviewed-by-reading the whole DMAR surface. *Method: move QEMU-grounded facts into the design
docs; keep the silicon residuals open; confirm the DMAR is small-and-ours + no dead code.*

## Non-Goals — and the invalid checkpoints we RETIRE

- **No driving the 82599 (Sprint 6)** beyond the minimal DMA needed to induce the confinement test.
- **No claiming the real-silicon confinement** — QEMU proves the mechanism + the induced fault; the real
  VT-d is honest-open until the bench card.
- **No general-purpose IOMMU framework** — the DMAR is sized to confine this one NIC to this one arena.
- **Retired invalid checkpoints:** any (B) verdict claiming **BLOCKED/airtight on level-(i) config
  evidence** (enabled ≠ blocked — the induced fault is required for DMA-reach); **G1-pure (table bytes
  correct) reading as "enabled/blocked"** — it is the construction proof, not the enforcement proof; an
  **out-of-arena fault claimed WITHOUT the paired in-arena success** (vacuous — nothing-mapped faults
  everything); a **source-correct IRT claimed as "IR blocked/airtight"** (rogue-MSI un-inducible in QEMU →
  honest-open); a **QEMU-green induced fault claimed as (iii) SOUND** (a run corroborates, it does not
  discharge — invalidation/ordering is a silicon residual); a bare "PASS" without the observed fault;
  "the setup is sound" asserted rather than reviewed; a gate for a feature this sprint doesn't build;
  self-asserted no-dead-code (must be the reviewer's reading).

## Method

- **The DMAR earns E3-grade scrutiny** (`bare-metal-substrate.md §7`): confinement (it blocks) *and*
  soundness (the tables/programming are correct), reviewed-by-reading as a trusted-base member. It follows
  the E3 architecture: **security-bearing table construction PURE (host-testable) above, register
  programming confined-`unsafe` below** — the seam-above-guards discipline that made the E3 arena provable.
- **The programming sequence is order-sensitive:** `RTADDR`+`SRTP` → global context-cache + IOTLB
  **invalidate** → set `TE`. Skipping invalidation lets stale/absent translations silently not take — a
  green-in-QEMU-fail-on-silicon trap. The account of this ordering vs the VT-d spec is part of (iii).
- **Three surfaces:** (G1) host `cargo-test` of the **pure table-construction logic** (the real shipped
  fns: `walk` maps an in-arena IOVA and *not* an out-of-arena one — a *table-bytes* property, **not**
  enforcement); (G2) QEMU `intel-iommu`, **observed→expected, self-pinning, reviewer re-runs** — remapping
  enabled + the **paired** in-arena-success/out-of-arena-fault (the enforcement proof); (G3)
  **reviewed-by-reading** the DMAR surface (small-and-ours, no dead code, confined `unsafe`).
- **Three-level verdicts, level stated per item** — enabled (i, config/source) vs blocked (ii, induced
  fault) vs sound (iii); a line claiming (ii)/(iii) on (i) evidence is a REJECT. **G1 is (i)-adjacent
  necessary-not-sufficient (table bytes), not (ii); the induced pair is (ii); (iii) is a silicon residual
  no QEMU run discharges.**
- **Honest-open** (`concerns-to-watch.md` rule): the silicon residuals stay visibly open; a confident
  config passing for a verified block is the failure this discipline prevents.
- **Self-pinning** carried (the QEMU boot banner pins the binary build-id + the qemu/OVMF + intel-iommu
  invocation) so the reviewer re-derives + re-runs.

## Acceptance — the gates (team-defined for the clean build)

1. **G1 — the DMAR table construction proven PURE (host `cargo-test`, review re-runs):** the real shipped
   `walk(tables, iova)` maps the arena extent correctly and **only** it — an in-arena IOVA resolves to its
   identity phys, and the **negative space is positively UNMAPPED**: a **hostile-IOVA sweep** (0, arena
   base±1, arena-end, 2×-arena, `u64` corners) all resolve `None`, a **different BDF** is unmapped, and
   **the DMAR-table region itself is unmapped** (the tables-not-in-the-map property). This is the
   confinement proven in pure logic, deterministically — the G1 analog of the E3 65k-status sweep and its
   `overlap_is_detected` positive assert. **G1 proves the table BYTES, not enforcement** — it is
   necessary-not-sufficient and does not on its own establish "enabled/blocked." Build-coherence pinned.
2. **G2 — confinement demonstrated in QEMU `intel-iommu`, OBSERVED, self-pinning (review re-runs):**
   DMA remapping **enabled** (`GSTS.TES=1` observed); the **pair is the proof** — an **in-arena DMA
   succeeds** *and* an **induced out-of-arena DMA FAULTS** at the IOMMU (a captured `FSTS`/`FRCD` fault
   whose recorded **IOVA equals the out-of-arena address**), confirmed a DMAR fault not a device error; the
   out-of-arena fault **alone is insufficient** (vacuous without the in-arena success). observed→expected,
   never a bare PASS. **G2 proves (ii) BLOCKED (the mechanism), it does NOT prove (iii) SOUND.**
3. **G3 — DMAR is trusted-base-grade (reviewed-by-reading):** the DMAR configuration is **small-and-ours**,
   every function on the confine-this-NIC path (no general framework, no dead code); the table math is
   pure; the register-programming `unsafe` is `deny`+`allow`-confined, SAFETY-noted, no leak; **the tables
   live in NIC-unreachable memory**. Each (B) item states its **level**; no line claims blocked/airtight/
   sound on weaker evidence.
4. **Honest-open recorded:** the residuals needing silicon — real VT-d confinement, real 82599,
   **(iii) soundness (the invalidation/enable-ordering account QEMU cannot discharge)**, **"rogue MSI
   blocked" (un-inducible in QEMU)**, and the 82599 ATS-advertise datasheet half — are in
   `concerns-to-watch.md`, **open**, not papered over; QEMU-grounded facts move into the design docs.
5. Independent review re-derives G1 (re-run host tests) + G2 (re-run QEMU `intel-iommu`) + G3 (read the
   source) — not the dev's summary; **rejects** any over-claim (G1-pure read as enabled/blocked, an
   out-of-arena fault without the paired in-arena success, a source-correct IRT read as IR-blocked, a
   QEMU-green run claimed (iii) sound, blocked on config-only evidence, a bare PASS, self-asserted
   no-dead-code, sound-by-assertion).

## Review Questions

- **R1 (PM) — the device-DMA-for-the-induced-test split with Sprint 6.** Testing the confinement needs a
  device DMA (in-arena succeeds, out-of-arena faults). Lean: a **minimal** QEMU device DMA (e.g. an e1000e
  descriptor write-back pointed at an in- vs out-of-arena address) *solely* to induce the fault — **not**
  the full 82599 driver (Sprint 6). Confirm this minimal-induced-DMA is in scope, with the full driver +
  real 82599 DMA deferred to Sprint 6.
- **R2 (PM) — QEMU `intel-iommu` as the test vehicle.** Lean: QEMU with `-device intel-iommu` (DMA
  remapping + interrupt remapping emulation) is the Sprint-5 test environment (as e1000e was for ECAM),
  proving the mechanism + the induced fault; the real platform VT-d re-verifies on the bench card
  (Sprint 6/7), honest-open until then. Confirm.
- **R3 (reviewer/PM) — the three-level bar retargeted to our DMAR (reviewer CONFIRMED; PM to ratify).**
  Now that the DMAR is *ours*, level (i) "enabled" is **source-checkable** (our table code), not a
  kernel-config citation — but the reviewer sharpened two residuals the *our-DMAR* shift adds: **(a) G1
  (pure table bytes) is necessary-not-sufficient and never reads as "enabled/blocked"** — only the induced
  fault proves the register-programming + invalidation actually enforce; **(b) (iii) SOUND is a silicon
  residual** (invalidation/enable-ordering) that no QEMU-green run discharges. DMA-reach still requires the
  (ii) induced pair; "rogue MSI blocked" and the silicon residuals stay **open** unless induced. Ratify the
  bar so the reviewer rejects config-only "blocked", G1-pure-as-enabled, and QEMU-green-as-sound, and
  accepts honest-open items as open, not failures.
- **R4 (PM) — interrupt remapping scope (dev + reviewer both lean the same way; PM to rule).** Land the
  **DMA-remapping core** (confine DMA to the arena + the induced fault — the highest-leverage confinement)
  this sprint; write the IRT **if natural in the same DMAR setup**, in which case it earns its **(i)-source**
  verdict — but the reviewer notes "rogue MSI to arbitrary address **blocked**" (ii) is **un-inducible in
  QEMU** (a well-behaved emulated device won't fire a rogue MSI), so IR-blocked stays **honest-open even
  with a source-correct IRT**. Do not claim (B) airtight without the induced proof. Rule: is a written-but-
  (ii)-open IRT in scope this sprint, or is the whole IRT a named-open follow-up?

---

## Outcome & Retrospective (Sprint close, 2026-07-16)

**Validated.** All four slices independently ACCEPTED by `zero.nic-review`, each re-derived from source +
its own host `cargo-test` (30/30) and QEMU (`q35,kernel-irqchip=split,intel-iommu,intremap=on`) runs, not
the dev's summaries. The substrate now writes and owns the VT-d DMAR: the NIC's DMA is confined to exactly
the Sprint-4 arena, proven by the induced out-of-arena fault (FRCD IOVA==induced `0x1a00000`,
source-id==NIC `0x18`, in-arena-succeeds load-bearing, double-confirmed by QEMU's own `vtd_iommu_translate`
diagnostic); the tables live in NIC-unreachable memory; interrupt remapping is written+enabled at
(i)-source with rogue-MSI (ii) honest-open; the GCMD silent-disable hazard is caught+guarded. 596-line
trusted-base surface, unsafe confined to eleven SAFETY-noted sites, no-leak compile-enforced, no dead code.
The rulings: R1 (minimal QEMU inducer in scope, full 82599 → Sprint 6), R2 (QEMU `intel-iommu` the vehicle),
R3 (the three-level bar, with G1-pure≠enforcement and (iii)-sound-is-silicon), R4 (IRT written, (i)-source,
rogue-MSI (ii) honest-open) — all ratified by the PM. The `kernel-bypass-and-dma.md §2` grounding is
**proposed as a diff, pending designer/human ratification** — not applied; `concerns-to-watch.md` records
the silicon residuals open.

### Lead retrospective — zero.nic-lead (Sprint 5, 2026-07-16)

**Forming the sprint as a team paid the whole sprint's dividend before a line was built.** The two most
load-bearing properties of the final contract were not mine — the dev's *tables-in-NIC-unreachable-memory*
catch (a device that can DMA to its own page tables is unconfined) and the reviewer's three acceptance
sharpenings (G1-pure proves table **bytes**, not enforcement, so it never reads as blocked; the induced
fault is vacuous unless **paired** with in-arena-success; (iii) SOUND is a silicon residual no QEMU run
discharges). Routing the draft through both seats *before* promotion, not after, is what made the bar
correct rather than merely plausible.

**The slice→gate discipline and the separation of duties did the real work.** Every slice boundary was a
full independent-acceptance gate; the reviewer re-derived from source + its own runs each time; I never
accepted my own routing. The two mid-sprint findings — the DD-bit false-FAIL (Slice 2) and the GCMD
silent-disable hazard (Slice 3) — were surfaced *by the dev honestly* and *confirmed independently by the
reviewer*, which is exactly the loop working: the builder found the sharp edge, the arbiter certified it
wasn't a leak/regression, and I held the two apart. The DD-bit episode is the keeper — the dev's own
`!out_completed` predicate was a false-leak generator, and the fix was **converging to the pre-defined
FRCD bar, not loosening it**; the reviewer verified that distinction rather than taking it on faith.

**Doc-authority held without a whipsaw this time.** The design-of-record grounding was *proposed as a
diff* and routed up for ratification, never self-applied — the [[normative-spec-is-the-authority]] lesson
(the §3 reclamation whipsaw) applied cleanly. The reviewer scrutinized the diff for over-claim in its own
lane and flagged the direction decision up rather than ruling it.

**What Sprint 6 inherits (process):** the same slice→gate + honest-open discipline, and the standing line —
*proven-in-QEMU is not proven-on-silicon*. The bench card discharges the open residuals; until it does, the
docs ground only what QEMU proved.

### Developer retrospective — zero.nic-dev (Sprint 5, 2026-07-16)

**Built** — the substrate writes+OWNS the VT-d DMAR (pure host-tested tables mapping ONLY the arena + the
order-sensitive enable, confined unsafe); confinement DEMONSTRATED in QEMU (ii) BLOCKED — an induced
out-of-arena e1000e DMA faults at the IOMMU (FRCD iova==induced, sid==NIC) while in-arena succeeds; the (B)
airtightness with LEVELS (IR written+enabled (i)-source, ATS-off (i)-source, no-p2p (i)-source), grounded
as proven-in-QEMU with residuals open.

**Evidence that changed the plan** — (a) the DD-bit finding: e1000e sets DD even when the buffer read
faulted; my first predicate gated on `!completed` (false FAIL), I converged to the pre-defined bar
(in-arena-success + FRCD iova==induced + sid==NIC) and surfaced DD=true as info — the block proof is the
IOMMU's fault record, not device completion. (b) the GCMD silent-disable hazard: GCMD sets persistent
functions to the written bit, so enabling IRE alone would have silently disabled TE (the DMAR) — caught it,
reconstructed via `persistent_gcmd`, made it OBSERVABLE (read TES back). (c) CAP/SAGAW geometry (read the
levels, not baked-4). (d) the tables-outside-arena property (my Slice-1 catch) — enforced at build +
host-tested + observed.

**Surprised** — the false-FAIL from the DD gate (most instructive; the fix was converging to the bar, not
loosening it); the IRT fell naturally (intremap boots every earlier slice unchanged); reuse paid off
(acpi.rs walk → DMAR/DRHD base, 2MiB arena → single large-page leaf).

**Sprint 6 inherits** — confinement is proven-in-QEMU NOT silicon: (iii) real-VT-d/82599 soundness +
rogue-MSI (ii) + 82599-ATS datasheet are the OPEN residuals for the bench card; the e1000e inducer is a
stand-in, Sprint 6 drives the real 82599 (rings/IVAR/EIMS/real MSI-X, driver going no_std) through the SAME
reactor+DMAR seams (real MSI-X replaces the self-IPI behind the same `Host` seam); the DMAR context table
covers the NIC's BDF, the dark X520 function gets no mapping (bench fact to confirm).

### Reviewer retrospective — zero.nic-review (Sprint 5, 2026-07-16)

**Certified** (re-derived from source + my own host `cargo-test` 30/30 and QEMU
`q35,intel-iommu,intremap=on` boots, both re-run per slice): the substrate now WRITES AND OWNS the VT-d
DMAR. The confinement is proven PURE in host tests (tables map only the arena, every other IOVA — including
the tables' own region — walks to None), and the **(ii) BLOCKED** enforcement landed in QEMU (induced
out-of-arena DMA faults at the IOMMU; in-arena succeeds). Interrupt remapping is written+enabled at
**(i)-source**. The trusted base is small-and-ours + auditable: 596 lines, dmar pure/zero-unsafe, unsafe
confined to eleven SAFETY-noted allow-sites over a deny root, no-leak compile-enforced, no dead code.

**Findings that changed the plan:**
- **DD-bit is-not-a-leak.** e1000e sets the descriptor DD REGARDLESS of the buffer fault (in-arena ring
  write-back). The dev's `!out_completed` predicate was a wrong invention (false-leak generator); the FRCD
  fault — not DD — is the block signal. `blocked()` converged to the pre-defined FRCD bar.
- **GCMD silent-disable hazard.** GCMD sets every persistent function to the written bit, so writing IRE
  alone would CLEAR TE and silently disable the DMAR. Caught + guarded by `persistent_gcmd()` (reconstructs
  enabled bits from GSTS); verified TE-still-enabled(true) after IR enable.
- **(B)-level discipline.** Each airtightness item carries its honest level — IR/ATS/no-p2p at (i)-source,
  the (ii) rogue-MSI + silicon items honest-open; nothing reads as (ii)/airtight on (i) evidence.
- **FRCD-IOVA==induced pairing.** The block proof is the JOINT bar: in-arena-success (load-bearing: the
  IOMMU uses OUR tables) AND out-of-arena FRCD fault at iova==the-induced-address + sid==the-NIC. The
  out-of-arena fault alone would be vacuous.
- Minor: `lib.rs:17` unsafe-site count "ten"→"eleven" (folded into the commit).

**Honest-open residuals** (in `concerns-to-watch.md`, open): (iii) real-silicon soundness — QEMU
corroborates the mechanism, does not discharge real VT-d/IOTLB/errata/DRHD; rogue-MSI (ii) — un-inducible
in QEMU; 82599-ATS advertise — datasheet/bench half. Everything is proven-in-QEMU, not proven-on-silicon.

**Sprint 6's reviewer inherits** the silicon re-verify on the bench 82599: real VT-d actually ABORTS the
out-of-arena DMA (not just QEMU's emulated fault); the IRT source-validation actually STOPS a crafted rogue
MSI; and whether the real 82599 ADVERTISES ATS / the platform honours a pre-translated request. The design
doc (`grounding/kernel-bypass-and-dma.md §2`, proposed) grounds ONLY what QEMU proved — hold that line: no
silicon claim until the bench discharges it.
