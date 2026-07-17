# Sprint 006 — Pure-Direct Bring-Up & Proven Data Transmission (re-scoped)

> The `nic` team's sixth build sprint, re-scoped after the **pure-direct pivot** (operator, ratified +
> committed `1149aef`). The bare-metal substrate (Sprints 4–5) is **scrapped**: the ratified runtime is a
> **userspace process on a running Linux system** that takes the NIC and drives it **directly** — sysfs
> unbind, `mmap` BAR0 via `resource0`, hugepage DMA with `pagemap`-resolved physical addresses, **no VFIO,
> no IOMMU**, interrupts realized as **bounded polling**. This is ixy's non-VFIO path. The goal is concrete
> and end-to-end: **proven data transmission between the two bench cards** (`41:00.0 ↔ 63:00.0`) over the
> DAC. The surviving Sprint 1–3 crates (transport/reactor/driver) come back to the front and are retargeted
> behind their existing seams; the seam discipline that contained the last pivot contains this one.
>
> **Standing hard constraint (operator, absolute) — sharper now, because we run on the LIVE production
> host with NO IOMMU:** only the **bench cards** (`41:00.0`, `63:00.0`) are unbound / bus-mastered /
> `mmap`'d / DMA'd — **never the production NIC** (`61:00.0` / `enp97s0f0`, live 10G). Every sysfs op and
> every DMA target is gated by a **fail-closed BDF allowlist + device-identity assert**; a mismatch refuses.
> With no IOMMU, a DMA bug is unconfined on the running host — the physical-address discipline is
> safety-load-bearing.
>
> Team: `nic` (lead `zero.nic-lead`, dev `zero.nic-dev`, review `zero.nic-review`, pm `zero.nic-pm`).
> Authority: [`grounding/kernel-bypass-and-dma.md`](../../nic/grounding/kernel-bypass-and-dma.md) (the
> pure-direct spec, `1149aef`), [`grounding/device-bring-up-and-link.md`](../../nic/grounding/device-bring-up-and-link.md),
> [`grounding/driver-minimality.md`](../../nic/grounding/driver-minimality.md),
> [`grounding/data-path-and-events.md`](../../nic/grounding/data-path-and-events.md),
> [`concerns-to-watch.md`](../../nic/concerns-to-watch.md), [`epics.md`](../../nic/epics.md) (E1),
> `reference/ixy` (`pci.c`, `memory.c` — the direct path). Salvage: S1 transport unchanged; S2 reactor
> algorithm survives (wakeup → bounded polling); S3 driver guard+copy-boundary survives (DMA → hugepages,
> regs → mmap'd BAR0); **S4–5 substrate scrapped**. Sprint 7 = integration + the §2 policy audit lands
> elsewhere.

---

## Scope Boundary

**This sprint owns**, as a userspace process on the running host, scoped to the bench cards:

1. **The direct-path control module** (the ixy `pci.c`/`memory.c` shape, replacing the scrapped
   substrate): **unbind** the bench NIC from its kernel driver (`/sys/bus/pci/devices/<bdf>/driver/unbind`),
   **enable bus-mastering** (config offset 4 bit 2 via `.../config`), **`mmap` BAR0** (`.../resource0`),
   and **hugepage DMA** (2 MiB hugetlbfs, `mlock`, physical addresses via `/proc/self/pagemap`), keeping
   the 82599 128-byte ring/buffer alignment. All **BDF-allowlist-scoped + fail-closed**.
2. **E1 device bring-up + link on the real 82599**, driven through the mmap'd BAR0: the datasheet §4.6.3
   reset + the two mandatory waits (`EEC.ARD`, `RDRXCTL.DMAIDONE`) + the **minimal bench-determined** link
   restart → a **10G SFI link over the DAC**, on **both** bench ends (the link needs both PHYs up).
   Defaults-first (AUTOC2 SFI + bit 19 left EEPROM-provided).
3. **Retarget the surviving crates behind their seams** — **driver (S3):** DMA source → hugepages +
   pagemap physical, register access → mmap'd BAR0 (the guard + copy-boundary logic **unchanged**);
   **reactor (S2):** wakeup → **bounded polling** (the algorithm **unchanged** — "block" is a bounded
   poll / short sleep, no delivered interrupt). **transport (S1):** unchanged.
4. **Proven data transmission** — a frame **TX'd from one bench card is RX'd on the other** over the DAC,
   payload intact through the copy boundary — the sprint's goal (`41:00.0 ↔ 63:00.0`).
5. **Retire the scrapped substrate crate** (S4–5) — remove it, having traced that nothing surviving
   depends on it.

**This sprint does NOT own** (named, with where it belongs):

- **IOMMU/DMAR confinement** — *removed* by the pivot. The DMA posture is now **"owned, not contained"**
  (`kernel-bypass-and-dma.md §2`); its **policy audit + the `architecture.md §2.1`/spine reconciliation**
  land **post-PoC with the policy layer**, not here. This sprint records the residual honestly; it does not
  ratify the posture.
- **The box-side capture / proxy egress / head / file-quarantine / session semantics** — above this
  component (`epics.md` scope-out).
- **The final integration-validation pass (Sprint 7)** — throughput/latency at the real RTT, the reactor
  on one core under full load, buffer sizing under a redraw storm.
- **The dark-feature space** — multiqueue/RSS/Flow-Director/FCoE/DCB/1588/offloads; one RX + one TX ring.

## Problem

The bare-metal substrate (S4–5, committed `c9ceca2`/`355dd73`) is scrapped by the pivot. The surviving
crates (transport/reactor/driver, Sprints 1–3) were built behind seams, but their DMA/register/wakeup
backings targeted the now-scrapped substrate (or the earlier epoll draft). **Nothing yet drives the real
82599 via the direct path, and no data has crossed the DAC.** And we now run on the **live production
host** with **no IOMMU** — so the operation must be provably scoped to the bench cards.

## Goal

**Proven data transmission between the two bench cards over the pure-direct path** — the 82599 brought up
through the mmap'd BAR0, DMA over pagemap-resolved hugepages, a frame moving `41:00.0 ↔ 63:00.0` on the
running host, with the surviving crates retargeted behind their seams and the **production NIC provably
untouched**.

> "A confinement whose correctness rests on the untrusted kernel is not real containment — it launders the
> risk through kernel trust rather than removing it… we own the device and the risk directly."
> — `kernel-bypass-and-dma.md §2` (operator, 2026-07).

## Final State

- The direct-path module unbinds / bus-masters / mmaps / hugepage-DMAs the **bench cards only**,
  fail-closed on any non-allowlisted BDF.
- Both bench ends reach a **10G SFI link** over the DAC via the mmap'd BAR0, minimal write-set recorded.
- The driver's DMA/register backing and the reactor's wakeup are retargeted; guard + copy-boundary +
  reactor algorithm **unchanged** (tests re-run green).
- A frame **crosses the DAC** `41:00.0 → 63:00.0`, payload intact — data transmission proven.
- The substrate crate is retired; the **no-IOMMU DMA-anywhere** residual is recorded open for the post-PoC
  policy audit; the production NIC was never touched.

## In Scope — the ordered slices

**Slice 1 — The direct-path control module + the fail-closed BDF safety invariant.** Re-confirm the
running-host anchors read-only first (sysfs/`lspci`): the bench BDFs `41:00.0`/`63:00.0`, that they are
82599 (`8086:10FB`, subsys `8086:0003`), which port is the DAC end, hugepage availability; identify the
production `61:00.0`/`0x7a12` to **exclude**. Then build the ixy-shape module: unbind, BME, `resource0`
mmap, hugepage+pagemap DMA — every sysfs op and DMA target gated by the **allowlist + identity assert**,
fail-closed. *Method: ixy `pci.c`/`memory.c` shape; the allowlist assert is the PRIMARY safety invariant;
read-the-identity-before-unbind (device-id is confirmation of a pinned BDF, never discovery).*

**Slice 2 — Bring-up to link (both ends), via mmap'd BAR0.** Per bench end: the §4.6.3 reset + the double
interrupt disable + the two mandatory waits; then the **minimal** link restart, bench-determined by
escalation (nothing → `Restart_AN` → `reset_pipeline`, least that links). Confirm **10G SFI link-up over
the DAC** on both ends. *Method: defaults-first (AUTOC2 unwritten); record the escalation evidence (a
smaller form was tried and did not link); no single-card SFI loopback test.* **HARD CHECKPOINT after this
slice — link is the biggest silicon unknown.** If link proves iterative/expensive, the sprint **splits**
here: 6a (module + link) / 6b (retarget + transmission). Slices 3–4 are largely host-testable, so silicon
risk concentrates in this slice and Slice 4.

**Slice 3 — Retarget behind the seams (host-testable; the freeze is split, not blanket).** Two layers, and
the freeze claim is **re-scoped** — "re-run the existing tests" is *invalid* for the wakeup, whose tests
exercise the removed eventfd mechanism (the dead-path trap):
- **Genuinely frozen** (provable by an **empty git-diff** + tests re-run): the driver **guard +
  copy-boundary** and the `RxRing`/`TxRing`/`RxDrain` **seam traits** (above the seam); the reactor's pure
  logic — `compute_timeout` (the 3 edges), poll-then-drain, single-held-sink, malformed-drop totality,
  back-pressure re-arm.
- **Real rewrite** (needs **NEW positive tests**, an eventfd re-run proves nothing): the reactor's
  **block/wakeup primitive** — eventfd/epoll → **bounded ring-poll** (a bounded poll pulls a ring-`DD`
  frame; the retransmit deadline composes into the poll interval, was `compute_timeout`→epoll-timeout; no
  busy-spin when idle). **Baseline = the fresh `substrate::reactor`** (its two-method `Host` seam
  `block(timeout)->WakeReason` was purpose-built for a pluggable block, so bounded-poll is a **drop-in Host
  impl**) — **not** the epoll-coupled `reactor/` crate. This is **salvage-as-MOVE**: `substrate::reactor`
  (its pure algorithm + `Host`/`RxSource` seams + its 6 tests) is **extracted into a surviving home** here,
  *before* Slice 4 `git rm`s the rest of the substrate — so the substrate is still **fully removed** (gate
  6 holds) and what survives is re-homed code, not a live dependency on the scrapped crate. The frozen-half
  diff baseline is that reactor's **accepted Sprint-4 source** (the move keeps it byte-identical).
- **The driver seam impl (`dma.rs`) is a real change below the seam, not a drop-in** — it assumes bare-metal
  identity mapping (`virt==phys`): `TX post()` writes the **virtual** pointer into `buffer_addr`
  (`dma.rs:238`), RX refill sets no `pkt_addr`. The retarget is a **virt/phys split**: the **pagemap
  physical** address goes into `buffer_addr`/`pkt_addr`/`RDBAL`/`TDBAL` (distinct from the virtual pointer
  used for copy/post; bounded — the hugepage is contiguous, add a `phys_base`), and the **RX refill
  `pkt_addr` write is new code**. This is the **safety-load-bearing** change (a wrong physaddr DMAs to
  arbitrary host RAM, no IOMMU); it does **not** hide under "retarget didn't alter logic."

*Method: seam discipline for the frozen half (empty diff + tests); read-the-new-code + new positive tests
for the rewritten wakeup and the virt/phys DMA backing; most of this slice is host-testable without silicon.*

**Slice 4 — Prove data transmission + retire the substrate.** **TX a frame from `41:00.0`, RX it on
`63:00.0`** over the DAC (raw frame first — R4). The bar is **non-vacuous, un-fakeable**: a **unique nonce
payload** (not zeros), **RX buffers pre-cleared and proven-cleared** before the send, the **RX `DD` set by
the real device**, the payload **byte-equal across the two genuinely-separate cards** (distinct BDFs/BARs,
over the DAC — the wire, not a software loopback), routed **through the real `copy_out`/`RxDrain` seam**
(so the copy boundary is exercised), and — the **un-fakeable hardware tell** (the FRCD analog) — the
82599's **RX packet counter (`GPRC`) incremented by exactly the frames sent**. Then **`git rm` the rest of
the substrate** (the reactor already extracted in Slice 3 — salvage-as-move; trace nothing surviving
depends on the crate). *Method: a real frame across the real DAC; the payload + the device counter are the
proof; the DMA-anywhere risk is owned + honest-open, not claimed contained; and a frame crossing intact
**once** proves the path **works**, it does **not** discharge the `dma.rs` crossing-soundness residual
(volatile visibility / `DD`-read ordering / no-tearing) — that stays open.*

## Non-Goals — and the hard constraints

- **No IOMMU/DMAR** — removed by the pivot; the DMA posture is **owned**, policy-audited **post-PoC**, not
  here. Do not re-introduce or claim containment.
- **Only the bench cards** (`41:00.0`/`63:00.0`), fail-closed BDF allowlist + identity assert (`8086:10FB`
  / subsys `0x0003`); the production `61:00.0`/`0x7a12` is **never** unbound / bus-mastered / mmap'd /
  DMA'd, and is denylisted as additive defense. **Absolute.**
- **No kernel network stack in the path; no VFIO/UIO** — interrupts are **bounded polling**, not delivered.
- **Defaults-first / minimal write-set** (E1); one RX + one TX ring; no dark features.
- **Do not delete a fallback because production does not hit it** — trace callers/tests first; this
  especially governs **retiring the substrate crate** (confirm no surviving crate imports it) and
  preserving the surviving guard/copy-boundary/algorithm untouched.

## Starting Surface

The running-host anchor inventory (read-only, sysfs/`lspci`) and the retarget surface:

```
bench BDF (41:00.0, 63:00.0) -> [82599 8086:10FB / subsys 0003 / DAC-port / hugepages / root] -> allowlist-freeze
production BDF (61:00.0 / 7a12) -> DENYLIST / exclude
ixy pci.c (remove_driver/enable_dma/pci_map_resource) + memory.c (memory_allocate_dma/virt_to_phys) -> the module shape
driver(S3) DMA+register seam / reactor(S2) wakeup seam -> retarget targets
substrate(S4-5) crate -> retire (trace dependents first)
```

## Method

- **The ixy direct-path shape** is the reference — `pci.c` (`remove_driver`, `enable_dma`,
  `pci_map_resource`) + `memory.c` (`memory_allocate_dma`, `virt_to_phys`). Small, reviewable, one-time
  (off the per-frame path).
- **Two-legged fail-closed safety (source-proven, now safety-load-bearing with no IOMMU on the live
  host).** *Leg 1 — which device:* a **PURE decision function** `gate(bdf, config_bytes) -> Allow | Refuse`
  above a confined sysfs-read seam, asserting `bdf ∈` the frozen bench allowlist **and** vendor `0x8086` +
  device `0x10FB` + subsys `0x0003`; **host-tested fail-closed** on wrong-BDF / wrong-vendor / wrong-device
  / wrong-subsys / short-read → `Refuse`, no op attempted (no card needed). Every unbind / config-write /
  `resource0` mmap / DMA-setup routes through the **one choke-point** (no bypass to a state-changing op),
  and the order is **assert-then-act** — identity is read+asserted **before** any state change (BME on the
  wrong device is the dangerous write). The production NIC (`8086:7a12`) is refused by the allowlist **and**
  the identity assert **independently** (complementary); its BDF is denylisted as a third **additive** layer.
- *Leg 2 — where DMA goes:* pagemap-resolved hugepage physaddrs are the **only** legitimate DMA targets;
  each is **validated** (in-hugepage-range, `mlock`ed), not assumed; the descriptor guard (S3) bounds every
  device-written field. A wrong physaddr DMAs to arbitrary host RAM (the owned §2 risk, no IOMMU backstop) —
  so Leg 2 is checked. **Neither leg substitutes for the other**: Leg 1 bounds *which device*, Leg 2 bounds
  *where its DMA writes*.
- **Seam discipline** — retarget the driver's DMA/register backing and the reactor's wakeup primitive, not
  the logic; the surviving crates' tests re-run as the algorithm-freeze proof.
- **Per-frame kernel-free** — poll the descriptor `DD` bit in host memory, write the tail register as a
  CPU store to the mapped BAR; bounded-poll idle-wakeup, the retransmit deadline composed into it.
- **Honest-open** — the no-IOMMU DMA-anywhere posture is the owned residual (`concerns-to-watch.md`), for
  the post-PoC policy audit; nothing here claims containment.

## Acceptance — the gates (team-defined; bench-gated by nature)

1. **The direct path works on the real 82599** — unbind + BME + `resource0` mmap + hugepage/pagemap DMA
   succeed on the bench card; the **fail-closed allowlist is proven** (a non-allowlisted / wrong-identity
   BDF *refuses* the op, shown).
2. **Link up (both ends)** — both bench ends reach a **10G SFI link** over the DAC, minimal write-set +
   the escalation evidence recorded.
3. **Data transmission proven (the goal), non-vacuous + un-fakeable** — a **unique nonce payload** TX'd
   from `41:00.0` is **RX'd on `63:00.0`** over the DAC: RX buffers **pre-cleared + proven-cleared**, RX
   `DD` **set by the real device**, payload **byte-equal across the two separate cards**, through the real
   `copy_out`/`RxDrain` seam, **and the `GPRC` RX counter incremented by exactly the frames sent** (the
   hardware tell — a buffer compare alone can be a software artifact). Observed→expected, self-pinned.
4. **Freeze split — frozen half by empty diff, rewritten half by NEW tests.** The **genuinely-frozen** set
   (driver guard/copy-boundary + `RxRing`/`TxRing`/`RxDrain` traits; reactor `compute_timeout`/
   poll-then-drain/single-sink/malformed-drop/back-pressure) proven by an **empty git-diff** vs its accepted
   baseline + tests re-run. The **rewritten** block/wakeup primitive (→ bounded ring-poll) and the
   `dma.rs` **virt/phys split** proven by **NEW positive tests** — *no eventfd/epoll test counts as evidence
   the new path works* (that is the dead-path trap).
5. **Two-legged safety evidenced (source-proven, not run-observed).** Leg 1: the pure `gate()` allowlist +
   identity assert is host-tested fail-closed, every state-changing op routes through it (assert-then-act);
   the production `61:00.0`/`0x7a12` refused by allowlist **and** identity independently, **demonstrably
   never touched** (write-target BDFs enumerated, primary absent). Leg 2: every DMA physaddr is a
   validated in-hugepage pagemap address (not a virtual pointer, not assumed).
6. **Substrate retired cleanly (salvage-as-move)** — `substrate::reactor` extracted into a surviving home
   (Slice 3), then the S4–5 crate **fully removed**, no surviving crate depending on it (traced) — what
   survives is re-homed code, not a live dependency on the scrapped crate.
7. **Honest-open recorded — TWO residuals, neither discharged by the transmission pass** — (a) the
   no-IOMMU DMA-anywhere posture (owned, policy-audited post-PoC, `architecture.md §2.1`/spine
   known-inconsistent until then); (b) the `dma.rs` crossing-soundness residual (volatile visibility /
   `DD`-read ordering / no-tearing) — a frame crossing intact **once** proves the path *works*, it does
   **not** prove the crossing is *sound*. Both stay open in `concerns-to-watch.md`; containment claimed
   nowhere; production untouched.
8. Independent review re-derives the direct-path module + the retargeting + the **source-proven** safety
   invariant from source, and (evidence-mode per running-host access) re-runs or re-derives the
   transmission evidence from the self-pinning capture and says so; **rejects** any over-claim (a claim of
   containment, a write outside the allowlist, a run-observed safety absence, the surviving logic altered
   under cover of "retarget").

## Review Questions

- **R1 (PM/operator — the residual production-risk; where the FIRST DMA bring-up runs).** The PoC runs as
  **root** doing sysfs unbind + BME + `mmap` + hugepage physical DMA with **no IOMMU** — a DMA/descriptor
  bug is **unconfined** and could corrupt the running host. The operator has **owned** this abstract risk
  (`kernel-bypass-and-dma.md §2`, audited post-PoC). *Revised lean (the reviewer punctured the earlier
  one): do the FIRST DMA bring-up on a **spare/rebootable host**, then move to the production host once
  physaddr-correctness is proven.* The earlier "the fail-closed BDF + pagemap discipline bounds the blast
  radius" is **circular** — the BDF guard bounds *which device* we touch, not *where a wrong physaddr
  writes* (arbitrary host RAM, any BDF); the pagemap-correctness that *would* bound that is the very
  capability being brought up, unproven until Slice 3. So the blast radius is **largest at the first DMA**,
  before its own guard is proven — which is exactly the wrong moment to be next to live production. The
  operator rules the host; this gates the **write** part of Slice 1 (the read-only anchors are zero-risk).
- **R2 (PM/operator — substrate disposition; RESOLVED to a note, one deviation to confirm).** The team
  resolves: **salvage-as-MOVE** — extract `substrate::reactor` (the ratified reactor algorithm + `Host`/
  `RxSource` seams + its 6 tests) into a surviving home (Slice 3), then `git rm` the rest of the substrate
  (Slice 4) — **fully removed**, nothing surviving depending on the crate. **One refinement to the salvage
  map to confirm:** the surviving reactor is the **fresh `substrate::reactor`** (it has the clean pluggable
  `Host::block` seam), **not** the epoll-coupled Sprint-2 `reactor/` crate the map named — retargeting the
  epoll one would be the same Waker surgery the Sprint-4 rewrite already did. Everything else in the
  substrate (ecam/iommu/dmar/lapic/interrupts/heap/calibrate/e1000e/serial/banner/main) is a **full scrap**
  (config is sysfs, `resource0` gives the BAR, no IOMMU); marginal non-load-bearing transfers (the
  hugepage carve math, the bus-master bit constant) at the dev's discretion. Confirm the refinement.
- **R6 (PM/operator — sprint sizing, a decision not a mid-sprint surprise).** This is ambitious: three
  empirical-hardware milestones (link / DMA / transmission) + a real seam-impl change (the virt/phys DMA
  split, not a swap) + the reactor extraction + the crate removal. *Lean: one sprint with a **hard
  checkpoint after Slice 2** (link — the biggest silicon unknown), pre-authorized to **split into 6a
  (module + link) / 6b (retarget + transmission)** if link proves iterative.* Slices 3–4 are largely
  host-testable, so silicon risk concentrates in Slice 2 + Slice 4. Confirm the one-sprint-with-checkpoint
  shape (or pre-split).
- **R3 (bench, dev leads — iterative).** The minimal link-restart form is discovered by escalation on the
  real card (least that links); acceptance is link-up + the minimal set + the escalation evidence. No
  ruling needed unless it escalates.
- **R4 (PM/operator — the transmission bar).** Is "proven data transmission" a **raw frame** loopback
  (`41 → 63`, proving the direct path + link + DMA + copy boundary), or the **full transport (S1) session**
  running over the link? *Lean: raw frame first (it proves bring-up + DMA + link + the copy boundary — the
  hardware milestone), then wire the transport as a follow-on; the transport logic is already built + tested
  and does not need silicon to be correct. Confirm the milestone's bar so Slice 4 targets the right proof.*

---

## Outcome & Retrospectives (Sprint close, 2026-07-17)

**Validated.** All slices independently ACCEPTED by `zero.nic-review`, each source half re-derived at zero
hardware risk and each hardware run-outcome bin-sha hash-bound to the committed source. **Milestone
achieved: proven data transmission `41:00.0 → 63:00.0` across the DAC — pure-direct, no VFIO, no IOMMU, on
the live production host** — a unique nonce frame byte-equal through the real `copy_out`/`RxDrain` seam,
reproduced with fresh nonces, the `RXDGPC 0→1` split separating the write-free TX-first from the RX write,
production `61:00.0` provably untouched. The rulings: **R1** (production host + the three mitigations as
gates + first-DMA on a spare host → re-examined → the hard-stop dropped as disproportionate given the
already-accepted §2 baseline), **gate-2 (i)** (accept the weaker `STRICT_DEVMEM` fallback + TX-first),
**stop-line (ii)** then dropped, **R4** (raw frame first), **R6** (one sprint, no split — link came up on
the minimal form first try). The scrapped substrate (Sprints 4–5) is retired. Two residuals stay
honest-open for the post-PoC policy audit: the no-IOMMU owned-DMA posture is now **exercised** (not
abstract), and the `dma.rs` crossing-soundness is undischarged — *path-works ≠ crossing-sound ≠ contained*.

### Lead retrospective — zero.nic-lead (Sprint 6, 2026-07-17)

**A second architectural pivot mid-effort, conceded cleanly.** The bare-metal frame (Sprints 4–5) was
scrapped for pure-direct userspace on running Linux. The move was to drop the frame, not defend the retired
work — and the **seam discipline contained it a second time**: the surviving Sprint-1–3 std crates came back
to the front, retargeted behind their existing seams, and the whole R1 production-blast-radius fork the
bare-metal boot created simply *dissolved*. The lesson that keeps proving out: build against seams and a
pivot costs a backing swap, not a rewrite.

**Surfacing forks with a lean, and conceding when the lean is punctured.** R1 (where the first DMA runs) I
carried up with a "run on the live host, the BDF+pagemap discipline bounds the blast radius" lean — the
reviewer showed it **circular** (you can't lean on the pagemap correctness you're bringing up to bound the
risk of bringing it up), and I dropped it for a spare-host recommendation. Then the operator re-examined and
I gave an **honest reassessment** — conceding the probability genuinely is low and "especially risky" was
over-framed — while flagging the one thing I know I under-weight (the silent-persistent-consequence tail)
and returning the proportionality call to them. Neither over-defending the accreted caution nor
over-conceding to please; label the instrument, hand them the weight.

**Compile-enforced safety, because convention is not enough at these stakes.** No IOMMU on a live
production host meant the device-scoping had to be a *source invariant*, not a promise: the un-forgeable,
per-BDF-bound `Allowed(Bdf)` witness (the sole state-change choke-point). And the honest-open discipline
held to the end — the reviewer's insistence that residual #7 record the §2 risk as **exercised**, not
softened to "abstract," is exactly the record staying truthful about what the ratified ruling actually did.

**Evidence discipline that made the milestone trustworthy.** Every hardware run bin-sha hash-bound to the
committed source; capture-fallback *mandatory* (the reviewer re-running unconfined DMA would itself have
become the risk the safety bar exists to prevent); and the reviewer held even the celebrated milestone run
to the sprint's own coherence standard (the missing bin-sha pin — closed before commit, not waved through).

**What Sprint 7 inherits (process):** the seam discipline; source-proven-not-run-observed with the
bin-sha-per-hardware-run standard; the honest-open residuals routed to the policy audit; and the
`architecture.md §2.1`/spine reconciliation to the no-IOMMU posture, still owed.

### Developer retrospective — zero.nic-dev (Sprint 6, 2026-07-17)

**Built (across the bare-metal → pure-direct pivot):** Sprints 4–5 built a bare-metal substrate + IOMMU/DMAR
with an induced-fault BLOCKED proof, both then SCRAPPED by the pivot (userspace on running Linux, no IOMMU).
Sprint 6 built the `directpath` crate — the ixy `pci.c`/`memory.c`-shaped control module: the
compile-enforced `Allowed(Bdf)` gate (un-forgeable, BDF-bound, sole choke-point), pagemap Gate 1 + physaddr
Gate 2 (System-RAM range + VA↔PA self-check, `STRICT_DEVMEM` → pagemap-self-consistency fallback), the
reversible PCI ops, 82599 link bring-up (defaults-first), the datapath bring-up over a single
Gate-2-validated 2 MiB hugepage, and the `slice4` milestone. The reactor MOVED out of the scrapped substrate
into its own crate via the `RxDrainAsSource` adapter. Substrate retired.

**Evidence that changed the plan:** the reactor seam mismatch (committed driver `RxDrain` owned-copy-at-driver
vs the fresh reactor's `RxSource` borrow) → surfaced, not forced → the `RxDrainAsSource` adapter with the
liveness catch (a pure-read `has_frame` hangs; needs a live re-pull + two-slot lookahead/front split). The
virt/phys DMA retarget (no IOMMU ⇒ virt≠phys; the committed `dma.rs` DMA'd to the *virtual* pointer = arbitrary
host RAM) → descriptors carry the pagemap-resolved, Gate-2-validated *physical* address. FOUR real-silicon
bring-up bugs below the frozen guard seam (real advanced TX descriptor; `TXDCTL.WTHRESH=0`;
tail `(slot+1)%depth`; RX enable-before-arm) — each diagnosed from a register dump, not guessed. The TX-first
grounding: `TPR` + `RXDGPC` (not `GPRC`, whose DMA-gating wasn't datasheet-provable) is the clean
wire-arrival-vs-host-write split — then empirically confirmed on silicon.

**What Sprint 7 inherits:** the two honest-open residuals (the exercised no-IOMMU posture + the `dma.rs`
crossing-soundness); the single-queue single-frame PoC scale → the reactor-driven bounded-poll `Host` loop
(deliberately not built this sprint — it belonged after the milestone) + steady-state ring management; the
seam discipline (guard byte-frozen, only the backing changed); and the owed `§2.1`/spine reconciliation.

### Reviewer retrospective — zero.nic-review (Sprint 6, 2026-07-17)

**Certified** (re-derived from source + my own host runs; hardware run-outcomes from the dev's self-pinning
captures, flagged): the crossing to the real 82599 on the pure-direct path — a userspace process on the LIVE
production host, no VFIO, no IOMMU — culminating in proven data transmission `41:00.0 → 63:00.0` over the DAC.
Across the slices: the pure fail-closed BDF+identity gate; the compile-enforced `Allowed(Bdf)` token; the
pagemap leg (Gate 1); the reversible non-DMA first hardware contact; 10G SFI link-up (defaults-first,
minimal); the reactor salvage-as-MOVE + adapter; and the byte-equal transmission milestone. The
safety-load-bearing halves were proven in SOURCE at zero hardware risk; I never drove the live host.

**Findings that changed the plan:** compile-enforced device scoping (the un-forgeable per-BDF `Allowed(Bdf)`
witness makes "no state-changing op on the wrong device" bypass-impossible); the `RxDrain`/`RxSource`
seam-contract mismatch → the two-slot adapter (live `has_frame` re-pull + split lookahead/front, reconciling
liveness against borrow-stability); the TX-first vs RX-write distinction + the counters (the first
device-WRITE is the RX per-buffer frame-write; TX-first validates the ring's VA↔PA via DD-completion at
read-only harm but is content-blind; `RXDGPC 0→1` with `TPR` primary is the un-fakeable tell); the
`STRICT_DEVMEM` Gate-2 fidelity gap (the `/dev/mem` ground-truth degraded to a circular fallback → routed up,
operator re-ratified); four below-the-seam real-hardware bugs (the guard/copy-boundary stayed byte-frozen).

**Discipline that held:** source-proven, not run-observed; the evidence anchor degrades on hardware to the
dev's self-pinning capture, so every hardware run was bin-sha hash-bound to the committed source (the one gap
— the milestone capture initially lacking the bin-sha pin — closed before commit). Capture-fallback was
mandatory: re-running unconfined DMA myself would have made me the risk the safety bar exists to prevent.

**Sprint 7's reviewer inherits** the two now-exercised residuals for the post-PoC policy audit — (1) the
no-IOMMU owned-DMA-anywhere posture is exercised (the first live unattended RX-write ran on production);
`architecture.md §2.1`/the spine reconcile to whatever the audit returns; (2) the `dma.rs` crossing-soundness
stays undischarged (visibility/DD-ordering/no-tearing; the run leaned on x86 TSO + cache-coherent DMA without
explicit fences) — neither discharged by the transmission pass. Also inherited: the coherence standard — a
bin-sha-self-pinned capture for every hardware run.
