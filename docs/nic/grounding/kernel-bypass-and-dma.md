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

## 4. The kernel-free interaction — bypassed stack, and polling (not interrupts)

Once set up, the per-frame path touches no kernel: we poll the descriptor's `DD` bit in host memory (no
MMIO) and write the tail register as a direct CPU store to the mapped BAR. The kernel network stack is
entirely out of the path (spine assertion 16).

**Interrupts become polling.** The pure-direct path has no kernel mechanism to deliver an MSI-X interrupt
to our process — that would require UIO or VFIO, the kernel tooling this model rejects. So the idle-wakeup
is **bounded polling**, not an eventfd/epoll wait: the reactor's poll-then-block holds, with "block"
realized as a bounded poll / short sleep rather than a delivered interrupt. This is affordable at the
subsystem's real demand (hundreds of frames/sec against ~200k/s single-session headroom); it replaces the
interrupt→eventfd→epoll idle-wakeup of the earlier (VFIO/bare-metal) drafts. The reactor *algorithm* is
unchanged; only its wakeup primitive is.

## 5. Bench-gated / build-time open items (this topic)

1. **The direct path on the real 82599** — unbind, bus-master enable, `resource0` `mmap`, and hugepage DMA
   with `pagemap`-resolved physical addresses, on the bench card.
2. **Polling idle-wakeup** — confirm the bounded poll / short-sleep loop meets the latency and CPU budget
   at the real frame rates, and that the retransmit deadline composes into it (was the `epoll` timeout).
3. **The DMA posture (§2)** — the policy audit (post-PoC), and the `architecture.md §2.1` / spine
   reconciliation that follows from it.
