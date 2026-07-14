# Kernel-bypass and DMA confinement

> How packets move between the wire and our process with **no kernel in the path at all**, and how the
> IOMMU — a confinement **we configure ourselves** — bounds the NIC's DMA so "we gave a device direct
> memory access" does not mean "the device can scribble all of RAM." This is the subsystem's reason for
> being: the kernel is excluded from the trusted base for being un-reviewable (spine assertion 16), so we
> run on the bare metal, own the hardware, and configure the IOMMU in our own reviewable code — no kernel
> to trust for the confinement, and no kernel network stack (a large, box-reachable parser) in the path.
>
> Grounded in the Intel VT-d spec (DMA-remapping / DMAR programming), the UEFI spec (the boot-time
> handoff to owned hardware — see `bare-metal-substrate.md`), and `reference/ixy` for the DMA
> ring/buffer data-path concepts (`memory.c`) — **not** its VFIO control plane, which does not apply to
> the bare-metal model.

The runtime model is the operator's ratified direction (2026-07): the trusted core owns the hardware with
no kernel above it and configures the IOMMU itself — no VFIO, no kernel intermediary.

## 1. Owning the device and the IOMMU on the bare-metal substrate (one-time setup, not in the data path)

We enter as a UEFI application, call `ExitBootServices`, and take ownership of the machine — after which
no firmware code is resident above us and there is no kernel. The handoff gives us the two things we need:
the physical **memory map** and the **ACPI table pointers** (MCFG, DMAR). From there the device and the
confinement bring-up are entirely ours (substrate mechanics — ECAM, physical memory — are grounded in
`bare-metal-substrate.md`; this doc owns the confinement):

- **Find the device** via ECAM PCIe config (the ACPI MCFG table gives the config-space base): scan for
  the 82599 (`8086:10FB`), read **BAR0** (the register block — identity-mapped physical memory we touch
  directly), set **Bus-Master-Enable** in the Command register, read the **MSI-X capability**.
