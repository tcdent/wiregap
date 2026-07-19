# Proxy ↔ NIC/Substrate — Boundary Contract

### The DMA handoff: shape now, API fill-in deferred

> **Contract, not spec — SHAPE-NOW HALF ONLY.** States the trust-face, the settled
> proxy-side invariants, and the requirements the proxy places *on* the seam — as demands
> on an interface, **never asserting a NIC API specific as fact** (the NIC team's public
> API is finalizing; identifiers may shift in their legibility pass). The **fill-in half
> is deferred**: descriptor layout, register semantics, and concrete API calls land only
> once the finalized API is pinned to an exact commit (sprint `docs/teams/proxy/003-*.md`,
> Slice 3). Status: **awaiting the shape-now variant gate** (GB1–GB6 against settled
> ground only). Clause tagging per `contract/box-proxy.md`; owners: **nic**, **policy**,
> **proxy**.

---

## 0. Trust-face (GB5) — `CITE(proxy: architecture.md §3 tier 4, terminal.md §5; nic: nic/architecture.md §2.1)`

**Neither a gate nor a plain relay: a treat-as-hostile handoff.** Below the seam sits the
X520 and its firmware — bulky vendor code, untrusted, its hostile reach split per vector:
the *content* vector **contained** by the small-and-ours driver, the *DMA-reach* vector
**owned, not contained** (pure-direct, no IOMMU). **The safe surface begins at the
treat-as-hostile read of the DMA handoff**: the driver reads descriptor/buffer content as
if from an adversary, and everything above that read is small-and-ours or a design
property. The proxy's side of this boundary is *upward* of that read — it never touches
descriptors, rings, registers, or device-addressable memory at all.

## 1. Settled invariants at the seam — `CITE(nic: nic/architecture.md §2.1, §3.1)`

1. **Content-vector guard** (the live vector-2 containment): bound the reported length,
   treat status as hostile, never dereference an unchecked field, drop-and-refill.
2. **The copy boundary**: every frame is copied out of the DMA region into
   application-owned memory at the earliest possible point; every event carries that copy,
   never a ring pointer — "device-writable memory never flows past the reactor." The TX
   side mirrors it: the transport copies *into* a DMA buffer at the send edge; the DMA
   region is touched only by the transport, only at the two edges.
3. **Fail-closed device identity**: a compile-enforced gate before any state-changing
   device operation — no path to unbind/bus-master/mmap/DMA a device not allowlisted.
4. **DMA-reach is owned** — nothing hardware-bounds where the device can DMA; whether the
   threat model tolerates that residual is the policy layer's call, audited after the PoC
   (`CITE(proxy: architecture.md ledger L8; policy)`).
5. **Kernel standing**: the link bypasses the kernel; where kernel enforcement appears
   elsewhere in the proxy it is ledger L11's question — this boundary adds no new kernel
   dependency (`CITE(proxy: architecture.md ledger L11)`).

## 2. Privilege shape — settled — `CITE(proxy: architecture.md ledger L15, resolved; nic: code-verified)`

Spend privilege at bring-up, drop before box bytes: **all privileged operations happen in
construction** (unbind, BAR mmap, DMA hugepage setup, bus-master enable, link bring-up,
ring setup), and **steady-state serving is unprivileged and zero-syscall** (mapped-BAR
MMIO + event park + unprivileged clock; mappings persist across the uid drop; the device
DMAs independently). The proxy drops to an unprivileged uid between construction and
serving — least privilege by construction, in-process. Code-verified by the NIC team
(L15, resolved 2026-07-18). *Residual R-1 below carries the one caveat.*

## 3. Requirements the proxy places on the seam — `DEFINE` (demands, not API facts)

The proxy requires of the seam, in whatever concrete form the finalized API takes:

- **D-1 Submission.** A way to submit an outbound frame with defined memory ownership at
  the call (copy-in at the edge or completion-bounded hold — never app-held
  device-addressable memory), and **visible** behavior on a full ring (drop/deny is
  acceptable; silent loss into an unbounded queue is not).
- **D-2 Receipt.** Inbound frames delivered as **app-owned copies** (the §1.2 copy
  boundary held below the seam) — the proxy never sees DMA memory.
- **D-3 Completion/quiescence.** A way to observe transmit completion or quiescence, so
  shutdown and backpressure decisions are made from state, not assumption.
