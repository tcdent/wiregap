# Kernel-bypass and DMA confinement

> How packets move between the wire and our userspace process with **no kernel network stack in the
> path**, and how the IOMMU bounds the NIC's DMA so "we gave a device direct memory access" does not
> mean "the device can scribble all of RAM." This is the subsystem's reason for being: delete the
> kernel TCP/IP stack (a large, box-reachable parser) while keeping the IOMMU (a confinement the
> hardware provides).
>
> Grounded in `reference/ixy` (`libixy-vfio.c`, `memory.c`, `pci.c`) and the kernel docs
> (`reference/linux/Documentation/{driver-api/vfio.rst, userspace-api/iommufd.rst}`).

## 1. The VFIO control plane (one-time setup, not in the data path)

`vfio_init` (`libixy-vfio.c:223`) performs the full sequence, confirmed against the kernel docs:
find the device's `iommu_group` via sysfs → open `/dev/vfio/vfio` (container) →
`VFIO_GET_API_VERSION` → `VFIO_CHECK_EXTENSION(VFIO_TYPE1_IOMMU)` → open `/dev/vfio/<groupid>` →
`VFIO_GROUP_GET_STATUS` (assert `VIABLE`) → `VFIO_GROUP_SET_CONTAINER` → `VFIO_SET_IOMMU(TYPE1)` →
`VFIO_GROUP_GET_DEVICE_FD` → bus-master enable (one config-space write, `:27`). BAR0 is `mmap`ed via
`VFIO_DEVICE_GET_REGION_INFO` (`vfio_map_region:287`); rings/buffers are mapped with
`VFIO_IOMMU_MAP_DMA` (`vfio_map_dma:299`). All of this is **boot-time/setup, non-attacker-facing**
(the box does not control the PCI bus) and **not in the per-frame path**.

## 2. The confinement guarantee (the load-bearing security property)

The kernel VFIO doc states it directly: VFIO gives *"direct device access to userspace, in a secure,
IOMMU protected environment"* — *"DMA is by far the most critical aspect... allowing a device
read-write access to system memory imposes the greatest risk,"* mitigated because modern IOMMUs
*"isolate [devices] from each other and from arbitrary memory access."* Concretely,
`VFIO_IOMMU_MAP_DMA` bounds the NIC's DMA to exactly the IOVAs we map; unmapped memory is
unreachable by the device. This is the basis for keeping the IOMMU as a confinement while removing
the kernel network stack as a parser.

**Identity mapping, no physical-address leak.** In VFIO mode ixy maps `iova = vaddr`
(`memory.c:110`): the device's view is identity-mapped to our process virtual addresses, and **no
`/proc/self/pagemap` is needed**, so no physical addresses are exposed. The non-VFIO path (`pci.c`
sysfs `resource0` + `virt_to_phys`, `memory.c:23`) requires root and has **no** DMA confinement —
it is the path we reject; `pci.c` documents what we do not do.

**The IOMMU group is the unit of ownership.** The kernel doc: a group is *"a set of devices which is
isolatable from all other devices."* Isolation granularity is the group, not always the single
device — multi-function enclosures may place several functions in one group. The X520-DA2 exposes
**two PCIe functions** (two ports; we light one, leave the other dark). If both functions share an
IOMMU group, we must bind both to `vfio-pci` to own the group — fine, since we own the card. The
group composition is a platform fact to confirm at the bench (§5).

