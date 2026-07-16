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

## Whether the IOMMU actually contains a rogue NIC (the airtightness half)

The privilege-escalation surface from the untrusted box NIC over the wire converges on two places
(`grounding/kernel-bypass-and-dma.md §2`): the driver's vector-2 handling (grounded), and the IOMMU
being *airtight*. DMA-reach remapping is grounded; the rest of airtightness is **asserted, not
verified.** We state it as the attack surface anyway — this is where a rogue NIC would try to escape:

- **Interrupt remapping.** We suspect that without IOMMU interrupt remapping governing the NIC's
  MSI/MSI-X writes, a rogue NIC could craft memory writes to arbitrary addresses (an escalation, not a
  DoS). Unverified: that it is enabled and effective in our platform + VFIO/iommufd config.

- **ATS (Address Translation Services).** We suspect that if the 82599 supports ATS and it is enabled, a
  rogue NIC could present pre-translated requests that skip the IOMMU check. Unverified: whether the part
  supports it, whether it is off, and whether our config forces it off.

- **Peer-to-peer DMA.** We suspect device-to-device DMA could be a path around the memory-confinement
  intent. Unverified: whether it is reachable/relevant in our single-NIC, IOMMU-on topology.

These together are the **unverified half of "the IOMMU contains a rogue NIC."** Until grounded against
the VFIO/IOMMU docs and the datasheet, that sentence is a claim, not a fact.

The stakes are higher than verification: this (with vector 2) is the containment that lets the firmware
be **untrusted-but-contained** rather than trusted-by-diligence (`architecture.md §2.1`). Grounding it
is what **deletes the firmware from the trusted base** and eliminates the subsystem's last hope-and-pray
member. Until then, that posture is committed but not yet fact — the gap here is exactly what stands
between "almost" and "done."

**A third channel, explicitly scoped out — kept here so the scope-out stays visible.** Taking "the NIC
is untrusted" at its word exposes a surface neither over-the-wire nor IOMMU-covered: **boot-time PCI
config-space enumeration** (stock kernel parses NIC-controlled BAR/capability values before any IOMMU
mapping; `grounding/kernel-bypass-and-dma.md §2`). We scope it **out of the runtime threat model**
because it is reachable only by a NIC compromised at/before provisioning (supply-chain / physical) —
which the parent threat model already excludes — not by the runtime box-over-wire adversary. This is a
*stated* scope-out, not a verification: the concern to watch is whether that provisioning-time /
physical-control assumption ever weakens, because this is the one "NIC untrusted" channel the IOMMU
does not bound.

## DMAR now ours + confined in QEMU — the residuals that need silicon (Sprint 5, 2026-07-16)

Sprint 5 wrote and owns the VT-d DMAR in the substrate and demonstrated the confinement **in QEMU
`intel-iommu`** — the mechanism, not silicon. What QEMU proved has moved into the design docs
(`grounding/kernel-bypass-and-dma.md §2`, pending ratification): the DMA-reach confinement is realized,
and the **(ii) BLOCKED** proof landed — an induced out-of-arena device DMA faults at the IOMMU (FRCD IOVA
== the induced address, source-id == the NIC) while the in-arena DMA succeeds. What QEMU **cannot**
discharge stays open here, on purpose:

- **(iii) SOUND — the setup's real-silicon soundness.** QEMU validates our table construction + the
  enable ordering (`RTADDR`+`SRTP` → context-cache + IOTLB invalidate → `TE`) *as emulated*. Unanalyzed:
  whether the same tables + sequence confine on the **real platform VT-d + the real 82599** — cache/IOTLB
  behaviour, errata, the actual DRHD topology. A QEMU-green fault **corroborates**; it does not discharge.
  Residual: the confinement is proven-in-QEMU, not proven-on-silicon.

- **Rogue-MSI blocked (the interrupt-remapping (ii)).** We wrote the IRT (source-validated against the
  NIC's SID) and enabled interrupt remapping — its **(i)-source** verdict. But "a rogue NIC MSI to an
  arbitrary address is **blocked**" is **un-inducible in QEMU**: a well-behaved emulated device fires no
  rogue MSI. Unverified: that the IRT's source-validation actually *stops* a crafted MSI on silicon. Do
  not read "we wrote+enabled the IRT" as "interrupt remapping is airtight."

- **82599 ATS-advertise (the ATS-off datasheet half).** Our config does not enable ATS (context
  translation-type = 0) — the **(i)-source** verdict, with bypass-blocked *inferred*. Unverified: whether
  the real 82599 **advertises** ATS and whether the platform would honour a pre-translated request — the
  datasheet + bench half, Sprint 6.

These are the honest residuals of "the IOMMU contains a rogue NIC": DMA-reach is now grounded
(proven-in-QEMU) and the airtightness items carry their **(i)-source** verdicts, but the **(ii)/(iii)**
and silicon halves above are **not closed**. Until the bench card, "contained on silicon" remains a
claim, not a fact — the gap that still stands between proven-in-QEMU and proven.

## Why this file exists

These are real, and they are also exactly the kind of thing that *looks* handled the moment it is written
down. This list's only job is to keep them visibly **un**-handled until the work is actually done — so that
the next person to read the subsystem (including us, later) inherits the open question, not a false sense
that someone already settled it.
