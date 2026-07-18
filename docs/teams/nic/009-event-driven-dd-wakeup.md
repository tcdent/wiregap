# Sprint 009 (8) — Event-Driven Wakeup on the RX Descriptor DD Line (`UMWAIT`-on-DD, no MSI-X)

> The `nic` team's ninth sprint (the PM's **Sprint 8**). The subsystem is operational, measured, and
> crossing-sound (7a `33281e8`, 7b `71b35c7`), and the §7 busy-drain restored the work-present margin
> (`pump_all`). What remains from `grounding/kernel-bypass-and-dma.md §5` item 2 is the **idle** half: the
> bounded-poll `PollHost` waits by **sleeping a fixed poll interval** (`sleep(50µs)`≈112 µs observed) — the
> ~50–260 µs idle-wakeup latency the 7a §7 finding surfaced. This sprint makes the idle-wait **event-driven,
> with no kernel interrupt bridge and no MSI-X programming at all**: the NIC already writes the RX
> descriptor's **`DD` status** to our cache-coherent memory on **every frame** (`grounding/kernel-bypass-and-dma.md §4`:
> "the RX descriptor `DD` bit … is NIC-emitted event state we read directly — the same principle"), so we
> **`UMWAIT`/`UMONITOR` the next descriptor's `DD` cache line directly**. The NIC's `DD` writeback wakes the
> idle CPU — the same coherent device posted-write an MSI-X message would have been, but reusing the exact
> path already **proven on silicon in 7b**. The reactor becomes what it was always built to be
> (`reactor.rs:17` — "MSI-X … confined entirely in the `Host` impl"): **busy-drain while there is work (the
> §7 `pump_all`, unchanged), block on the `DD` line when idle.**
>
> **The simplicity cut (designer-ruled, 2026-07-18):** the earlier plan programmed MSI-X to write a
> dedicated doorbell word; the designer's simplicity review found that **redundant** — `UMWAIT`-on-`DD` gets
> the identical wake with **no MSI-X plumbing**. The dev grounded it against the 82599 datasheet (rev 3.5):
> `DD`-writeback and an MSI-X write are **datasheet-equivalent on the wake axis** — §3.1.4.5.3 item 3
> verbatim, *"The 82599 cannot relax ordering for descriptor writes or an MSI write"* (the same two
> un-relaxable classes); with `CTRL_EXT.NS_DIS=1` (already required for RX) the `DD` write is snooped, so
> `UMONITOR` fires on it via the same coherency — and the working `DD`-poll path proves it coherent today.
> **No hard blocker → the cut is taken.** It deletes the sprint's largest risk surface (the novel,
> un-referenced MSI-X delivery — `reference/ixy` delivers MSI-X only via VFIO/eventfd, so it was
> datasheet-only) and collapses the sprint to **two slices**.
>
> **Commit to the event-driven wait — no fallbacks (operator directive, 2026-07-18, load-bearing):**
>
> > "eliminate all polling — no fallback tight-poll path, no defensive dead code. Commit to the event-driven
> > doorbell wait. If the efficient wait primitive is unavailable on this hardware, that is a blocker to
> > surface, not a code path to maintain. Build it clean."
>
> So the `UMWAIT`-on-`DD` wait is the **single committed wait path**. Whether a **device** posted-write (the
> `DD` writeback) **reliably wakes** `UMWAIT` on the bench CPU is the sprint's **make-or-break dependency**,
> proven **standalone first** (Slice 1). If it does not prove out, that is a **blocker surfaced to the PM** —
> **not** a tight-poll fallback, a bounded-poll retention, or any maintained code path. The lost-wakeup
> **bound** is **self-owned**: `block(None)` always passes `UMWAIT` an **explicit finite deadline** (our
> re-poll bound), never relying on `IA32_UMWAIT_CONTROL` (which can be 0 = unbounded → a missed store would
> hang). With the `MONITOR`-recheck idiom this is a bounded, CPU-*idle* wait — **not** busy-polling.
>
> **Platform note (grounded at Slice 1, 2026-07-18):** the body below names the primitive Intel-centrically
> as `UMWAIT`/`UMONITOR` (WAITPKG) — that was the lead's framing before the bench CPU was known. The bench
> (`elsenor`, AMD EPYC 7551P / Zen 1) has **no WAITPKG** (`UMWAIT` `#UD`s), but **does** have AMD's
> user-mode monitor-wait, **`MONITORX`/`MWAITX`** (`CPUID Fn8000_0001_ECX[29]=1`, opcodes execute) — the
> primitive we build on. Read every `UMWAIT`/`UMONITOR` below as `MWAITX`/`MONITORX`. **Two simplifications
> follow:** (1) `MWAITX` takes a **native per-call TSC timeout in `EBX`**, so the self-owned bound is the
> deadline passed directly — this **retires** the `IA32_UMWAIT_CONTROL` / re-entry-loop / global-MSR
> discussion above (kept for the record; the mechanism is simpler); (2) `MWAITX` has **no wake-reason flag**,
> so the "`CF==0` = woke-on-write-not-timeout" instrument translates to **TSC-delta ≪ (generous) `EBX`
> timeout** + `DD`=1 on the awaited descriptor + `RX-counter++` (arguably a more direct read on the device
> write). The full substitution + rationale land in the sprint postmortem; the DD-monitor design, the bar,
> the pins, and no-fallback are all unchanged.
>
> Team: `nic` (lead `zero.nic-lead`, dev `zero.nic-dev`, review `zero.nic-review`, pm `zero.nic-pm`,
> designer `zero.nic-design`). Authority: [`grounding/kernel-bypass-and-dma.md`](../../nic/grounding/kernel-bypass-and-dma.md)
> (§4 the event model — **the design-of-record for this sprint**, updated to the DD-monitor cut, §5 the open
> items), [`architecture.md`](../../nic/architecture.md) (§3/§5 reactor/events), [`grounding/crossing-soundness.md`](../../nic/grounding/crossing-soundness.md)
> (the 7b device-write-to-our-memory ordering account — the `DD` write is exactly that crossing). Builds on
> 7b (`71b35c7`); the operational stack (`reactor/` core, `transport/`, `driver/` guard/copy-boundary) is
> **frozen** — this sprint touches only a new `Host` impl and its descriptor-line arming. The two 7b
> residuals (owned-DMA policy audit; the platform-scoped crossing residual) stay open, untouched.

