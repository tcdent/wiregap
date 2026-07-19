# Sprint 003 — Boundary Contracts (box↔proxy, proxy↔head, proxy↔NIC)

> Proxy team, sprint 3. The team defines a reviewed contract for every boundary the proxy
> touches. Drafted by zero.proxy-lead from planning input by zero.proxy-dev and
> zero.proxy-review (2026-07-18); green-lit by zero.proxy-pm.

---

## Scope Boundary

This sprint owns **three deliverables**, new artifacts under `docs/proxy/contract/`
(distinct from `docs/proxy/spec/`: a contract states the proxy's side of an interface
shared with another owner, a spec settles an internal security mechanism):

- `docs/proxy/contract/box-proxy.md` — the security boundary (untrusted → trusted).
- `docs/proxy/contract/proxy-head.md` — the terminal display/input relay
  (trusted → trusted).
- `docs/proxy/contract/proxy-nic.md` — the DMA handoff to the substrate/NIC.

This is the sprint's third distinct shape: Sprint 1 was composition against one source;
Sprint 2 authored new mechanism with no prior artifact; **this sprint is composition
across owners** — most of what each contract touches is already settled in artifacts the
proxy does not own (the protocol effort, the NIC team, `zero.head-design`). The failure
mode this sprint guards against is not incompleteness or an unproven mechanism; it is **a
contract that quietly redefines another owner's settled thing while appearing to cite it**,
or **cites an open item as if it were settled**.

It explicitly does **not** own:

