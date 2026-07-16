# The bare-metal substrate

> The platform-ownership layer beneath the driver: how we get from power-on to owning the machine with
> **no kernel**, and the three primitives — PCIe config access, physical memory, interrupts — that the
> driver, the reactor, and the IOMMU confinement all stand on. It exists because the kernel is excluded
> from the trusted base for being un-reviewable (spine assertion 16): rather than trust a kernel we
> cannot read, we run on the bare metal and build the few primitives we need in our own reviewable code.
>
> This doc owns the substrate mechanics (UEFI handoff, ECAM, the arena, IDT/LAPIC/timer). The **DMA
> confinement** — the DMAR/IOMMU we configure on top of these primitives — is owned by
> `kernel-bypass-and-dma.md`; this doc stops at the primitives it stands on.
>
> Grounded in the UEFI spec (the boot-services handoff + `ExitBootServices`), the ACPI spec (the MCFG
> and DMAR tables the handoff points us at), the x86-64 architecture (ECAM, the Local APIC, the IDT,
> MSI-X), and the `rust-osdev` crate ecosystem (`uefi`, `x86_64`, `acpi`).

The runtime model and its build split are the operator's ratified direction (2026-07): the trusted core
owns the hardware with no kernel above it; this team builds the bare-metal foundation, using vetted crates
for the non-security boot boilerplate and hand-writing everything the security guarantee rests on.

## 1. The boot-chain / owned-runtime split (the mechanism, and why)

The "bare-metal foundation" is two halves, and only one is ours to obsess over:

- **CPU/platform bootstrap** — real→long mode, enabling paging, training memory, discovering the memory
  map and ACPI tables. Huge, pure boilerplate, runs once at boot, and **not attacker-facing** (the box
  reaches us over the wire at runtime, never through the boot chain).
- **Owned-hardware runtime** — PCIe config, the IOMMU, physical memory, interrupts, the driver. Small,
  security-load-bearing, must be reviewed.

The simplest foundation writes the least of the first half. So we **enter as a UEFI application, call
`ExitBootServices`, and own the machine from a clean, documented handoff** — the firmware has already
done the entire mode-switching bootstrap and hands us exactly what we need (below). The firmware is the
platform-trust floor *below* us (boot-time, in the same category as trusting the silicon), **not a kernel
above us**: after `ExitBootServices` no firmware code is resident, and nothing mediates our access to the
hardware. Hand-writing the reset-vector-to-long-mode bootstrap would spend the "reviewable in one
sitting" budget on code that carries no security weight — the opposite of minimal-where-it-matters.

## 2. The crate split — vetted boilerplate vs hand-written security-load-bearing

Ratified: **use the vetted `rust-osdev` crates for the non-attacker-facing boot/CPU boilerplate**
(`uefi` for the entry/handoff, `x86_64` for IDT/descriptor bit-packing, `acpi` for table parsing), and
**hand-write everything the review guarantee rests on** — the DMAR/IOMMU setup, the PCIe config scan, the
driver. The line: vetted deps for the boot/CPU boilerplate the wire adversary never touches; hand-written
for the runtime attack surface. A **crate audit and potential in-housing is planned before project
completion** — the crates are a get-off-the-ground convenience at the boundary, not a permanent article
of trust. Where a crate straddles the line (an `acpi` parse feeds the DMAR base; `x86_64` structs back
the IDT we install), the *security-bearing use* of its output is ours and reviewed; the crate does the
boilerplate decode.

## 3. UEFI / `ExitBootServices` handoff

Before exit, UEFI boot services hand us the two things the runtime needs:

- **The memory map** — which physical ranges are usable (feeds §5's arena), plus the `map_key` required
  to exit.
- **The ACPI configuration table** — RSDP → the **MCFG** table (the ECAM base for §4's PCIe config) and
  the **DMAR** table (the DMA-remapping hardware for `kernel-bypass-and-dma.md`).

We call `ExitBootServices(map_key)`; the firmware unloads its boot-time drivers, and from that point
there is no kernel and no resident firmware above us. UEFI leaves us in **long mode, identity-mapped**,
which we keep (§5). The `uefi` crate provides the pre-exit API surface; the post-exit runtime is ours.

## 4. PCIe config-space access — ECAM

From the ACPI **MCFG** table we get the ECAM (memory-mapped config) base. A register is a plain volatile
MMIO access at:

```
addr = ECAM_base + (bus << 20) + (dev << 15) + (func << 12) + offset
```

No port I/O, no address/data-register locking — the reason to prefer ECAM over legacy CF8/CFC. The
hand-written scan touches only a handful of fields: **walk bus/dev/func** reading vendor/device at offset
0 until we find the 82599 (`8086:10FB`); then read **BAR0** (the register-block physical base — §5
identity-maps it), set **Bus-Master-Enable** in the **Command** register (offset 0x04), and read the
**MSI-X capability** (for §6). That is ~one accessor function plus a small loop.

This scan reads **NIC-controlled config-space values before the IOMMU is configured** — the third surface
scoped out in `kernel-bypass-and-dma.md §2`. Under a kernel that parser was a large stock enumerator;
here it is ours and tiny, which is why it can be held to reviewed-by-reading. *(CF8/CFC stays available as
a no-ACPI fallback — it reaches the 256-byte header, enough for BARs + Command — but on our UEFI platform
ECAM is present and cleaner.)*

## 5. Physical memory — the arena allocator

After `ExitBootServices` we have the UEFI memory map, and UEFI left us **identity-mapped**. We keep that
map and do **not** build a virtual-memory subsystem. We also **identity-map the IOVAs in the DMAR
tables** (`kernel-bypass-and-dma.md §2`), so there is a **single address space** — CPU virtual =
physical = IOVA — across the CPU, the NIC, and the IOMMU.

No heap is needed. At init we allocate a fixed, known set of regions **once**, from a bump pointer over
one reserved usable region: the DMAR tables, the RX/TX descriptor rings, and the packet-buffer pool —
honoring alignment (rings 128-byte per the 82599 datasheet; tables and buffers 4 KB-page). **No free**;
the reactor's ACK-bound buffer lifecycle (`data-path-and-events.md §3`) manages the pool internally. This
is a few dozen lines: a bump pointer with alignment, no fragmentation, no lifetimes — trivially held in
the head. Buffer-pool sizing stays a bench item (§8).

**Two categories, and the DMAR-isolatability coupling to Sprint 5.** The arena holds two kinds of region,
and the distinction is security-critical. **NIC-reachable DMA regions** — the RX/TX rings and the
packet-buffer pool — are the only memory the device ever touches; they go *into* the Sprint-5 second-level
map, laid out **aligned and contiguous to the DMAR's page granularity** so that map can bound the device
to *exactly* them and nothing adjacent falls inside a mapped page. **The DMAR tables themselves are not
DMA-able**: they are read by the IOMMU *hardware*, never by the NIC, and are deliberately **left out of
the second-level map** — the tables that define the device's confinement must sit outside what that
confinement exposes, or a rogue NIC could DMA to its own translation tables and rewrite its permissions (a
total containment break). So the arena's job is not just to make the reachable regions confinable; it is
to keep the two categories cleanly separable — reachable regions page-isolatable and mapped, the DMAR
tables unmapped and unreachable. Getting this shape right in the arena is cheap now; a re-layout once the
DMAR is built on top is expensive — a constraint the arena honors from the start, not a Sprint-5
discovery.

## 6. Interrupts — IDT / Local APIC / one-shot timer

- **IDT** — a 256-entry table set once (`lidt`); we install exactly one real handler (our MSI-X vector)
  plus the CPU-exception handlers (a fault panics, rather than running on undefined state). The `x86_64`
  crate provides the descriptor structs; the vectors we point them at are ours.
- **Local APIC** — enabled via the spurious-interrupt-vector register; every interrupt ends by writing
  `0` to the EOI register. The NIC's MSI-X message points at the LAPIC (a `0xFEE00000`-format address,
  data = our chosen vector), so the interrupt is delivered **straight to our IDT vector** — and that MSI
  write is governed by the **interrupt remapping we configure in the DMAR** (`kernel-bypass-and-dma.md`
  §2 (B)).
- **The ISR is near-empty.** In the reactor's poll-then-block loop, the interrupt's only job is to wake
  the CPU from `hlt`: busy-poll the ring; when it's empty, arm the interrupt and `sti; hlt`; the NIC (or
  the timer) wakes it; EOI; loop. The handler is EOI + return.