---

## Scope Boundary

**This sprint owns:**

1. **The device-wake coherency on the `DD` line — the make-or-break, proven standalone (before
   integration):** on the bench CPU, `UMONITOR` the next RX descriptor's `DD` cache line and `UMWAIT`; a
   real device RX frame's `DD` writeback wakes it; prove by **named instruments** (below) that the **device
   write** woke the **idle** CPU. If it does not reliably wake, that is a **blocker surfaced to the PM** —
   the sprint stops; there is no fallback.
2. **The event-driven `Host` (single wait path):** a new `Host` impl (working name `DdWaitHost`) whose
   `block(timeout)` **`UMWAIT`s on the next descriptor's `DD` line** (→ `WakeReason::NicMsix`) or the
   deadline (→ `WakeReason::Timer`) — the **only** wait, replacing `PollHost` (which is **retired**). Its
   contract, load-bearing: **check-before-wait** (`MONITOR`-recheck-`UMWAIT`: read `DD`, arm `UMONITOR` on
   its line, re-check, then `UMWAIT` — closing the "wake that arrived just before the call" window) and a
   **self-owned bounded** wait (`block(None)` passes an explicit finite re-poll deadline, MSR-independent).

**This sprint does NOT own / explicitly deletes:**

- **No MSI-X, no doorbell.** We program **no** MSI-X table, **no** config-space MSI-X capability, **no**
  `IVAR`/`GPIE`/`EIMS`/`EICR` cause-arming, and allocate **no** doorbell word — the whole subsystem the
  designer cut. We monitor the `DD` bit the NIC already writes. No IOMMU/VFIO/APIC/IDT (per §4).
- The **reactor algorithm** — `step` / `park` / `pump_all` / `arm_from_deadline` / `can_pull_rx` stay
  **byte-identical** (git-verified) — **and the seam contract itself**: the `Host` trait signature + the
  `WakeReason` enum. A new `Host` method, a changed `block()` signature, or a new `WakeReason` variant **is
  the seam changing** — a red flag, not a backing change. `WakeReason::NicMsix` already exists (its name is
  a historical label for "the NIC's RX wakeup"; the wake is now the `DD` write, not an MSI-X message —
  keeping the variant keeps the seam frozen).
