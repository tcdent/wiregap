# Sprint 001 — The App-Composition Architecture Document

> Proxy team, sprint 1. The team produces a single architecture document that holds the
> entire proxy in one coherent view. Drafted by zero.proxy-lead from planning input by
> zero.proxy-dev and zero.proxy-review (2026-07-18); green-lit by zero.proxy-pm.

---

## Scope Boundary

This sprint owns **one deliverable**: the proxy's app-composition architecture document at
`docs/proxy/architecture.md`. It composes and places what the subsystem docs already
establish; it settles nothing they left open.

It explicitly does **not** own:

- **Fixing source docs.** `terminal.md` §5's stale IOMMU exemplar text (see Problem) is
  flagged, not edited — the fix routes to the terminal-doc/nic owner via the PM.
- **Subsystem design work.** The fs open-discipline primitives, the quarantine inertness
  mechanism, the cloud recognizer placement, and the exact recognized cloud shape are open in
  their sources and stay open here (owned by the respective subsystem efforts / design).
- **Spine changes.** Tensions with `principles.md` route to zero.policy, never resolve in-doc.
- **Extending the taxonomy.** The four tiers of `terminal.md` §5 are stated as-is; edge
  classes are carried as placement notes and an open question, not a new tier.
- **Promotion tooling** (out of scope per `filesystem.md` §4) — noted as a boundary, not placed.

## Problem

The proxy's design lives in four exploration documents (`terminal.md`,
`cloud/approach.md`, `cloud/anthropic-api.md`, `filesystem.md`) plus the spine
(`principles.md`). No single document holds the whole proxy at once: the scrutiny-tier
taxonomy that organizes every component is defined inside the terminal doc, which closes by
saying it "belongs, in full, in the app-composition view, where every subsystem's components
are placed against it at once" — a view that does not yet exist. The cross-cutting topics —
configuration, observability, lifecycle, state, process shape — have no home at all.

