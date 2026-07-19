# Sprint 004 — The Security-Invariant Core (Total Functions, Build Handoff)

> Proxy team, sprint 4. The team specifies the total functions that ARE the security
> guarantee, ready for the build handoff. Drafted by zero.proxy-lead from planning input
> by zero.proxy-dev and zero.proxy-review (2026-07-19); green-lit by zero.proxy-pm.

---

## Scope Boundary

This sprint owns **four deliverables**, new spec files under `docs/proxy/spec/`, one per
total-function proof:

- `docs/proxy/spec/scan-safe.md` — the shared safe-alphabet segment scan (the floor both
  downstream targets stand on).
- `docs/proxy/spec/leaf-validator.md` — the filesystem write-path leaf-array total
  function (consolidation).
- `docs/proxy/spec/address-recognizer.md` — the cloud route/address total function
  (consolidation).
- `docs/proxy/spec/anthropic-recognizer.md` — the Anthropic body-shape recognizer
  (genuinely new; architecture ledger L4/L5).

**Scoping resolved before drafting**: the PM's brief named "the byte-map, the
leaf-validator, and the egress recognizers" as the targets. Independently verified by
lead, dev, and reviewer against committed source: "the byte-map" is terminology from the
superseded `docs/explore/console-and-protocol-design.md` (a 256-value raw-byte terminal
filter). The current, committed `docs/proto/display.md` §4/§6 explicitly supersedes and
retires it ("the filter is gone, not the neutralization... no parser to drive"), and
`terminal.md`/`architecture.md` confirm the terminal boundary carries no proxy-side gate
at all. **There is no byte-map to specify.** Confirmed by the PM with design: the
*pattern* the old byte-map named — a total function over a finite alphabet, verified by
enumeration — survives as the already-committed `scan_safe` (the shared segment scan used
by both the leaf-validator and the address recognizer, `proto/library.md` §8). **Three
real targets, specified as four deliverables** because `scan_safe` is specified once as
shared machinery rather than duplicated inside each consumer.

It explicitly does **not** own:

