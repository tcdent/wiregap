# Sprint 000 — Static-Anchor Grounding (read-only live-card probe)

> The `nic` team's pre-build grounding pass (the PM's "Sprint 0"), ahead of the E1–E5 build epics.
> A **read-only** diagnostic run of the already-built `live-card-probe` against this host's live,
> production 82599 NIC, to turn the subsystem's *static* design anchors from "bench-gated, needs
> silicon" into confirmed values. No code is written; no device state is modified. This document is
> the contract: the single source of truth for what this sprint owns, what it does not, and what
> evidence closes it.
>
> Team: `nic` (lead `zero.nic-lead`, dev `zero.nic-dev`, review `zero.nic-review`, pm `zero.nic-pm`).
> Authority: [`docs/nic/epics.md`](../../nic/epics.md) §E6 static tier,
> [`docs/nic/tools/live-card-probe.md`](../../nic/tools/live-card-probe.md),
> [`docs/nic/grounding/device-bring-up-and-link.md`](../../nic/grounding/device-bring-up-and-link.md),
> [`docs/nic/grounding/kernel-bypass-and-dma.md`](../../nic/grounding/kernel-bypass-and-dma.md) §5.1.

---

## Scope Boundary

**This sprint owns** running the read-only probe and verdicting its readings against the documented
static anchors:

1. Execute `docs/nic/tools/live-card-probe.sh` against the host's live 82599 NIC.
2. Capture its structured report (the `expected → observed → verdict` artifact).
3. For each of the static open items below, compare observed against the grounding-note anchor and
   record a per-item verdict (match / mismatch).

**This sprint does NOT own** (named, with where it belongs instead):

- **Any device-state change** — no register/NVM/PCI-config write, no `set_*`, no driver unbind, no
  `vfio-pci` bind, no BAR0 `mmap`, no read of read-to-clear/RW1C registers. The probe is read-only by
  construction (`live-card-probe.md` §2); this sprint adds zero new device interaction.
- **The dynamic items** — link-up on EEPROM default, minimal `Restart_AN` form, single-ring
  throughput, reactor-under-load, buffer sizing. These require *driving* a card and gate **per-epic
  as the prototype cards arrive** (`epics.md` §E6 dynamic tier). Out of scope here.
- **Driver/transport implementation** (E1–E5) — no Rust is written this sprint.
- **Folding confirmed values back into the grounding notes.** A clean match leaves the anchor as-is
  (no edit needed). Only a **mismatch** would prompt a note update or a re-scope — surfaced as a
  Review Question to the PM, not silently edited (see Non-Goals).

## Problem

The NIC subsystem's design rests on several *static* facts about the 82599 board that were filed as
assumptions ("bench-confirm", "needs silicon"): LESM is disabled in NVM, `AUTOC2[19]=1` (erratum 13)
is EEPROM-set, the link-electrical config is SFI/10G-serial from EEPROM, the IOMMU group composition,
and the silicon stepping (which gates whether the B0 errata apply). The prototype cards have not
arrived, but this host's **primary network interface is a live 82599** carrying the
production-identical DAC. A read-only probe can confirm these anchors **now**, de-risking the build
epics — but the target is a live production link, so the run must not perturb device state.

## Goal

Each static anchor is confirmed against the live card and recorded with its observed value, or any
divergence is surfaced honestly as an open item rather than papered over. The build epics inherit
confirmed anchors instead of assumptions, and the "untrusted-but-contained" / "defaults-first"
postures gain empirical grounding for this board.

> "Run the live-card-probe tool against the host's existing NIC to confirm the static design anchors
> (LESM state, AUTOC2 defaults, IOMMU group, stepping)… The NIC is the live network interface — do
> not modify device state, read only." — `zero.nic-pm`, sprint handoff.

## Final State

- The probe has run to completion against the live 82599 and emitted its structured report.
- Identity is confirmed first (82599ES / `ixgbe`); the probe degraded gracefully if it were not.
- Every static open item below has an `expected → observed → verdict` line, grounded in the report.
- The live link is **unchanged** — still up, same driver, no device state altered by the run.
- Matches are recorded as confirmed anchors; any mismatch is logged as an honest open item with the
  divergence named, and routed to the PM as a Review Question.

