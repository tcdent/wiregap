# The Terminal Subsystem

### A greenfield replacement for the terminal/multiplexer stack

> An exploration document for the proxy's terminal subsystem — the channel that carries a
> contained agent's screen to the operator and the operator's keystrokes back. Companion to the
> cloud ([`cloud/approach.md`](./cloud/approach.md)) and filesystem ([`filesystem.md`](./filesystem.md))
> subsystem docs and the foundational *principles*. Unlike those, this subsystem has **no
> proxy-side gate**: its guarantee is a property of how the wire protocol and the head are
> *designed*, not a check the proxy performs at runtime. The document is therefore an overview —
> of the components, their roles and delineations, and the level of scrutiny each warrants.

---

## 0. Replace the stack, don't secure it

PTY/TTY is a six-decade accretion no one can fully specify — janky, in-band, reflective, and
directly hostile to the posture. The design does not try to *secure* it. It **replaces the whole
stack**: a greenfield wire protocol for displaying and interacting with terminal applications,
plus a head that replaces the multiplexer (tmux).

Security is not bolted onto this — it **falls out of the modernization**. The wire is built so
that *nothing harmful can be represented on it*: it carries display state (a grid of cells,
text, styles) and a deliberate, limited control vocabulary, and it has no slot for an escape
sequence, an in-band command, or a reflected query. The threat that haunts every terminal —
output bytes interpreted as control — is not *filtered*; it is **uninstantiable**, because there
is no interpreter on our side and no control on the wire (assertion 12's middle mode: *define a
channel in which the bad thing cannot be expressed*).

The same move solves a product problem and a security problem at once. A from-scratch head gives
the operator flexible pane arrangement and control without fighting a legacy multiplexer; and
*not* carrying legacy terminal formats means there is no legacy format on the wire to attack.
Dropping legacy support is the product simplification **and** the guarantee — the inverse of
"the convenience is the attack surface."

---

## 1. Conversion, not sanitization

Legacy apps still need to run, and they do — against a **PTY emulator on the box** that resolves
their terminal output and **emits our proprietary data structure** instead of PTY bytes. The app
interacts with an emulated PTY and never knows the difference; the emulator absorbs all the jank;
what crosses the wire is clean, bounded, control-free structure.

So the operation is not *sanitizing* a dangerous stream — there is no dangerous stream downstream
of the box. It is **converting a subset of PTY into our protocol**. Anything the protocol cannot
represent simply does not cross — not stripped, but unrepresentable. We *run* legacy apps without
ever *dealing with* them past the box.

---

## 2. The guarantee lives at two design loci, not a gate

Where internet has the route recognizer and filesystem has the leaf-validator — total functions
in the box-reachable security core — **terminal has no runtime gate.** Its guarantee is two
*structural design* properties, each at its own locus (terraces, not a filter):

1. **The wire protocol cannot represent control or harm.** Its inexpressiveness *is* the
   security. This is the protocol effort's domain, not this subsystem's; the governing ethos —
   *"don't let anything harmful ever be represented on the wire"* — is its rule.
2. **The head has no control interpretation.** It renders the grid and paints glyphs; it
   *emulates* nothing. This is a maintained design invariant of the head, not a runtime check —
   a future feature that interpreted control on the wire would breach it, so "renders, never
   interprets" is the property reviewed.

Either property alone would close the threat; together they are belt-and-suspenders at two
loci. Neither is a proxy check. The proxy's role in between is a **memory-safe relay**.

---

## 3. The components, their roles, and their scrutiny

Four components across three trust zones:

| Component | Role | Zone | Scrutiny |
|---|---|---|---|
| **Box-side PTY emulator** | run legacy apps; resolve their PTY output; emit our data structure | untrusted (box) | **None.** A compromise here is a non-event — it can only emit (mis)representations of a screen, which cannot act. Loose code is fine. |
| **The wire protocol** | the channel — carry display state + a deliberate control vocabulary, and nothing else | the box↔proxy↔head wire | **Highest; carries the guarantee.** Reviewed for *inexpressiveness* — no representation for control/escape/reflection. Defined by the protocol effort (the other agent's domain). |
| **The head** | render the grid; multiplex panes; capture input — the operator's window, the tmux replacement | trusted (operator's machine), box-reachable | **Two things:** its design invariant *"renders, never interprets"* (security, reviewed), and its **frame parser**, which consumes box bytes → **memory-safe** (box-reachable, built to the bar). Its renderer/multiplexer is feature logic — a bug is a misread or a crash, never an interpretation. |
| **Proxy terminal relay** | move display frames box→head and input head→box | trusted (proxy), box-reachable | **Memory-safety only.** A relay, not a gate — it performs no check, because the protocol already cannot express harm. Must not be exploitable by box bytes; there is no security *logic* in it to review. |

The shape worth noticing: the highest-scrutiny element is a **protocol property**, the second is
a **head design invariant**, and the proxy's own code is **mere memory-safe plumbing**. This
subsystem carries a real guarantee with *zero* proxy-side security logic.

---

## 4. The direction asymmetry, restated

The same trust-direction binary as the other subsystems, but here both directions resolve to "no
gate":

