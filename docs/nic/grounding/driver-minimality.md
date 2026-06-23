# Design principle: minimal driver surface — run on documented defaults

> The governing principle for how much of the device the driver touches. It emerged while
> grounding the AUTOC/SFI link setup (see `device-bring-up-and-link.md §3`) and decides, for every register,
> whether we write it or leave it alone. It is the register-level expression of the spine's
> "keep the trusted base small and verifiable" and the box-reachability test.
>
> Stated by the operator, 2026-06-20. This is a design decision the eventual formal `docs/nic/`
> definition must carry as a first-class assertion.

## The principle

**Every register the driver writes is a register the driver owns and a reader must understand.**
Writing is not free; it is surface. So the driver runs on the device's **documented defaults** in
as many cases as possible, and writes a register only when the write earns its cost.

> "We want to run with defaults in as many cases as possible because those are well-documented
> defaults and we do not need to reassert them. That's just a waste of mental and physical energy."

Reasserting a register to *confirm* it holds its default value is the specific anti-pattern this
rejects — it buys nothing and adds surface:

> "The idea of having to read default registers and then rewrite the default registers to verify
> that the value is exactly what they are should only be employed if we define or decide that
> there's a security risk with a particular register."

## The sorting rule — write a register only if

1. **It is load-bearing for a security property.** The registers that bound the adversary or carry
   the containment/integrity guarantees: the IOMMU/DMA mapping (confinement), `SRRCTL.DROP_EN`
   (overflow drops rather than buffers, folding into the retransmit path), the hardware CRC enables
   (integrity), the ring base/length/bounds. We own these because they *are* the guarantee.
2. **The datasheet documents it as required for our configuration.** The §4.6 mandatory init steps;
   the "required when not using DCB/VTd" writes (`DTXMXSZRQ`, `RTTDCS.ARBDIS`); the §4.6.7-tail
   quirk bits (`CTRL_EXT.NS_DIS`, the `DCA_RXCTRL` reserved bit). These are required writes, not
   reassertions.
3. **The default is actually wrong for our use** and the feature is one we need.

**Otherwise: run on the documented default.** A documented default is an *anchor* — checkable once
at the bench, stable, and (critically) **set at provisioning time in board NVM, which the box
cannot reach.** A value the adversary cannot influence and whose misconfiguration produces a
*visible feature failure* (e.g. no link) rather than a containment breach is not a security
register, and reasserting it only enlarges the surface.

## The bench read-back is diligence, not driver code

The explore docs' "read back NVM-configured registers before any writes"
(`transport-stack.md §4.1.2`) is **one-time diligence to understand the defaults and confirm our
mental model** — performed once on real silicon, not baked into the driver as reassertion. It tells
us which writes are redundant so we can *remove* them; it is not a license to write more.

## Why this is the same idea as the security spine

The box-reachability test asks "can the box leverage this?" to decide what must be impenetrable.
This principle is its register-level twin: a register the box cannot influence, whose failure is
cosmetic/feature-level, owes the driver no write. We spend our finite "hold it in one sitting"
budget on the registers that bound the adversary and the writes the hardware genuinely requires —
and nowhere else. Minimizing the surface *is* the guarantee, because a driver small enough to fully
reason about is the only kind whose correctness is not a bet.
