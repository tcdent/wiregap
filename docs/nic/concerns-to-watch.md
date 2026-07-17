# Concerns to watch — suspected hazards, deliberately left unsolved

> A standing list of things we *suspect* are hazards in the NIC / transport subsystem but have **not**
> analyzed to resolution. It records each concern and **withholds the solution on purpose.** Writing a
> confident fix here would signal "handled, move on" — and the danger is precisely that a half-understood
> hazard, once it *looks* addressed on paper, stops getting our attention. Each entry is an open question
> to return to, not a settled item.
>
> Rule for this file: **if an entry ever reads like a confident answer, something has gone wrong.** When
> a concern is genuinely resolved — on real silicon, with the actual code — move the resolved part into
> the design docs and leave behind only the residual that is still unknown.
>
> Status of every entry below: **not yet deeply analyzed.** Opened 2026-06-21.

## The unsafe DMA / memory boundary

The reactor crosses from device-co-owned memory — the NIC writes it concurrently, outside the language's
model — into our owned memory (`architecture.md §3.1`). We have a *shape* for that crossing. We do not
have a verified account of its soundness. Suspected hazards, each unproven:

- **Externally-mutated memory vs. the language model.** The NIC mutates DMA memory with no instruction
  in our code doing it. We suspect a naive access could let the compiler assume a stability that is not
  there. Unanalyzed: whether our access is actually sound, or merely usually-works.

- **Ordering between "frame ready" and "read the frame."** We suspect a visibility/ordering hazard
  between observing the descriptor-done signal and reading the payload — that we could read before the
  device's writes are fully and coherently visible. Unanalyzed: what guarantees we actually have versus
  assume, and how that changes across platforms.

- **The copy operation itself.** The operator's own question stands open: *is there any way the copy does
  something subtly wrong?* — tearing, aliasing the DMA region as a safe reference, undefined behavior that
  "looks fine." We have argued it informally; we have not shown it sound.

- **The size of the unsafe surface.** We assert the unsafe region is small and confined. We have not
  verified that nothing leaks a raw pointer or a device-backed reference out into safe code, where the
  confinement would silently fail without announcing itself.

- **Trusting device-written fields.** We note (`grounding/data-path-and-events.md §1`) that the
  firmware-written length must be bounded, not trusted. Residual we have not closed: whether we have
  enumerated *every* field we read from device-written memory, or only the obvious one.

## The NIC's DMA is owned, not contained — the residual of dropping the IOMMU (2026-07-16)

The subsystem drives the NIC on the pure-direct path with **no IOMMU** (`grounding/kernel-bypass-and-dma.md`):
the device DMAs to physical addresses we resolve, unbounded by any hardware confinement. This is the
operator's ratified posture — an IOMMU confinement would rest on the kernel that programs it, and the
kernel is excluded from the trusted base, so it would launder the risk through kernel trust rather than
remove it. The rogue-NIC DMA risk is owned directly.

What stays **open, on purpose:**

- **The owned firmware-DMA risk.** With no confinement, a compromised NIC firmware — bulky vendor code we
  cannot read, written from box-controlled wire bytes (`grounding/data-path-and-events.md §1`) — could DMA
  anywhere in proxy memory. We do not mitigate this in hardware; we own it. The concern to watch is
  whether that ownership actually holds: whether the parent threat model tolerates un-contained firmware
  DMA on the trusted side, or whether some containment that does *not* rest on the untrusted kernel is
  later wanted. This is the security-load-bearing question, and it is **audited with the policy layer
  after the PoC is built** — until that audit returns, the posture is committed but not ratified, and
  `architecture.md §2.1` / the spine remain to be reconciled to it.

- **Vector 2 is unchanged, and still open where it always was.** The driver interpreting adversary-written
  descriptors is the one guard the posture shift does not touch (`grounding/kernel-bypass-and-dma.md §3`);
  its residual — whether we have enumerated *every* device-written field we read, not just the obvious
  length — stays exactly as it was.

## The residuals are now EXERCISED on silicon — Slice 4 transmission (2026-07-17)

Slice 4 proved data transmission across the DAC (`41:00.0` → `63:00.0`, pure-direct, no IOMMU): a unique
per-run nonce transmitted by 41 arrived at 63 **byte-equal**, out of pre-cleared buffers, through the real
`copy_out`/`RxDrain` seam, DD set by the device, `RXDGPC` confirming the DMA-to-host (reproduced across
runs; TX-first showed `RXDGPC=0` with RX unarmed, then `RXDGPC>=1` after arming). This closes
**PATH-WORKS** only. The transmission pass **EXERCISED** the two residuals below; it did **not** discharge
either.

- **The no-IOMMU owned-DMA posture is EXERCISED at the first live RX-write (unattended, on production);
  owned per §2 (policy-audited post-PoC); path-works != crossing-sound != contained.** The hard stop was
  dropped by operator ruling, so the first RX per-buffer device-write ran unattended on the live host,
  gated only by gate-2(i) + TX-first — **both CONTENT-BLIND on the RX buffer**. The byte-equal proof reads
  the buffer AFTER the device already wrote it, so it is an end-to-end PATH proof, NOT a pre-write guard.
  The §2 DMA-anywhere risk is therefore **concrete and realized in this run**, not abstract or future.

- **`dma.rs` crossing-soundness stays UNDISCHARGED (visibility / DD-ordering / no-tearing).** A frame
  crossing intact proves the path works; it does **not** prove the crossing is *sound* — the
  device-concurrency visibility, the DD→read memory ordering, and no-tearing hazards enumerated under "The
  unsafe DMA / memory boundary" above — **nor** that it is *contained*. The run relied on x86 cache-coherent
  DMA + TSO + the UC-MMIO-doorbell ordering WITHOUT explicit fences; that it worked is evidence of
  path-works, not a memory-ordering soundness proof. Neither residual is discharged by the transmission pass.

## How this went wrong in the past

Two containment approaches were explored and abandoned before the pure-direct posture. First, a
**kernel-configured IOMMU (VFIO)** — rejected because the confinement's correctness would rest on the
excluded kernel, so it was not real containment. Then, **owning the VT-d DMAR ourselves** on a bare-metal
substrate (Sprint 4–5; the DMAR table construction was even proven-in-QEMU) — rejected because it required
a bare-metal runtime the system does not use: we stay a userspace process on running Linux. The current
posture — own the device directly, own the DMA risk — replaces both. Do not re-introduce either: not the
kernel-IOMMU (it trusts the kernel), not the own-the-DMAR substrate (it is bare-metal). The residual above
is the honest cost of that choice, and it is the policy layer's to audit.

## Why this file exists

These are real, and they are also exactly the kind of thing that *looks* handled the moment it is written
down. This list's only job is to keep them visibly **un**-handled until the work is actually done — so that
the next person to read the subsystem (including us, later) inherits the open question, not a false sense
that someone already settled it.