- The **guard / copy-boundary / transport** — empty-diff.
- **No fallbacks, no defensive dead code, no dual wait paths, no polling** (operator directive). `UMWAIT`
  unavailable/incoherent = a **surfaced blocker**, not a code path. **`PollHost` is retired** — it is the
  polling being eliminated.
- The **owned-DMA policy audit** and the **`architecture.md §2.1`/spine reconciliation** — 7b residual.
- Production `61:00.0` — never touched, under any circumstance (bench cards `41`/`63` only).

## Problem

`directpath/src/pollhost.rs` implements `Host::block` as `std::thread::sleep(poll_interval)`, and its own
header still carries the **wrong** "There is no interrupt delivery on the pure-direct path" assumption that
`grounding/kernel-bypass-and-dma.md §4` corrects. On idle, the reactor sleeps a fixed interval rather than
waiting on the NIC's event; the sleep granularity is the ~50–260 µs idle-wakeup latency behind the 7a §7
margin finding. The NIC already writes the RX descriptor `DD` status to our cache-coherent memory on every
frame — a coherent device posted-write we could wait on directly — but nothing does; the idle path only
sleeps.

## Goal

The reactor's idle-wait is **event-driven on the NIC's own `DD` writeback**, with **no kernel interrupt
bridge and no MSI-X**: it busy-drains while there is work (§7 `pump_all`, unchanged) and **`UMWAIT`s on the
next descriptor's `DD` line** when idle, unblocking on the NIC's `DD` write or the retransmit deadline. The
`UMWAIT`-on-`DD` wait is the **single wait path**, bench-proven by named instruments as the make-or-break
dependency (proven first; if it does not prove out, a **surfaced blocker**, never a fallback). This is an
**efficiency + architectural-intent** goal (the reactor becoming what §4/§5 describe), **not** a §7
correctness fix — `pump_all` already discharged §7's work-present margin. The reactor core + seam contract
are untouched: the whole change is one new `Host` impl.

## Final State

- No MSI-X is programmed and no doorbell exists; the idle wait is `UMWAIT`/`UMONITOR` on the next RX
  descriptor's `DD` cache line.
- On each RX frame, the NIC's `DD` writeback wakes the idle `UMWAIT`; the `DD` write is proven to wake the
  monitor reliably on the bench CPU (Slice 1, by the named instruments), across steady-state and ring-wrap.
- `DdWaitHost` (the single `UMWAIT`-on-`DD` wait path) replaces `PollHost`; `block` is **check-before-wait**
  and **bounded by a self-owned explicit deadline** (`block(None)` passes a finite re-poll deadline, not the
  `IA32_UMWAIT_CONTROL` default); the retransmit-deadline (`Timer`) path still fires; the wait is
  event-driven `UMWAIT`, **never a fixed sleep and never a poll**.
- Reactor core **and seam contract** (`Host` trait + `WakeReason`) git-byte-identical; guard/copy/transport
  empty-diff; `PollHost` **retired** (no dual paths, no dead code), after the DD-monitor path is accepted on
  silicon.
- Production `61:00.0` never touched (the `Allowed(Bdf)` gate + the `8086:10FB`/subsys-`0003` identity
  assert; deny `7a12`).

## In Scope (ordered slices)

