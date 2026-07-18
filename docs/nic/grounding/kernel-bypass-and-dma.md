# Kernel-bypass and direct device ownership

> How we drive the proxy's NIC **directly** from a userspace process on a **running Linux system** — the
> kernel network stack, and the kernel itself, out of the interaction — and the DMA posture that follows:
> the device DMAs to physical memory we own, with **no IOMMU containment**. This is the subsystem's reason
> for being: spine assertion 16 excludes the kernel from the trusted base and drives "a minimal,
> datasheet-built driver over a kernel-bypassed link." We take the device entirely and talk to the
> hardware ourselves.
>
> Grounded in `reference/ixy` — the **direct (non-VFIO) path**: `pci.c` (`remove_driver`, `enable_dma`,
> `pci_map_resource`) and `memory.c` (`memory_allocate_dma`, `virt_to_phys`).

The runtime model is the operator's ratified direction (2026-07): Linux stays running and untrusted; we
take the NIC device entirely and drive it directly — pure direct path, **no kernel tooling** (no VFIO, no
kernel-mediated IOMMU). The device is ours; we own the whole stack.

## 1. Taking and driving the device (the pure direct path)

We are a userspace process on a running Linux system. We take the proxy's NIC and drive it directly — no
kernel driver for it, no VFIO, no kernel framework. One-time setup, off the per-frame path:

- **Unbind the NIC from its kernel driver** — write the PCI address to
  `/sys/bus/pci/devices/<addr>/driver/unbind` (`pci.c:remove_driver`). After this the kernel is not
  touching the device.
- **Enable bus-mastering** — set bit 2 (Bus Master Enable) in the Command register at config-space offset
  4, via `/sys/bus/pci/devices/<addr>/config` (`pci.c:enable_dma`).
- **Map the registers** — `mmap` `/sys/bus/pci/devices/<addr>/resource0` (BAR0) (`pci.c:pci_map_resource`).
  From here we drive the 82599 by reading and writing its registers directly, as CPU loads/stores to the
  mapped BAR.
- **DMA memory** — allocate 2 MiB hugepages (hugetlbfs), `mlock` them so they never swap, and resolve
  their **physical addresses** via `/proc/self/pagemap` (`memory.c:virt_to_phys`). The device DMAs to
  those physical addresses **directly** — there is no IOMMU translation in the path (§2). 82599 rings/
  buffers keep their 128-byte alignment requirement.

This requires root (config-space write, `resource0` `mmap`, `pagemap`, hugepages) — expected, since we own
the machine. None of it is in the per-frame path.

## 2. The DMA posture — owned, not IOMMU-contained (security-critical; policy-audited post-PoC)

The device DMAs to physical addresses we resolve; **no IOMMU translates or bounds its DMA.** This is a
deliberate posture, not an omission:

**Rationale (operator, 2026-07).** An IOMMU confinement would have to be programmed by the **kernel**, and
the kernel is excluded from the trusted base (spine assertion 16). A confinement whose correctness rests
on the untrusted kernel is not real containment — it launders the risk through kernel trust rather than
removing it. The rogue-NIC DMA risk is the same whether or not a kernel-configured IOMMU is in the
picture; so we own the device and the risk directly rather than add kernel trust for no real reduction.

**The posture shift.** The NIC moves from *"untrusted-but-**contained** (by the IOMMU)"*
(`architecture.md §2.1`) to *"**owned** directly; the DMA risk is owned."* The "own the IOMMU ourselves"
and "VFIO" directions that were briefly explored are both rejected: the first requires a substrate we do
not run (bare-metal), the second requires the kernel tooling this posture excludes.

**The honest residual (stated, not papered over).** With no IOMMU, a compromised NIC firmware — bulky
vendor code we cannot read, populated from box-controlled wire bytes (`data-path-and-events.md §1`) —
could DMA anywhere in proxy memory. This risk is **owned** per the decision above, not mitigated by
hardware confinement. It is exactly the kind of hazard `concerns-to-watch.md` keeps visibly open.

**Audit.** This posture is security-load-bearing and changes `architecture.md §2.1` and the spine's
"untrusted-but-contained" framing. Per the operator it is **audited with the policy layer after the PoC is
built** — not settled here. Until that audit, §2.1 and the spine are known-inconsistent with this
direction: this doc records the direction; the policy audit ratifies or adjusts the posture, and §2.1 /
the spine are reconciled to whatever it returns.

## 3. The surviving guard — descriptor content bounded by the driver

One guard is unchanged by the posture shift and remains load-bearing: the descriptor ring and DMA buffers
are **adversary-influenced** — the firmware writes them from box-controlled wire bytes — and the
read-verified driver that interprets them is the guard. Bound the reported length to the buffer size,
treat unexpected status bits as drop-and-refill, never derive a memory access from an unchecked descriptor
field, copy each frame out of DMA memory early (`data-path-and-events.md §1`, `architecture.md §3.1`). This
is runtime-independent — it held under every runtime model considered and holds here.

