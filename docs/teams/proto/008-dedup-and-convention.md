# Sprint 008 — Behavior-Preserving Dedup & the Shared-Core Convention

> A behavior-preserving cleanup sprint, before the coverage closeout: dedup the **mechanical** boilerplate
> the encode/decode surface repeats (the verbatim-tail impl; the fixed-prefix geometry check) **without
> changing a single byte of behavior**, and clarify the library §8 **grammar-reuse / shared-core
> convention** so the boundary is explicit. The audit (dev + reviewer, both re-derived from source) found
> the 1,668-line surface is **lean, not bloated** — most is irreducible auditable surface (the dispatch
> table, the verb/field consts, the per-verb fields) — so this sprint takes only the **auditability-neutral-
> or-positive** reductions and **declines** the big cuts that would hide the reviewable TCB.
>
> Team: `proto` (lead `zero.proto-lead`, dev `zero.proto-dev`, review `zero.proto-review`).
> Grounded in the 001–007 contracts + the closeout audit; touches [`library.md`](../../proto/library.md) §8.

---

## Scope Boundary

**This sprint owns** (all byte-identical behavior; no anchor edits):

1. **`verbatim_verb!` macro** — the 4 pure verbatim-tail verbs (`NetResponse`, `NetPayload`, `FileData`,
   `Paste`) share an identical `Wire` impl (`decode = Ok(Name(body.to_vec()))`, `encode = body`). A macro
   dedups the **impl** while keeping each a **distinct type** (`NetResponse` ≠ `Paste`). No security logic
   (a pure passthrough — no routing, no value-set), so macroing it hides nothing auditable; it *names* the
   raw-passthrough at each site. (~24–40 lines.)
2. **The fixed-prefix geometry helper** — `split_first_chunk::<N>() + ok_or(ShortBody{needed:N, got})` is
   the same geometry-check operation at ~25 sites. A narrow helper (e.g. `take_prefix::<N>`) names that
   shared grammar (audited once) while the **per-verb field reads stay explicit**. Applied to the
   **non-anchored** decoders only (input / pane_state / list_entry / net / envelope / viewport / focus /
   mouse) — never the anchored files. Auditability-positive. (~50 lines.)
3. **The library §8 convention clarification** — state the shared-core boundary explicitly (below).

**This sprint does NOT own** (named, with the reason):

- **A carry-raw enum *routing* macro** (Button/MouseKind/KeyKind/Scheme/ErrorCode/FileType) — **excluded.**
  `Cell`'s `payload24!` (the cited precedent) is a payload-*newtype* wrapper, **not** the carry-raw routing;
  `Cell::from_u32`/`to_u32` is hand-written and explicit. The `from_*` routing (named-vs-`Other`,
  enforce-by-construction — "can `Other` hold a defined value?") is the **security-critical, auditor-checked
  logic**; macroing it hides exactly what the reviewer verifies each acceptance and is a single point of
  failure across all carriers. The routing **stays explicit**.
- **Macroing the dispatch table** — it is the auditable verb→decoder TCB artifact; stays explicit.
- **A whole-body geometry macro** (`fixed_body!`) — obscures the per-verb byte layout; declined.
- **Any anchor edit** (segment/cell/dimensions/errors), **any behavior change**, **any type merge** (types
  stay distinct), and **the coverage closeout** (the next sprint).

## Problem

The operator found 1,668 auditable lines heavy and asked whether core code could be shared. The audit's
honest finding: the surface is **explicit by design**, and the explicitness *is* the auditability — most
of the line count is irreducible (the dispatch table, the consts, the per-verb fields) or is the
spec-grounding doc-comments (an asset). The genuinely-shared grammars are **already** factored and reused
(Modifiers by mouse+key; Dimensions by create-pty+resize; ErrorBody by file-error+net-error;
`scan_safe`/`scan_unsafe`; `decode_record_tail`; `decode_list_entries`). What remains is two pieces of
**mechanical** boilerplate (the verbatim impl; the geometry check) and an **unstated convention** for what
may and may not be shared.

## Goal

Dedup only the mechanical boilerplate, **byte-identical** — every existing vector passes unchanged, the
five anchors stay byte-identical, the build matrix + per-slice coherence + the coverage figure are
unchanged. The carry-raw routing, the dispatch table, and the consts stay **explicit** (the reviewable
TCB). The library §8 convention is clarified so the shared-core boundary is a written rule. The auditable
line count drops modestly; auditability is **neutral-or-better**.

## Final State

- A `verbatim_verb!` macro; the 4 verbatim verbs use it (distinct types preserved).
- A `take_prefix::<N>`-style geometry helper; the non-anchored fixed-prefix decoders call it; per-verb
  field reads stay explicit.
- library §8 states the convention: **dedup the implementation (codegen for uniform mechanical
  boilerplate), never the types or the dispatch table or the consts; name + reuse genuine shared
  *grammars* (the same semantic thing — Modifiers/Dimensions/ErrorBody); never factor a *coincidental*
  layout (e.g. `key` and `ErrorBody` share a shape, not a grammar — factoring would wrongly couple); the
  carry-raw enum *routing* stays explicit (the security-critical surface), while a simple payload-newtype
  wrapper (`payload24!`) may be macro'd.**