**The X520 firmware is not trusted — it is contained; its trustworthiness is immaterial.** (Posture:
`architecture.md §2.1`.) Every other member of this subsystem's trusted surface is either tiny-and-ours
(the driver, reviewed by reading; `driver-minimality.md`) or off-surface (the kernel TCP/IP stack is
deleted; the TLS stack and stock Linux host the box cannot leverage). The X520's on-chip firmware is
neither: it is **bulky vendor code we cannot read**, and it is **box-reachable** — it populates our
descriptor rings from the wire bytes the box controls (`data-path-and-events.md §1`), and it
manipulates AUTOC during link setup (the LESM finding, `device-bring-up-and-link.md §3`). So it is the
one member that is *neither* reviewable-by-reading *nor* enumerable — and rather than close that gap
with trust (diligence on code we cannot read, the subsystem's old "hope-and-pray" member), we **remove
the need to trust it**: a maximally hostile NIC's only in-scope effect is escalation (bounded below) or
data corruption (caught by CRC + retransmit); everything else is DoS/link-down, out of scope. Its
soundness is therefore immaterial, the box's own logic applied to a device on the trusted host. What we
*do* still record as diligence is the choice of *part* — the 82599 is the reference for userspace
networking, complete public datasheet, long field history — but that diligence now backs *functioning*
(will the link work), not *containment* (which is structural, below).

And it is **honest *and* bounded — by two distinct guards, for two distinct vectors.** The diligence
is not the only thing standing between this firmware and the trusted host, but it is important to name
*which* guard covers *which* failure, so neither is silently load-bearing for something it does not
cover:

- **Vector 1 — DMA reach (bounded by the IOMMU).** A misbehaving or compromised firmware that tries to
  DMA *outside* our mapped buffers — to scribble arbitrary host RAM — is blocked: the IOMMU confines
  the device's DMA to exactly our mapped IOVAs (above), so its reach is the memory we handed it, never
  the host's RAM. "Keep the kernel where it bounds the adversary" (the IOMMU) is precisely this.
- **Vector 2 — descriptor content (bounded by the driver, NOT the IOMMU).** The IOMMU does *not* stop
  the firmware from writing malformed or malicious descriptors *into the rings we mapped* — those rings
  are mapped, so the writes are allowed. That residual is carried by the **read-verified driver that
  interprets the descriptors** (`data-path-and-events.md` — the surface "understood completely"): it
  must robustly handle *any* descriptor content the firmware can produce, treating the ring as
  adversary-influenced input. This is exactly *why* the driver being small and reviewed-by-reading is
  load-bearing — it is the guard for the vector the IOMMU leaves open.

So the firmware needs no trust: it is walled on both vectors — the IOMMU bounds where it can reach, and
the read-verified driver bounds what malformed ring content can do. Neither guard is asked to cover the
other's vector. Containment, not soundness, is what carries the firmware — which is why its
trustworthiness can be immaterial.

**The privilege-escalation surface this confinement bounds (scoped) — and where it is not yet grounded.**
The threat we scope here is narrow and specific: the **untrusted box-side NIC**, over the twinax (DAC)
wire, used for **privilege escalation** on the **trusted proxy side** — *not* denial-of-service, *not*
link-down, *not* physical damage, which are explicitly out of scope. Also out of scope: guarding the
trusted (proxy) NIC against *becoming* compromised — we do not try to prevent that; we bound its
*consequence*. The two are the same coin: we can treat the trusted NIC's compromise as out of scope
*because* the IOMMU contains what a rogue NIC can do.

The adversary model is maximal at the wire: the box owns its NIC **down to the firmware**, so it can
drive *arbitrary twinax signaling* — not merely malformed Ethernet frames, but out-of-spec PHY,
abusive link negotiation, hostile framing, content crafted to trip the receiver. Tracing that input up
the proxy receive path, every layer is either out of scope or bounded, and the escalation surface from
the **runtime over-the-wire adversary** converges on **two places**:

- **(A) The driver's handling of box-controlled descriptor/buffer content — vector 2.** In *our* code,
  the prime surface, and the one we have grounded (bound the length, treat status as adversarial, never
  deref an unchecked field, copy out early; `data-path-and-events.md §1`).
- **(B) The IOMMU's containment being *airtight*.** DMA remapping bounding *reach* (vector 1) is
  necessary but not sufficient — it is airtight only if nothing lets the device step *around* it. That
  requires, at least: **interrupt remapping** governing the device's MSI/MSI-X writes (or a rogue NIC
  crafts memory writes to arbitrary addresses via MSI — an escalation, not a DoS); and the device's
  IOMMU-**bypass features disabled** — **ATS** (Address Translation Services: translated requests that
  skip the IOMMU check) and **peer-to-peer DMA**.

Everything else *over the wire* collapses cleanly: the electrical/SerDes and link-negotiation layers
reduce to bit-errors / link-state, i.e. DoS/down (out of scope); and a proxy-NIC firmware compromise
triggered over the wire is bounded by (B), which is why preventing it is out of scope.

**A third surface the posture-flip exposes — config space / boot-time PCI enumeration — explicitly
scoped out, stated not assumed.** Once we declare the NIC *untrusted*, we must account for *every*
channel by which it reaches the trusted host, not just the over-the-wire ones. The kernel's PCI
enumeration reads NIC-controlled **config-space** values (BAR sizes, capability structures) and parses
them in **stock kernel software, before any IOMMU mapping** — so it is **neither over-the-wire nor
IOMMU-covered**, and the (A)+(B) convergence misses it. A maximally hostile NIC could present hostile
config space to trip that parser. Disposition: this is reachable **only by a NIC already compromised at
or before provisioning** — a firmware implant placed via supply-chain or physical access — because the
box, over the wire, cannot alter the proxy NIC's config space. The **runtime** box-over-wire adversary
this subsystem defends does **not** reach it, and the triggering adversary (supply-chain / physical) is
already **out of scope in the parent threat model** (`containment-by-construction.md §2` — physical
access is the locked-door concern). So it sits **outside the runtime threat model** — but it is the one
"NIC untrusted" channel the IOMMU does not cover, so if the provisioning-time / physical-control
assumption ever weakens, this is where it bites first. Recorded, not silently dropped
(`concerns-to-watch.md`). *(Credit: zero.policy flagged this in spine review — the convergence is two
places for the wire, three once the posture is taken at its word.)*

