# Absolute Zero Trust — Foundational Assertions

> The regrounding spine for the system: the smallest set of assertions from which the
> rest of the design follows, each stated positively. Where an assertion turns on rejecting
> a common term or framing, the reasoning lives in an appendix rather than the assertion. A
> fresh reader — human or agent — should be able to reground in the design's intent from this
> page alone. Companion to *Containment by Construction* and the transport/console design
> documents, which expand the mechanisms these assertions govern.

---

## What we are accomplishing

1. **The operator is answerable.** The operator — the human — is unconditionally answerable
   for every action the system enacts, whether they approved it or not. The whole design is
   derived from this single fact.

2. **The job.** The system ensures the set of acts it can enact never exceeds the set of acts
   the operator has approved. The acts that must be prevented are the ones the operator would
   answer for without having chosen them.

3. **The agent is a book, not an actor.** The agent is protected expression — a dynamically
   generated book. Authoring content is not an act; acting on it is, and the act has exactly
   one accountable author: the operator. The system is the technical embodiment of the line
   between protected expression and accountable action.

## Posture — absolute zero trust

4. **Trust is zero and irrelevant.** The agent is never authenticated, verified, or granted
   authority. The design makes its trustworthiness immaterial — not by distrusting it, but by
   removing every place where its trustworthiness could matter.

5. **Possession is not capability.** The agent knows freely and acts not at all beyond its
   boundary. It may read any injection, collect any credential, hold a live secret — and effect
   nothing, because every dangerous use is structurally unreachable.

## The defended invariant

6. **No reachable execution.** The box causes no execution with effect beyond its boundary.
   Inside its boundary it executes without limit; that is a non-event. The line is
   *externally-effective* execution, never execution as such.

7. **Effect is defended; information is not.** The box egresses whatever it wants. It holds no
   secret it did not collect itself, and the operator can revoke any credential it gathers.
   Disclosure is not a harm in this system; effect is the only harm.

## The boundary — one line, three faces

8. **Structural.** Inside the boundary the box has no actuator that reaches past it. Outside is,
   by definition, where its code could have effect.

9. **Constitutional.** Inside the boundary is protected expression. Outside is where expression
   becomes action.

10. **Responsibility.** Inside the boundary nothing can bear consequence. Outside is where an act
    requires an author who can — the operator.

11. **The three faces are one line.** Promotion is the point where they meet and the operator
    stands: protected expression becomes accountable action, contained compute becomes reaching
    effect, and a consequence-blind author hands to a consequence-bearing one — in a single act.

## Mechanism

12. **Guarantees are structural.** The forbidden thing is made unrepresentable or unreachable —
    never caught by inspection, never resting on the agent's restraint. Because the operator's
    answerability is unconditional, a caught act is already an authored one; this is why the
    responsibility face requires the structural face.

13. **The system classifies authorship, not acts.** Identical bytes are forbidden when the box
    causes them and legitimate when the operator chooses them. The only question the system asks
    is who authored the act.

14. **Authorship reaches the box in two forms, both the operator's.** Standing approval by class
    (the egress allowlist) and per-instance review (promotion). The box acts only within
    authorship the operator has already granted; it never authors.

## The trusted core — the proxy

15. **Reviewability is the guarantee.** The proxy is the entire trusted computing base, and here
    the guarantee is that a human understood it completely. It stays small enough for the operator
    to hold in a single sitting — on the order of 500 lines.

16. **Borrow vetted ideas; exclude bulky code.** The trusted path takes decades-vetted concepts
    and rejects code it cannot read. The kernel is excluded for being un-reviewable, not for being
    unsound; the proxy drives a minimal, datasheet-built driver over a kernel-bypassed link, and
    its constructs are verifiable by enumeration — total functions over finite domains.

17. **The proxy enforces that emitted requests adhere to the rules set for the route.**

## The untrusted zone — the box

18. **Rigor follows the boundary, not the code.** The box receives none. Its code is rewritten by
    the adversary in flight, and hardening it buys no containment.

19. **The box carries no internal restrictions.** Its internal freedom is total and its
    cleanliness is an operational matter, because nothing it does inside reaches anything outside.

## Naming

20. **The posture is _absolute zero trust_. The mechanism is _containment by construction_** —
    effect-confinement achieved by capability-absence.

21. **The design is a synthesis, not an invention.** It composes established lineages — confinement
    and capability security on the technical side, evolved constitutional principle on the legal
    side. Its distinctive move is to defend effect alone and discard confidentiality.

---

## Appendix A — Why not "zero trust," the industry term

Industry zero trust ("never trust, always verify"; NIST 800-207, BeyondCorp) is continuous
re-verification. Trust is minimized and re-priced per request, but every permitted action is still
a *granted* trust that verification resolved to positive — it is a sliding scale, zero only as a
default posture. This system sets trust to zero permanently and makes it irrelevant: the agent is
never granted anything because the design removes the need. "Absolute" marks that the posture is
not a point on the industry scale but off it. The term "zero trust" is kept because the contrast —
the moment a security practitioner says "that is not what zero trust means" — is the teaching moment.

## Appendix B — Why not "air gap" or "data diode"

Both describe information-flow direction: an air gap permits no channel, a data diode permits flow
in one direction only. Both invite a defender to protect confidentiality — to stop data leaving.
This system protects effect, not information; the box egresses freely. Adopting either term
re-imports the wrong invariant, so they are avoided even where the physical intuition overlaps (a
single constrained aperture; a display path with no return wire).

## Appendix C — Lineage references

- **Confinement** — Butler Lampson, "A Note on the Confinement Problem" (1973): running an
  untrusted program so it cannot affect or leak beyond its authorized scope. This system adopts the
  structural goal and narrows it to *effect*, deliberately discarding the leakage concern that is
  confinement's usual center of gravity.
- **Capability / object-capability security** — authority flows only through unforgeable
  references; there is no ambient authority. The box holds the null capability toward any reaching
  execution: full ambient authority inside its boundary, none crossing it.
- **Constitutional grounding** — the citizen is accountable for the executed actions of systems
  they own and operate; information and expression are protected; the actor bears responsibility.
  The architecture locates accountability where the law already places it, rather than inventing a
  new locus for it.
