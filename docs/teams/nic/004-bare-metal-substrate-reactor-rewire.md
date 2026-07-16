# Sprint 004 — Bare-Metal Substrate + Clean Reactor (NIC-specific, no_std, built fresh)

> The `nic` team's fourth build sprint, re-scoped after the **bare-metal pivot** and the operator's
> **clean-build direction**: build the minimal substrate that binding *this* 82599 requires — **not a
> general-purpose OS layer** — and a **clean, from-scratch, `no_std` reactor built to the ratified
> behavior as its spec**, not a port of the Linux `reactor`. No std conventions, no compatibility layers,
> **no Waker dual-impl**, no dead code — *auditable and readable in one sitting is the bar* (that review
> **is** the security guarantee). Non-security boot boilerplate rides vetted crates (`uefi`, `x86_64`,
> `acpi`); everything security-load-bearing is hand-written and reviewed-by-reading. **No prototype cards**
> — QEMU (UEFI/OVMF) proves the mechanism (a self-IPI stands in for the NIC MSI-X). This document is the
> contract; its acceptance is **team-defined for the clean build** (per the operator), not carried from
> the std framing.
>
> Team: `nic` (lead `zero.nic-lead`, dev `zero.nic-dev`, review `zero.nic-review`, pm `zero.nic-pm`).
> Authority: the bare-metal substrate **grounding doc** (`docs/nic/grounding/bare-metal-substrate.md`, in
> flight from `zero.nic-design`, under human review — this contract refines to it on clearance). Preserves
> Sprint 1 (`transport`, `53bc323`, `no_std`, unaffected). Supersedes the E4 Linux `reactor`
> (`d745215`) for the bare-metal target — the clean reactor is built fresh, not ported. Restructure:
> Sprint 5 = IOMMU/DMAR confinement; Sprint 6 = device bring-up (real 82599 registers + rings, driver
> `no_std`); Sprint 7 = integration.

---

## Scope Boundary

**This sprint owns**, sized to exactly what binding **this one 82599** needs — nothing general-purpose:

1. **UEFI handoff** — boot as a UEFI app, take the memory map + ACPI (MCFG for ECAM, DMAR for S5),
   `ExitBootServices`, own the machine from a clean handoff; serial console. (`uefi` crate for boilerplate;
   the handoff hand-checked.)
2. **ECAM — this device only, no enumerator.** Reach *only* this BDF's config space (MMIO via the MCFG
   base): **read** the firmware-already-assigned BAR0 base (we read, we do not allocate BARs), set
   `COMMAND` bus-master + memory-enable, walk the capability list for the **MSI-X** capability. No general
   PCI enumeration.
3. **Physical arena — this NIC's rings + buffers only.** A single contiguous, **identity-mapped**
   (`virt == phys`, so a programmed descriptor address *is* the arena address) extent for the RX/TX
   descriptor rings (16 B/entry, aligned) + packet buffers, carved **DMAR-confinable** (aligned +
   contiguous to the DMAR page granularity so Sprint-5 confines exactly these regions, nothing adjacent).
   **This is the sprint's primary security-adjacent target:** bounds, no-overlap, DMA-region isolation.
4. **IDT / LAPIC / MSI-X vector + one-shot timer + `hlt` — this NIC's interrupt only.** Minimal exception
   handlers + exactly the NIC MSI-X vector and the timer vector; the LAPIC one-shot timer for deadlines;
   `hlt` parks until an interrupt. **Boundary:** the substrate provides the *mechanism*; driving the
   82599's own registers (`RDBAL`/`IVAR`/`EIMS`/`RXDCTL`…) is **Sprint 6**. This sprint proves the
   mechanism in QEMU with a **self-IPI standing in for the NIC MSI-X** (no card driven).
5. **The clean `no_std` reactor — built fresh from the ratified behavior, not a port.** Re-express, from
   scratch and OS-free: **poll-then-block** ordering, **deadline composition** (salvage only the three
   edge-math *facts* — None→park-indefinitely, past→fire-now, future→one-shot at the deadline — re-typed
   to the bare-metal timer arm, no `i32`-ms/epoll shape), **sole-sink-holder**, and **back-pressure
   re-expressed as RX flow control** (hold `RDT` while the stop-and-wait transport cannot accept — *not*
   the old `EPOLLONESHOT`). The fast-path property (was "zero eventfd reads during a stream") becomes
   **"zero `hlt` parks while there is work."** The wakeup-source distinction (was `WakeReason`) survives
   as **NIC-MSI-X vs timer**. **Dropped entirely, not cfg-dead:** epoll/eventfd/`libc` events, `std::io`,
   the whole **app-fd class** (PM-ruled above the boundary). `transport` (`no_std`) drives it unchanged.