**Grounding status — stated honestly, because the gap is the point.** Vector 2 and DMA-reach (vector 1)
are grounded. The rest of (B) — that interrupt remapping is enabled and governs the NIC's MSIs, and that
ATS and peer-to-peer are disabled/untrusted on the 82599 and in our VFIO/iommufd config — is **asserted,
not verified.** We name it as the attack surface regardless: **this is where a rogue NIC would try to
escape the IOMMU, and we have not yet confirmed it cannot.** Tracked as open in `concerns-to-watch.md`;
until grounded, treat "the IOMMU contains a rogue NIC" as a claim with a known unverified half. Closing
(A)+(B) is not just "verify containment" — it is the work that **deletes the firmware from the trusted
base**, realizing the §2.1 posture (`architecture.md`); it is the highest-leverage grounding in the
subsystem.

### How this posture evolved (do not re-introduce the old one)

Earlier framing treated the firmware as a **trusted member accepted by diligence** — "the reference
part, public datasheet, field history, so we trust it" — recorded as the subsystem's honest soft spot.
That diligence-trust was a quiet **exception to assertion 16** ("reject code you cannot read"), *not*
something 16 endorsed; it was the last hope-and-pray member, trust placed in code we cannot read, an
un-answerable bet. The current posture **removes the exception**: the firmware is *untrusted-but-
contained*, its soundness immaterial, and the only diligence that remains backs the *part choice for
functioning*, not *containment*. So this makes the subsystem *more* 16-compliant, not less — nothing in
the spine changes. Do not re-introduce "the firmware is vetted, so trust it" — the whole point is that
we removed the place where its trustworthiness could matter. Containment (A + B), once grounded, is what
makes that real.

(This also retired the three-trust-bases model from the earlier box-reachability grounding —
read / enumerate / **diligence**. The diligence basis is gone: the trusted base aims for no
hope-and-pray member, only small-and-ours, structural-hardware, or untrusted-contained.)

## 3. The kernel-free hot path

Once set up, the per-frame path touches **no kernel code**: packets DMA straight into our mapped
buffers; we poll the descriptor's `DD` bit in host memory (no MMIO) and write the tail register as a
direct CPU store to the `mmap`ed BAR. The QEMU/VMexit model does **not** apply — this is a bare-metal
userspace process, not a guest, so the tail-write does not trap. This is what makes the
"kernel-bypassed link, single-digit-µs RTT" claim exactly (not approximately) true, and it is why the
kernel TCP/IP stack — the prior largest by-assumption surface — is gone from the data path entirely.
What remains of the kernel is boot-time PCI enumeration + the IOMMU: one-time, non-adversarial, and a
confinement rather than a surface.

## 4. iommufd / cdev — the API direction (build-time, not a security difference)

ixy uses the **legacy** group/container ABI. The kernel docs name the forward direction: **IOMMUFD**
(`/dev/iommu`) replaces the container with an **IOAS** (I/O Address Space) holding the IOVA map, and
the **VFIO device cdev** interface (`/dev/vfio/devices/vfioX` + `VFIO_DEVICE_BIND_IOMMUFD`) replaces
the group→`GET_DEVICE_FD` dance with device-centric binding. *"Long term, VFIO users should migrate
to device access through the cdev interface ... and native access through the IOMMUFD provided
interfaces."* Backwards-compat exists (`CONFIG_IOMMUFD_VFIO_CONTAINER`, or symlinking
`/dev/vfio/vfio`).

**Crucially, the confinement guarantee is identical** under legacy TYPE1 and iommufd — both bound
DMA to mapped IOVAs through the IOMMU. The choice is API ergonomics and future-proofing, **not** a
security-property difference. New code should target **iommufd/cdev**; the architecture is unchanged
either way.

## 5. Bench-gated / build-time open items (this topic)

1. **IOMMU group composition of the X520-DA2** — does our port sit in its own group, or share with
   the second (dark) function? Determines whether we bind one or both functions to `vfio-pci`.
   (Security-neutral: we own the whole card regardless.)
2. **iommufd/cdev vs legacy** is a build choice to settle at implementation; both give identical
   confinement. No design question rides on it.
3. **Confirm the IOMMU actually bounds DMA** on the bench (a deliberately out-of-range device access
   is blocked) and **zero syscalls in the per-frame loop** under steady-state RX/TX
   (`transport-stack.md §4.2`).