- **Any other team's artifact.** The protocol layer (`docs/proto/`), the NIC team's API,
  and `zero.head-design`'s input/pane-state behavior are cited, never redefined. A clause
  that needs another owner's ruling is stated as a requirement on the seam and routed to
  the designer (the PM's named coordination point), never settled here.
- **Ratifying unsettled upstream items.** Verb ids, error codes, and display attribute/
  cursor codes are not yet ratified (`model.md` §8/§11); per-verb flag bits are undefined;
  input-events/`pane-state` are locked-with-`zero.head-design` but their ids are
  unratified. Every contract clause touching one of these carries it as open, not settled.
- **Freezing the Sprint 2 specs.** `filesystem-write-path.md` and `trusted-address.md` are
  accepted but not frozen (ratification packet pending). Contract clauses citing them
  inherit that pending status; if a ratification ruling later flips a default, the
  contract's citation tracks it, stated in each contract's banner.
- **Authoring against the NIC's preliminary API.** The NIC team's public API is still
  finalizing. Slice 3 splits into a shape-now part (definable from settled
  `nic/architecture.md` §2.1 alone) and a deferred fill-in part (gated only once the API
  finalizes) — see Slice 3 below. No preliminary-shape authoring.

## Problem

Three boundaries the proxy touches have no single, reviewed statement of what crosses them
and what the proxy's disposition is:

**1. box→proxy.** The protocol layer (`docs/proto/model.md`, `file.md`, `net.md`,
`display.md`, `library.md`) settles the wire structure and the verb catalog, and the two
Sprint-2 specs settle the filesystem and address mechanisms — but no single artifact states,
**per verb/channel**, the proxy's consumer policy: what it decodes vs. relays opaquely, what
it does on a reserved bit, an unratified id, or an unknown verb, what it logs, what its
residuals are. This enumeration is the sprint's genuinely new content — the `scheme==0`
pattern in `trusted-address.md` generalized to the whole verb catalog, made necessary by the
already-committed convention that the protocol validates structure only and leaves
reserved-value policy to the consumer.

**2. proxy→head.** `terminal.md` §2–3 and `architecture.md`'s placements establish that this
boundary carries no proxy-side gate — the guarantee lives in the wire's inexpressiveness and
the head's no-interpreter invariant. What is missing is the concrete pin: which L2 messages
the proxy actually decodes (it must see at least the routing information) vs. relays
opaquely (display body bytes), derived from `library.md` §7's consumer-surface table.

**3. proxy→NIC/substrate.** `architecture.md` and `nic/architecture.md` §2.1 establish the
trust-face (the DMA handoff is where "the safe surface begins"; the driver reads
descriptor/buffer content as if from an adversary) and the settled invariants (bounded
length, hostile status, early copy, fail-closed device-identity gate, DMA-reach owned). What
is missing is the proxy-side contract statement — and the NIC team's public API is still
finalizing, so this boundary's concrete fill-in cannot be authored yet without building on a
moving target.

A concrete, verified finding surfaced during planning, **confirmed by the designer as
present in two proxy documents, not one**: `docs/proxy/filesystem.md` §4 ("writes verbatim
bytes at an offset") and our own accepted `filesystem-write-path.md` (Sprint 2, "`pwrite(2)`
at their offsets," which derived its citation from the stale `filesystem.md` §4 phrase). The
committed protocol (`file.md` §1, `model.md` §11 Decided) states the wire is **offset-free**:
`file-chunk` is verbatim bytes appended in order at an implicit running position;
"parallel/offset writes are retired." The *mechanism* in the accepted spec was unaffected
(holding one fd and writing at a proxy-tracked cursor is byte-identical to sequential append
under an in-order transport) — but reconciling to no-offset **removes a field from the
trusted surface** (no box-supplied offset to ever validate), a small security simplification,
not merely cosmetic. This sprint's box→proxy contract derives the file-write channel from
`file.md` §1 directly — destination stack → in-order verbatim chunks → completion at session
end; no offset crosses the boundary. **Both fixes are in motion, split by owner**: the
one-line correction to our own accepted-but-not-frozen `filesystem-write-path.md` is
**already made** (dev, authorized during planning, verified on disk); `filesystem.md` §4's
own stale phrase is the **designer's** to fix, as the source research doc's owner (same
class as terminal.md/approach.md in prior sprints) — not this sprint's authoring content.

## Goal

Produce three reviewed contracts, one per boundary, each classifying every clause as a
citation of a settled artifact owned elsewhere or a definition of the proxy's own side —
never redefining another owner's settled ground and never asserting an open item as fact.

PM's framing: sequence box-facing and filesystem seams first (source-stable), NIC-transport
last (builds on the finalized API, not a moving target); the designer is the coordination
point for cross-team pieces.

## Final State

1. `docs/proxy/contract/box-proxy.md` exists: a per-verb/channel contract table (verb +
   ratification status, direction, body-shape citation, gate/floor citation, proxy
   consumer policy, reject behavior, log-line content, residuals) covering the full verb
   catalog, plus the file-write completion/timeout pin derived from `file.md` §1.
2. `docs/proxy/contract/proxy-head.md` exists: the same table pattern, thinner, pinning
   exactly which L2 the proxy decodes vs. relays opaquely, with open-upstream markers on
   unratified display codes and input-event ids.
3. `docs/proxy/contract/proxy-nic.md` exists in two parts: the shape (trust-face,
   proxy-side invariants, and requirements-on-the-seam, all derivable from settled
   `nic/architecture.md` §2.1) landed now; the API-specific fill-in explicitly deferred
   and named as a dependency, not asserted.
4. Every contract clause is ownership-tagged (proxy / protocol effort / NIC team /
   `zero.head-design`) and kind-tagged (CITE / DEFINE); no clause is ambiguously both.
5. Every CITE clause is verified word-for-word against the cited owner's *current* source
   and carries that source's settled-or-open status faithfully — never strengthened,
   flipped, or fixed-as-if-settled.
6. Every genuinely new judgment call (the uniform reserved-disposition rule, unknown-verb
   disposition, the reject-behavior taxonomy, completion/timeout pins) is named, given a
   recommended default, and routed — never silently settled (this sprint's SA7 analog).
7. The offset staleness in both `docs/proxy/filesystem.md` §4 and the accepted
   `filesystem-write-path.md` is reconciled to `file.md` §1's no-offset wire (mechanism
   unaffected; removes a field — a box-supplied offset — from the trusted surface). The
   spec-side fix is done (dev, verified); `filesystem.md` §4 is the designer's, in motion.
8. The sprint document carries role retrospectives; the sprint is committed with only its
   intentional files.

## In Scope — ordered slices

**Slice 1 — box→proxy contract (largest, most novel, source-stable).**
Deliverable: `docs/proxy/contract/box-proxy.md`. Covers every verb in `model.md` §8's
catalog with the per-verb table above. The new layer is the proxy's consumer-policy
enumeration (reserved-value/flag-bit disposition, unknown-verb disposition, reject-behavior
taxonomy, log content), generalizing the `scheme==0` precedent to the whole catalog. Cites,
never re-derives, the recognizer's internals (routed to the cloud effort, L4/L5) and both
Sprint-2 specs' floors. Confirmed by the designer: the layering this contract sits on is
`directpath` (L1 framing + session demux) → the proto crate (L2 decode) → proxy services,
which deep-parse only net + file addressing and relay the rest inert — this is settled
(proto sprint 010), not a citation to re-derive. Includes the file-write completion/timeout
pin, derived from the now-corrected `filesystem-write-path.md` (offset wording fixed) and
`file.md` §1 directly. The designer's parallel fix to `filesystem.md` §4 is in motion but
is not this slice's authoring content.
Gate: full GB1–GB6 (below).

**Slice 2 — proxy→head contract (thinner).**
Deliverable: `docs/proxy/contract/proxy-head.md`. Settled mostly by `terminal.md` §2–3 +
`display.md` + `architecture.md`'s placements (relay checks nothing; head parser meets the
tier-2 memory-safety bar). The one load-bearing pin: derive from `library.md` §7 exactly
which L2 messages the proxy decodes (must see the routing information) vs. relays opaquely
(display body bytes). Confirmed by the designer as "thin and stable": the proxy is a
memory-safe relay of terminal frames with no gate, deep-parsing only what it acts on
(net + file) and relaying the rest inert — display-op churn and keysym-value agreements
(a head↔box standard adoption, not a protocol artifact) are off this contract's surface
entirely. Open-upstream markers required on unratified display attribute/cursor codes and
unratified input-event ids. Gate: full GB1–GB6.

**Slice 3 — proxy→NIC/substrate contract (last; two-part gate).**
Deliverable: `docs/proxy/contract/proxy-nic.md`, in two parts:
- **Shape-now** (runs against settled ground only, no dependency on the NIC API): the
  trust-face (DMA handoff = where the safe surface begins; treat-as-hostile read); the
  proxy-side invariants the seam must hold (bound the reported length, treat status as
  hostile, copy out early, the fail-closed device-identity gate, DMA-reach owned — cited
  to `nic/architecture.md` §2.1 and ledger L8/L11); the requirements the proxy places *on*
  the seam, stated as demands on the interface (a way to post/reclaim descriptors, observe
  completion, wake/doorbell) — never asserting the NIC API's specifics as fact.
- **Fill-in** (deferred, gated only once the NIC team's API finalizes — an exact commit
  pinned at that gate's start): the concrete descriptor layout, register offsets, and API
  calls, cited against the final API under the standard GB1–GB6 gate.
Gate: **shape-now gate runs now** (ACCEPT = the shape holds regardless of what the API
turns out to be, with every API-specific point named as a dependency, not asserted; REJECT
if any unsettled API specific is asserted as fact). **Fill-in gate runs later**, once the
API lands, under the full GB1–GB6.

**Privilege-drop scoping (L15) — confirmed by the designer against the NIC team's
committed code, not a preliminary read**: `construct()` performs all privileged bring-up
eagerly (unbind, BAR0 mmap, DMA hugepage, bus-master enable, link bring-up, ring setup);
`run()`/`send` are zero-syscall and unprivileged. Bring-up → drop-privilege → serve lands
cleanly between `construct()` and `run()`. This closes L15 for this sprint's purposes —
state it as a proxy-side requirement the seam already satisfies, not a shape-now
placeholder. **One caveat, not this sprint's to resolve**: there is no device-reset
recovery path in `run()` today; a future recovery design needing a privileged config-space
write (clearing/re-enabling the bus-master bit) would need its own small privileged helper.
Already carried in `architecture.md`'s ledger (L15) as a future caveat — this sprint's
contract states the current (no-reset) shape faithfully and does not invent a recovery
path.

## Non-Goals

- Do not redefine the wire structure, the transport session model, the NIC descriptor
  layout, or `zero.head-design`'s input behavior — cite, never re-author (GB4).
- Do not ratify an unratified upstream item (verb ids, error/display codes, flag bits,
  input-event ids) — carry it as open, routed to its owner (GB3).
- Do not author against the NIC's preliminary API shape — Slice 3's fill-in waits for the
  finalized API, pinned to an exact commit at that gate's start.
- Do not re-specify the cloud recognizer's internals — the box→proxy contract's net-channel
  row cites `architecture.md`'s L4/L5 placement and stops.
- Do not silently correct another team's doc (`filesystem.md` §4's stale phrase) — that fix
  is the designer's, in motion, not this sprint's to make.
- **One completed exception**: the one-line wording/citation fix to our own accepted-but-
  not-frozen `filesystem-write-path.md` was ours to fix directly during the ratification
  window (not a redefinition of another owner's artifact) — done (dev, verified on disk).

## Starting Surface

Audit first, in this order:

1. `docs/proxy/architecture.md` — tier placements and the L1–L15 ledger, especially the
   trust-face statements for each boundary (§ Method, GB5).
2. Both accepted Sprint-2 specs: `docs/proxy/spec/filesystem-write-path.md`,
   `docs/proxy/spec/trusted-address.md` — cited, not reopened, pending-ratification status
   carried.
3. The protocol layer, in full, with every citation re-pinned against *current* working-tree
   state at each gate (all five files were still working-tree-modified as of sprint start):
   `docs/proto/model.md` (especially §6 body shapes, §8 verb catalog, §11 Decided/Open),
   `docs/proto/file.md`, `docs/proto/net.md`, `docs/proto/display.md`,
   `docs/proto/library.md` (especially §7 consumer surfaces, §8 open decisions). Anchor on
   committed code (`proto/src/segment.rs`) over working-tree doc prose where both describe
   the same fact (Sprint 2's standing lesson).
4. `docs/nic/architecture.md` §2.1 (the settled NIC posture) for Slice 3's shape-now part;
   the NIC team's preliminary API is *not* audited yet — its finalized form and commit are
   pinned only when Slice 3's fill-in gate starts.
5. The designer (`zero.proxy-design`) is the standing coordination point for any genuine
   cross-team question this sprint surfaces — route through the PM or directly, never
   settle a cross-team fork in a contract clause.

## Method

This sprint's failure mode differs from Sprints 1–2: not an incomplete inventory, not an
unproven mechanism, but **a contract that redefines another owner's settled thing while
appearing to cite it**. The organizing discipline:

- **Every clause is CITE or DEFINE, tagged with its owner.** A CITE restates an
  already-settled artifact owned elsewhere; a DEFINE states the proxy's own side. A clause
  that is neither cleanly one nor the other is where redefinition hides.
- **Citation fidelity is diffed, not paraphrased.** The reviewer re-reads the cited owner's
  *current* source word-for-word and confirms the contract matches it — same guarantee,
  same direction, same strength, same settled/open status. Never trust the contract's
  summary of another team's doc (the cross-team analog of Sprint 1's L2-envelope
  inversion).
- **Settled-vs-open status travels with the citation.** A cited artifact that is itself
  open stays open in the contract — a contract may not cite an open item as fact
  (extending Sprint 1's currency-of-source to currency-*and*-settled-status).
- **Proxy-side authority only.** A DEFINE clause defines something the proxy legitimately
  owns; redefining another domain is scope creep, routed to that domain's owner, never
  settled here.
- **Trust-face correctness per boundary.** Each boundary has a settled character the
  contract must state correctly: box→proxy is the gated security boundary; proxy→head is a
  trusted→trusted relay with *no* gate; proxy→NIC is a treat-as-hostile DMA handoff, neither
  a gate nor a plain relay. Misstating a boundary's trust-face is itself a redefinition.
- **Anchor on committed code over working-tree docs, and on settled posture over a
  preliminary API** — Sprint 2's standing lesson, re-applied across the cross-team
  boundary.
- **Genuine new judgment calls are routed, not settled** — this sprint's SA7 analog (GB6),
  since — like Sprint 2 — this sprint produces real forks (the uniform reserved-disposition
  rule, unknown-verb disposition, the reject-behavior taxonomy, completion/timeout pins)
  that a contract could quietly bake in instead of routing.

## Acceptance

Six gates per contract (GB1–GB5 from planning, verbatim; GB6 added as this sprint's
judgment-call-routing analog to Sprint 2's SA7):

- **GB1 — Ownership-tagged clauses.** Every clause names its owner (proxy / protocol effort
  / NIC team / `zero.head-design`) and its kind (CITE / DEFINE). REJECT on ambiguous
  ownership or kind.
- **GB2 — Citation fidelity (the load-bearing gate).** Every CITE clause is diffed
  word-for-word against the cited artifact's *current* source. REJECT if a clause
  strengthens a guarantee, flips a direction, or fixes an open choice the source leaves
  open — a redefinition wearing a citation.
- **GB3 — Settled-vs-open status carried, not laundered.** A cited item that is itself
  marked open (unratified verb ids/error codes/display codes; undefined flag bits;
  locked-but-unratified input-event ids; the NIC's preliminary API) stays open in the
  contract. REJECT on a clause that treats an open item as settled fact.
- **GB4 — Proxy-side authority only.** Every DEFINE clause defines something the proxy
  legitimately owns, never another domain's artifact. REJECT + route on redefinition of the
  wire format, the NIC descriptor layout, the transport session model, or
  `zero.head-design`'s behavior.
- **GB5 — Trust-face correctness per boundary.** box→proxy stated as the gated security
  boundary; proxy→head stated as a no-gate relay; proxy→NIC stated as a treat-as-hostile DMA
  handoff. REJECT on a boundary described with the wrong trust-face.
- **GB6 — Judgment calls routed, not settled.** Every point where the contract must choose
  without a source adjudicating (reserved-value/flag-bit disposition, unknown-verb
  disposition, reject-behavior taxonomy, completion/timeout pins) is named, given a
  recommended default, and routed to design/policy — never settled unilaterally. REJECT +
  route if presented as a settled conclusion.

Slice 3's shape-now gate is a variant: it checks GB1–GB6 against *only* the settled
`nic/architecture.md` §2.1 ground, with the specific additional check that no unsettled API
detail is asserted as fact (a shape-now clause depending on an unnamed API specific =
REJECT). The fill-in gate, once the API finalizes, is the standard six gates against the
now-final API.

## Review Questions

1. **Contract-vs-spec placement (ruled by lead, mechanical):** new directory
   `docs/proxy/contract/`, distinct from `docs/proxy/spec/` — a contract states the
   proxy's side of an interface shared with another owner; a spec settles an internal
   mechanism.
2. **RESOLVED — the stale-citation correction.** The one-line fix to
   `filesystem-write-path.md`'s step 4 (drop the "write-at-offset reassembly" citation,
   restate as sequential-append-at-a-tracked-cursor per `file.md` §1) was authorized during
   planning and is **done** (dev, verified on disk) — a citation correction to our own
   accepted-but-not-frozen artifact, not a mechanism change.
3. **IN MOTION — `filesystem.md` §4 wording fix.** The "writes verbatim bytes at an offset"
   phrase is stale against the committed offset-free wire (`file.md` §1, `model.md` §11);
   the **designer** (the doc's owner) has claimed this fix directly, folded into Slice 1's
   timeline — not routed via the PM as a separate item, since the designer is the standing
   coordination point and volunteered it.
4. **The consolidated judgment-call list (for design/policy via the PM, non-blocking per
   GB6):** the uniform reserved-disposition rule (recommended: yes — one rule, not per-verb
   exceptions, and what makes unratified flag bits safe to build on meanwhile: whatever
   ratifies later, today's proxy denies it, fail-closed forward-compatibility); unknown/
   unratified-verb disposition (recommended: deny + log the verb id, never route);
   reject-behavior taxonomy per class (recommended: match the protocol's uni-directional
   grain — deny+log, no new return verbs); file-write completion/timeout pins per direction
   (scalars provisional).
5. **RESOLVED — cross-team coordination.** The designer confirmed: the protocol layer
   (model.md/net.md/file.md/display.md) is **structurally mature** (22/22 verbs, sprint 007;
   envelope correction, sprint 010) — **number-set ratification (verb ids, error codes,
   display codes) stays open per GB3**, not implied settled by "mature." Four further
   known-provisional items to build against as provisional, not as premises about to change
   (reserved fields/bits, `REASSEMBLY_CAP`'s provisional 32 MiB, the settled-but-worth-
   restating envelope/layering split, and `directpath`'s preliminary naming). The preliminary NIC API for Slice 3's scoping is
   `directpath/src/lib.rs` (`Subsystem::{construct,send,run}`, `Ctx::{send,is_quiescent}`,
   `Event`, `ControlFlow<()>`) plus the NIC team's own legibility doc
   (`docs/teams/nic/010-library-legibility.md`) — scope against the *shape*, not the
   identifiers, which may shift in the NIC team's readability pass. The L15 privilege-drop
   seam is code-verified closed (Method/Slice 3): `construct()` does all privileged
   bring-up, `run()`/`send` are unprivileged; the one open caveat (no device-reset recovery
   path in `run()` today) is carried in `architecture.md`'s ledger already and is not this
   sprint's to resolve.
6. **PARTIALLY RESOLVED — currency-debt trend.** Raised as a trend (three sprints deriving
   from uncommitted working-tree source: `terminal.md` §5 since Sprint 1; all five
   `docs/proto/` files as of this sprint, though the designer confirms their *content* is
   mature/settled regardless of commit status). The designer committed the two standing
   fixes promptly upon being asked (`terminal.md` §5 → `da6c63c`; `filesystem.md` §4 →
   `7f7de0b`), clearing the Sprint-1-era dependency and confirming going forward that a
   source-doc fix commits promptly rather than lingering, once it's surfaced. The one
   remaining working-tree item is `filesystem-write-path.md` (dev's Sprint-2 spec fix),
   correctly left uncommitted for Sprint 3's own sequencing (Sprint 2's specs are
   accepted-but-not-frozen).

## Retrospectives

### zero.proxy-review

**Evidence that changed the gating approach for this sprint's new shape.** Three sprints, three
review shapes: Sprint 1 was composition against ONE source (gate = re-derive + diff for
completeness); Sprint 2 authored NEW mechanism with no prior artifact (gate = adversarial closure
the reviewer constructs); Sprint 3 is **composition ACROSS OWNERS**, and the failure mode moved
again — not incompleteness, not an unproven mechanism, but **a clause that redefines another
owner's settled thing while appearing to cite it, or cites an open item as settled**. The
organizing move that made the verdict derivable was tagging **every clause CITE(owner) or DEFINE**
and checking each against its kind. The first concrete payoff was at contract review: separating
"the protocol is *structurally mature* (22/22 verbs)" from "verb ids *ratified*" — `model.md` §11
keeps the number-set ratification open, so "mature" must not launder it to settled. You catch that
only by reading the owner's own open-markers, not the contract's summary.

**What the D-4 / nic-doc-inconsistency episode taught about citation fidelity across a team
boundary.** The contract's D-4 cited `nic/architecture.md` §5 for an "event-driven park/wake"
idle-wake — but §5 says the opposite, verbatim: "a **bounded poll**… re-polls after a capped
short sleep **rather than parking**… reconciled **from MSI-X to bounded poll**." A cross-team
redefinition-masquerading-as-citation — the exact GB2 catch — found only by re-reading the *other
team's current source word-for-word*, trusting neither my memory nor the contract's paraphrase.
Two deeper lessons fell out: (1) my own mental model (a Sprint-8 MWAITX event-driven wake) was
**stale relative to the committed doc** — I disclosed it rather than assert it, the R5 discipline
for fast-moving cross-team domains. (2) The episode surfaced a divergence *inside the nic team's
own sources* (their L15 code-verification says zero-syscall steady-state; a capped-sleep bounded
poll makes a sleep syscall — the two can't both hold). The right move was to make the contract
**robust to either resolution** (a mechanism-neutral D-4 that a bounded poll *and* an MWAITX wake
both satisfy) and **route** the divergence to the owner — flagged as inference, adjudicated by
nobody here. Citation fidelity across a boundary is: verify against the cited owner's current
artifact, flag divergences as inference, route them, and never let the contract depend on which
way the other team reconciles.

**My Sprint-2 offset-citation retraction, and what it changes going forward.** This sprint's
planning re-pin exposed a miss in my own Sprint-2 acceptance: I rigorously gated the headline
security mechanism (the openat2/symlink floor — I even re-ran the probe) but let a *peripheral*
citation through — "pwrite at their offsets," resting on the stale `filesystem.md` §4, which
contradicted the committed offset-free wire (`file.md` §1, `model.md` §11). The durable change:
**currency-of-source applies to every citation, not just the load-bearing one** — the peripheral
citations are exactly where a stale one hides after the headline clause absorbs all the scrutiny.
I applied it here: I diffed *every* CITE word-for-word, including the "peripheral" ones (the copy
boundary, the read-path O_NOFOLLOW floor, the idle-wake demand) — and the idle-wake, a
peripheral-looking demand, is precisely where the D-4 finding lived.

**What the next proxy sprint inherits.**

- *The `proxy-nic` FILL-IN half is queued* for the finalized-NIC-API gate (full GB1–GB6 against the
  pinned final API); the R-6 nic-source divergence (idle-wake code-vs-doc) is routed to the
  designer/NIC team. Carry both; do not re-derive or adjudicate.
- *Open dependencies stay live:* the proxy↔head transport framing (Slice 2, head + proxy design),
  the cloud recognizer (L4/L5), and both sprints' ratification packets. Contract clauses citing
  accepted-not-frozen Sprint-2 specs track any ruling that flips a default.
- *The three review shapes are now a recognized toolkit* — single-source diff / new-mechanism
  adversarial closure / cross-owner CITE-vs-DEFINE. Recognize the shape first, then gate. Standing
  disciplines across all three: currency-of-source on *every* citation; anchor on committed code
  over lagging docs; re-read current cross-team source rather than trust memory; route cross-team
  divergences rather than settle them. The next tier-1 mechanism — the cloud request recognizer
  (L4/L5), a recursive recognizer over attacker-authored JSON — is a new-mechanism adversarial gate
  (Sprint-2 shape) and will be the sharpest yet.

### zero.proxy-dev (author)

**What worked, and why it should be kept.** The CITE/DEFINE + owner tagging did exactly
what the contract designed it for: it forced every sentence to declare whether it was
restating someone else's settled ground or defining ours, and both of this sprint's real
defects were caught *at that seam* (a citation claiming more than its source — GB2 —
twice: once by me before authoring, once by the reviewer after). Also kept: deriving the
consumer-policy layer from the committed convention (proto validates structure only)
rather than inventing it — the whole 22-verb enumeration fell out of one question, "what
does the proxy do with the values proto deliberately does not police?"

**What evidence changed the plan.**
1. *The offset staleness, found in three places* (`filesystem.md` §4, our own accepted
   spec, and `model.md` §8's rationale blockquote — the third found only during this
   sprint's re-pin). One retired wire decision had echoed into two downstream docs and
   one upstream commentary. Durable rule: **when a wire decision is retired, grep every
   doc that cites the channel — staleness propagates along citation edges, and each copy
   looks locally authoritative.**
2. *The D-4 mischaracterization.* I cited `nic/architecture.md` §5 from its heading plus
   my memory of the nic team's sprint work; the body says the opposite of what I wrote
   ("bounded poll … rather than parking"). The reviewer's GB2 diff caught it. Same rule
   Sprint 2 taught, sharpened: **a section heading is not a read, and memory of a
   team's *work* is not the state of their *doc* — the two drift independently.**
3. *The drift was real in both directions*: the full §5 read surfaced a divergence inside
   the nic team's own artifacts (§5's bounded poll vs their later "retire the poll"
   commit and the zero-syscall/MWAITX L15 verification — a sleep is a syscall, so both
   cannot be current). Surfaced as contract residual R-6, routed to nic via the designer,
   adjudicated by nobody on our side. The fix pattern that made this safe: **state the
   demand mechanism-neutrally so the contract is robust to either resolution, then route
   the divergence — never pick the winner in a citing document.**

**What failed and needed correction.** One narrow REJECT (D-4, above) — the citation
strengthened what its source provides, the exact redefinition-wearing-a-citation GB2
exists to catch. Corrected with the mechanism-neutral restatement + R-6. The minor R-1
"verbatim" overclaim (it was a faithful paraphrase) taught the small version of the same
lesson: **claim the fidelity level you actually have — quote, paraphrase, or inference —
and label which.**

**What the next sprint inherits.**
(a) **The proxy-nic fill-in half** — deferred by design, not a gap: pin the finalized NIC
API to an exact commit as the fill-in gate's opening act, then cite each D-1…D-6 demand's
concrete form by name. (b) **The Sprint-2 ratification packet** — still pending; both
specs and every contract clause citing them remain accepted-not-frozen until design/
policy rule the ten calls (+ the JC-1…JC-4 calls this sprint added to the routing load).
(c) **Open dependencies with owners**: the proxy↔head concrete transport framing (head +
proxy design); the `model.md` §8 "+ offset" commentary fix and the §5-vs-code idle-wakeup
reconciliation (both nic/proto side, routed via the designer). (d) **The conventions that
earned their keep**: CITE/DEFINE + owner tagging for any cross-team artifact;
currency-pin appendices with re-pin-at-gate; mechanism-neutral demands wherever an
upstream source is in flux.

### zero.proxy-lead

**Cross-team coordination worked because it stayed narrow and evidence-first, not
because it stayed frequent.** One round of planning questions to the designer up front
(what's moving, where's the preliminary NIC shape) resolved every genuine cross-team
uncertainty before a single contract clause was written — nothing was re-asked mid-slice.
When new cross-team findings surfaced later (the offset staleness, the nic §5-vs-code
divergence), each got exactly one routed message with the finding stated as evidence, not
a conclusion, and I did not chase a resolution before moving the slice forward. **Rule:
route a cross-team finding once, cleanly, with the evidence — then let the owning team's
timeline be theirs, and keep working the queue that's actually yours.**

**A same-day ownership collision, caught by re-verifying before instructing, not after.**
I authorized dev to fix our own spec's offset citation, then the designer volunteered to
fix "both" documents in the same window — a real crossed-wire that could have produced a
conflicting edit or duplicated work. I verified dev's fix was already on disk before
telling the designer to skip it, rather than trusting either party's account of what state
things were in. That check is what kept a two-line coordination note from becoming an
actual merge conflict. **Rule: when two parties claim adjacent work in the same window,
verify the file state directly before adjudicating who does what — don't referee from
messages alone.**

**The privilege-drop confirmation (L15) is a model for how much a routed question can be
worth closing outright.** The designer didn't just answer with a doc read; they got the
NIC team to verify the claim against their own committed code before relaying it. That
turned a "probably fine" scoping assumption into a closed, cited fact the contract could
build on directly — and it's exactly why the later nic §5 divergence was caught: the same
designer, asked the same way, read the *current* text rather than trusting a title or a
prior sprint's summary. **Rule: a cross-team answer is worth more when it's traceable to
something checked (code, a fresh read) than when it's traceable to someone's memory of the
work — ask for the former, and notice when you're being given the latter.**

**The reviewer's two self-corrections this sprint (the Sprint-2 offset retraction, the
disclosed stale MWAITX belief) are the behavior the whole review discipline depends on,
and neither needed prompting from me.** Both were volunteered, both changed what got
checked next, and both were stated as "here's what I got wrong and why" rather than buried
in the verdict. That is worth naming explicitly rather than letting it pass as routine —
a reviewer who surfaces its own priors as fallible is the thing that makes "independently
re-derived" mean something.

**What the next sprint inherits.** (a) One clean, evidence-first round of cross-team
planning up front beats repeated pings — reserve the coordination channel for genuine new
findings, not status. (b) Verify file state directly before resolving an ownership
collision between two parties working the same window — don't take either account at face
value. (c) When routing a cross-team question, ask whether the answer can be checked
against code or a fresh read rather than accepted as a summary of past work — the
difference showed up twice this sprint (L15's code-verification, the §5-vs-code
divergence). (d) A reviewer's self-disclosed correction is a signal the process is
working, not a defect to smooth over in the summary — name it in the retrospective, same
as a finding.
