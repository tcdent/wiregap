# NIC Driver / Transport — Build Epics

> A brief outline of the epics for building the NIC driver/transport component, for the PM to break
> down and delegate. This is the **driver + transport** only — the box↔proxy link layer
> ([design authority: `docs/nic/`](.)). It is **not** the whole system. The full design is in
> `architecture.md`, the `grounding/` notes, and `concerns-to-watch.md`; this file just maps the
> work and the goals, it does not restate the design.

## What we are building, and why

One small, fully-reviewable userspace driver + transport that carries reliable multiplexed sessions
over a single 1m DAC link between the box and the proxy. It is **the same code on both ends**; on the
proxy end it is part of the **trusted base**, which is what sets the bar.

The goals, in priority order — every epic serves these:

1. **Minimal trusted surface is the prime directive.** This is the one trusted-base member that is
   neither tiny-and-trivial nor static-firmware. Smaller and simpler *is* the deliverable. Reject
   features; reject cleverness; write the fewest lines that meet the promise.
2. **UART simplicity at a higher bit rate.** A dumb point-to-point byte pipe with an ACK and an ID —
   not a network stack. No routing, reordering, congestion, negotiation, or adaptive anything. The
   medium has none of those problems, so the code has none of those mechanisms.
3. **Understand every line.** The security guarantee *is* that a human reviewed the whole thing in one
   sitting. Code that cannot be held in the head is a defect even if it works.
4. **The NIC is untrusted-but-contained** (`architecture.md §2.1`). The driver is the guard for
   adversary-written descriptor/buffer content; the IOMMU is the confinement. Realizing this posture —
   not assuming it — is load-bearing.
5. **Make it impossible, not caught.** Structural guarantees over checks-and-filters, everywhere.

**Language: Rust**, with `unsafe` confined to the smallest possible DMA/MMIO boundary and everything
else in safe Rust. The marked `unsafe` blocks are the review targets.

## Scope boundary (what this build is NOT)

Not the box-side capture (TUN / HTTP-proxy / emulator), not the proxy egress guard, not the head/
console, not the file-quarantine logic, not the application-layer binding semantics. The transport
carries **opaque sessions**; what rides inside them (pane / file / request meaning) is above this
component. If a task is about *what a session means*, it is out of scope here.

---

## The epics

**E1 — Device bring-up & link.** Get the 82599/X520 from power-on to a live 10G **SFI** link over the
passive DAC, with the absolute minimal, **defaults-first** register set (write only what is
security-load-bearing, datasheet-required, or wrong-at-default; run on documented defaults otherwise).
The minimal link-restart form is bench-determined. *Authority: `grounding/device-bring-up-and-link.md`,
`grounding/driver-minimality.md`.*

**E2 — Kernel-bypass, DMA & IOMMU confinement.** Own the NIC from userspace (VFIO, target
iommufd/cdev) with **no kernel network stack in the path**; map BAR0 and the DMA rings; confirm the
kernel-free hot path (bare-metal, no VMexit, zero per-frame syscalls). **Make the IOMMU containment
airtight** — DMA remapping, interrupt remapping, ATS-off, no peer-to-peer — because the
untrusted-but-contained posture depends on it. *Authority: `grounding/kernel-bypass-and-dma.md`,
`concerns-to-watch.md`.*

**E3 — Data path & the copy boundary.** RX/TX rings and descriptors, treating **all device-written
content as adversarial** (bound the length, treat status as hostile, never deref an unchecked field,
drop-and-refill). Implement the **copy boundary**: copy each frame out of DMA memory into app-owned
memory at the earliest point; events carry the copy, never a ring pointer. This is also the
**unsafe/safe line** — confine the volatile/raw-pointer DMA touches and get the DD-handshake ordering
right. Per-descriptor cleanup; ACK-bound buffer reclamation; CRC/`DROP_EN` reliance.
*Authority: `grounding/data-path-and-events.md §1–3`, `architecture.md §3.1`, `concerns-to-watch.md`.*

**E4 — The event reactor.** The single-threaded heartbeat: an event-driven **idle-wakeup** that parks the
CPU via `MONITORX`/`MWAITX` on the NIC's own `DD` writeback (no MSI-X, no eventfd, no kernel bridge);
**busy-drain-then-park** (busy-poll while there is work, `MWAITX`-park on the `DD` line when the ring is
empty); the retransmit deadline composed into the same `MWAITX` wait; single-producer, no locks.
*Authority: `architecture.md §3, §5`; `grounding/data-path-and-events.md §4`.*

**E5 — The session protocol.** The UART+ACK+ID transport: frame = `[session ID][payload]`; bare ACKs
distinguished by direction; **strict stop-and-wait**; ~10 ms retransmit interval + ~1–5 s give-up
(silent internal reclaim, no event); **multiplexing-by-instantiation** (a session is an instance of the
one protocol; the reactor interleaves them); session lifecycle — open-by-emission, **no close signal**,
**no cross-epoch state**, dormant ≠ dead; session IDs **originator-allocated from a partitioned space**
(box for requests, proxy for responses — disjoint by construction; both endpoints originate,
`architecture.md §4`), 64-bit monotonic per side, unique within an epoch. No flag byte beyond what is needed, no adaptive RTO, no session/epoch tracking.
*Authority: `architecture.md §4, §6, §7, §8`.*

**E6 — Validation (mostly interleaved) + a final integration pass.** Bench validation is **not** a
monolithic final sprint. Most of it **belongs inside E1–E5**: each epic carries its own bench-gated
items and closes them as hardware permits — an epic is not "done" until its silicon-asserted claims are
verified or honestly tracked open (e.g. **E1 cannot complete without silicon** — the minimal link form
is bench-*determined*). What is genuinely final in E6 is a small **integration-validation pass**: the
cross-cutting, end-to-end measurements that only exist once the stack is assembled (single-ring
throughput at the real RTT, the reactor on one core under full load, buffer sizing under a realistic
redraw storm). Plus: ground the `concerns-to-watch.md` hazards **honestly** — ground them or keep them
visibly open, never paper over with a confident-sounding fix.

*Two hardware tiers gate this:* the **static** items (LESM-disabled, AUTOC2 defaults, IOMMU group,
stepping) are readable **now** on this host's live card via `live-card-probe` (built, read-only, zero
link risk — the cable is production-identical), so run them **up front** to de-risk the anchors; the
**dynamic** items (link-up, throughput, the reactor under load) need the prototype cards we can
VFIO-bind and drive, so they gate **per-epic as the cards arrive**. *Authority: `concerns-to-watch.md`,
each grounding §"open", `tools/live-card-probe.md`.*

---

## Cross-cutting principles (bind every epic, not a separate workstream)

- **Defaults-first / minimal write-set** — fewest register writes, fewest lines, fewest features.
- **Adversarial device input** — the descriptor ring and DMA buffers are attacker-influenced; the
  driver is the guard (`architecture.md §2.1` vector 2).
- **Confined `unsafe`** — the DMA/MMIO touches are the only `unsafe`, small and marked; everything else
  is safe Rust the borrow checker proves.
- **Honest open items** — bench-gated and `concerns-to-watch` items stay visibly unverified until
  actually verified; a documented fix that *looks* solved is the failure this discipline prevents.
- **Reviewable in one sitting** — if a reviewer cannot hold a unit in their head, simplify until they
  can. That review *is* the security guarantee.