## In Scope — the readings, each against its anchor

The probe emits one report covering all readings; the ordered slices below are the logical clusters
the evidence is verdicted in, not separate runs.

**Evidence grades** — two distinct strengths, kept honest per reading:
- **Authority-grade** — a direct NVM/EEPROM word read (the source of truth the hardware loads from).
- **Corroboration-grade** — a post-init live register read via `ethtool -d`: representative of the
  EEPROM default *only* under the documented assumption that neither `ixgbe` nor our driver synthesizes
  the bit (`device-bring-up-and-link.md` §3). Real, but bounded by that assumption — it must not inherit
  authority-grade confidence.

**Slice 1 — Identity & topology.**
- Device + subsystem ID, **stepping/revision** → 82599ES confirmed. The tool emits the PCI revision as
  raw INFO; the dev **interprets** it into a verdict: stepping → *do the B0 errata (e.g. #13) apply?*
  **Provisional anchor (mark-for-confirm):** 82599 **B0 = PCI revision `0x01`** (Intel 82599 spec
  update 331521 revision-ID table); erratum 13 is documented "B0, NoFix", so B0 → errata apply. The dev
  confirms the revision↔stepping map against the spec-update doc if it is in the reference tree; if not,
  record the raw revision as an honest open item and route the map to the PM. Note: the `AUTOC2[19]=1`
  verdict (Slice 2) stands regardless of stepping — the stepping verdict only contextualizes whether the
  erratum-13 *workaround* is live, it does not gate the bit reading. *Anchor: `live-card-probe.md` §3
  row 1; `device-bring-up-and-link.md` §3.*
- Current driver binding / link state / FW version → `ixgbe` bound, link up (baseline).
- **IOMMU group composition** — the dev **verdicts** the observed group members against the expected
  topology: does our port share a group with the second (dark) PCIe function? (tool emits this as
  INFO/observe; the match/mismatch against the two-function expectation is the dev's verdict.)
  *Anchor: `kernel-bypass-and-dma.md` §2 "the group is the unit of ownership", §5.1 — expectation:
  the X520-DA2's two functions; if shared, both bind to own the group.*
- *Method:* identity confirmation is the probe's first action (`live-card-probe.md` §5 last bullet).

**Slice 2 — Link-electrical config (read via `ethtool -d`, not raw MMIO) — corroboration-grade.**
- **AUTOC2[19]** (erratum 13) → expected `1` (EEPROM-set). **Corroboration-grade:** the verdict rests
  on the live post-init register being EEPROM-representative (no-synthesize assumption above), not on an
  NVM read. *Anchor: `device-bring-up-and-link.md` §3, §4 "actively depended-upon default".*
- **AUTOC2 serial PMA/PMD select** → expected SFI (`0x2<<16`), corroboration-grade. *Anchor: §3.*
- **AUTOC.LMS** → expected `10G_SERIAL`; **LINKS** speed → expected 10G, corroboration-grade. *Anchor: §3.*

**Slice 3 — NVM/firmware & cable.**
- **LESM enabled?** (NVM FW-module word `0x2`) → expected disabled (`0x0000`/`0xFFFF`), the preferred
  minimal posture (no sw/fw semaphore). **Authority-grade** — a direct NVM word read. *Anchor:
  `device-bring-up-and-link.md` §3 "LESM", §5 item 2.*
- **SFP+/DAC module EEPROM** (SFF-8472 identity) → expected passive DAC (`da_cu`), production-identical
  cable. *Anchor: `device-bring-up-and-link.md` §3 "module detection", §5 item 4; `live-card-probe.md`
  §5 bullet 1.*
- *(Observe-grade, out-of-band — not a gating anchor.)* Whether the stock driver needed
  `allow_unsupported_sfp` to accept the DAC. The probe does **not** read this; its evidence path is
  `dmesg` (ixgbe SFP+ accept/refuse lines) and the modparam at
  `/sys/module/ixgbe/parameters/allow_unsupported_sfp` — both read-only, both outside the probe.
  Record if cheaply observable; it informs `device-bring-up-and-link.md` §3 but does not gate the sprint.

## Non-Goals

- Do not write any device state, ever — this is the load-bearing constraint, not an aspiration.
- Do not edit the grounding notes on a clean match; a confirmed anchor needs no doc change.
- Do not delete or rewrite a documented fallback (e.g. the full `reset_pipeline` errata dance, the
  sw/fw semaphore) just because this card's reading makes it look unnecessary — trace callers and the
  dynamic-tier evidence first; those are bench-gated for the prototype cards, not this sprint.
- Do not treat a single board's reading as the prototype target's truth — it is a strong prior for
  *this* board (`live-card-probe.md` §5 bullet 2); re-read the mailed cards when they arrive.
- **FC-register reset defaults** (`FCTTV`/`FCRTL`/`FCRTH`/`FCCFG`) — the *other half* of grounding open
  item `device-bring-up-and-link.md` §5 item 3, which bundles them with `AUTOC2[19]`. The probe does not
  read them (`check_registers` pulls only AUTOC/AUTOC2/LINKS) and the PM scope did not name them.
  **Punted to E1's pre-write register read** on a prototype card. Routed to the PM as R4 to ratify the split.
- **The (B)-tier confinement checks** (interrupt remapping on, ATS-off, no peer-to-peer) — the
  "asserted not verified" half of `kernel-bypass-and-dma.md` §5; belongs to the dedicated (B) grounding
  effort, not this static sprint, even though `lspci` is already in hand.

## Starting Surface

Audit first: reproduce the **structural read-only proof** from the tool source (this is gate 3's
basis, not a runtime hope) — (1) single gateway: every device touch routes through `run_ro`; (2)
die-closed allowlist: only read verbs (`ethtool -i/-d/-e/-m`, bare `ethtool <if>`, `lspci`) pass, any
other dash-prefixed token hits `die` (note case: read `-e` allowlisted, write `-E` dies); (3) no
sysfs/proc write path (`set_*`/`unbind`/`mmap`/`resource0` appear only in comments); resting on the one
named external invariant — **ethtool's CLI grammar has no write form whose first token is the devname**.
Then identify the live 82599's BDF and interface name. Inventory shape the dev produces before/at run:

```
reading -> mechanism (ethtool -i/-d/-e/-m | lspci | sysfs) -> open-item -> evidence-grade (authority|corroboration) -> privilege (root|unpriv) -> observed -> verdict
```

The `evidence-grade` and `privilege` fields make a `SKIP` legible (needed-root vs real-absence) and
keep corroboration-grade readings from inheriting authority-grade confidence in the artifact itself.

## Method

- Read-only, kernel-mediated only: ride `ethtool`/`sysfs`/`lspci` exactly as the live `ixgbe` driver
  already does; no raw device access (`live-card-probe.md` §2, §6).
- **Run as root** (CAP_NET_ADMIN). Slice 1 is unprivileged-OK, but Slice 2 (`ethtool -d`) and Slice 3
  (`ethtool -e`/`-m`) degrade to `[SKIP]` without root — and a SKIP is not a verdict (gate 2). Root is
  required for the sprint to meet its acceptance.
- Identity-confirm before reading any 82599-specific offset; report "not an 82599" rather than
  misreading another NIC.
- **Order the active reads last:** `ethtool -m` is a live SFP+ I²C transaction — the most device-active
  reading (the driver's own routine I²C read, but a real transaction, not a config-space read). Run it
  last. (The probe adds no interaction beyond what `ixgbe` already does, but it is not literally "zero.")
- **LESM short-dump pitfall:** the LESM pointer-chase can `[SKIP]` if the default `ethtool -e` dump is
  shorter than a chased offset. If LESM comes back SKIP, re-dump with an explicit length before marking
  Slice 3 unresolved.
- **Pin the artifact to its run** (the build/log-coherence analog for a shell tool): capture the exact
  invocation, `id -u` (proves root), `ethtool` version, resolved IFNAME + BDF, the **sha256 of
  `live-card-probe.sh`**, and — because the `ethtool -d` read-only guarantee lives in the *kernel
  driver's* `get_regs`, not the probe — the **driver name + version (`ethtool -i`) and kernel
  (`uname -r`)**, so the audited in-tree `ixgbe get_regs` provably matches the one that actually ran.
  Without the script hash + uid + driver/kernel version, "observed" is not independently re-derivable.
- Capture the full structured report verbatim as the evidence artifact, and confirm it reached
  `print_footer` (a `set -euo pipefail` abort would truncate it mid-run — see gate 1).
- Report concrete grounding — register field values, NVM word, SFF-8472 fields, BDF, IOMMU group id
  and members — never "looks correct."

## Acceptance — the gates that close the sprint

1. Probe ran **as root**, **to completion** on the live 82599 (report reached `print_footer`, not
   truncated by a `set -euo pipefail` abort), and identity was confirmed (82599ES / `ixgbe`).
2. Each in-scope Slice-1/2/3 reading has an `expected → observed → verdict` line grounded in the
   report — and the verdict is a real OK/WARN, **not a `SKIP`**. A `SKIP` on a core anchor (LESM,
   `AUTOC2[19]`, SFI, module) is a **gate-2 failure**, not a satisfied line. Each line carries its
   evidence-grade (LESM authority; AUTOC2[19]/SFI corroboration). Stepping carries its provisional-B0
   anchor or an honest "B0-applicability open"; IOMMU members are verdicted against the two-function
   topology.
3. The link is unperturbed, established **structurally**: the read-only-by-construction proof (single
   gateway + die-closed allowlist + no-sysfs-write, on the named ethtool-grammar invariant) means there
   is nothing to perturb — no before/after diff is required (the single-pass tool emits none; any link
   snapshot is optional confirmation only). The "`ethtool -d` reads no RW1C register" half is verified
   against this host's `ixgbe get_regs` source, not assumed from the tool comment (see R2).
4. Matches recorded as confirmed anchors **with their evidence-grade**; every mismatch named as an
   honest open item and routed to the PM. Sprint does **not** silently rewrite a grounding note to make
   a mismatch disappear.
5. Independent review re-derives the verdicts from the actual report artifact (not the dev's summary),
   confirms the artifact-pin (script sha256 + `id -u` + tool version + BDF + **driver/kernel version**),
   verifies the captured driver is the **in-tree `ixgbe`** whose `get_regs` was audited (so the RW1C
   exclusion proven in source matches the binary that ran — an out-of-tree/different `ixgbe` would
   require re-verification), and re-derives the read-only proof from the tool source plus the
   `ixgbe get_regs` register list.

## Review Questions

- **R1 (PM/human):** If a reading **mismatches** an anchor (e.g. LESM enabled, or `AUTOC2[19]=0`, or
  the port shares an IOMMU group with the dark function), does this sprint fold the corrected value
  into the grounding note, or is that re-scope its own follow-up? (Default assumption: surface the
  mismatch here; folding-back is a separate, PM-paced step.)
- **R2 (reviewer) — resolved at formulation, re-confirm at acceptance:** read-only-ness is confirmable
  from the tool source (single gateway + die-closed allowlist + no-sysfs-write, on the named
  ethtool-grammar invariant). The **one residual** to discharge at acceptance: the "`ethtool -d` reads
  no RW1C register" claim is *inherited from the kernel `ixgbe get_regs`*, not enforceable by the
  probe's allowlist — verify it against this host's `ixgbe get_regs` register list (`ixgbe_ethtool.c`),
  so gate 3 rests on structure end-to-end rather than on a tool comment.
- **R3 (PM):** Confirm the team-doc numbering: this is filed `000-` to leave `001+` for the build
  epics, matching your "Sprint 0" framing. Object if you want it `001-`.
- **R4 (PM):** Ratify the **FC-register defaults** split: grounding open item §5.3 bundles
  `AUTOC2[19]` *and* the FC-register reset defaults, but the probe reads only the former and your scope
  named only "AUTOC2 defaults." Lean: punt the FC-register half to E1's pre-write register read on a
  prototype card (it is read-only-answerable but needs a tool change, so not this run-as-is sprint).
  Object if you want it pulled into this sprint instead.

**Review-question dispositions (PM-ruled):** R1 — surface mismatches here, grounding-note fold-back is
a separate PM-paced step (moot: zero mismatches). R3 — `000-` numbering confirmed. R4 — FC-register
half punted to E1. The IOMMU sequencing fork — **Option B**: no production reboot; record group
composition as known-open, observe at E1 under its required `intel_iommu=on` boot.

---

## Outcome & Postmortem

**Verdict: ACCEPTED, sprint closed.** Independently re-derived by `zero.nic-review` from the run
artifact and source (not the dev's summary). All five acceptance gates green.

**Run pin (binds evidence to what ran):** invocation `sudo -n bash docs/nic/tools/live-card-probe.sh
enp97s0f0`, uid 0; script sha256 `48740f61c8adf6078e9ea20bee00461efc7ec5b568e8f685a47cf836cb1f052f`;
ethtool 6.1; kernel/driver `6.12.74+deb12-amd64` (in-tree `ixgbe`, `modinfo intree:Y`); target
`enp97s0f0` / `0000:61:00.0`. Link unperturbed (operstate/carrier/speed/driver identical pre/post).

**Confirmed anchors (this board — a strong prior, not the prototype's truth; re-read the mailed cards):**
- Identity: `8086:10fb` 82599ES SFI/SFP+, X520-DA2 (2-function). **Stepping rev `0x01` = B0** →
  erratum 13 applies, `AUTOC2[19]=1` dependency live.
- **LESM DISABLED** (NVM authority: `NVM[0x0F]=0x05b8` → `word(0x5ba)=0x0000`) — minimal posture, no
  sw/fw semaphore, link path purely ours.
- `AUTOC.LMS=10G_SERIAL`, `AUTOC2` SFI (`[17:16]=0x2`), `AUTOC2[19]=1`, `LINKS` up/10G —
  corroboration-grade (post-init `ethtool -d`, EEPROM-representative under the no-synthesize assumption).
- DAC = passive `da_cu` (`SFP-H10GB-CU3M`, 3m), accepted under default policy (`allow_unsupported_sfp=N`).

**Open items carried to E1 (no grounding-note rewrite this sprint — fold-back is the queued PM-paced step):**
- **IOMMU group composition** of `61:00.0`/`.1` — known-open (VT-d off this boot); confirm at E1 under
  `intel_iommu=on`, against the confirmed 2-function topology; bind one/both to `vfio-pci`.
- **FC-register reset defaults** (R4) — the unread half of grounding §5.3; E1 pre-write register read.
- **(B)-tier containment** (interrupt remapping on, ATS/p2p off) — still asserted-not-verified; the
  dedicated (B) grounding effort.
- **`ethtool -d` read-only-ness is a per-driver-version property** — re-audit `ixgbe get_regs` against
  whatever driver E1 actually runs; `reference/linux` (7.1.0) is a *behavior* reference, not the
  running-kernel authority.

**Corrections folded (these refine the contract above; they do not rewrite it):**
- *M1 anchor basis:* the provisional "rev `0x01` = B0 per a spec-update revision-ID table" overstated
  the mechanism — 331521 has **no** revision-ID table. The B0 verdict rests on Table 1-1 (all
  production 82599 are B0-only) + Table 2-4 (erratum 13 `B0=Yes;NoFix`). The value (`0x01`=B0) stands.
- *Gate-5 pin wording:* final state is "running driver proven in-tree for 6.12.74; RW1C invariant
  **confirmed byte-against** the running 6.12.74 `ixgbe_ethtool.c get_regs` (EICR deliberately skipped
  663-666; stats via `IXGBE_GET_STAT` software shadow; no `IXGBE_READ_REG` of EICR/stat-counters in
  `get_regs`)" — not the interim "carried by inference."

### Retrospective — nic-lead (zero.nic-lead)
- **Approach:** Formed the sprint as a team (dev+review critiqued the draft before promotion), routed
  one self-contained read-only run, arbitrated every report against the gates, and deferred the
  production-reboot/sequencing fork to the PM rather than ruling it myself.
- **Evidence that changed the plan:** The no-perturb guarantee is *structural and partly kernel-owned*,
  not "looked safe" — so gate 5 tightened twice: first to pin the run artifact (script hash + uid +
  driver/kernel version), then to bind the RW1C proof to the *running* `get_regs`, not the reference
  tree. The IOMMU SKIP was an inability-to-observe (VT-d off), categorically distinct from the
  lazy/no-root SKIP my gate-2 rule targets — so it routed up as a fork, not a fail.
- **What I held the line on:** Refused to let the 7.1.0→6.12.74 version-gap inference pass gate 5 at
  "high confidence" — got the smallest grounding (a 2-minute version-pinned source spot-check) rather
  than carry an inference past the slice boundary into E1. Resolved the missing M1 anchor with a
  provisional value instead of bouncing a lookup upward.
- **What E1 inherits (process):** the team rhythm — formulate → team-review → PM green-light → route
  one owner → arbitrate against gates → independent acceptance → retro → scoped commit. And: evidence
  must bind to what ran (build/version coherence), every time.

### Retrospective — nic-dev (zero.nic-dev)
*"Read-only is inherited, not asserted; pin the proof to what ran."*
- **Tried:** one read-only probe run as root vs the live 82599 (`enp97s0f0`/`61:00.0`), `-m` last, each
  anchor verdicted in the evidence-grade+privilege shape, link proven unperturbed pre/post.
- **Evidence that changed the plan:** the no-perturb guarantee is the KERNEL's, not the probe's —
  `run_ro()` proves the probe writes nothing, but `ethtool -d`'s RW1C-freedom lives in `ixgbe get_regs`,
  so acceptance had to pin driver+version+kernel AND audit `get_regs` at the version that ran. The
  reference tree (7.1.0) is NOT the kernel that ran (6.12.74); that gap reduced the RW1C claim to
  inference until re-derived against the 6.12.74 source.
- **Failed/surprised:** a false "ethtool missing" blocker that was just `/sbin` off the unprivileged
  PATH (verify under sudo before escalating); and the standing trap that `reference/linux` is a
  different kernel than the host runs — cite the running version for "what ran," never the reference tree.
- **E1 inherits — confirmed:** 82599ES rev `0x01`=B0 (erratum 13 applies, `AUTOC2[19]=1` live); LESM
  DISABLED (no sw/fw semaphore); `AUTOC.LMS=10G_SERIAL` + `AUTOC2` SFI + LINKS up/10G; DAC `da_cu`
  (`SFP-H10GB-CU3M`) accepted with `allow_unsupported_sfp=N`. **Open:** IOMMU group composition of
  `61:00.0`/`.1` (verify once `intel_iommu=on`, bind one/both to `vfio-pci`); FC-register defaults (R4);
  the (B)-tier containment half; `reference/linux` is a 7.1.0 *behavior* reference, not running-kernel authority.

### Retrospective — nic-review (zero.nic-review)
- **Approach:** Re-derived everything from the artifact + source, never the dev's summary — the
  read-only proof structurally (`run_ro` single-gateway + die-closed allowlist + no-sysfs-write) and all
  10 anchors from raw register/NVM values and 331521 (LESM chase, `AUTOC2[19]`/SFI/LMS bit math,
  B0/erratum-13 cross-check).
- **Evidence that changed the verdict (headline):** Caught that the reference tree I audited `get_regs`
  against is kernel **7.1.0**, but the driver that ran is in-tree **6.12.74** — so the inherited
  RW1C-safety I'd "proven" was the wrong version. Downgraded a clean ACCEPT to ACCEPT-with-flagged-residual.
- **What I flagged and why:** Gate-3's "`ethtool -d` touches no RW1C register" can't be carried by a
  cross-version inference — link pre/post is blind to EICR/stat-counter clears, so the proof must rest
  on the *running* driver's `get_regs`. Discharged only after the version-pinned 6.12.74 source
  (`uname -v` == src-pkg `6.12.74-2~bpo12+1`) confirmed EICR-skip + `IXGBE_GET_STAT` shadow byte-against
  what ran.
- **What E1 inherits:** The read-only-ness of `ethtool -d` is a per-driver-version property, NOT carried
  forward — re-audit `get_regs` against whatever driver E1 actually runs. IOMMU group composition is
  PM-ruled known-open: confirm at E1 under its required `intel_iommu=on` boot, against the
  already-confirmed 2-function topology. Treat all anchors as a strong prior for THIS board only —
  re-read the mailed prototype cards the same way.