- **Configure the IOMMU ourselves** (this doc's core): from the ACPI **DMAR** table, locate the
  DMA-remapping hardware; build the **root table → context table → second-level page tables** in memory
  we own, mapping **only** our rings/buffers' IOVAs to their physical addresses; program the DMAR
  registers (set the root-table pointer, invalidate the context-cache + IOTLB, set Translation Enable);
  and set the airtightness controls (interrupt remapping, ATS-off, no peer-to-peer — §2 (B)).

All of this is **boot-time/setup, non-attacker-facing** (the box does not control the proxy's PCI bus)
and **not in the per-frame path**. The register-level DMAR sequence is datasheet-built against the VT-d
spec, exactly as the NIC bring-up is built against the 82599 datasheet (§4.6).

## 2. The confinement guarantee (the load-bearing security property)

The IOMMU bounds the NIC's DMA to exactly the IOVAs our page tables map; every unmapped IOVA faults, so
unmapped memory is unreachable by the device. **The correctness of that confinement rests on our own
reviewable code** — the DMAR tables we build and the enable sequence we run — not on a kernel we cannot
review. This is what lets the IOMMU remain a confinement while we run with no kernel at all: the one
confinement we depend on is one we configure and can read.

**One address space, no physical-address leak.** We own physical memory directly (the substrate's arena
over the UEFI memory map) and **identity-map** it — CPU virtual = physical = IOVA — so there is a single
address space to reason about across the CPU, the NIC, and the IOMMU. There is no `/proc/self/pagemap`
and no physical-address-exposure question; those are artifacts of a userspace-under-Linux driver, which
this is not.

**We own the whole IOMMU.** "IOMMU groups" are a kernel abstraction for sharing one IOMMU among mutually
distrusting owners; with no kernel, that sharing does not exist — we own the IOMMU wholesale. The
hardware fact that survives is that the X520-DA2 exposes **two PCIe functions** (two ports; we light one,
leave the other dark): our DMAR **context-table** setup covers both functions, giving the dark function
no mapping (or leaving it disabled) so it cannot DMA. A platform fact to confirm at the bench (§4).

**The X520 firmware is not trusted — it is contained; its trustworthiness is immaterial.** (Posture:
`architecture.md §2.1`.) Every other member of this subsystem's trusted surface is either tiny-and-ours
(the driver and the substrate, reviewed by reading; `driver-minimality.md`) or off-surface (there is no
kernel and so no kernel network stack — the prior largest by-assumption parser simply does not exist).
The X520's on-chip firmware is neither: it is **bulky vendor code we cannot read**, and it is
**box-reachable** — it populates our descriptor rings from the wire bytes the box controls
(`data-path-and-events.md §1`), and it manipulates AUTOC during link setup (the LESM finding,
`device-bring-up-and-link.md §3`). So it is the one member that is *neither* reviewable-by-reading *nor*
enumerable — and rather than close that gap with trust (diligence on code we cannot read, the subsystem's
old "hope-and-pray" member), we **remove the need to trust it**: a maximally hostile NIC's only in-scope
effect is escalation (bounded below) or data corruption (caught by CRC + retransmit); everything else is
DoS/link-down, out of scope. Its soundness is therefore immaterial, the box's own logic applied to a
device on the trusted host. What we *do* still record as diligence is the choice of *part* — the 82599 is
the reference for userspace networking, complete public datasheet, long field history — but that
diligence now backs *functioning* (will the link work), not *containment* (which is structural, below).

And it is **honest *and* bounded — by two distinct guards, for two distinct vectors.** The confinement is
not the only thing standing between this firmware and the trusted host, but it is important to name
*which* guard covers *which* failure, so neither is silently load-bearing for something it does not cover:

- **Vector 1 — DMA reach (bounded by the IOMMU we configure).** A misbehaving or compromised firmware
  that tries to DMA *outside* our mapped buffers — to scribble arbitrary host RAM — is blocked: our DMAR
  tables confine the device's DMA to exactly our mapped IOVAs (above), so its reach is the memory we
  handed it, never the host's RAM. The confinement the design leans on is precisely this, and it is now
  ours to build and read.
- **Vector 2 — descriptor content (bounded by the driver, NOT the IOMMU).** The IOMMU does *not* stop the
  firmware from writing malformed or malicious descriptors *into the rings we mapped* — those rings are
  mapped, so the writes are allowed. That residual is carried by the **read-verified driver that
  interprets the descriptors** (`data-path-and-events.md` — the surface "understood completely"): it must
  robustly handle *any* descriptor content the firmware can produce, treating the ring as
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
drive *arbitrary twinax signaling* — not merely malformed Ethernet frames, but out-of-spec PHY, abusive
link negotiation, hostile framing, content crafted to trip the receiver. Tracing that input up the proxy
receive path, every layer is either out of scope or bounded, and the escalation surface from the
**runtime over-the-wire adversary** converges on **two places**:

- **(A) The driver's handling of box-controlled descriptor/buffer content — vector 2.** In *our* code,
  the prime surface, and the one we have grounded (bound the length, treat status as adversarial, never
  deref an unchecked field, copy out early; `data-path-and-events.md §1`).
- **(B) The IOMMU's containment being *airtight*.** DMA remapping bounding *reach* (vector 1) is
  necessary but not sufficient — it is airtight only if nothing lets the device step *around* it. That
  requires, at least: **interrupt remapping** governing the device's MSI/MSI-X writes (or a rogue NIC
  crafts memory writes to arbitrary addresses via MSI — an escalation, not a DoS); and the device's
  IOMMU-**bypass features disabled** — **ATS** (Address Translation Services: translated requests that
  skip the IOMMU check) and **peer-to-peer DMA**. In the bare-metal model **every one of these is a
  control we set in our own DMAR configuration** — not a platform default we inherit and hope for.

Everything else *over the wire* collapses cleanly: the electrical/SerDes and link-negotiation layers
reduce to bit-errors / link-state, i.e. DoS/down (out of scope); and a proxy-NIC firmware compromise
triggered over the wire is bounded by (B), which is why preventing it is out of scope.

**A third surface the posture-flip exposes — config space / boot-time PCI enumeration — explicitly scoped
out, stated not assumed.** Once we declare the NIC *untrusted*, we must account for *every* channel by
which it reaches the trusted host, not just the over-the-wire ones. **Our own** PCIe config scan reads
NIC-controlled **config-space** values (BAR sizes, capability structures) **before the IOMMU is
configured** — so it is **neither over-the-wire nor IOMMU-covered**, and the (A)+(B) convergence misses
it. A maximally hostile NIC could present hostile config space to trip that parser. Two things bound it.
First, the parser is now **ours and tiny** — a handful of field reads (vendor/device ID, BAR0, Command,
the MSI-X capability) in reviewed code, not a large stock-kernel enumerator — so the surface is far
smaller than it was under a kernel. Second, and dispositive: this is reachable **only by a NIC already
compromised at or before provisioning** — a firmware implant placed via supply-chain or physical access —
because the box, over the wire, cannot alter the proxy NIC's config space. The **runtime** box-over-wire
adversary this subsystem defends does **not** reach it, and the triggering adversary (supply-chain /
physical) is already **out of scope in the parent threat model** (`containment-by-construction.md §2` —
physical access is the locked-door concern). So it sits **outside the runtime threat model** — but it is
the one "NIC untrusted" channel the IOMMU does not cover, so if the provisioning-time / physical-control
assumption ever weakens, this is where it bites first. Recorded, not silently dropped
(`concerns-to-watch.md`). *(Credit: zero.policy flagged this in spine review — the convergence is two
places for the wire, three once the posture is taken at its word.)*

**Grounding status — stated honestly, because the gap is the point.** Vector 2 and DMA-reach (vector 1)
are grounded. The rest of (B) — that interrupt remapping is enabled and governs the NIC's MSIs, and that
ATS and peer-to-peer are disabled on the 82599 and in our topology — is now **code we write and review in
our DMAR configuration**, and must still be **confirmed on silicon** (that our enable sequence takes, and
that a crafted out-of-bounds DMA / crafted MSI is actually blocked). This is a better place to stand than
under a kernel: the airtightness is no longer *asserted about a platform we cannot read* — it is *our
configuration to get right and to verify*. But until confirmed on the bench, "the IOMMU contains a rogue
NIC" remains a claim with a known unverified half. Tracked as open in `concerns-to-watch.md`. Closing
(A)+(B) is not just "verify containment" — it is the work that **deletes the firmware from the trusted
base**, realizing the §2.1 posture (`architecture.md`); it is the highest-leverage grounding in the
subsystem.

### How this posture evolved (do not re-introduce the old one)

Earlier framing treated the firmware as a **trusted member accepted by diligence** — "the reference part,
public datasheet, field history, so we trust it" — recorded as the subsystem's honest soft spot. That
diligence-trust was a quiet **exception to assertion 16** ("reject code you cannot read"), *not* something
16 endorsed; it was the last hope-and-pray member, trust placed in code we cannot read, an un-answerable
bet. The current posture **removes the exception**: the firmware is *untrusted-but-contained*, its
soundness immaterial, and the only diligence that remains backs the *part choice for functioning*, not
*containment*. So this makes the subsystem *more* 16-compliant, not less — nothing in the spine changes.
Do not re-introduce "the firmware is vetted, so trust it" — the whole point is that we removed the place
where its trustworthiness could matter. Containment (A + B), once grounded, is what makes that real.

(This also retired the three-trust-bases model from the earlier box-reachability grounding —
read / enumerate / **diligence**. The diligence basis is gone: the trusted base aims for no hope-and-pray
member, only small-and-ours, structural-hardware, or untrusted-contained.)

## 3. The kernel-free hot path

The per-frame path touches no kernel because **there is no kernel**: packets DMA straight into our mapped
buffers; we poll the descriptor's `DD` bit in host memory (no MMIO) and write the tail register as a
direct CPU store to the identity-mapped BAR. The QEMU/VMexit model does **not** apply — this is bare-metal
code that owns the machine, not a guest, so the tail-write does not trap. This is what makes the
"kernel-bypassed link, single-digit-µs RTT" claim exactly (not approximately) true. The one-time bring-up
is ours (the substrate), and the IOMMU is a confinement we configure — there is no persistent kernel
surface anywhere in the path.

## 4. Bench-gated / build-time open items (this topic)

1. **The X520-DA2's two functions** — confirm the dark function is left unmapped/disabled in our DMAR
   context setup so it cannot DMA. (Security-relevant now that the mapping is ours to get right.)
2. **Confirm the IOMMU actually bounds DMA** on the bench: a deliberately out-of-range device access
   faults rather than reaching host RAM.
3. **Confirm VT-d enables from our code** (the enable sequence takes — Translation-Enable status set) and
   that our DMAR configuration is effective *before* the device is allowed to bus-master.
4. **Confirm the (B) airtightness on silicon** — interrupt remapping is effective (a crafted MSI to a
   non-remapped address is blocked), ATS is off, and peer-to-peer is unreachable in our single-NIC
   topology. Now our configuration to verify, not a platform default to assume.
5. **Confirm the per-frame loop is pure CPU + MMIO** with no privilege transitions or hidden traps
   (`transport-stack.md §4.2`).