**This sprint does NOT own** (named, with where it belongs):

- **IOMMU/DMAR confinement (Sprint 5)** — our hand-written DMAR (now a *trusted-base* surface: a DMAR bug
  is a containment hole → E3-grade confinement/soundness). E2's security framing (three-level
  enabled/blocked/sound verdict, the induced out-of-range-DMA-block, honest-open hazards) carries there,
  retargeted to our DMAR. The arena here is laid out **DMAR-confinable** so S5 can confine it.
- **Driving the 82599 (Sprint 6)** — its own registers, the link, the real MSI-X→wakeup, the driver going
  `no_std`, real rings. This sprint is *mechanism*, proven in QEMU with a self-IPI.
- **The Linux `reactor` (E4)'s `std` code** — superseded for bare-metal; the clean reactor is built fresh,
  the old `std`/epoll/app-fd scaffolding is **not** carried (no compat layer, no dead code).
- **The transport (E5)** — `no_std`, zero OS surface, reused **unchanged** (a fact, not a seam-swap claim).

## Problem

The stack was built on a Linux host; the project is **bare-metal — no kernel**. Nothing boots the
machine, reaches *this* 82599's config via ECAM, carves its DMA-able memory, or handles its interrupt
without the kernel. And the operator's direction is explicit: do **not** preserve the `std` reactor behind
a compatibility seam — that carries dead code onto a trusted-base member whose whole guarantee is
"reviewable in one sitting." So the reactor must be **rebuilt clean** to the ratified behavior, and the
substrate sized to **this NIC only**.

## Goal

A minimal, NIC-specific bare-metal substrate (UEFI handoff, this-device ECAM, a DMAR-confinable arena for
this NIC's rings/buffers, this NIC's MSI-X + timer interrupts) and a **clean, from-scratch `no_std`
reactor** whose ratified behavior (poll-then-block, deadline composition, sole-sink, RX-flow-control
back-pressure — no app-fd, no OS shapes) is **proven correct fresh**, driving the reused `no_std`
transport — every line on the live 82599 path, no dead code, reviewable in one sitting.

> "Reviewable in one sitting *is* the security guarantee. Code that cannot be held in the head is a defect
> even if it works." — `epics.md`, on the trusted-base bar.

## Final State

- Boots as a UEFI app, `ExitBootServices`, runs on the bare machine (serial); QEMU-provable.
- ECAM reaches this 82599 only: BAR0 base read, bus-master/mem-enable set, MSI-X cap found — observed.
- The arena is one contiguous identity-mapped DMAR-confinable extent for this NIC's rings+buffers; bounds
  / no-overlap / DMA-isolation hold (observed addresses).
- IDT/LAPIC handle this NIC's MSI-X vector + the timer; the one-shot timer arms+fires; `hlt` parks/wakes.
- The clean `no_std` reactor runs the ratified behavior on the substrate driving `transport`: busy-poll,
  `hlt` when idle (zero parks while there is work), wake on timer/self-IPI, deadline fires the timer →
  retransmit, one-in-flight RX-flow-control back-pressure — with the **pure decision logic proven fresh on
  the host** and the loop proven on the substrate in QEMU.
- Every function/type is on the live 82599 path — no cfg-husks, no compat shims, no app-fd remnants, no
  general-purpose generality, no dead code; the new bare-metal `unsafe` is `deny`+`allow`-confined, sound.

## In Scope — the ordered slices

**Slice 1 — UEFI handoff + serial + a host+QEMU test harness.** Boot as a UEFI app, take the memory map +
ACPI, `ExitBootServices`, serial console; a serial-assert harness (observed→expected, self-pinning boot
banner). *Method: QEMU UEFI/OVMF boot; assert the handoff + serial; capture the self-pinned log.*

**Slice 2 — ECAM (this device) + the physical arena.** Reach this BDF's config, read BAR0, set bus-master,
find MSI-X; carve the identity-mapped DMAR-confinable arena for this NIC's rings+buffers. *Method: observe
BAR0/bus-master/MSI-X for the (QEMU) device; assert the arena's actual addresses in-range, no-overlap, one
DMAR-confinable extent — observed, not "PASS".*