- **Display (box → head, onto trusted ground)** would be the dangerous direction — but it is made
  safe by the protocol's inexpressiveness and the head's lack of an interpreter, not by a gate.
  The worst the box can do is make the screen *show* something misleading — a misread, openly
  accepted as legibility, not security; it can never make the head *act*.
- **Input (head → box, onto untrusted ground)** is the safe direction. Operator keystrokes and
  resize flow into the contained box — well-formed (text and resize as distinct message types),
  unsanitized, because the box may be sent anything. Off the box-reachable surface entirely.

And **no reflection:** the channels are split and one-directional, the head generates no
responses, and there is no query frame — so display content can never loop back into the input
stream as if typed (the classic terminal vulnerability, designed out).

---

## 5. Scrutiny tiers — a taxonomy that generalizes

Laying these components against their scrutiny levels surfaces a taxonomy that is not specific to
terminal; it runs through every subsystem and is worth stating once. The organizing rule: **the
trusted base carries no hope-and-pray member** — nothing trusted merely because we examined it and
judged it probably benign. Every member is one of these, sorted by *what it can do with box bytes*
and *how its safety is assured*, never by where its code runs:

1. **Security-invariant core** — box-reachable code (or a design property) that decides *what
   crosses* or *how it is represented*. Total functions reviewed by enumeration, held to the
   reviewability budget; the highest scrutiny. *(Internet: the route recognizer. Filesystem: the
   leaf-validator. Terminal: the protocol's inexpressiveness + the head's no-interpreter
   invariant — design properties, not proxy checks.)*
2. **Box-reachable supporting code (small-and-ours)** — parses box bytes but does not gate (the
   driver's descriptor-validating and parsing code, the protocol parser, the proxy relay, the
   head's frame parser). Scrutinized for **memory safety** (no privilege escalation), reviewed by
   reading — not for security logic.
3. **Off-surface feature logic** — handles only already-safe bytes (the head's renderer, the
   quarantine writer, the cloud originator). A bug fails the *feature*, never containment.
4. **Untrusted-contained** — components whose *trustworthiness is made immaterial*: we depend on
   them to *function*, never to *behave* (assertions 4–5 applied to a device, not just to the box).
   The box, bounded by structural egress-absence; **and the X520 NIC and its firmware** — bulky
   vendor code we cannot read. Its hostile reach splits two ways: the adversary-written
   descriptor/buffer **content is contained** by the descriptor-validating driver (bound the
   reported length, treat status as hostile, copy out early), and corruption is caught by hardware
   CRC + retransmit — but its device **DMA reach is *owned*, not contained.** The subsystem runs
   pure-direct with **no IOMMU**, so nothing hardware-bounds where the device can DMA; the
   operator's ratified posture is to *own* that residual rather than route containment through the
   kernel-configured IOMMU trust the spine excludes (`nic/architecture.md §2.1`). We ask, per
   vector, whether it is *contained* or *owned* — a checkable question — never whether the firmware
   is benign. **Scrutiny of the containment, and an honest account of what is owned — not of the
   component.**

What makes the *contained* vector sound is the small-and-ours driver reading as if from an
adversary — copy descriptor content out early, never deref an unchecked field; the one remaining
**structural-hardware** floor is the physical point-to-point link (the device talks to exactly one
peer), not an IOMMU. The safe surface *begins at that treat-as-hostile read*, and everything from
there up is small-and-ours or a design property. There is **no rung where we trust bulky code we
cannot read** — the old "firmware, trusted by diligence" was exactly that rung, and containing the
content vector removes it. The DMA-reach vector is the one thing that resolves to *owned*, not to a
floor.

Honesty: this posture is **committed but not yet fully ratified.** The content-vector guard (a
driver robust to *any* descriptor content) is verified by reading; the **DMA-reach vector is owned,
not contained** — whether the threat model tolerates un-contained firmware DMA on the trusted side
is the **policy layer's call, audited after the PoC**, owned-and-committed until then. That, plus a
config-space / provisioning surface scoped to physical and supply-chain, is tracked in the NIC
subsystem's concerns-to-watch — not assumed away.

The terminal subsystem is the clearest illustration that the categories are about *capability over
box bytes and assured containment*, not location: its highest-scrutiny elements are a protocol
property and a head design invariant, its proxy code is plumbing, the firmware one hop below is
untrusted (its content vector contained, its DMA-reach owned), and the operator's *own machine* runs box-reachable code (the head's
parser) that must therefore meet the memory-safety bar. This taxonomy belongs, in full, in the
app-composition view, where every subsystem's components are placed against it at once.

---

## Summary

The terminal subsystem replaces the terminal/multiplexer stack with a greenfield wire protocol
and a from-scratch head. Security falls out of the replacement: the wire **cannot represent
anything harmful**, so the escape-interpretation threat is uninstantiable, not filtered. There is
**no proxy-side gate** — the proxy is a memory-safe relay, and the guarantee lives at two design
loci: the protocol's inexpressiveness (the protocol effort's domain) and the head's "renders,
never interprets" invariant. Legacy apps run against a box-side PTY emulator that emits our
structure: we run them without ever dealing with them.