## 4. The kernel-free interaction — bypassed stack, and the NIC's own event to our memory

Once set up, the per-frame path touches no kernel: we poll the descriptor's `DD` bit in host memory (no
MMIO) and write the tail register as a direct CPU store to the mapped BAR. The kernel network stack is
entirely out of the path (spine assertion 16).

**The idle-wakeup is the NIC's own event, in memory we already read — no kernel, no system interrupt
machinery.** On every received frame the NIC writes back the **RX descriptor** (`DD` bit set) by DMA into
the descriptor ring — a device write to memory *we own*, and the very signal the reactor already reads to
detect a frame. So the idle-wait needs no new mechanism: we **wait on the next descriptor's cache line**
(a userspace `MONITOR`/`UMWAIT` on the line the NIC is about to write) and the NIC's `DD` writeback wakes
us. No MSI-X, no APIC, no IDT, no kernel — the NIC signals us directly in memory we own, exactly like the
DMA it already is. (The NIC's MSI-X — a Dword memory write to an address we program, datasheet §9.3.8 —
could be redirected to a doorbell word for the same effect, but it is **redundant**: the `DD` writeback is
already the per-frame NIC event the reactor consumes, so monitoring it directly is the minimal mechanism.
The `EICR` cause register is a further NIC-emitted signal we read directly.)

So the reactor is **event-driven**, as it was always meant to be: busy-drain while there is work, then
**wait on the descriptor** when idle — unblocking on the NIC's `DD` writeback (a frame arrived) or on a
deadline (the retransmit timer) — with **no polling-sleep compromise and no kernel interrupt bridge**.
What stays for the integration to prove on silicon (§5) is the *efficient wait*: whether the device's
`DD` write reliably **wakes** a userspace `MONITOR`/`UMWAIT` on this CPU (a CPU-capability + write-vs-wait
coherency question — the same un-relaxable coherence class the descriptor poll already relies on). That is
proven on silicon, not assumed.

The wait is a **bounded** `UMWAIT` — the OS caps its duration (`IA32_UMWAIT_CONTROL`) so a shared core is
yielded — with a **lightweight re-entry** when it times out: a low-frequency *idle* re-check (the CPU sits
in a low-power C0 substate between events, never a spin), not a return to polling. The NIC's `DD` write
wakes it immediately regardless, so the re-entry is only the idle **scheduling** tax, never on the
frame-latency path. A truly-unbounded idle (the CPU doing nothing at all until the NIC writes) reduces to
one thing — **owning the core**: a dedicated, isolated core (`isolcpus`/`nohz_full`) with the OS `UMWAIT`
bound removed, at which point nothing competes for the cycles and the wait idles until the NIC's write. It
is a *deployment* arrangement, not a code change, and the same `UMWAIT`-on-`DD` code serves both. The
lightweight re-entry loop is accepted (operator, 2026-07-18); the dedicated-core option is left open, not
required.

**How this went wrong before — do not re-introduce, at two levels.** First, an earlier draft asserted
"interrupts become polling — the pure-direct path has no kernel mechanism to deliver an MSI-X interrupt."
That was **wrong**: an MSI-X "interrupt" is a memory write to an address *we* own, needing no kernel — the
polling compromise, and its sleep-granularity latency, followed from that false assumption, not any real
constraint. Second, the *fix* first reached for MSI-X redirected to a doorbell — a whole capability to
program (table, cause arming, re-arm) for a signal the NIC **already emits** as the `DD` writeback.
Monitoring the `DD` line directly is the minimal mechanism; the doorbell over-engineered the answer.

## 5. Bench-gated / build-time open items (this topic)

1. **The direct path on the real 82599** — unbind, bus-master enable, `resource0` `mmap`, and hugepage DMA
   with `pagemap`-resolved physical addresses, on the bench card.
2. **The event-driven DD-wait wakeup** — integrate the descriptor-line wait (§4) into the reactor and
   validate on silicon: the reactor's idle-wait is a userspace `MONITOR`/`UMWAIT` on the next RX
   descriptor's cache line, and the NIC's `DD` writeback (or the retransmit deadline) unblocks it. The
   make-or-break to prove **standalone, first**: whether the device's `DD` write reliably **wakes** the
   `MONITOR`/`UMWAIT` on this CPU (CPU capability + write-vs-wait coherency). No fallback — if it does not
   wake reliably, that is a blocker to surface, not a poll path to keep.
3. **The DMA posture (§2)** — the policy audit (post-PoC), and the `architecture.md §2.1` / spine
   reconciliation that follows from it.