- **The retransmit deadline** — which was `epoll_wait`'s timeout — is the **LAPIC timer as a one-shot**
  (divide-config + initial-count): it fires an interrupt that wakes the same `hlt`. So "block on the NIC
  interrupt *or* the retransmit deadline, one wait, two wake reasons" becomes "`sti; hlt` with the MSI-X
  vector and the LAPIC-timer vector both armed." This is the bare-metal realization of the idle-wakeup;
  `data-path-and-events.md §4` (written around eventfd/epoll) reframes to it.

**Relationship to the reactor.** What is frozen is the reactor *algorithm* — poll-then-block,
`compute_timeout`, dispatch, back-pressure, session interleaving — held semantically unchanged. Its OS
coupling splits in two, and the reactor side is 004's design: the **clock** is already a clean seam (a
caller-supplied clock, into which the bare-metal timer/TSC drops with no shape change), but the
**wakeup/multiplexer is not a seam today** — the committed `engine.rs` holds concrete `epoll`/`eventfd`/
`Vec<libc::epoll_event>` directly, so it does **not** compile bare-metal unchanged. 004's adjustment
extracts a **`Waker` seam** (std impl = epoll/eventfd; bare-metal impl = `hlt`-until-LAPIC + the one-shot
timer), takes the reactor `no_std`+alloc, and `cfg`-gates the PM-ruled-out app-fd class to std-only — the
algorithm itself untouched, and *provably* so: the existing reactor tests re-run on the std `Waker` impl
to prove the freeze, while the bare-metal impl is QEMU-tested. What **this** doc fixes is only the
mechanism the bare-metal `Waker` consumes: the wakeup primitive is an MSI-X→IDT vector plus the LAPIC, and
the deadline is a LAPIC one-shot, both waking a `hlt`.

## 7. What this layer puts in the trusted base

The **hand-written** parts — the ECAM config scan (§4), the arena (§5), the LAPIC/MSI-X/timer setup (§6),
and, in Sprint 5, the DMAR configuration — are **small-and-ours trusted-base members**, reviewed by
reading. The **crates** (`uefi`/`x86_64`/`acpi`) are boot-time, non-attacker-facing boilerplate at the
boundary, with an audit + in-housing pass planned before completion (§2).

**The DMAR earns E3-grade scrutiny, not a citation.** When the IOMMU was kernel-configured it was
something we *cited* (a platform default we asserted). Hand-written and inside the trusted base, **a DMAR
bug is a containment hole** (`architecture.md §2.1`) — so the DMAR configuration earns the same
confinement-*and*-soundness scrutiny E3 gives the unsafe DMA boundary: a three-level verdict
(remapping **enabled** → out-of-range DMA **blocked** → the setup **sound**), an induced out-of-range-DMA
attempt that must fault, and hazards kept honestly open until silicon confirms them rather than papered
with a confident-sounding config. This is the old E2 security framing **retargeted** to our own DMAR; it
lands primarily in the Sprint-5 IOMMU grounding (`kernel-bypass-and-dma.md` + `concerns-to-watch.md`),
noted here because it is the confinement-adjacent stance the substrate hands up.

## 8. Bench-gated / build-time open items (this topic)

1. **`ExitBootServices` + retain control** on the real platform: the handoff yields a clean memory map +
   ACPI pointers, and we still own the machine after exit.
2. **ECAM present and the 82599 found** via the scan; BAR0 maps; Bus-Master-Enable takes.
3. **The arena over the real UEFI map**: alignment honored; buffer-pool sizing measured, not copied from a
   default (`data-path-and-events.md §6`).
4. **The interrupt path fires**: an MSI-X message lands on our IDT vector; LAPIC EOI + the one-shot timer
   wake the `hlt`; the poll-then-block loop composes the retransmit deadline.
5. **The crate boundary**: audit `uefi`/`x86_64`/`acpi` before completion; identify in-housing candidates.