- **Re-opening Sprint 2's settled mechanisms.** `filesystem-write-path.md` and
  `trusted-address.md` are cited and *proven* (the total-function enumeration added), not
  re-specified. A real defect found while proving is a Sprint-2-spec finding, routed —
  never a silent re-spec (Sprint 3's discipline).
- **The recognizer's placement (L4).** In-transport module vs. isolated service is
  explicitly open in `anthropic-api.md` §8 and `architecture.md`'s ledger, owned by the
  cloud subsystem design. The Anthropic recognizer spec is placement-neutral: its totality
  holds regardless of where it runs (Sprint 3's mechanism-neutral discipline, re-applied).
- **Ratifying the vetted client-side identity strings, the top-level field whitelist, the
  JSON profile/depth/size caps, or the beta-header policy.** Each is a genuine judgment
  call, named and routed with a recommended default (§ Acceptance, the SA7 analog) — not
  settled unilaterally.
- **Chasing API currency.** The Anthropic admit-list is a dated snapshot
  (`anthropic-api.md`, grounded 2026-06-20, already a month stale as of this sprint). The
  spec pins the snapshot date and treats a new vetted-tool version as a deliberate re-admit
  (the promotion pattern), fail-closed until then — it does not chase "current."

## Problem

The proxy's security-invariant core — the total functions that *are* the guarantee, per
the spine (assertions 15–16: reviewable by enumeration, held to a ~500-line budget) — is
currently split across accepted-but-not-yet-proven mechanisms and one genuinely unspecified
one:

**1. The shared floor (`scan_safe`).** Committed as code (`proto/src/segment.rs`) and used
by two consumers, but never specified in its own right as the total function it is.

**2. The leaf-validator.** Its mechanism is settled (`filesystem-write-path.md`, Sprint 2,
accepted), but its own totality — every leaf-array input maps to admit-or-deny, no
unhandled case — has not been stated as a standalone, enumerable proof.

**3. The cloud route/address recognizer.** Its representation is settled
(`trusted-address.md`, Sprint 2, accepted; `box-proxy.md`'s net-address row, Sprint 3), but
likewise wants its own total-function proof: the domain is `(scheme u32) × (safe segment
stack)`, every point maps to a register match or a deny.

**4. The Anthropic body-shape recognizer.** Genuinely unspecified — `architecture.md`'s
ledger **L4** (placement) and **L5** ("exact recognized request shape... builders at
spec-freeze; frozen as trusted surface") are both explicitly open, and
`anthropic-api.md` §8 names this as the specific expansion the spine's reviewability
budget sanctions, owed the highest scrutiny. This is the sprint's hardest, largest, and
highest-stakes content — an estimated 60–70% of the total effort.

**The category shift for L4/L5's recognizer specifically**: the leaf-validator and address
recognizer are *flat* (a fixed-alphabet scan, a bounded segment stack) and the proxy *owns*
the representation — a bad byte is structurally unrepresentable. The Anthropic recognizer
is *recursive* over an attacker-authored JSON body the proxy does **not** own — Anthropic's
own server will parse the same bytes. That shift introduces a genuinely new risk class no
prior sprint faced: **the JSON parse itself is trusted surface.** A general, unbounded-
grammar JSON parser handed the box's bytes before any field logic runs is exactly the
"parse an arbitrary grammar" bet the spine forbids (Sprint 2's SA6) — and it would be
**uncounted bulk against the reviewability budget**, the single biggest risk to both
soundness and the line-count ceiling. The sound shape is a **bounded, total structural
reader** — fixed depth, fixed size (reusing the existing reassembly cap), a restricted
accepted profile, deny-default at every branch — specified, counted, and reviewed as the
core of this spec, not imported as a free library.

A second, equally new risk class: because the recognizer's output is forwarded **verbatim**
(never rewritten — `approach.md` §1/§3, `anthropic-api.md` §6's `cch` body-hash
reinforcement), the recognizer must **parse the same bytes Anthropic's server will parse**.
Any parse-disagreement (duplicate JSON keys, non-canonical unicode escapes, number/
whitespace/trailing-data handling) is an exploitable gap the box can drive — a class with
no analog in the flat mechanisms, where the proxy owns the representation and this question
cannot arise.

**A confirmed staleness in the team's own composition document**: `architecture.md`'s
ledger rows **L6** and **L7** still read "source-open... filesystem design review" —
independently caught by both dev and reviewer — but Sprint 2's `filesystem-write-path.md`
settled both and was gate-accepted. This sprint's close is the natural point to reconcile
the ledger (L6/L7 point at the accepted specs; L5 marked closed by this sprint's S4 spec),
since Sprint 4 touches exactly these rows — this is our own document, ours to fix directly,
not routed externally.

## Goal

Specify each of the three total functions (plus the shared `scan_safe` floor) as a total
function over a finite domain, verifiable by enumeration, with the whole set held to the
spine's ~500-line reviewability budget — ready for the build handoff: the artifact dev
implements against and review certifies against.

## Final State

1. `docs/proxy/spec/scan-safe.md` exists: the total-function proof for the committed
   `scan_safe` (256-byte-value domain → in-alphabet / delimiter / reject; the ≥1-alnum and
   empty/traversal reject arms; verify-never-transform), cited to `segment.rs`. Both
   downstream specs reference it; its line count is counted once.
2. `docs/proxy/spec/leaf-validator.md` exists: the array-level total-function proof over
   the leaf-array domain (element count, per-leaf validity via `scan-safe.md`), citing
   `filesystem-write-path.md` for mechanism, adding only the enumeration proof.
3. `docs/proxy/spec/address-recognizer.md` exists: the total-function proof over
   `(scheme) × (safe segment stack)` → register-match-or-deny, citing `trusted-address.md`
   and `box-proxy.md`'s net-address row for mechanism, adding only the enumeration proof
   (injectivity already proven in `trusted-address.md` §3, cited not re-derived).
4. `docs/proxy/spec/anthropic-recognizer.md` exists: the new total-function spec —
   envelope exact-match, a bounded structural JSON reader (counted, not a free library),
   top-level key whitelist, per-entry `tools[]` recognition, beta-header whitelist — with
   every judgment call (admit-list strings, field whitelist, JSON profile/caps,
   beta policy) named, given a recommended default, and routed; placement-neutral per L4.
5. **RESOLVED (design, 2026-07-19) — the ~500 is a forcing function for simplicity, not a
   quota to divide or a cap to squeeze under.** It is illustrative of a holdable magnitude;
   the real signal is **whether each box-reachable piece is in its simplest reviewable form,
   and whether a human can still hold the whole thing in one sitting.** A **budget ledger**
   (one table, maintained across all four slices) tracks each of the four members' line
   counts against *that* signal — `scan_safe` counted once, the bounded JSON reader counted
   *inside* the Anthropic number (never as uncounted library bulk) — not against a raw
   number as pass/fail. **Leanness effort concentrates on the Anthropic recognizer
   specifically**: it is the proxy's own code and the piece most at risk of bloat; tier-2
   plumbing (driver/transport/reactor) is the NIC team's lever, already minimal, and out of
   this sprint's leanness scope. The bounded-JSON-reader discipline remains a hard
   constraint — because it is the simplest design that guarantees deny-by-default, not
   because it hits a line target. **Trip-wire (carried forward, not expected at the current
   estimate)**: if the recognizer is minimized to its genuinely simplest form and the whole
   TCB still cannot be held in one sitting, that is a **scope question** (the proxy doing
   too much) escalated to the PM directly — not a design problem to solve inside this
   sprint.
6. `architecture.md`'s ledger is reconciled at sprint close: L6/L7 point at the accepted
   Sprint-2 specs; L5 is marked closed, citing `anthropic-recognizer.md`; L4 remains open,
   explicitly not settled by this sprint.
7. The sprint document carries role retrospectives; the sprint is committed with only its
   intentional files.

## In Scope — ordered slices

Four slices, shared floor first, hardest last — each slice gates independently
(Sprint 2/3 pattern).

**Slice 1 — `scan_safe` (small, mostly consolidation).**
Deliverable: `docs/proxy/spec/scan-safe.md`. The one total-function proof for the
committed shared scan: domain = 256 byte values, mapped to in-alphabet / delimiter (`0x1F`)
/ reject; the ≥1-alphanumeric and empty/traversal rejection arms enumerated; the
verify-never-transform property stated. Cites `segment.rs` as the committed artifact.
Gate: full SA1–SA7 (below).

**Slice 2 — the leaf-validator total-function proof (consolidation).**
Deliverable: `docs/proxy/spec/leaf-validator.md`. Cites Slice 1 for the per-leaf scan; adds
the array-level proof (element-count bound, ≥1-element requirement) that closes the
leaf-validator as a named security-core member: total over the leaf-array domain, every
input maps to admit or deny, no unhandled case. Cites `filesystem-write-path.md` for
mechanism — does not re-derive it. Gate: full SA1–SA7.

**Slice 3 — the cloud route/address recognizer total-function proof (consolidation+).**
Deliverable: `docs/proxy/spec/address-recognizer.md`. Cites Slice 1 for the scan; the
domain is `(scheme u32) × (safe segment stack)`, every point maps to a register-key match
or a deny. Cites `trusted-address.md` (Address type, assembly, byte-exact match,
scheme-0-only) and `box-proxy.md`'s net-address row for mechanism; the injectivity proof is
cited from `trusted-address.md` §3, not re-derived. Gate: full SA1–SA7.

**Slice 4 — the Anthropic body-shape recognizer (genuinely new, largest, own gate).**
Deliverable: `docs/proxy/spec/anthropic-recognizer.md`. The concrete shape, stages in
order:
- **Stage 0 (the crux)** — a bounded, total structural JSON reader: fixed maximum nesting
  depth (deny past it), fixed maximum size (reusing the existing reassembly cap,
  `library.md` §8), a restricted accepted profile (not general RFC-8259) — deny-default at
  every branch. This is the explicit, counted, reviewable core; the sprint's central
  discipline is that this reader is small-and-ours, never a general parsing library.
- **Stage 1** — envelope exact-match (verb/domain/path), reusing the exact-match
  discipline from Slice 3.
- **Stage 2** — top-level field whitelist (known-inert keys only; any other key denies the
  whole request — this is what excludes `mcp_servers`/`container` *by construction*, not by
  enumeration of what to block).
- **Stage 3** — `tools[]` per-entry, recursing to full depth: a `type`-tagged entry admits
  only on exact string match to a vetted client-side identity; a bare (no-`type`) entry
  admits as structurally-bounded custom-tool shape; anything else denies the whole request.
- **Stage 4** — beta-header whitelist (capability-inert only; empty admit-set is the
  recommended default).
Output is `Admit(forward verbatim) | Deny(reason, legible)` — never rewrite, never
strip-and-proceed. Gate: full SA1–SA7 **plus SA8 (parse-fidelity, below)** — the strictest
gate the team has run, given zero rewrite margin downstream.

## Non-Goals

- Do not re-open Sprint 2's settled mechanisms (`filesystem-write-path.md`,
  `trusted-address.md`) — cite and prove, never re-specify. A defect found while proving
  routes as a Sprint-2-spec finding, not a silent fix.
- Do not settle the Anthropic recognizer's placement (L4) — state the spec as
  placement-neutral; route the fork to cloud/design.
- Do not import a general-purpose JSON parsing library for the Anthropic recognizer's
  Stage 0 — the bounded structural reader is specified, counted, and reviewed as
  small-and-ours; a general parser is exactly the unsound unbounded-grammar bet the spine
  forbids, and it would hide uncounted bulk against the budget.
- Do not chase the Anthropic API's "current" state for the admit-list — pin the snapshot
  date, fail closed on drift, treat a new version as a deliberate re-admit.
- Do not let the recognizer's simplicity look better than it is by excluding a dependency
  (a JSON lib, a regex engine) from what's counted and reviewed — every reviewable logic
  line the recognizer's soundness rests on is in scope, full stop (design's ruling: ~500 is
  a forcing function for simplicity, not a quota — but that makes hiding complexity worse,
  not beside the point).
- Do not treat a scalar (depth cap, size cap, element-count bound) as a blocking judgment
  call — assign, mark provisional, proceed; the *shape* of the cap (bounded-and-fail-closed)
  is settled, only the *number* is provisional.

## Starting Surface

Audit first, in this order:

1. The committed shared floor: `proto/src/segment.rs` (`scan_safe`, committed clean),
   `proto/library.md` §8 (residence).
2. The two accepted Sprint-2 specs, cited for mechanism, not reopened:
   `filesystem-write-path.md`, `trusted-address.md`.
3. The Sprint-3 boundary contract that already touches these seams:
   `docs/proxy/contract/box-proxy.md`'s net-address and net-payload rows (the recognizer's
   internals are explicitly marked "a hole with an owner" there — this sprint is that
   owner's work, for the address recognizer's share and the Anthropic recognizer's).
4. `docs/proxy/architecture.md`'s ledger, especially L4–L7 (L6/L7 confirmed stale, to
   reconcile at close; L5 is this sprint's target; L4 stays open).
5. `docs/proxy/cloud/anthropic-api.md` in full, especially §6 (the admit-list, exact) and
   §8 (open design questions — placement, exact recognized shape, reviewability
   requirement).
6. The superseded byte-map reference (`docs/explore/console-and-protocol-design.md`) and
   its retraction (`docs/proto/display.md` §4/§6) — read once, for the scoping record; not
   a target.

## Method

This sprint reuses Sprint 2's SA1–SA7 frame for Slices 1–3 (flat/consolidation proofs) and
sharpens it for Slice 4, where the recognizer's recursive, attacker-owned-representation
shape introduces a genuinely new risk class:

- **The attack surface for a recursive recognizer is every node and every
  capability-bearing field**, not a byte class. Re-deriving it means enumerating every
  capability-bearing field (not just `tools[]` — also `mcp_servers`, `container`, beta
  headers), every recursive descent point, and the depth boundary. The reviewer constructs
  the adversary's shapes independently and confirms each denies against the **real
  recognizer's output**, never a hand-computed "expected" (Sprint 2's standing discipline).
  This synthetic corpus — admissible and inadmissible shapes classified by the actual
  recognizer — becomes the permanent regression guard.
- **Parse-fidelity is a first-class, separately-gated property (SA8).** Because the
  recognizer reads bytes it does not own and the proxy forwards them verbatim, the
  recognizer must parse the *same* bytes Anthropic's server will parse or reject anything
  ambiguous (duplicate keys, non-canonical encodings) fail-closed. This has no analog in
  the flat mechanisms and is treated as its own gate, not folded quietly into general
  closure.
- **Simplicity is measured, not assumed — the ~500 is a forcing function, not a quota**
  (design's ruling, 2026-07-19). A living ledger table (§ Acceptance) tracks every member's
  line count across all four slices as a data point, not a pass/fail sum; `scan_safe`
  counts once; the JSON reader counts inside the Anthropic number so it can't hide bloat.
  The real property is **simplest-reviewable-form plus human-holdable-in-one-sitting**,
  gated at every checkpoint — leanness scrutiny concentrates on the Anthropic recognizer,
  the piece most at risk and the proxy's own code, not on tier-2 plumbing.
- **Judgment calls are routed, not settled** (the SA7 discipline, carried forward): three
  of four (field whitelist, JSON profile shape, beta policy) are now resolved by design;
  the vetted-identity strings and L4's placement remain routed — the recognizer's
  totality does not depend on any of them being ratified yet.

## Acceptance

Seven gates per spec (SA1–SA7, carried from Sprint 2, sharpened where the recursive
recognizer demands it), plus an eighth for Slice 4 specifically, plus the budget-ledger
check:

- **SA1 — Requirement traceability.** The spec states the exact invariant of its source
  requirement, verbatim where available — not a weaker cousin. For Slice 4: "deny *unless*
  proven wholly inert," admit only the two named shapes, recurse to full depth,
  default-deny on anything unrecognized (`anthropic-api.md` §6/§8, quoted).
- **SA2 — Totality/enumerability.** A total function over a finite, enumerable domain; no
  unhandled case. For Slice 4, two sub-parts: the *parse* is a bounded total function over
  the frame-bounded body (a terminating grammar, not the unbounded-grammar bet, provided
  depth is bounded and fail-closed); the *recognition* is exact-match-whitelist with
  default-deny **at every node**, not just top-level.
- **SA3 — Adversarial closure (the load-bearing gate, sharpest for Slice 4).** The reviewer
  independently constructs the adversary's attempts and confirms each is denied by the
  *real* recognizer's output. For Slice 4, the attack surface is every capability-bearing
  field and every recursive descent point — required closure rows include a server-side
  type, `mcp_servers`, `container`, a capability-gating beta header, a new-version stamp of
  a vetted tool, an unknown top-level key, and a depth bomb.
- **SA4 — Residual named and bounded.** Every mechanism's residual is named and shown
  bounded; REJECT on an unstated one.
- **SA5 — Primitive-semantics grounding.** Every claim about what a primitive (a syscall, a
  parsing routine) actually does is verified against its real contract, not assumed.
- **SA6 — Constructive where possible; recognize-don't-rewrite where not.** Slices 1–3 stay
  constructive (a bad byte/segment is unrepresentable). Slice 4 cannot be constructive (the
  box authors the body) — SA6 reshapes to *recognize-don't-rewrite*: deny-unless-exact-
  inert-match, never edit into acceptability.
- **SA7 — Judgment calls routed, not settled.** Every genuine fork (admit-list strings,
  field whitelist, JSON profile/caps, beta policy, L4 placement) is named, given a
  recommended default, and routed — never settled unilaterally. REJECT + route if presented
  as a settled conclusion.
- **SA8 — Parse-fidelity (Slice 4 only, the genuinely new gate).** The recognizer parses
  the *same bytes* the authority (Anthropic) will parse, or rejects anything ambiguous
  fail-closed. Required closure rows: duplicate JSON keys, non-canonical unicode escapes,
  and any other parse-disagreement vector the reviewer independently derives. REJECT on any
  gap between what the recognizer accepts and what the authority would interpret
  differently.

**The budget/leanness check (superseded framing — design ruling, 2026-07-19)**: the ~500
is a forcing function for simplicity, not a quota to divide among members or a ceiling to
sum against. The ledger tracks, per slice, each member's line count as a data point toward
one real question: **is this piece in its simplest reviewable form, and can a human still
hold the whole set in one sitting?** Leanness scrutiny concentrates on the Anthropic
recognizer specifically (Slice 4) — the piece most at risk of bloat and the proxy's own
code; tier-2 plumbing is out of this sprint's leanness scope entirely. A recognizer that
is *not* in its simplest form, or a whole set that a human cannot hold together even after
minimization, is a REJECT + design finding — the latter case specifically escalates to the
PM as a scope question (per the trip-wire, Review Question 4), not something this gate
tries to resolve by trimming further.

## Review Questions

1. **Spec file location (ruled by lead, mechanical, same class as prior sprints):** four
   new files under `docs/proxy/spec/` (`scan-safe.md`, `leaf-validator.md`,
   `address-recognizer.md`, `anthropic-recognizer.md`) — one total-function proof each, so
   the security-core set is a legible directory. The Sprint-2 specs remain cited for
   mechanism, not absorbed.
2. **Ledger reconciliation ownership (ruled by lead):** `architecture.md`'s ledger (our own
   composition document, not another team's) is reconciled by this team at sprint close —
   L6/L7 pointed at the accepted Sprint-2 specs, L5 marked closed citing
   `anthropic-recognizer.md`, L4 left open. Not routed externally; ours to fix directly.
3. **The consolidated judgment-call list — three of four RESOLVED by design (2026-07-19),
   one still pending the operator directly:**
   - JA-1 — the exact vetted client-side identity strings admitted at freeze (pinned to a
     dated API snapshot); **pending** — routed by the PM directly to the operator (the
     tool-identity-level standing-authorization register), not yet returned.
   - JA-2 — **RESOLVED (design): the minimal known-inert top-level field set, deny the
     rest, extend later via the promotion pattern** — exactly the recommended default.
   - JA-3 — **RESOLVED (design): the bounded/fail-closed shape is confirmed; numbers stay
     provisional placeholders; keep depth/size as shallow as the whitelisted fields
     actually require, tying the outer bound to the existing 32 MiB reassembly cap.**
   - JA-4 — **RESOLVED (design): an empty admit-set — deny all capability-gating beta
     headers by default.** Design flagged this as the most important of the three: beta
     headers are a real capability-escalation vector; any future beta header is an
     operator-reviewed addition, never a default.
   - L4 — recognizer placement (in-transport module vs. isolated service); not this
     sprint's to settle, routed to the cloud subsystem design; the spec states its
     placement-neutrality explicitly.
4. **RESOLVED (design, 2026-07-19) — budget apportionment reframed, superseding the
   "whole-TCB line-sum vs 500" framing this Review Question previously carried.** Design
   confirmed the ~500 (spine §15) is a **forcing function for simplicity, not a quota**: no
   four-member/tier-2 split to arithmetic-check, no raw sum to gate against a hard ceiling.
   The operative question is **simplest-reviewable-form + human-holdable-in-one-sitting**,
   tracked per Final State item 5. Leanness effort concentrates on the **Anthropic
   recognizer specifically** — the proxy's own code, the piece most at risk of bloat — not
   on tier-2 (the NIC team's lever, already minimal, out of this sprint's scope). The
   bounded-JSON-reader discipline stands as a hard constraint because it is the simplest
   design that guarantees deny-by-default, not because of a line target. Trip-wire carried
   forward: if the recognizer reaches its genuinely simplest form and the whole TCB still
   cannot be held in one sitting, that is a scope question (the proxy doing too much) for
   the PM directly — not expected at the current estimate.
5. **Currency-debt trend (surfaced for the PM's attention, consistent with prior
   sprints):** the Anthropic admit-list snapshot is already a month stale relative to this
   sprint's start date; the spec pins and fail-closes rather than chasing currency — worth
   the PM's continued awareness as a standing operational pattern, not a defect of this
   sprint.

## Retrospectives

### zero.proxy-review

**Evidence that changed the gating approach across this sprint's split shape.** This sprint held
two shapes at once: Slices 1–3 were *consolidation* (prove an already-accepted mechanism as a
total function by enumeration), Slice 4 was a genuinely *new* recursive-recognizer gate. The
approach split accordingly. For consolidation, the load-bearing discipline was the **honest-
boundary table** — explicitly separating this member's NEW content (its own enumeration/composition
proof) from what it INHERITS (the committed `scan_safe`, the accepted Sprint-2 specs), so the proof
claims only its own ground and the budget counts only new lines. Slice 2 named the ≥1-element lower
bound as *inherited* from `scan_safe`'s `EmptyStack` (not double-counted); Slice 3 **cited** the
`trusted-address.md` §3 injectivity proof rather than re-deriving it, adding only the composition
totality. The gate for a consolidation is: re-derive the new arithmetic/enumeration, verify the
cited proof is faithful and actually relied upon, and confirm the boundary table doesn't over-claim.
For the recognizer, the gate was the strictest the team runs (SA1–SA7 + SA8) and the method was a
full independent recursive-attack-surface re-derivation.

**What SA8 / parse-fidelity taught about gating a recognizer that reads bytes it doesn't own.** The
flat mechanisms (Slices 1–3) let the proxy *own* the representation — a bad byte is unrepresentable.
The recognizer does not: it forwards the body **verbatim** and the *authority* (Anthropic) re-parses
the same bytes, so a parse-*disagreement* is exploitable — a risk class with no analog in the flat
proofs. Two things made the gate tractable. First, the sound guarantee is **"admit nothing whose
interpretation is parser-dependent"** (provable by the recognizer's own logic), not "parse
identically to the authority" (unprovable against a parser we don't control) — admitting only
unambiguous bytes forces every conformant parser, including the upstream's, to agree. Second, the
whole soundness rests on one lever: the bounded reader must be a **deny-default whitelist** (admit
only fully-recognized canonical structure, deny the default of every branch), which makes the
parse-differential closure complete *by construction*. The reviewer's job is to verify the reader is
deny-default — **not** that the ambiguity *enumeration* is complete (that enumeration is
illustrative; a denylist of known ambiguities would be the unsound shape). And the rejection must be
*reject, never rewrite-to-canonical*, or it breaks the box's own body-integrity hash and launders
intent.

**The spec-vs-code distinction I raised at Slice 4, and why it matters for the next sprint.** The
code is not built — the budget is *estimated from spec*. So the Slice-4 gate certifies the
**design** closes the surface; it does **not** certify a built recognizer. The
R-IGNORE/permissive-parse risk (a build that admits an unrecognized construct instead of denying)
lives in the **build**, not the spec. Therefore the build-time obligation stands and is
load-bearing: a **synthetic full-field corpus** (every SA3/SA8 row, admissible and inadmissible)
classified by the **real recognizer** is the permanent regression guard that proves the code
implements the specified deny-default. Accepting the spec is not trusting an unbuilt recognizer —
"a behavior is proven only where exercised." This must survive into the record so a future build
effort inherits it as a gate, not rediscover it or mistake spec-acceptance for code-certification.

**What the next proxy sprint inherits.**

- *The build handoff itself.* The four total-function specs are the artifact the build implements
  against and review certifies against. The next sprint is plausibly the build — and its acceptance
  gate is code-fidelity-to-spec, whose centerpiece for the recognizer is the synthetic-corpus-vs-
  real-code obligation above.
- *Two open items to carry, not re-derive:* JA-1 (the operator-pending vetted tool-identity strings
  — the recognizer's totality is independent of the set's contents, but the build needs the
  operator's ratified set, snapshot-pinned, fail-closed on drift); and L4 (recognizer placement,
  cloud/design's, the spec is placement-neutral).
- *The ledger reconciliation at close* (ours to do): L6/L7 → the accepted Sprint-2 specs, L5 →
  closed citing `anthropic-recognizer.md`, L4 → open.
- *The standing review disciplines, now battle-tested across four sprints:* currency-of-source on
  *every* citation (peripheral ones hide a stale one); verify-on-disk, never assume a pin;
  cite-the-accepted-proof, don't re-derive; the honest-boundary table for consolidations; the
  deny-default-completes-by-construction lever for structural recognizers; and the spec-vs-code
  framing for any design gate ahead of a build.

### zero.proxy-dev (author)

**What worked, and why it should be kept.** Ordering the slices shared-floor-first paid off
structurally: `scan_safe` proven once (Slice 1) became the cited floor under both
consolidation proofs, and the consolidations (Slices 2–3) established the total-function
proof *format* — domain partition, composition-is-total, honest-boundary table — that the
hard recursive case (Slice 4) then followed. By the time I reached the recognizer, the
proof shape was routine and all the novelty could go into the one place it belonged: the
bounded reader. Keep shared-floor-first for any layered-proof sprint.

**What evidence changed the plan (or caught an error).**
1. *Two currency-pin corrections, both caught by the disk check, not by memory.* Slice 2
   found `filesystem-write-path.md` was committed (`bf56dc6`), not working-tree-dirty as I'd
   carried forward from Sprint 3; Slice 3 caught my own fresh error pinning
   `trusted-address.md` at `bf56dc6` when it was `c4690bf`. The rule that earned permanent
   status: **verify every commit pin on disk, never carry a remembered one** — a pin is a
   claim about the world, and the world moved twice inside one sprint.
2. *The budget reframe (design's ruling mid-Slice-1).* The ~500 flipped from quota to
   simplicity-forcing-function while my Slice-1 report was in flight. Re-reading the updated
   contract on disk rather than reframing from the lead's summary is what kept §8's new
   language accurate. Rule: **when a framing changes upstream, re-read the changed artifact,
   don't paraphrase the change notice.**
3. *The counted-on-disk vs estimated-from-spec distinction.* It emerged from being honest
   about `scan_safe` (real code, 35 lines) sitting in the same ledger as three
   not-yet-implemented members. Making the basis explicit is what kept the budget from
   reading as more settled than it was.

**What needed correction.** No slice was rejected for a defect, but three things were
sharpened by review or by me: the §8 quota→simplicity reframe (one-line, design's ruling);
the two pin corrections (mine, caught pre-ship); and the two closeout additions to
`anthropic-recognizer.md` — the load-bearing **build-time obligation** note (this gate
accepts the *design*; the corpus-against-real-code is the standing obligation that proves
the *build* implements the deny-default) and the one-line `tool_choice` inertness rationale
(references a tool, never defines/executes one).

**The one design lever worth carrying forward.** For a structural recognizer over an
attacker-owned representation, **owning a bounded reader closes three risks at once** — it
keeps the parse off the unbounded-grammar bet the spine forbids, it removes the
parse-disagreement gap (ambiguity is *rejected*, not resolved by a foreign parser's rules),
and it keeps the largest trusted-surface member counted and enumerable instead of hidden in
a library. The SA8 reframe is the general form: **you cannot prove you parse identically to
an authority you don't control, so admit nothing whose interpretation is parser-dependent.**

**What the next sprint inherits.**
(a) **The build handoff itself** — four total-function proofs (`scan-safe.md`,
`leaf-validator.md`, `address-recognizer.md`, `anthropic-recognizer.md`) are the artifacts
the implementer builds against and review certifies against; the Anthropic recognizer
carries a **required** corpus-against-real-code obligation, not optional.
(b) **JA-1 is the one open input** — the vetted tool-identity strings, pending the operator,
pinned to the 2026-06-20 snapshot, fail-closed on drift; the recognizer's totality does not
wait on it.
(c) **L4 (recognizer placement) stays open**, routed to cloud/design; the spec is
placement-neutral.
(d) **The ledger is reconciled** (L5 closed, L6/L7 pointed at the Sprint-2 specs) — and one
soft imprecision is surfaced for the lead: `architecture.md`'s lifecycle prose tags an
*atomic-completion* sub-question (write-to-temp-then-rename) as "L6," which the now-closed L6
(open-discipline primitives) does not actually cover — likely its own small open item, the
lead's call whether it earns a ledger row.
(e) **The two reusable conventions** — total-function-proof format for a security-core member
(domain partition + composition + honest-boundary table + adversarial closure), and the
counted-on-disk-vs-estimated-from-spec budget basis so a build-vs-design ledger never reads
as more settled than it is.

### zero.proxy-lead

**A design ruling mid-flight is a contract update, not a footnote — and the team's
independent convergence on it was the real signal.** Design's budget reframe (the ~500 is
a forcing function for simplicity, not a quota) arrived while Slice 1 was already in the
reviewer's queue. I updated the contract's Final State, Method, Acceptance, and Review
Questions before routing anything further, rather than letting the old framing coexist with
the new ruling in different parts of the document. What made this easy to get right: the
reviewer, entirely independently, had pushed a *stricter* version of the same tension during
planning (the whole-TCB arithmetic risk) — and when the ruling landed, they recognized it as
a correction of their own prior framing, not a relaxation to resist. Two people converging
from opposite directions on the same fork, then one external ruling resolving it cleanly for
both, is about as strong a signal as this loop produces that the fork was real and the
resolution is sound. **Rule: when a mid-sprint ruling changes a framing already in play,
propagate it through the whole document immediately, not just the section it literally
answers** — a contract with one section on the new ruling and three still assuming the old
one is worse than no ruling at all.

**The recognizer's SA8 reframe is the sharpest single piece of reasoning this team has
produced, and it happened without my prompting.** The contract asked for "parse-agree-with-
the-authority" as the parse-fidelity gate. Dev independently replaced it with "admit nothing
whose interpretation is parser-dependent" — a strictly stronger, *provable* property (it
doesn't require modeling a parser we don't control) that the reviewer confirmed subsumes
the weaker framing. Neither the contract nor I supplied this; it emerged from dev actually
sitting with the requirement rather than executing the contract's wording literally. **Rule:
a contract's gate description is a floor, not a ceiling — when an author replaces a hard-
to-verify property with a stronger, verifiable one and can show the substitution is sound,
that is exactly the kind of improvement to welcome into the sprint record, not flag as
scope drift.**

**The spec-vs-code distinction the reviewer raised at the Slice-4 gate is this sprint's
most important inheritance, and it almost went unrecorded as a "verdict footnote."** An
ACCEPT on a total-function proof of an unbuilt mechanism is easy to misread later as "the
recognizer is trusted" — it is not; it is "the design, if built faithfully, is trusted."
The reviewer stated this explicitly and unprompted, and I made sure it now lives inside the
spec itself (the build-time obligation note), not just in a chat verdict that will scroll
out of view. **Rule: when a gate's scope is narrower than its plain-language verdict
implies ("ACCEPT" sounds total, but here it is spec-only), get the caveat into the artifact
itself, not just the routing message — the artifact is what the next reader inherits, not
the conversation.**

**Scoping "the byte-map" out correctly, before any planning began, saved the sprint from a
wasted first slice.** The PM's brief named a mechanism that had been retired two commits
ago in the team's own history. Verifying this against `display.md` myself before sending
planning asks — rather than assuming the brief was current and building around a fourth,
nonexistent target — meant dev and reviewer's planning input arrived already aligned on
three targets, not reconciling a false fourth. **Rule: verify a brief's named deliverables
against current source before framing planning asks around them, especially when the brief
references older architectural vocabulary** — the cost of checking is minutes; the cost of
two people planning around a retired mechanism is a full round-trip.

**What the next sprint inherits.** (a) The build handoff is real: four total-function specs,
ready for implementation, with the synthetic-corpus-vs-real-code obligation as the standing
certification gate once code exists — the next sprint that touches the recognizer's
implementation inherits that obligation, not a green light. (b) JA-1 (tool-identity strings)
is still pending the operator directly; L4 (placement) is still cloud/design's. (c) The
small L16 ledger split (atomic completion, mistagged under the now-closed L6) is a genuine,
if minor, finding worth tracking — filesystem design review owns it. (d) The two process
disciplines worth carrying forward without re-deriving them: propagate a mid-flight ruling
through the whole contract immediately, and treat a strictly-stronger self-derived property
as a welcome improvement, not a deviation to flag.