- The five anchors byte-identical; all vectors unchanged; the new auditable line count reported.

## In Scope — ordered slices

- **8A — the dedup refactor.** `verbatim_verb!` + the geometry helper, applied byte-identically. *Gate:* all
  existing vectors pass **unchanged**, the 5 anchors byte-identical, the build matrix + per-slice coherence
  + the coverage figure unchanged, G2/G6.d, the line-count delta reported.
- **8B — the §8 convention clarification** (the library.md edit). *Gate:* review confirms it matches the
  lived reality (the already-reused grammars) + the rulings here.

The developer produces an **8A pre-carve manifest** (the exact sites + the per-caller `ShortBody` check)
and **stops for review before the carve.**

## Non-Goals

- The carrier routing macro; the dispatch macro; the whole-body geometry macro (all hide the TCB).
- Any behavior change (this is byte-identical), any anchor edit, any type merge.
- The coverage closeout (next sprint) and the `list_entry:143` keep/remove/hook call (operator-pending).

## Starting Surface

The 4 verbatim verbs (`net.rs`/`file.rs`/`input.rs`); the non-anchored fixed-prefix decoders; `library.md`
§8. The five anchors (segment b3275467 / cell 8bf962e2 / display a24b83a2 / dimensions 7e339e0c / errors
a0893df6) stay **byte-identical** — none is touched. The refactor changes non-anchored files, but their
**behavior** is byte-identical (vector-proven).

## Method

- **Byte-identical behavior is the whole discipline.** Every existing vector + round-trip + the both-fuzz
  corpora pass **unchanged** (a dedup that changes a byte fails a vector). The five anchors are recomputed
  byte-identical. The 8-config build matrix + per-slice coherence + the coverage figure are unchanged. This
  is a pure extract, not a behavior edit.
- **The geometry helper must preserve each caller's `ShortBody{needed, got}` exactly** — verify *per
  caller* that `got = body.len()` at the split point (a partial-consume caller computing `got` differently
  would change its `ShortBody`; the vectors pin each, but verify per caller before factoring, don't rely on
  the vectors alone).
- **The macro names the pattern, keeps types distinct.** `verbatim_verb!` dedups the impl; the types stay
  separate. No security logic enters a macro (the carry-raw routing is excluded for exactly that reason).
- **Auditability is the metric, not line count** — every change is neutral-or-positive on reviewability;
  the big line-reducers that hide the TCB are out of scope.

## Acceptance

- **Behavior preservation (the core gate).** All existing vectors + round-trips pass **unchanged**; both
  committed fuzz corpora pass (no re-min needed — no new inputs); the five anchors byte-identical. A changed
  vector result or a changed anchor = REJECT.
- **The geometry helper** preserves each caller's `ShortBody{needed, got}` (per-caller verified); the
  per-verb field reads remain explicit in source.
- **`verbatim_verb!`** produces byte-identical encode/decode for the 4 verbs; the types stay distinct.
- **The build matrix** (8 configs × 2 targets, `-D warnings`) + the per-slice coherence are unchanged
  (green); the **coverage figure** is unchanged (the refactor is behavior-neutral; same lines exercised).
  G6.d both targets.
- **The §8 clarification** matches the lived reality + the rulings; review confirms.
- **The line-count delta** is reported (the new auditable count); auditability neutral-or-better.

## Decided Forks (lead rulings)

1. **Carrier routing macro — EXCLUDED** (the routing stays explicit; `payload24!` is a newtype wrapper, not
   the routing — auditor-governed; the security-critical surface stays readable).
