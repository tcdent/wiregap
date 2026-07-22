# Proxy ↔ NIC/Substrate — Boundary Contract

### The DMA handoff: shape settled, API fill-in pinned (sprint 008 Slice 1)

> **Contract, not spec.** States the trust-face, the settled proxy-side invariants, and the
> requirements the proxy places *on* the seam. The **shape-now half** (trust-face, settled
> invariants, the §3 D-1–D-6 demands) was accepted against settled ground (sprint
> `docs/teams/proxy/003-*.md`). The **fill-in half is now pinned** against the finalized
> `directpath` public API (§4, sprint `docs/teams/proxy/008-*.md` Slice 1): each demand
> mapped to its concrete API form, and the one real gap found (D-3, quiescence) **named and
> routed to the nic team** rather than accepted as satisfied. One currency caveat: the
> finalized API is pinned to a working-tree content anchor, not yet a commit (R-7). Clause
> tagging per `contract/box-proxy.md`; owners: **nic**, **policy**, **proxy**.

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

## 4. Fill-in — the finalized API, pinned (sprint 008 Slice 1) — `CITE(nic: directpath crate)`

The finalized `directpath` public API is pinned and each §3 demand mapped to its concrete
form. The entire public surface is the thin facade — `Subsystem`/`Ctx`/`Event` plus the
re-exported `Bdf`/`SessionId`; every hardware module (`gate`/`pci`/`mem`/`datapath`/…) is
`pub(crate)`, off the surface (D-6 by construction), and the `internals` hatch is
`cfg(feature = "bench")`, compile-absent from the proxy/TCB build. **The API identifiers below
are the nic team's, cited — not asserted or owned by this contract.**

**Pin.** `directpath/src/lib.rs`, content-anchored at sha256
`3c69e5ddca431e8cb48dacfb161d41a1ea8af8b63fb10e2a9bd244d82c17e724` (re-pinned 2026-07-22 when the
NIC team landed the session-continuation surface — the prior anchor `3b9319af…` predates `send(id)`).
*Commit pin still pending (R-7):* the surface remains uncommitted working-tree state (a 440-line diff
against repo HEAD `0ebfaa0`, no new commit), so the content anchor stands and is **re-verified at each
dependent slice** per the recurring-pin discipline — re-pin to the NIC team's commit hash once landed.

The cited surface:
- `Subsystem::construct(local: Bdf, allowlist: &[Bdf], resolve_peer: impl FnOnce([u8;6]) -> [u8;6]) -> io::Result<Self>`
- `Subsystem::open(&mut self, &[u8]) -> SessionId` / `Ctx::open(&mut self, &[u8]) -> SessionId` — originate a **new** session.
- `Subsystem::send(&mut self, SessionId, &[u8]) -> Result<(), SendError>` / `Ctx::send(&mut self, SessionId, &[u8]) -> Result<(), SendError>` — continue an **existing** session.
- `Subsystem::run(&mut self, impl FnMut(&mut Ctx<'_>, Event) -> ControlFlow<()>)`
- `enum Event { Delivered(SessionId, Vec<u8>), Idle }`; `Bdf::parse(&str) -> Option<Bdf>`; `pub use transport::{SessionId, SendError}`.