**Slice 3 — IDT / LAPIC / MSI-X vector + one-shot timer + `hlt`.** Minimal IDT + this NIC's MSI-X vector +
the timer vector; LAPIC; the one-shot timer; `hlt`. *Method: arm the timer, `hlt`, assert the handler ran
+ the CPU woke; a self-IPI stands in for the NIC MSI-X.*

**Slice 4 — The clean `no_std` reactor + integration.** Build the reactor fresh (poll-then-block, deadline
composition, sole-sink, RX-flow-control back-pressure; no app-fd, no OS shapes) with the **pure decision
functions structured (state in → decision out)** for host testing; run it on the substrate driving
`transport`. *Method: host `cargo-test` of the pure logic (G1); the loop on the real substrate under QEMU
(G2); reviewed-by-reading for no-dead-code + confined `unsafe` (G3).*

## Non-Goals — and the invalid checkpoints we explicitly RETIRE

- **No general-purpose / reusable substrate** — sized to this one 82599, nothing more.
- **No std conventions, compat layers, Waker dual-impl, or dead code.** The app-fd class is **removed**
  (not cfg-gated dead code). `no_std` stays.
- **No driving the 82599 registers / link / real rings (Sprint 6); no IOMMU/DMAR (Sprint 5).**
- **Retired invalid checkpoints (manufacturing these is the failure the operator warned of):**
  "re-run the 19 std tests" (no std impl exists — its *intent* survives as G1 on the clean pure functions);
  "engine.rs git-diff-zero / seam-swap" (the reactor is *replaced*, not preserved); **any gate for a
  dropped feature** (app-fd back-pressure, fd/token dispatch, `EPOLLONESHOT` re-arm — testing a removed
  feature *is* the manufactured checkpoint); "compiles bare-metal" **alone** (necessary, not sufficient);
  QEMU "PASS" without the observed value; "no dead code" **self-asserted** (must be the reviewer's
  reviewed-by-reading); carrying the old gate *structure* onto the clean build.

## Method

- **The clean surface defines the gates.** The reactor's surface is deadline + NIC-wakeup only (no
  app-fd); every gate matches that surface. A gate for a dropped feature is invalid.
- **Two evidence surfaces + a review gate.** (G1) host `cargo-test` of the **pure `no_std` logic** — the
  real shipped decision functions (deadline edge-math, poll-then-block/back-pressure decisions),
  structured pure, exercised via `cfg(test)`; this is the standard `no_std` host-test idiom (the audited
  library stays pure `no_std`; only the test build links `std`), **not** the killed std-Waker impl. (G2)
  QEMU UEFI/OVMF, **observed→expected, self-pinning**, the reviewer **re-runs it** where the toolchain
  permits. (G3) **reviewed-by-reading** the security surface + no-dead-code.
- **Auditable-and-readable is a first-class gate**, not an aspiration — the small-and-ours discipline as
  acceptance.
