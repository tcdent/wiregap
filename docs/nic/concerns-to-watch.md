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
model — into our owned memory (`architecture.md §3.1`). This crossing now HAS a verified account:
**`grounding/crossing-soundness.md`** (7b Slice 1, certified 2026-07-18) — a **positive, platform-scoped**,
cited account of **both** directions (RX-read `DD`→payload; the write stores→doorbell; the TX buffer-reuse
control-flow leg), realized in `dma.rs` as an acquire-on-the-atomic-`DD`-load + a release before the
doorbell. The ordering / visibility / no-tearing hazards that stood here — externally-mutated memory vs. the
compiler's stability assumption, the `DD`→payload ordering, the copy's tearing/aliasing — are **DISCHARGED
there** (moved to the design doc per this file's rule). What remains open is only the **narrowed residual**:

- **The platform + language-model boundary (the irreducible residual).** The account is sound on x86-64 +
  cache-coherent DMA + this 82599 over PCIe — not proven general; and Rust's abstract machine does **not**
  model memory an external device mutates, so the `Acquire`/`Release` synchronize *in the hardware sense*
  with the device (outside the language machine) — a **hardware-level cited argument, not a
  language-machine proof**. This platform-scope + language-model boundary is why the crossing is
  *platform-scoped-sound*, not *unconditionally sound*. It is stated, not papered, and it is what to
  re-derive against any new platform/fabric/device.

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
**PATH-WORKS** only. At the time, the transmission pass **EXERCISED** both residuals below without
discharging either. Status now (7a operation + 7b account) is updated per residual:

- **The no-IOMMU owned-DMA posture is EXERCISED — and now at SUSTAINED SCALE (still owned, not contained).**
  The hard stop was dropped by operator ruling, so the first RX per-buffer device-write ran unattended on
  the live host, gated only by gate-2(i) + TX-first — **both CONTENT-BLIND on the RX buffer** (the byte-equal
  proof reads the buffer AFTER the device wrote it: a PATH proof, not a pre-write guard). **7a operational
  grounding:** the owned-DMA posture then ran at sustained scale on the live production host — steady-state
  rings (128 round-trips over 2 ring wraps), K=32 concurrent multiplexed sessions, and the E6 measurement
  storm (320 concurrent) — with the production NIC (`61:00.0`) never touched (compile-enforced `Allowed(Bdf)`
  + gate-2 physaddr validation) and no incident. This grounds *what sustained operation exercised*; it is
  **operational path-works, NOT a containment proof**. The §2 DMA-anywhere risk stays **owned, concrete, and
  un-contained** — the policy-layer audit still owns the verdict (whether the parent threat model tolerates
  un-contained firmware DMA on the trusted side). OPEN, on purpose.

- **`dma.rs` crossing-soundness is now DISCHARGED (platform-scoped), 7b Slice 1.** The visibility /
  `DD`-ordering / no-tearing hazards are a positive, cited account in `grounding/crossing-soundness.md`, and
  the implicit-x86-TSO reliance is replaced by explicit ordering code (acquire-on-the-atomic-`DD`-load +
  release-before-doorbell). What the Slice-4 run leaned on WITHOUT fences is now made explicit and cited;
  only the **narrowed platform + language-model residual** (see "The unsafe DMA / memory boundary" above)
  stays honest-open. This residual — distinct from the owned-DMA/containment one above — is the *sound-on-the-
  stated-platform* boundary, not a soundness gap.

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