- **D-4 Idle wake.** An idle-wake that is **bounded and non-busy when idle, prompt on
  work arrival** — no proxy-side busy-poll for idle links (busy-drain is the *active*
  path only). Mechanism-neutral by intent: the committed doc's **bounded poll** ("re-polls
  after a capped short sleep rather than parking on an interrupt," reconciled from MSI-X
  — `CITE(nic: nic/architecture.md §5)`) satisfies it, and so does the event-driven
  DD-monitor wake the nic team's later artifacts describe (see R-6 — an internal
  divergence in the nic team's sources, routed, not adjudicated here).
- **D-5 Bring-up/serve split.** Construction performs all privileged work; the serving
  surface requires none (§2) — the seam must not reintroduce a privileged call into
  steady-state.
- **D-6 Nothing below the seam leaks up.** No descriptor, register, ring index, or
  device-addressable pointer appears in the seam's upward types; link-state and errors
  surface as events/status, not as raw device state.

These demands are satisfied *in shape* by the committed subsystem architecture as it
stands (`nic/architecture.md` §3; §5 per D-4's mechanism-neutral reading; the committed
transport/reactor seam traits) — the fill-in half cites the finalized API's concrete
form for each, by name, once pinned.

## 4. Fill-in — deferred dependencies (named, not asserted)

Awaiting the finalized NIC public API (exact commit pinned at the fill-in gate's start):
the concrete construction/serve/send entry points and their types; the seam's error and
link-down surfacing; buffer-ownership signatures; the wake mechanism's surfaced form;
any configuration the proxy passes at construction. Until then, **no clause in this
contract names an API identifier as fact** (the preliminary shape exists and was used
for scoping only, per the sprint contract).

## 5. Judgment calls (GB6) — none new

The demands in §3 restate the committed architecture's own posture as seam requirements;
no point of genuine choice is settled here. The one open policy question this boundary
carries (DMA-reach tolerance, L8) is already routed to policy; the one open recovery
question is R-1, already ledgered.

## 6. Residuals — named and bounded

- **R-1 — No device-reset recovery path exists today.** If steady-state recovery is ever
  added, a device reset's bus-master re-enable is the one privileged operation, isolated
  to a small helper then — a faithful restatement of `architecture.md` ledger L15's
  caveat (`CITE(proxy)`), not invented into this contract.
- **R-2 — DMA-reach owned** (§1.4): the residual is policy's to weigh (L8), audited
  after the PoC; this contract states it, never narrows it.
- **R-3 — Crossing soundness is the nic team's own tracked ground** (the
  device-memory access/ordering account, `nic: grounding/crossing-soundness.md`, verified
  Sprint 7b with a platform-scoped residual). Below the seam; cited as theirs, not
  restated.
- **R-4 — Preliminary-API naming flux**: scoping used the preliminary shape
  (per the sprint contract) but no identifier is asserted; the fill-in gate re-derives
  every concrete claim from the pinned final API.
- **R-5 — Bench-pin inheritance**: the NIC subsystem's own bench pins (elsenor) carry
  their deploy-host re-verification duty; this contract adds no new pin.
- **R-6 — Idle-wakeup divergence inside the nic team's own sources** — **RECONCILED,
  pending the nic team's own commit** (2026-07-18). Surfaced here, routed to **nic** via
  the designer, adjudicated by neither this contract nor the proxy team. `nic/architecture.md`
  §5 (as of `6580d82`) stated the idle-wakeup as a bounded poll, contradicting the
  event-driven MONITORX/MWAITX wake the nic team's committed code (and commit `74e85e7`)
  actually implements. `zero.nic-design` confirmed the diagnosis — committed code is
  ground truth; the doc body was a stale/incomplete edit, not a revert — and found the
  drift wider than §5 alone (also §3/§7/§9 and `data-path-and-events.md` §4, which had
  never been reconciled through the pure-direct pivot). The fix is written and awaiting
  the nic team's own commit gate. Bound (unaffected by the outcome): D-4 was
  mechanism-neutral throughout — both resolutions satisfy the demand — and this
  contract's §2 cites the committed proxy-side L15 row faithfully as its own source, so
  no clause here changed meaning while this was open.

## 7. Currency pins (2026-07-18)

`docs/nic/architecture.md` @ `6580d82` (§2.1/§3.1/§5, §8 read in full this session); `docs/proxy/architecture.md` @ `dec2453` (L8/L11/L15 rows);
`docs/proxy/terminal.md` @ `da6c63c`. The NIC public API is deliberately **unpinned** —
that pin is the fill-in gate's opening act, not this half's.