2. **`verbatim_verb!` — INCLUDED** (no security logic, names the passthrough, distinct types; the closest
   call — review confirms it's auditability-neutral, not negative).
3. **The geometry helper — INCLUDED**, non-anchored decoders only (auditability-positive; per-caller
   `ShortBody` verified).
4. **The dispatch table, the consts, the carry-raw routing — stay EXPLICIT** (the reviewable TCB).
5. **Types stay distinct; no anchor edit; byte-identical behavior.**
6. **§8 convention clarified** as the written shared-core rule (above).

## Escalations

- **None.** The operator set the scope (the small safe refactor + the §8 clarification). The §8 wording is
  recorded for operator review at green-light. The `list_entry:143` coverage call + the coverage closeout
  remain the next sprint's, operator-pending.

## Review Questions

1. Confirm the §8 convention wording matches the design-of-record. (Operator/PM — review at green-light.)
2. Confirm the 8A creation manifest (the exact sites + per-caller `ShortBody`) before the carve. (Review.) —
   **Confirmed; 8A accepted byte-identical, 8B §8 landed.**

---

## Postmortem — Sprint 008 retrospectives

*Sprint 008 closed 2026-06-25: a behavior-preserving cleanup before the coverage closeout. 8A landed
byte-identical (the `verbatim_verb!` macro + the `take_prefix` geometry helper; line count 1,668 → 1,632,
−36), accepted by independent review on source-provable byte-identity; 8B landed the §8 shared-core
convention. The honest finding: 1,668 was lean, not bloated — the surface is explicit by design and the
explicitness is the auditability. The carrier *routing* macro was declined (it hides the security-critical
enforce-by-construction). **Disk-tied carry-forward:** the host disk was at 100% mid-sprint, so the
*fresh* coverage figure and the reviewer's *independent dynamic re-run* of the 8-config matrix + 122 tests
are deferred to the coverage sprint, once disk is freed; 8A's acceptance rests on the source byte-identity
(independent) + the dev's relied-on dynamic run, with that re-run condition logged. Durable lessons.*

### Developer (`zero.proto-dev`)

(1) *In a small reviewable TCB, line count is the wrong metric — auditability is*; most of 1,668 is
irreducible auditable surface (dispatch table + verb/field consts + per-verb fields) + the 648 grounding
comment lines (an asset); only ~2% was worth factoring (`verbatim_verb!` + `take_prefix`, behavior-trivial)
— the carrier-routing macro was correctly declined (it would hide the carry-raw `from_` routing). (2) *A
"pure extract" lives or dies on per-call-site invariant verification, not the suite passing* — verified
per caller that the existing `ShortBody{needed, got}` matched the helper (not via vectors); that reading
produced the got-trap, the CLASS A / CLASS B distinction, and the list_entry exclusion (conservatively —
the riskiest variable-loop decoder, marginal saving, a coverage call pending on the same file). (3) *Don't
burn the operator's resources to satisfy a gate* — the host disk was at 100%; the instrumented build was
not re-run (coverage preserved by construction; the fresh measurement routed to review's independent
re-run); a near-full disk also gives *spurious* failures (verify the environment before trusting a red).
*Process note:* surfacing ≠ landing — the build-resolved sibling §8 open-decisions were flagged as an
optional pass, not landed (library §8 is the designer's record, the owner's call).

### Reviewer (`zero.proto-review`)

(1) *Auditability is the metric, not line count — and on a TCB surface the two point opposite at exactly
the security code*; the reviewer who re-derives the surface is the right party to call which repetition is
the audit and which is waste; take only auditability-neutral (`verbatim_verb!`, a pure passthrough) or
positive (`take_prefix`, names the geometry grammar, keeps per-verb fields explicit) reductions, decline
the cut that hides the TCB (the routing macro). (2) *A pure extract's byte-identity is source-provable —
the make-or-break is the per-caller invariant, not the aggregate green*; classify CLASS A (trivial swap) /
CLASS B (safe only via consolidation to one `take_prefix::<TOTAL>` + destructure — the naive per-split
wrap is the needed-trap *and* the got-trap) / CLASS C (excluded); verify per caller from source *before*
factoring; this caught the contract's candidate list as too broad before the carve. (3) *Honest
partial-independence under an infra blocker* — accept on what you can independently verify (source
byte-identity + sha256 anchors + `ls` corpora), name precisely what you *relied on* (the dev's dynamic
run), condition the re-run (disk-freed, folded into coverage), and label inference; do not fake completeness
and do not stall. A pure extract's source-provability is what makes a disk-constrained acceptance
legitimate rather than a guess.

### Lead (`zero.proto-lead`)

**Process lessons.** (1) *Pin the mechanism before ruling a fork.* The carrier-macro divergence (dev:
worth it, cites Cell's `payload24!`; reviewer: auditability-negative) resolved only once I read `cell.rs` —
`payload24!` is a payload-*newtype* wrapper, **not** the carry-raw routing (which is hand-written), so the
precedent didn't hold; I excluded the routing macro, the auditor's caution governing. A fork argued against
an *assumed* mechanism is a phantom. (2) *An honest finding can push back on the premise — surface it as a
consideration, let the operator decide.* The audit found 1,668 lean and the biggest cut auditability-
negative; I reported that (not "you're wrong"), recommended the small-safe set + the §8 clarification, and
gave the operator the scope fork (they chose option 1). (3) *An infra blocker is the operator's resource —
surface it, don't burn it; accept on the independent proof + condition the deferred check.* The disk-full
went to the operator (their system; the regenerable `target/` is the safe clean); 8A accepted on the
source byte-identity with the dynamic re-run disk-tied. (4) *On a doc you don't own, surface + wait for the
owner* — the §8 sibling-decisions marking is the designer's call, surfaced not landed. (5) *Consolidate the
operator's decisions into one clean cluster* (disk / §8-siblings / commit-timing / list_entry:143) rather
than dribbling them.

**Carry-forward to the coverage sprint.** The fresh coverage figure + review's independent 8A re-run (the
8-config matrix + 122 tests), disk-tied; the `list_entry:143` keep/remove/hook call (operator-pending, on
the file 008 deliberately did not churn); the §8 written convention now governs any future verb. The 008
commit + the §8-sibling marking are the operator's call at close (disk-tied).