- **Slice 1 — the `DD`-line device-wake coherency, proven standalone (the make-or-break, before
  integration).** **First early-stop:** `CPUID.07H.0:ECX[5]` (WAITPKG) must be present — absent = `UMWAIT`
  is `#UD` = an immediate hard blocker (no fallback), check it *before* anything. Then, on the bench CPU,
  `UMONITOR` the next RX descriptor's `DD` cache line, **re-check** `DD`, then `UMWAIT` with an **absolute
  TSC deadline** (`EDX:EAX` is absolute — convert a `Duration` via `rdtsc + dur*tsc_freq`, pin
  **invariant-TSC**, and verify a known-duration wait in the harness so a units-slip can't silently make
  every wait instant-return or spin-to-deadline); a real device RX frame (nonce'd) writes `DD`. *Method —
  the un-fakeable proof is instrumented, not narrated:* **(1) truly idle** — the active-cycle counter
  (`APERF`, or a C-state residency MSR) near-zero across the wait (a spin burns `APERF`≈wall-clock TSC;
  `UMWAIT` idles, `APERF`≪TSC — that delta *is* the idle proof); **(2) woke on the `DD` write, not the
  timeout** — `UMWAIT` `CF==0` **and** `DD==1` on the awaited descriptor **and** `RX-counter++`
  (`GPRC`/`RXDGPC`), with a spurious-wake re-check; **(3) reliably** — repeated across many RX events. The
  **source property** closes attribution: our code **never writes `DD`=1** (refill writes only the
  read-format descriptor — buffer address, `DD`=0 — grep-verifiable), and the single-threaded reactor arms
  *after* refill (§Method), so the only writer of the monitored line during the wait is the NIC. The
  **"repeated"** instrument separates **reliable** wake (pass) from **intermittent** (wakes sometimes). **If
  it does not wake at all → STOP, surface a blocker to the PM.** If it wakes only **intermittently**, the
  frame is never lost (bounded latency via the `step` re-poll) but that is a **latency finding to surface**,
  not a silent pass. Runs directly against the `DD` writeback on the working RX infra — earlier + cheaper
  than the deleted MSI-X path.
- **Slice 2 — the event-driven `DdWaitHost` wired into the reactor; `PollHost` retired.** Build
  `DdWaitHost::block` on the proven `UMWAIT`-on-`DD` primitive — **check-before-wait**
  (`MONITOR`-recheck-`UMWAIT`, reading `DD` first), **self-owned bounded** (`block(None)` passes an explicit
  finite re-poll deadline). Because the OS max-wait (~tens of µs) is far shorter than the ~10ms retransmit
  deadline, `block(deadline)` is a **loop of low-power `UMWAIT`s** — CPU idle between (C0.1/0.2, `APERF`
  frozen — non-polling by the same Slice-1 instrument), each waking to **re-check `DD` + compare `rdtsc` to
  the real deadline**; **`CF` alone must NOT map to `Timer`** (`CF` fires at every OS-max-wait boundary, not
  the real deadline — a naive `CF=1`→`Timer` retransmits ~100× early): the recheck+clock-compare is
  authoritative (`DD` set → `NicMsix`; real deadline reached → `Timer`). Wire it into the live drive loop in
  place of `PollHost`, and **retire `PollHost`** (no dual paths, no dead code). *Method:* end-to-end
  round-trip on the bench, event-driven; the idle wait is `UMWAIT` (not a sleep, not a poll); the retransmit
  deadline still fires; **re-arm exercised across steady-state + ring-wrap** (not a single-frame PoC — the
  moving-target/wrap/re-arm surface is the RX-RDT-re-arm-bug zone); reactor core **+ seam contract**
  git-byte-identical; guard/copy/transport empty-diff; no dangling `PollHost` import. Measure idle-wakeup
  latency / RTT vs the prior bounded-poll baseline (~50–260 µs); report the number (efficiency +
  architectural-intent, **not** a §7 fix).

## Non-Goals

- **No MSI-X, no doorbell, no cause-arming** (deleted by the designer's cut) — and no IOMMU/VFIO/APIC/IDT.
- Do not touch the reactor core **or the seam contract** — the `Host` trait signature + the `WakeReason`
  enum are frozen. A new method / changed `block()` sig / new variant is the seam changing.
- Do not touch the guard, the copy-boundary, or the transport.
- Do not **assume** `UMWAIT` works — prove it by the named instruments (Slice 1), early and standalone.
- **No fallbacks, no defensive dead code, no dual wait paths, no polling** (operator directive). The
  `UMWAIT`-on-`DD` wait is the **only** wait. If `UMWAIT` (or the `DD`-write wake) does not prove out, that
  is a **blocker surfaced to the PM**, not a code path to build or maintain.
- **Retire `PollHost`** — it is the polling being eliminated (after the DD-monitor path is accepted on
  silicon; until then it is the running baseline, not a kept fallback).
- Do not enable RSC (`RSCCTL.RSCEN` stays 0) — it is the only RX write-back coalescer; off = prompt
  per-frame `DD` writeback.
- Do not touch production `61:00.0` — bench cards `41:00.0`/`63:00.0` only.

## Starting Surface

Audit first, produce the inventory before editing. Inventory shape:
`item -> location -> derivation-source {datasheet-fixed | pagemap-runtime | our-constant | reactor-cursor} -> value/behavior -> check`.

- **The RX descriptor ring (`directpath/src/datapath.rs` / `dma.rs`)** — the advanced write-back
  descriptor layout: `DD` is bit 0 of the status Dword at **byte 8** of the 16-byte descriptor (datasheet
  Table 7-16); **4 descriptors share a 64-byte cache line** (the non-isolatable false-share, benign). The
  **refill path** — confirm it writes only the read-format descriptor (buffer address, `DD`=0) and that all
  refills happen in `pump_all` **before** `block()` arms (the arm-after-refill pin). The **cursor** — the
  next-expected descriptor index `block()` must arm `UMONITOR` on each call, across ring-wrap and the
  same-line-as-just-consumed edge.
- **`CTRL_EXT.NS_DIS`** — must be 1 for RX (already required); confirm it is set (snooped RX writes =
  `UMONITOR` coherency). **`RSCCTL.RSCEN`** — must stay 0.
- **`directpath/src/pollhost.rs`** — the `Host` impl being replaced (and its stale "no interrupt delivery"
  header, corrected); retired at Slice 2.
- **`reactor/src/reactor.rs`** — the frozen seam (`Host` trait + `WakeReason` + `block`); confirm
  byte-identical before/after.

## Method (the discipline every slice follows)

- **Shadow on the live path** (bench cards `41`/`63` via the `Allowed(Bdf)` gate + identity assert);
  production `61:00.0` never touched.
- **Coherency is reused, not novel:** the `DD` writeback waking `UMONITOR` is the **same coherent device
  posted-write proven on silicon in 7b** and exercised by the working `DD`-poll path today — cite datasheet
  §3.1.4.5.3 item 3 ("cannot relax ordering for descriptor writes") + `NS_DIS=1` snooping. The only novel
  axis is the **wait** (does the CPU's `UMWAIT` wake on that write) — which is exactly Slice 1.
- **The three DD-monitor pins (correctness):** **RSCEN=0** (no WB coalescing → prompt DD); **arm-after-
  refill** (single-threaded reactor refills before `block()` arms → the only writer of the monitored line
  during the wait is the NIC, no self-wake); **arm the next-expected descriptor line** (at idle the reactor
  cursor equals the device's next write target — a wrong-line arm degrades to latency via the `step`
  re-poll, never loss). **Bench-test re-arm across steady-state + ring-wrap**, not a single frame.
- **The un-fakeable tell (Slice 1):** **source property** — our code never writes `DD`=1 (refill writes
  read-format only, grep-verified) — plus the `DD`-line store woke the *idle* CPU (`APERF`-idle + `CF=0`) +
  `DD`==1 + `RX-counter++` + byte-equal nonce, repeated. A wake the CPU could have caused itself, or a `DD`
  our own store could set, proves nothing.
- **The `DdWaitHost` contract (lost-wakeup safety, no poll):** **check-before-wait** (read `DD` before
  arming; `MONITOR`-recheck-`UMWAIT`) closes the just-arrived window; the **bound is self-owned** —
  `block(None)` passes an **explicit finite deadline**, not the `IA32_UMWAIT_CONTROL` MSR — a CPU-*idle*
  bound, not a poll, so a truly-missed wake degrades to *latency* via the `step` re-poll, never a hang.
- **Build/log coherence:** every hardware run is **bin-sha bound**; the reviewer re-derives binary
  coherence by clean rebuild (hash-match), never trusts the report's build. Each run emits a log signature
  only the new build can produce (per-frame nonces + the `APERF`/`CF` capture).
- **Seam freeze:** the reactor core (`step`/`park`/`pump_all`/`arm_from_deadline`/`can_pull_rx`) **+ the
  seam contract** (`Host` trait + `WakeReason`) + guard/copy/transport stay git-byte-identical; the change
  is backing below the `Host` seam.
- **Capture-fallback:** the reviewer never re-runs unconfined DMA on the live host; it takes the run-outcome
  from the dev's self-pinned, bin-sha-bound capture and re-derives binary coherence by rebuild
  (`silicon-evidence-anchor-degrades`).
- **Clean settled run before switching:** a DD-monitor-driven round-trip must settle (no lost wakeup, no
  spurious retransmit, re-arm holds across ring-wrap) before `PollHost` is retired.

## Acceptance (the gates that close the sprint)

- **Device-wake coherency — the make-or-break (Slice 1):** WAITPKG present (else immediate blocker);
  `UMWAIT`-on-`DD` validated by the three instruments — `APERF`/C-state near-zero (idle), `CF==0` (woke on
  the `DD` write, not the timeout), `DD`+`RX-counter` attribution, repeated across many RX events;
  known-duration wait verified (TSC units). **Pass/blocker:** **no wake at all** → **blocked** (surfaced to
  the PM), not routed to a fallback; **intermittent** wake → frame never lost (bounded latency via the
  `step` re-poll) but a **latency finding surfaced**, not a silent pass.
- **Event-driven reactor (Slice 2):** end-to-end round-trip on the bench, event-driven; idle `block` is the
  `UMWAIT`-on-`DD` wait (check-before-wait + self-owned explicit-deadline bound, MSR-independent), unblocks
  on the `DD` write — **never a sleep, never a poll**; the retransmit deadline (`Timer`) fires; re-arm
  exercised across **steady-state + ring-wrap**; reactor core **+ seam contract** git-byte-identical;
  guard/copy/transport empty-diff; `PollHost` **retired**, no dangling import; bin-sha hash-match. Idle-
  wakeup latency / RTT measured vs the ~50–260 µs baseline; the number reported (a measurement, not a
  pass/fail scalar).
- **Production `61:00.0` never touched** (gate + identity assert evidence).

## Review Questions (for the reviewer + the designer/human)

1. **Device-wake coherency (the make-or-break):** does the **`DD` writeback** (a device posted-write)
   reliably trigger the `UMONITOR`/`UMWAIT` wake on the bench CPU? Proven early + standalone (Slice 1) by
   the named instruments. Per the operator directive there is **no fallback** — if it does not prove out,
   it is a **blocker surfaced to the PM**, and the open question is how to proceed (different CPU / wait
   primitive / re-scope), not which fallback to build.
2. **False-share / re-arm:** the `DD` line is non-isolatable (4 descriptors/64-byte line). Confirm the
   adjacent-descriptor wake is benign (load-only, where we busy-drain not block) and the arm-after-refill
   pin holds under the single-threaded reactor — and that re-arm across ring-wrap is exercised, not a
   single-frame PoC (the RX-RDT-re-arm-bug zone).
3. **Lost-wakeup:** with check-before-wait (`MONITOR`-recheck) + a self-owned explicit-deadline bound + the
   `step` re-poll backstop, is the `Host::block` "must not lose a wakeup" contract met, with no poll?
4. **`DD` crossing ordering:** the `DD` read gates the payload (the 7b account already covers this
   acquire-ordered read); the *wait* adds no new dependent read — confirm no new ordering obligation beyond
   7b (the `UMWAIT` wake is a liveness hint; the `DD` acquire-load is the correctness gate).
5. **§7 / architectural framing:** confirm this sprint is efficiency + architectural-intent (the reactor
   becoming §4/§5's event-driven shape), **not** a §7 correctness fix — `pump_all` already discharged §7's
   work-present margin; here we only *measure* the idle-latency improvement.

---

## Retrospectives (Sprint Close)

*Appended at close. The contract above is written as-planned; these record what actually happened. The
sprint traveled: MSI-X doorbell plan (4 slices) → the designer's DD-monitor simplicity cut (2 slices) →
a WAITPKG-absent blocker at gate one → cleared on the same bench via AMD MWAITX → two live integration
bugs → a coherence reject fixed at the root. Delivered `s8_slice2` `c30ae927`, byte-matched from the
reviewer's own checkout.*

### Lead (zero.nic-lead)

- **Team-forming the contract caught the load-bearing gaps before the green-light.** Routing the draft
  through dev + reviewer critique before promoting to the PM surfaced the source-property un-fakeable tell,
  the named instruments, and the seam contract belonging in the frozen set — all folded before scope was
  fixed. A moving oral plan would have shipped the doorbell.
- **Resequencing on the operator's no-fallback direction paid off exactly as intended.** When the operator
  eliminated the fallback mid-formation (single event-driven wait path, no polling, no dead code), the wait
  primitive became a hard dependency — so I resequenced its device-wake coherency to a **standalone
  make-or-break gate before the integration**. The WAITPKG-absent blocker then surfaced at gate one,
  cheaply, before anything was built on it. Prove the one un-referenced unknown first when it has no
  fallback.
- **Diligence before escalation dissolved the blocker on the same bench.** The dev's first read was "no
  userspace monitor-wait exists" — but that checked only Intel WAITPKG, and the contract's Intel-centric
  `UMWAIT` framing was mine. Before surfacing a hardware/direction fork to the operator, I routed the AMD
  equivalent (`MONITORX`/`MWAITX`) check; it was present, and the sprint continued on the same bench. Do
  not make the human the failure-detector for an unfinished check — check the vendor's equivalent before
  declaring a capability absent. ([[userspace-monitor-wait-is-vendor-split]])
- **Held the coherence line at the gate; routed the smallest correction.** Upheld the reviewer's
  split-reject (source-ready / evidence-not) when the Slice-2 capture bound to a *superseded* binary with
  the `block(None)` fix in the delta — a green on a stale binary proves nothing about the delivered one.
  Routed a re-capture, not a rework; the dev fixed it at the root (self-hashing harness).
- **Kept scope authorization honest.** Caught the dev's "operator's directive" for the tooling cleanup that
  never routed through me and clarified it rather than let it stand — the operator had reached the dev
  directly. ([[direct-operator-contact-routes-through-lead]])
- **What I got wrong, conceded:** the Intel-centric primitive framing (above); and "event-driven RTT should
  drop toward the µs wire" — the dev correctly corrected it (the RTT floor is round-trip *processing*; the
  wake is µs, but a full round-trip is ~100µs+ of demux+copy+ACK+hops regardless). Event-driven removes the
  poll-sleep and hits the processing floor; it does not make processing free.
- **Next sprint inherits (coordination):** resequence-the-make-or-break-first for any hard dependency
  without a fallback; diligence-before-escalation; the doc human-review gate before commit; and the §7
  max-K question (ring caps K≤64; "hundreds" = ring-grow + re-measure) — the designer's call, queued.

### Developer (zero.nic-dev)

**What the evidence changed (plan bent to the bench):**
- The datasheet collapsed two slices: §3.1.4.5.3 item 3 (the 82599 cannot relax descriptor writes **or** an
  MSI write) + `NS_DIS` (RX snooped) make the `DD`-line writeback coherency-**equivalent** to an MSI-X
  doorbell — so the designer cut the whole doorbell plumbing to DD-line monitoring. Ground the wake
  **source** against the datasheet before building plumbing.
- The primitive was vendor-wrong until probed: the contract said Intel WAITPKG/`UMWAIT`; the bench is AMD
  Zen 1 (WAITPKG absent, `UMWAIT` `#UD`). AMD `MONITORX`/`MWAITX` is the primitive. Verify a CPU feature by
  CPUID **+ SIGILL-guarded real-opcode execution**, and check the bench vendor's equivalent before
  declaring absence.
- **Coherence ≠ the monitor-wake:** DD-poll-works proves *visible*, not that the store *wakes* the monitor
  (microarch, same as 7b coherence≠ordering). Proved positively on silicon (2000 frames, 0 missed, all
  sub-tick).

**What I caught (each surfaced, not buried):**
- **Single-frame-arm wrap stall** — the Slice-1 harness copied the depth-1 arm, stalled at 63/1-wrap = the
  exact RX-RDT re-arm bug. A re-arm proof must exercise re-arm.
- **RDH→RDT+1** — the Host located the monitored descriptor via the device head (races ahead of unconsumed
  frames) → `block` spun forever; `(RDT+1)%depth` (software tail) is the consumer's next slot.
- **`block(None)` must not park forever** — after give-up (no ACK/deadline) an event-driven `block(None)`
  that waits for a frame hangs; the frozen Host contract (inherited from PollHost) returns after a bounded
  low-power wait. *Retiring a mechanism inherits its contract, not just its call sites.*
- **Stale-capture coherence break (the gate reject)** — a shell-computed capture bin-sha drifted from the
  delivered binary, with the `block(None)` fix in the delta. Root fix: `benchtools` self-hashes
  `current_exe` at startup so numbers bind to bytes by construction. **Re-capture after any source change.**

**Honest nuances (corrected, not tuned):** the event-driven RTT floor is *processing* (~136µs), not the
wake (~25µs idle-wakeup, 3µs store floor) — event-driven removes the ~112µs poll sleep (1.5–1.9× lower RTT,
throughput ~doubled); "toward the µs wire" was the wrong expectation. `MWAITX` wakes on the ~4ms OS tick in
userspace (a third population) → event-driven = **sub-tick**; timer-enable = `ECX` bit1 (not bit0, which
would disable the hang-safety timer).

**Next sprint inherits:** live path = `DdWaitHost` (`MWAITX`-on-`DD`), PollHost gone. Open: no-IOMMU
DMA-anywhere + `dma.rs` crossing-soundness (pre-existing); §7 max-K (ring caps K≤64; "hundreds" = ring-grow
+ re-measure, designer's call). New characterized residuals: `MWAITX` C-state exit tail (bounded ≪ 10ms);
`block(None)` ~4–5ms low-power re-check cadence at true idle. Self-hashing-capture tooling discipline is now
structural.

### Reviewer (zero.nic-review)

**What the sprint proved.** Event-driven parking on the NIC's own `DD` cache-line write (`MONITORX`/
`MWAITX` on AMD Zen 1), replacing the bounded-poll sleep. Slice 1 proved the device `DD` store wakes an
idled CPU sub-tick and reliably; Slice 2 integrated it (`DdWaitHost`), retired PollHost entirely, and proved
the adjacent-line drain + re-arm across ring-wraps under real load. Every gate number bound to bytes bound
to source.

**What I held the line on.** *Method over counters — a green on a superseded binary proves nothing:* the
load-bearing catch was Slice 2's first capture self-declaring a bin-sha (`552279c3`) that did not match the
delivered binary (`6a518b1e`), with the `block(None)` fix **in that delta, on the gate path** — REJECT on
coherence, not physics. Coherence before gates (rebuild + byte-match before trusting a number).
Capture-fallback bound to the *delivered* binary. Numeric re-derivation (parsed the 2001-row raw wake CSV
myself, not the report). Inference labeled as inference. Split the verdict so the fix was a fast re-capture,
not a rework.

**Evidence cut both ways — a hash mismatch is a question, not a verdict.** The coherence discipline caught
one real incoherence (Slice 2) **and** prevented two false-rejects by diagnosing benign hash divergences to
root cause: Slice 1's absolute-path debug embedding (closed by the remap byte-match), and the Slice-2 re-gate
rustflags-spelling sensitivity (space vs equals form, identical codegen). Reject on a *proven* incoherence;
diagnose a divergence to root cause first.

**What the next sprint inherits.** (1) The self-hash discipline is structural — but self-hash proves
numbers→bytes, **not** bytes→source; the **reviewer rebuild** closes bytes→source and must keep being
required. Reproduce with the capture header's rustflags string *verbatim* (the hash keys on flag-spelling);
keep host/timestamp runtime, never `build.rs`-baked. (2) The re-arm-under-load gate pattern — a lockstep/
single-frame PoC validates only the *initial* arm; moving-target re-arm across ≥1 ring-wrap under real load
is a separate gate (this RX-RDT zone has bitten twice). (3) The `(RDT+1)%depth` monitor invariant (software
tail, not device head RDH). (4) Host-seam contract: `block(None)` must not park forever — finite EBX on
every call + check-before-wait. (5) Un-fakeable tell: unique-nonce payloads + RXDGPC + APERF-idle on every
hardware gate. (6) Residuals still **open** (in `concerns-to-watch`): no-IOMMU owned-DMA (vector-1), and the
platform-scoped crossing-soundness. One hardening for the `benchtools` owner: normalize the recorded
rustflags string so the bin-sha keys on codegen, not flag-spelling (recommendation, not a blocker).