One concrete contradiction was found independently by both dev and reviewer during
planning, and is now **resolved at the source**: `terminal.md` §5 previously grounded the
NIC's tier-4 containment in "the IOMMU (which caps its DMA reach)", contradicting the
committed `docs/nic/architecture.md` §2.1 (pure-direct, no IOMMU, DMA-reach owned under
policy audit). The designer has since corrected `terminal.md` §5 directly (verified on
disk 2026-07-18): it now states the NIC's reach splits per-vector — content **contained**
by the descriptor-validating driver, DMA-reach **owned**, not contained, no IOMMU — and
names the coexistence explicitly ("Scrutiny of the containment, and an honest account of
what is owned — not of the component"). The composition doc re-derives from `terminal.md`
as current source; no flag-and-route is needed for this item. **Standing routing rule
(PM, 2026-07-18): a contradiction between a proxy doc and another team's committed reality
is fixed by the proxy doc's owner, sourced from the committing team's authority — the
composition doc only ever flags, never corrects in place.**

## Goal

Produce the single architecture document that holds the entire proxy in one coherent view:
every subsystem component placed against the four-tier scrutiny taxonomy, and the
cross-cutting topics addressed with strict labeling discipline (grounded claims cited;
genuinely new ground surfaced as open, never invented).

PM's framing: *"This is the dependency root for everything that follows."*

## Final State

1. `docs/proxy/architecture.md` exists, present-tense, describing the proxy as designed.
2. It restates the four-tier taxonomy once, cited to `terminal.md` §5 as origin.
3. Every component derivable from the subsystem docs (+ the driver/NIC, per principles §16)
   appears in exactly one tier, with a grounding citation; non-obvious fits carry a
   defended placement note.
4. The NIC tier-4 row reflects the committed pure-direct as-built and current
   `terminal.md` §5 text (driver fail-closed identity + descriptor validation + physical
   point-to-point link + hw CRC/retransmit for the content vector **contained**; DMA-reach
   **owned**, not contained, no IOMMU). The row does not force a single label where the
   component legitimately splits per vector (designer's refinement, 2026-07-18) — an
   annotation reading as clean containment across both vectors would recreate the
   trusted-by-diligence rung G3 forbids.
5. The five cross-cutting topics are addressed: state, audit/observability posture, and
   configuration-the-what grounded in citations; lifecycle, process shape, and
   config/observability mechanics carried as named open questions with owner routing.
6. Every decision the sources left open appears as open, correctly attributed; none is
   silently settled.
7. The sprint document carries role retrospectives; the sprint is committed with only its
   intentional files.

## In Scope — ordered slices

**Slice 1 — The component inventory (pre-writing artifact).**
The dev produces the full inventory table, no prose: `Component | Subsystem | Tier |
Capability over box bytes | Grounding (doc + section) | Placement note (where non-obvious)`.
The artifact is the initial, table-only draft of `docs/proxy/architecture.md` itself — the
placement tables phase 2A then builds the prose around; no throwaway sidecar file.
Component granularity is scrutiny-unit granularity, per `terminal.md` §3/§5 precedent (the
head is three rows: invariant / frame parser / renderer).
All grounding reads happen here; contradictions surface here before any can bake into
prose. Gate: **G1 + G2 on the inventory table.** The reviewer independently re-derives the
inventory from source (fresh, re-pinned at review time) and diffs — a derivable component
missing, or a doc component untraceable to source, is a REJECT (G1); and the Tier and
Placement-note columns are checked for placement correctness and boundary-case defense at
this gate (G2), so a mis-tiered component cannot bake into 2A prose.

**Slice 2 — The document.** Phases (risk management, not gate boundaries):
- **2A** — skeleton + taxonomy restatement + per-subsystem placement sections from the
  accepted Slice-1 inventory.
- **2B** — grounded cross-cutting half: state, observability-as-posture, configuration-as-
  register (citations per the planning input).
- **2C** — open cross-cutting half: lifecycle, process shape, configuration mechanics,
  observability mechanics — each a named open question with the fork stated and an owner
  routed, never an invented answer.
- **2D** — dev's own coherence pass: cross-reference integrity, tense/voice, citation
  fidelity self-check.
Gate: full independent acceptance against G1–G6 below.

## Non-Goals

- Do not resolve anything a source marks unsettled — carry it as open with its owner.
- Do not edit any source doc, including the stale `terminal.md` §5 text (flag and route).
- Do not extend or refactor the taxonomy; carry edge cases as notes + an open question.
- Do not re-specify a subsystem's design in the composition view (reviewer's R5: this doc
  PLACES and COMPOSES; deepening a subsystem is a reject-and-route).
- Do not adopt confidentiality/exfiltration framing anywhere (spine §7).
- Do not delete or "correct" a source's fallback/exemplar just because it is stale —
  flag it with the committed contradiction cited.

## Starting Surface

Audit first, in this order:

1. The five sources: `docs/proxy/terminal.md`, `docs/proxy/cloud/approach.md`,
   `docs/proxy/cloud/anthropic-api.md`, `docs/proxy/filesystem.md`, `docs/principles.md`.
2. The committed architecture the composition must not contradict:
   `docs/nic/architecture.md` (esp. §2.1).
3. Cite-through set (ruled in planning): `docs/explore/containment-by-construction.md` and
   `docs/explore/console-and-protocol-design.md` — read for citation fidelity where the
   sources cite them normatively (console §1.1 is load-bearing for the trust-direction
   binary); content scope stays proxy-only.
4. Committed proto crates where doc text and artifact could diverge (dev's R5): cite the
   committed reality, flag the doc gap.

Inventory shape: the six-column table above, seeded during planning by both dev and
reviewer's independent first passes (which already agree on the unambiguous placements and
jointly flag the same boundary cases).

## Method

A document sprint has no build or run log; **the source docs are the anchor and independent
re-derivation replaces the settled run.** The discipline every slice follows:

- **Source-anchored authoring.** Every settled-sounding claim carries a citation to a
  current source; the dev reads the cited line before writing the claim, never quotes from
  memory.
- **Currency over faithfulness.** Completeness-against-source is necessary but not
  sufficient; a stale source faithfully reproduced is a defect. Load-bearing claims are
  diffed against the committed architecture docs, not only against each other (the R-NIC
  lesson).
- **Boundary cases are defended, not asserted.** The three known ones — the cloud
  safe-address representation (tier 1 vs 2), the fs quarantine-inertness floor, the fs
  open-discipline symlink floor — get explicit reasoning from the taxonomy's own sorting
  rule ("what it can do with box bytes and how its safety is assured, never by where its
  code runs"). One that cannot be defended from source becomes an open question, not a
  coin-flip.
- **Labeling discipline on new ground.** For the un-anchored cross-cutting half, the bar is
  not factual correctness (no source can make it checkable) but labeling: proposal vs fact
  vs open fork, every time.
- **Present tense throughout**; no "will/should" drift between aspiration and commitment.

## Acceptance

The reviewer's six gates, verbatim from planning (each independently checkable,
ACCEPT/REJECT):

- **G1 — Inventory completeness by re-derivation diff.** Reviewer independently enumerates
  components from source and diffs against the doc. Missing derivable component = REJECT;
  untraceable component = REJECT (unless legitimately-new cross-cutting, grounded per G5).
- **G2 — Tier placement correctness** against the taxonomy's own sorting rule and the fixed
  source anchors listed in the reviewer's planning input; a placement contradicting an
  explicit source scrutiny label = REJECT.
- **G3 — Zero spine contradictions**, with the named anti-pattern list:
  confidentiality/exfiltration framing; any trusted-by-diligence rung; any proxy-side
  terminal gate; any denylist/catch-known-bad framing on a gate. A contradiction is a
  defect (REJECT); a genuine tension is a routed fork (zero.policy).
- **G4 — Open decisions surfaced, not invented.** The five enumerated source-open items
  (fs open-discipline primitives; fs quarantine mechanism; cloud recognizer placement;
  cloud exact recognized shape; NIC containment residuals) each appear as open, correctly
  attributed. Silently settling any = REJECT.
- **G5 — Cross-cutting coverage with labeling discipline.** All five topics addressed;
  un-cited claim presented as settled = finding; new decision baked in silently =
  REJECT + route.
- **G6 — Consistency with the committed architecture.** No contradiction with committed
  docs; specifically the NIC tier-4 row reflects current `terminal.md` §5 and
  `nic/architecture.md` §2.1 (content contained by the driver; DMA-reach owned, no IOMMU),
  not a single blanket containment label.

## Review Questions

1. **RESOLVED (design, 2026-07-18) — taxonomy edges.** Design keeps the single sorting
   axis; no formal adversary-class tier. Placement notes are sharpened to name adversary
   class (box / upstream / none) as an attribute, not a new tier. Notes suffice; not a
   blocker.
2. **RESOLVED (design, 2026-07-18) — owned-residual representation.** The NIC's tier-4 row
   does not force one label: *contained* and *owned* legitimately coexist per vector on one
   component (content-vector contained by the driver; DMA-reach vector owned, no IOMMU).
   The composition doc's NIC row must show both, not collapse to a single annotation.
3. **RESOLVED (design + PM, 2026-07-18) — stale exemplar routing.** The designer owns and
   has already corrected `terminal.md` §5 directly, sourced from the committed
   `nic/architecture.md`. Standing rule going forward: a contradiction between a proxy doc
   and another team's committed reality is fixed by the proxy doc's owner — the composition
   doc only ever flags, never corrects in place.
4. **Cross-cutting open forks (for the PM/design, non-blocking):** the lifecycle, process
   shape, and mechanics questions the doc will name — routed onward when the doc lands.

## Retrospectives

### zero.proxy-review

**Evidence that changed the approach.** Three moments moved the review, each driven by looking
at the artifact rather than the report:

- *The Slice-1 gate widened from G1 to G1+G2.* The planning draft gated Slice 1 on
  completeness/traceability only (G1). Reading the proposed inventory schema showed the table
  already carries a `Tier` column and a `Placement note` column — so tier-placement correctness
  and boundary-case *defense* were checkable at the artifact, before prose. Gating them only at
  the Slice-2 close would have let a mis-tier or an undefended coin-flip bake into 2A. G2 moved
  to the Slice-1 gate; in the event Slice 1 was clean, but the discipline was the point.
- *Blocker 1 was a citation inversion no read-through would catch.* The §4 "wire, layered"
  prose stated proto sprint 010 "excised the L2 envelope." Verifying against committed
  `proto/library.md` and `model.md` law 1 showed the opposite layer: 010 excised proto's
  *L1-frame* envelope, leaving proto a flat L2 codec that owns *no* envelope. The prose's
  top-line conclusion was right; one historical clause inverted the layer it cited — in the
  exact subsection meant to disambiguate the layers.
- *Blocker 2 was a fork resolved in-doc, exposed by an internal inconsistency.* The TLS-stack
  handling named the `principles.md` §16 "bulky code we cannot read" tension honestly, then
  settled it by narrowing §16 to box-facing code. The tell that this was a spine-scope fork,
  not a placement call, was internal: L11 (the Linux kernel — the same class of bulky untrusted
  trusted-side code) routed to design+policy, while L10 (the TLS stack) routed to design alone.
  The fix re-routed L10 and reframed the reading as a recommended default pending policy. The
  CbC §3.3 grounding (it names the TLS stack "the single biggest dependency," says "TCB
  line-count is not the metric," and prefers a mature lib because a proxy bug fails the feature)
  showed the fork is genuine — CbC's pragmatism against the spine's §15/§16 strictness is
  exactly what policy must reconcile.

**What gating a document sprint taught the method.** A document has no build and no run log, so
the usual "re-derive against a settled post-build run" has no run — the *source docs are the
anchor and independent re-derivation replaces the run.* Three disciplines fell out of that and
should be treated as standing:

1. *Completeness is a re-derivation diff, never a read-through.* Prose fails silently — a
   missing component simply is not there and no compiler flags it. The reviewer builds an
   independent inventory from source first, then diffs; when the dev's granularity was finer and
   source-traceable, it was the reviewer's coarser pass that updated, not a defect.
2. *Currency-of-source is a gate separate from completeness-of-source.* A stale source,
   faithfully re-derived, reproduces the staleness (the `terminal.md` §5 IOMMU exemplar), and a
   citation can invert the thing it cites (Blocker 1). Load-bearing claims get word-for-word
   verification against *committed* source, not against the citing prose.
3. *The two halves of the deliverable are graded differently.* The component→tier inventory is
   past-shaped and anchored (re-derivable). The cross-cutting half is future-shaped and largely
   un-anchored, so it is gated on labeling discipline — proposal vs fact vs routed open fork —
   not on a factual correctness the sources cannot supply. A document can read as authoritative
   while quietly settling a fork it should route; the tell is an internal inconsistency, not a
   surface error.

**What the next proxy sprint inherits.**

- *The §11 ledger (L1–L15) is the live open-questions register* — the dependency root's own
  to-do list. The next sprint picks up its items (recognizer placement L4, config mechanics
  L12, observability mechanics L13, process composition L14, privilege shape L15, and the rest).
- *Two questions are pending upstream, not to be re-derived:* L8 (NIC DMA-reach residual —
  policy, audited after the PoC) and L10 (TLS/§16 — design+policy). Carry them; do not relitigate.
- *The commit-hygiene constraint stands:* the sprint commits selective-hunk (never `git add -A`)
  — only `architecture.md` and this sprint doc, with the `terminal.md` §5 per-vector fix as a
  named commit-dependency, and the unrelated nic/proto working-tree changes left out.
- *The gate discipline transfers by shape:* a future doc built on these sources gets
  re-derivation + currency-check; a code sprint gets the run-log coherence discipline back
  (a build-signature marker, numbers pinned to the change's own post-build run).

### zero.proxy-dev (author)

**What worked, and why it should be kept.** Inventory-before-prose did its job twice over:
the table-only Slice 1 was mechanically diffable (the reviewer re-derived and diffed rather
than judged), and both real contradictions/edges of the sprint surfaced at the table stage —
before any prose could bake them in. The adversary-class attribute (box / upstream / device /
none) proved load-bearing, not decorative: it is what made the TLS-stack and config-loader
edges legible under a single-axis taxonomy. Any future placement work should carry it.

**What evidence changed the plan.** `terminal.md` §5 changed on disk mid-sprint (the
designer's per-vector fix). Composing from my earlier-session read would have been composing
from a stale source; a diff-vs-HEAD plus mtime check before deriving the NIC row is what kept
the doc current. Durable rule: **re-pin source currency at every phase boundary, not once at
sprint start** — sources move while a sprint runs.

**What failed and needed correction (three, each with the rule it taught).**
1. *Provenance clause written from inference, not a read.* I described proto sprint 010 as
   "excised the L2 envelope" — inverted; 010 excised proto's own L1-frame `envelope.rs`
   (`library.md` "Envelope ownership"; `model.md` law 1). I had read the crate roots but not
   the library doc's envelope section before writing a historical clause. Rule: provenance
   and history sentences owe the same read-the-cited-line discipline as design claims — an
   unread citation is a guess wearing a citation.
2. *Prose settled what the ledger routed.* My cloud walk concluded the §16 box-facing reading
   for the TLS stack while routing only the narrow follow-on question — and inconsistently
   with the kernel row, which routed the same class (bulky/unreadable/trusted-host) to
   design + policy. Rule: when a placement note *narrows a spine assertion's scope* to make a
   component fit, the narrowing itself is the policy question — route the reading, not just
   the residual; and check ledger routing consistency by class (same class → same owner).
3. *Prose count contradicted the doc's own table.* §1 said "two design loci" while tier 1
   held four design properties; the 2D full fresh reread caught it. Rule: any proxy-wide
   count or summary in prose must be re-derived from the tables at 2D, never carried from
   narrative memory of the subsystem docs.

**What the next sprint inherits.** (a) The §11 ledger L1–L15 is the routing artifact —
L4–L8 source-open with owners, L9–L15 composition-born; nothing in it is settled. (b) One
commit dependency: the designer's `terminal.md` §5 fix (working-tree) lands before or with
`docs/proxy/architecture.md`. (c) The method line that earned its place: *currency over
faithfulness* — load-bearing claims are diffed against committed architecture docs and
crates, not only against the source docs; that check, not source completeness, is what
caught the one real contradiction this sprint.

### zero.proxy-lead

**What the team-formed plan bought.** Planning input from both dev and reviewer before the
contract existed caught the real contradiction (the `terminal.md` §5 IOMMU staleness) and the
right gate shape (G1+G2 at Slice 1, not just at close) *before* either was baked into a
document I'd have drafted alone from the source reads. Routing the contract itself through
both teammates for ACK — not just handing down a plan — surfaced the missing artifact-location
question (dev's clarification) that the draft had silently left open. Keep forming sprints this
way when the deliverable is genuinely new ground.

**What changed the plan mid-flight, twice.** (1) The designer fixed `terminal.md` §5 directly
between planning and Slice 1 — I verified the fix on disk myself before treating it as settled,
rather than trusting the report, and updated the contract's Problem/Final State/Review-Questions
sections before routing Slice 1, since the premise changed before work started. (2) The
zero.policy ruling on L10 arrived *after* the full G1–G6 ACCEPT — this did not reopen the gate;
it was folded as one more targeted fix to the one item the doc had correctly left pending, then
re-verified narrowly rather than re-run in full. Rule for next time: **a post-accept ruling that
resolves an already-open, already-labeled item is a delta, not a reopening** — gate only the
delta.

**A relay-fidelity miss, caught by the reviewer's discipline.** When I routed the PM's L10
ruling to dev, I compressed the memo's IOMMU-premise detail to "stale-premise" — dropping the
one clause that made the addendum's claim independently checkable. The reviewer correctly
flagged it as ungrounded inference, because from where they sat, it was. Rule: **when relaying
a ruling through a chain, forward the load-bearing specifics verbatim, not a paraphrase** — a
compressed relay reads identically to an invented one from the next link's vantage, and only
the compressor knows the difference.

**A genuine blocker with no invented workaround.** Routing the TLS/§16 spine question to
`zero.policy` per the contract's standing rule hit a dead end — no live pane, no ticket-tracker
credentials configured for my identity. I escalated the channel gap to the PM rather than
guessing at a substitute (e.g., silently bootstrapping tracker auth, or picking an arbitrary
default and moving on) — the PM owned the resolution (relaying through design) within one
turn. Rule: **a missing channel is the PM's to route, not the lead's to work around.**

**What the next sprint inherits.** (a) Team-formed planning before drafting the contract is
worth the extra round-trip when the deliverable has no precedent. (b) Verify a claimed
external fix on disk before building a sprint's premise on it — a report is not the file. (c) A
post-close ruling on an already-flagged open item is a scoped delta, gated narrowly; it is not
grounds to reopen the full acceptance. (d) Relay verbatim, not paraphrased, when a chain of
routing separates the source of a claim from the party who must verify it.