- **Crate boilerplate vs hand-written security, split explicitly** — `uefi`/`x86_64`/`acpi` only *decode*;
  the security-bearing *use* (the arena's decisions, the vector policy, the DMAR base later) is ours and
  reviewed.
- **`unsafe` confined + marked** (`deny`+`allow`, SAFETY comments, no leak) per the E3/E4 convention.

## Acceptance — the gates that close the sprint (team-defined for the clean build)

1. **G1 — pure logic proven FRESH (host `cargo-test`, review re-runs):** the clean reactor's **actual
   shipped pure functions** — the deadline edge-math (None→park, past→now, future→one-shot) and the
   poll-then-block / RX-flow-control **decision** functions (state in → decision out) — pass deterministic
   host tests via `cfg(test)`, matching the clean surface (deadline + wakeup only, **no app-fd**).
   Build-coherence pinned. *(This is not the killed "19 std tests"; it runs the one real pure logic, no
   std reactor impl.)*
2. **G2 — substrate + reactor on QEMU, OBSERVED, self-pinning (review re-runs):** boot; ECAM (BAR0 +
   bus-master + MSI-X **observed**); arena (actual addresses in-range, no-overlap, one DMAR-confinable
   extent); interrupts (MSI-X + timer handler ran, `hlt` woke); the reactor loop on the **real timer**
   (deadline fires → retransmit; self-IPI stands in for the NIC MSI-X; **zero `hlt` parks while there is
   work**; one-in-flight back-pressure holds). The boot banner pins the binary build-identity + the
   QEMU/OVMF invocation; every verdict states observed→expected, never bare "PASS".
3. **G3 — auditable / no dead code / confined `unsafe` (reviewed-by-reading):** independent review READS
   the clean reactor + substrate and confirms **every function/type is on the live 82599 path** — no
   cfg-husks, no compat shims, no app-fd remnants, no general-purpose generality, no dead code; and the new
   bare-metal `unsafe` is `deny`+`allow`-confined, each block sound with a SAFETY comment, no leak into
   safe logic. The substrate is confirmed **NIC-minimal** (sized to this 82599), and the arena
   **DMAR-confinable**.
4. Independent review re-derives G1 (re-run the host tests) + G2 (re-run QEMU where the toolchain permits,
   else the self-pinning captured log) + G3 (read the source) — not the dev's summary; it **rejects** any
   invalid/manufactured checkpoint (a gate for a dropped feature, a bare "PASS", a self-asserted
   no-dead-code, a compiles-alone claim).

## Review Questions

- **R1 (PM/human) — crate structure.** Lean: a `substrate` crate (UEFI + this-device ECAM + arena +
  IDT/LAPIC/MSI-X/timer; `uefi`/`x86_64`/`acpi` deps) and a fresh **`no_std` reactor** built to the
  ratified behavior (the Linux `reactor`'s `std` code not carried). Confirm.
- **R2 (PM/operator) — THE ONE FORK: is the host `cargo-test` of the pure `no_std` logic (G1) in scope, or
  does "clean" mean QEMU + reviewed-by-reading ONLY?** Both teammates recommend **G1 IN**: it tests the
  *real shipped* pure functions, is the cheapest strongest correctness signal, and is the **standard
  `no_std` idiom already used by the accepted `transport` crate** (33 host tests: pure `no_std` lib, the
  `cfg(test)` build links `std`) — it is **not** the killed dual-impl (which ran a std-EPOLL *reactor
  impl*). If ruled out, the pure logic falls to G2+G3 (heavier, less granular, no fast deterministic
  edge-math coverage). The operator set the "no std conventions" bar, so this reading is the operator's.
  *(The reviewer's related fork — a testability seam vs a fully-concrete reactor — is **resolved by
  convergence**, not a compromise: the clean reactor keeps a **minimal, NIC-shaped 2-method `no_std`
  seam** (`block until wakeup-or-deadline`, `now`) in the shipped code. This SERVES the auditability bar
  rather than fighting it — it keeps the pure poll-then-block loop host-provable and **`unsafe`-free**,
  and confines the hlt/LAPIC/timer MMIO in the seam impl; inlining the hardware into the loop would spread
  `unsafe` through the pure sequence and read worse. Both teammates converge on the minimal seam; its
  exact shape is the dev's Slice-4 call. Its host-testability benefit (G1) rides on this fork.)*
- **R3 (design/PM) — the grounding.** `004` refines to `bare-metal-substrate.md` on its human-review
  clearance. The **Sprint-5 IOMMU/DMAR** grounding carries E2's security framing (three-level verdict,
  induced-block, honest-open) retargeted to our DMAR — which enters the trusted base as small-and-ours.

---

## Outcome & Postmortem

**Verdict: ACCEPTED, sprint closed.** Independently re-derived by `zero.nic-review` from the working-tree
source + its **own** runs of both G1 (host cargo-test, 21/21) and G2 (QEMU, 3 boots) — the strongest
evidence path, available once the toolchain landed. Both Slice-1/2 findings folded; no outstanding findings.

**Delivered:** the `substrate` crate — a **NIC-specific** bare-metal foundation (UEFI/`ExitBootServices`
handoff + serial; this-device ECAM via ACPI MCFG; a DMAR-confinable identity-mapped DMA arena; a minimal
IDT + hand-rolled LAPIC one-shot timer + self-IPI; a global heap + TSC/LAPIC calibration) and a **clean,
from-scratch `no_std` reactor** — the ratified behavior re-expressed **fresh** as a *pure* loop over a
minimal 2-method seam (`now` + `block → WakeReason`), driving the **reused** `no_std` `transport`. **31
host tests** (banner/ecam/arena/interrupts/reactor) + a self-pinned 4-slice QEMU boot.

**Gates (team-defined for the clean build):** G1 host `cargo-test` of the **real shipped pure functions**
(non-vacuous — the predicates are shown to *reject* bad inputs); G2 QEMU **observed→expected**,
self-pinning, reviewer-re-run; G3 **reviewed-by-reading** — NIC-minimal, no dead code, confined `unsafe`
(nine sites, `deny`+`allow`+SAFETY, no-leak from public signatures). The invalid checkpoints were retired
by name (the "19 std tests", git-diff-zero, any gate for a dropped feature, bare "PASS", self-asserted
no-dead-code).

**The seam discipline's payoff, proven:** `transport`/`reactor`/`driver` are **git-unchanged** (empty
porcelain — the reused-unchanged claim proven, not asserted); the clean reactor lives in `substrate/`,
built fresh, not by patching. The ratified properties re-expressed cleanly: **zero `hlt` parks while
there is work** (the bare-metal analog of `eventfd_reads==0`); back-pressure as **RX flow control** (hold
`RDT`); the **MSIX_PENDING latch** (`swap` before the atomic `sti;hlt`) — the lost-wakeup race the reviewer
flagged in the E4 critique, closed correctly on the bare-metal side. The app-fd class is **grep-zero
gone**, not cfg-dead.

**Honestly surfaced, not worked around:** the two *necessary* substrate additions the reactor entailed
only when it met the metal (a global heap — `transport` uses `alloc` and the UEFI allocator dies at
`ExitBootServices`; TSC/LAPIC calibration — `now()` and the deadline-arm need a tick↔time map, rough
because they share the measurement so causality holds); QEMU 7.2 has **no 82599 model**, so the ECAM
*mechanism* (PCI-standard) was proven against **e1000e** (substitution surfaced, not faked), the
82599-specifics scoped to Sprint 6; `x86-interrupt` is still nightly-only, so the interrupt entry stays
**stable** via naked-function stubs (the stable-vs-nightly convention is an open, non-blocking operator
call — stable ships until ruled).

**Next sprints inherit:** **Sprint 5 (IOMMU/DMAR)** — the arena is already laid **DMAR-confinable** (one
2 MiB-aligned whole-DMAR-page extent) so S5 confines exactly it; E2's security framing (three-level
enabled/blocked/sound verdict, the induced out-of-range-DMA-block, honest-open) retargets to **our own
hand-written DMAR**, which now **enters the trusted base as small-and-ours** (a DMAR bug is a containment
hole — reviewed-by-reading like the guards, not a config we cite). **Sprint 6 (device bring-up)** — the
real 82599 config (0x10fb, the 64-bit prefetchable BAR0 — decode path **already host-tested** via
`ecam::decode_bar`), real RX/TX rings on the E3 `Volatile` rings, `IVAR`/`EIMS`, the driver going
`no_std`, and the real NIC MSI-X **replacing the self-IPI behind the same reactor seam** (a seam-impl
swap, not a loop change).