| Demand | Concrete API form (nic-owned, cited) | Verdict |
|---|---|---|
| **D-1 Submission** | `open(&[u8]) -> SessionId` originates a new session; `send(id, &[u8]) -> Result<(), SendError>` continues an existing one (both copy into a DMA buffer at the send edge — the §1.2 copy-boundary TX half). Visible-on-full: a full TX pool **drops** (not a silent unbounded queue); `send` also surfaces `SendError::WouldBlock` (a frame already in flight, stop-and-wait) / `NotSender` / `NoSuchSession`. **This is the session-continuation the box needs** to put `net-address` (via `open`) and `net-payload` (via `send(sid, …)`) under ONE SessionId — the per-session correlation the proxy already assumes ([[box-proxy-per-session-correlation-open-send]]). | **Satisfied** — with a shipped-surface caveat: the drop *count* (`tx_dropped()`) is `cfg(bench)`-only, so the proxy gets the required drop *behavior* but cannot meter it in production. Noted, not routed (behavior present; only the meter is bench-gated). |
| **D-2 Receipt** | `Event::Delivered(SessionId, Vec<u8>)` — the `Vec<u8>` is the app-owned copy; the copy boundary is held below the seam, the proxy never touches DMA memory | **Satisfied exactly.** |
| **D-3 Completion / quiescence** | `Ctx::is_quiescent()` exists **but is `cfg(bench)`-only**; the shipped surface carries only `Event::Idle` (drained-about-to-park), not a transmit-completion / round-fully-ACKed signal | **GAP — routed to nic (R-8).** `Event::Idle` covers absence-of-data, not "TX fully ACKed." If proxy shutdown/backpressure ever needs a real quiescence signal on the shipped surface, that fill-in is the nic team's. **Named, not accepted as satisfied.** |
| **D-4 Idle wake** | `Event::Idle` + the reactor park inside `run()` — the event-driven `DdWaitHost` MONITORX/MWAITX park on the RX-descriptor DD line: bounded, non-busy when idle, prompt on arrival | **Satisfied** — this is R-6's event-driven resolution: the wake *mechanism* is the nic team's committed code (`ddwaithost`/`mwaitx` @ `74e85e7`), surfaced through the pinned facade. |
| **D-5 Bring-up/serve split** | all privilege in `construct()` (gate → pci unbind/bus-master/BAR mmap → hugepage/pagemap → link → rings); `run()` is zero-syscall steady-state (mapped-BAR MMIO + MWAITX park + unprivileged clock) | **Satisfied** — matches §2 / architecture L15, nic code-verified. |
| **D-6 Nothing leaks up** | the only upward types are `SessionId`, `Vec<u8>`, `&[u8]`; no descriptor, register, ring index, or device pointer; hardware modules are `pub(crate)`, `internals` is `cfg(bench)` | **Satisfied.** |

Net: four demands satisfied cleanly; two (D-1, D-3) satisfied *in shape* but with a
shipped-surface observability caveat because the concrete signal is `cfg(bench)`-gated. Only
D-3 rises to a routed gap (R-8); D-1's meter is noted, not routed.

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
- **R-4 — Preliminary-API naming flux — DISCHARGED (sprint 008 Slice 1).** Scoping used the
  preliminary shape and asserted no identifier; the fill-in gate has now run (§4) and
  re-derived every concrete claim from the finalized API. The flux is closed — the identifiers
  in §4 are the real ones, cited to the nic crate. (Superseded by R-7's remaining currency
  caveat: the API is pinned by content, not yet by commit.)
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
- **R-7 — API pinned by content, not yet by commit (currency).** The finalized `directpath`
  facade (§4) lives in the working tree, not a committed tree. It now includes the landed
  session-continuation surface (`open` + `send(id, …)`, ruled 2026-07-21, landed 2026-07-22) —
  a 440-line working-tree diff against repo HEAD `0ebfaa0`, still **no new commit**. §4 is pinned
  to the current sha256 content anchor (`3c69e5dd…`, re-pinned 2026-07-22) as the interim, exactly
  as the currency discipline prescribes for a stable-but-uncommitted source, and the anchor is
  **re-verified at each dependent slice** (the recurring-pin discipline). The exact-commit pin is
  **still owed once the NIC team commits the surface**; the proxy re-pins §4 to that hash then.
  Bounded: the API *content* is landed and accepted, so every §4 claim holds; only the commit
  anchor is pending. (This residual has now absorbed two re-pins — the original facade and the
  `send(id)` addition — each a content-anchor bump, not a new residual.)
- **R-8 — D-3 quiescence gap on the shipped surface.** The only completion/quiescence signal
  (`Ctx::is_quiescent`, and `Subsystem::tx_dropped`) is `cfg(bench)`-only; the shipped surface
  carries just `Event::Idle` (drained-about-to-park), which is not a transmit-completion /
  round-fully-ACKed signal. D-3 is therefore **not** satisfied on the shipped surface. Routed to
  **nic**: if the proxy's shutdown/backpressure logic ever needs a real quiescence signal, that
  fill-in is theirs to add. Named here, not silently accepted as satisfied.

## 7. Currency pins (2026-07-22)

`directpath/src/lib.rs` (the finalized public API §4 pins, now incl. `open`+`send(id)`) @ working-tree
sha256 `3c69e5ddca431e8cb48dacfb161d41a1ea8af8b63fb10e2a9bd244d82c17e724` — **content-anchored,
commit pin owed** (R-7: a 440-line working-tree diff against repo HEAD `0ebfaa0`, no new commit;
re-verify this hash at each dependent slice). `docs/nic/architecture.md` @ `6580d82` (§2.1/§3.1/§5,
§8); `docs/proxy/architecture.md` @ `dec2453` (L8/L11/L15 rows); `docs/proxy/terminal.md` @ `da6c63c`.