### Retrospective — nic-lead (zero.nic-lead)
- **Approach:** coordinated two operator frame-punctures in one sprint — the Linux→bare-metal pivot, then
  the clean-build re-scope — conceding each cleanly without defending the prior plan. Formed the sprint as
  a team, routed one slice at a time, arbitrated against the *team-defined* clean gates, and deferred the
  genuine forks (the clean-build direction, the host-test-harness reading, the stable-vs-nightly
  convention) to the operator rather than ruling conventions myself.
- **Evidence that changed the plan:** the operator rejected the Waker dual-impl the team had just
  converged on (and I had folded) — it carried dead compatibility code onto a trusted-base member whose
  guarantee is "reviewable in one sitting." I dropped it; the team rebuilt a *better* design — a fresh,
  pure, host-provable reactor with zero dead code — in one formulation pass. And the seam discipline held
  under a full OS swap: `transport` reused with zero edits, the pivot contained to the substrate layer.
- **Held the line on:** letting the team define valid acceptance for the clean build and **retiring the
  invalid checkpoints by name** (the 19-std-tests, git-diff-zero — carrying the old gate *structure* onto
  a clean build is the manufactured checkpoint the operator warned of); surfacing the genuine convention
  forks up rather than deciding them; and the no-dead-code / auditable-in-one-sitting bar as a first-class
  gate, not an aspiration.
- **Next sprint inherits (process):** on a trusted-base member, a clean purpose-built rebuild can beat an
  elegant compatibility-preserving seam-swap — "reviewable in one sitting *is* the security guarantee";
  and when the human punctures a frame the team just converged on, concede fast and let the team rebuild —
  it cost one formulation pass, not a sprint.

### Retrospective — nic-dev (zero.nic-dev)
*"The seam discipline held — a fresh, pure, host-provable reactor on a NIC-sized bare-metal substrate,
transport reused unchanged, every necessary addition surfaced honestly."*
- **Built:** the NIC-specific bare-metal substrate (UEFI handoff+serial; this-BDF ECAM; a DMAR-confinable
  arena; minimal IDT + hand-rolled LAPIC one-shot/self-IPI) + the clean from-scratch `no_std` reactor —
  the ratified behavior as a *pure* loop over a 2-method seam (`now` + `block→WakeReason`) driving the
  reused transport; proven per slice by G1 host `cargo-test` + G2 self-pinned QEMU + G3 review-by-reading.
- **Evidence that shaped it:** the clean-build re-scope (Waker dual-impl / std-carry / 19-std-tests /
  git-diff-zero killed as invalid; rebuild fresh, team-defined gates); the seam discipline paid off —
  transport driven with **zero edits**; the ratified behavior re-expressed + proven (zero-parks-while-work,
  RX-flow-control back-pressure, the MSIX_PENDING latch carrying the woke-by class).
- **Surprised:** two **necessary** substrate additions surfaced only when the reactor met the metal (the
  heap; TSC/LAPIC calibration) — entailed, surfaced, not worked around; QEMU 7.2 has no 82599 model (ECAM
  proven on e1000e, substitution surfaced); `x86-interrupt` still nightly-only, so the entry stays stable
  via naked stubs.
- **Next sprints inherit:** S5 — the arena is already DMAR-confinable; E2's framing retargets to our DMAR
  (now small-and-ours). S6 — the real 82599 config (64-bit BAR decode already host-tested), real rings on
  the E3 `Volatile` rings, `IVAR`/`EIMS`, the driver going `no_std`, the real NIC MSI-X replacing the
  self-IPI behind the *same* reactor seam (a seam-impl swap, not a loop change). Open: stable-vs-nightly.

### Retrospective — nic-review (zero.nic-review)
- **Acceptance approach on a bare-metal sprint:** re-derive every gate from source + my own runs; once the
  dev landed qemu+OVMF+the uefi target I re-ran **both** surfaces myself (host 21/21 AND the QEMU boot,
  3 boots) rather than trust the captured log — the strongest path, upgraded from the fallback the moment
  the toolchain existed (re-checked under a full PATH per the /sbin lesson first). Observed→expected over
  serial, never bare PASS; self-pinning; the **git-check** that `transport`/`reactor`/`driver` are
  unchanged (reused-unchanged **proven** by empty porcelain); and review-by-reading no-dead-code as a
  **first-class** gate.
- **Evidence that sharpened a verdict:** at Slice 1-2 I flagged two findings from source — the stale
  confinement doc (named 2 `unsafe` sites; there were 4) and the coded-but-unexercised 64-bit BAR decode
  (e1000e is 32-bit); both folded (a pure host-tested `decode_bar` covering both paths; `lib.rs` now lists
  all nine sites). I confirmed the QEMU determinism is **measurement-not-logic** by booting 3× (calibration
  wobbles ~2 MHz/boot, every behavioral assert byte-identical — `now()` and the arm share the window so the
  absolute error cancels). The e1000e substitution is a sound *mechanism* proof (ECAM is PCI-standard) with
  the 82599-specifics honestly scoped to S6.
- **Held the line on:** the clean-surface gates **match the clean surface** — no gate for a dropped feature
  (the operator's manufactured-checkpoint warning); the app-fd class is **grep-zero gone**, not cfg-dead.
  And the sprint-shaping earlier catch: "engine.rs byte-unchanged" was infeasible (coupled to
  `libc::epoll_event`/`std::io`/`OwnedFd`), so the gate became semantic-logic-unchanged-built-fresh — which
  the fresh reactor proved, with the lost-wakeup race (my own E4 [A](b) flag) closed correctly.
- **Next sprints (S5/S6) inherit:** re-verify on the **bench card** what QEMU/e1000e could not (the real
  82599 config ids, its 64-bit BAR, real NIC MSI-X); the DMAR grounding retargets E2's framing to our-own
  hand-written DMAR (small-and-ours, reviewed-by-reading, a bug is a containment hole); the arena's extent
  is already DMAR-confinable so S5 confines exactly it; carry the standing gates (build-coherence /
  self-pinning, confined-`unsafe`, observed-not-PASS, review-by-reading no-dead-code).
