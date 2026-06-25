# Sprint 009 — Coverage Closeout

> The final closeout sprint: bring the library's **reachable** surface to full test coverage with 5
> targeted tests, **document** the one structurally-unreachable defensive arm (`list_entry:143`) as
> intentional, and report the fresh coverage figure. The operator is **satisfied with the current coverage
> for their purposes** — this does **not** chase literal 100% (the documented arm legitimately stays
> uncovered). It also closes the disk-deferred obligation from 008: the reviewer's **independent dynamic
> re-run** (the 8-config matrix + the full test suite), now that the host disk is freed.
>
> Team: `proto` (lead `zero.proto-lead`, dev `zero.proto-dev`, review `zero.proto-review`).
> Grounded in the closeout audit (the coverage measurement: line 95.75%, the 43 coverable lines + the 1
> unreachable arm) + the 008 contract (the deferred re-run).

---

## Scope Boundary

**This sprint owns:**

1. **5 targeted tests** for the 43 coverable library lines the audit found (all **integration tests via the
   public API** — see Method; the anchored files stay byte-identical):
   - `reject.rs` `Display::fmt` (27 lines, the whole impl — no test formats a `Reject` via `Display`):
     format every variant.
   - `segment.rs` `UnsafeSegment` redacting `Debug` (~3): `format!("{:?}", …)` on a decoded unsafe segment.
   - `segment.rs` `SafeSegmentStack`/`UnsafeSegmentStack::is_empty` (6): assert `is_empty()`.
   - `cell.rs` `RowCol::row()`/`col()` (6): construct via the pub `RowCol::new` + assert the accessors.
   - `input.rs` `Focus` `TrailingBytes` arm (1): decode a 2-byte focus body → `TrailingBytes`.
2. **Document `list_entry:143`** — the defensive multi-segment arm — as an **intentional unreachable
   defensive reject** (keep + document, the operator's call). A doc comment: structurally unreachable given
   the 0x1F-free split (the slice handed to `scan_unsafe` is 0x1F-free → always exactly one segment), a
   deliberate defense-in-depth guard per 3A. `list_entry.rs` is **not** an anchor.
3. **Report the fresh coverage figure** (the instrumented build, disk now freed): the reachable surface at
   100% with the one documented arm uncovered (≈99.9% line, by the audit's count).
4. **The reviewer's independent dynamic re-run** (closing the 008 disk-deferred reliance): the 8-config
   build matrix + the full test suite, independently re-derived.

**This sprint does NOT own** (named, with the reason):

- **Literal 100%** — the operator does not want it chased; the documented `list_entry:143` arm stays
  uncovered (covering it would need removing the guard or a test-only forcing hook — both rejected).
- **Removing or hooking the `list_entry:143` arm** — keep + document (the operator's ruling).
- **Any behavior change** — these are test + doc-comment additions; the existing vectors/corpora unchanged.
- **Editing the anchored files** (segment/cell/display/dimensions/errors) — the tests reach their pub API
  from `tests/`, never via a `#[cfg(test)]` mod inside an anchor.
- **The §8 sibling-decisions doc-hygiene** — the **designer** handles that (operator's call).

## Problem

The audit measured library coverage at 95.75% line (98.28% branch); of the misses, **43 library lines are
cleanly coverable** by 5 targeted tests (accessors / `Display`/`Debug` impls / an unexercised reject arm
that the existing tests never reach — they compare by `PartialEq` and never format, never call the
accessors, never feed a 2-byte focus), and **1 line is structurally unreachable** (`list_entry:143`, the
3A defensive arm). The reviewer's 008 dynamic re-run was disk-deferred; the disk is now freed.

## Goal

Every **reachable** library line is exercised by a test; the one unreachable defensive arm is documented as
intentional (not a coverage gap); the fresh figure is reported; the anchors stay byte-identical; and the
008 reliance is closed by the reviewer's independent re-run. The crate is then complete, deduped, covered,
and documented — ready for the operator's full code review.

## Final State

- 5 new integration tests (in `tests/`, via the public API) covering the 43 lines.
- A doc comment on `list_entry:143` marking the arm intentional-unreachable (with the why).
- The fresh coverage figure recorded (reachable 100%, the documented arm the only uncovered line).
- The reviewer's independent re-run green (the 8-config matrix + the full suite).
- The five anchors byte-identical.

## In Scope — single slice (9A)

**9A — the coverage closeout.** The 5 integration tests + the `list_entry:143` doc comment; the fresh
coverage measurement; the reviewer's independent dynamic re-run. The dev produces a 9A pre-carve manifest
(the exact 5 tests, the pub-API surface each uses, the lines each covers) and **stops for review.**

## Non-Goals

Literal-100% chase; removing/hooking the `list_entry:143` arm; any behavior change; any anchored-file edit
(the tests are integration via the pub API); the §8 sibling-marking (designer's).

## Starting Surface

The 5 coverable targets (reject `Display` / segment `Debug` + `is_empty` / cell `row`/`col` / focus
2-byte), each reached from `tests/` via the public API; `list_entry:143` (the doc comment, non-anchor); the
instrumented coverage build (disk freed). The five anchors (segment b3275467 / cell 8bf962e2 / display
a24b83a2 / dimensions 7e339e0c / errors a0893df6) stay byte-identical.

## Method

- **The coverage tests are INTEGRATION tests via the public API** (`tests/*.rs`), never a `#[cfg(test)]`
  module added inside an anchored file — so `segment.rs` and `cell.rs` (the accessors/`Debug`/`is_empty`
  targets live there) stay **byte-identical**. The pub API reaches each: `RowCol::new` + `row()`/`col()`;
  the decoded `SafeSegmentStack`/`UnsafeSegmentStack` + `is_empty()`; a decoded `UnsafeSegment` +
  `format!("{:?}")`; the `Reject` variants + `format!("{}")`; a 2-byte focus body via `decode_message`.
- **No behavior change** — only test + doc-comment additions; the existing 33 vectors + round-trips + both
  fuzz corpora (decode 410 / reassemble 532) stay UNCHANGED (no new corpus inputs — these are unit-shaped
  tests, not fuzz seeds).
- **The fresh coverage figure** comes from the instrumented build (disk freed), the established
  replay/instrumented method; report line/region/branch, with the one documented arm the sole uncovered
  line.
- **The reviewer's independent dynamic re-run** (the 008 deferral): re-run the 8-config matrix + the full
  suite from a settled build, independently — closing the reliance flagged at 008's disk-constrained
  acceptance.
- **`list_entry:143` is documented, not covered** — the comment states it is structurally unreachable (the
  0x1F-free split) and a deliberate defense-in-depth guard; literal coverage of it is explicitly out.

## Acceptance

- **The 5 tests pass and cover their target lines** — the 43 coverable lines move to covered (verified by
  the fresh instrumented figure); each test reaches its target via the **public API** (no anchored-file
  edit).
- **The fresh coverage figure** is reported: the **reachable** surface at 100% (line/region as the audit's
  count allows), the only uncovered line being the documented `list_entry:143` arm. (Not literal 100% — by
  the operator's ruling.)
- **`list_entry:143`** carries the intentional-unreachable-defensive-arm doc comment.
- **The five anchors byte-identical** (the tests are integration, the doc is on non-anchor `list_entry.rs`);
  the existing vectors + both corpora unchanged.
- **The reviewer's independent dynamic re-run is green** — the 8-config matrix (× 2 targets, `-D warnings`)
  + the full test suite (now including the 5 new tests), independently re-derived (closes the 008 reliance).

## Decided Forks (lead rulings)

1. **`list_entry:143` — keep + document** (the operator's call; the unreachable defensive arm is fine).
2. **No literal-100% chase** (the operator is satisfied with the reachable-100% + the documented arm).
3. **The coverage tests are integration tests via the public API** — the anchored files stay byte-identical
   (no `#[cfg(test)]` added inside an anchor).
4. **The §8 sibling-marking is the designer's** — out of this sprint's scope.

## Escalations

- **None.** The operator set the scope (5 tests + document the arm, no literal-100%). The operator will do
  a **full code review** soon and may have notes afterward — those are handled as they come, not
  pre-empted here.

## Review Questions

1. Confirm the 9A manifest (the 5 tests + their pub-API surface + the `list_entry:143` doc) before the
   carve. (Review.) — **Confirmed; 9A accepted.**
2. The reviewer's independent dynamic re-run is folded into 9A acceptance (closing the 008 reliance). —
   **Done; the reviewer independently re-ran the matrix + suite + measured coverage; reliance closed.**

---

## Postmortem — Sprint 009 retrospectives (and the crate close)

*Sprint 009 closed 2026-06-25: the coverage closeout. 5 integration tests (via the public API, anchors
byte-identical) brought the **reachable** library surface to full coverage — **function 100% / branch 100%
/ line 99.82%**, the sole uncovered library line the operator-ratified documented defensive arm
(`list_entry:147`); a `#[cfg(test)]` panic (test scaffolding) is the only other uncovered line, outside the
src/ gate. The reviewer independently re-ran the 8-config matrix + the full 127-test suite **and measured
the coverage itself** (manual instrumentation, figures reproduced), closing the 008 disk-deferred reliance.
No behavior change; five anchors byte-identical. **★ With 009 the crate is CLOSED: complete (22/22),
deduped (1,632 auditable lines), covered (reachable-library 100% + one documented arm), documented. ★**
This entry carries the 009 method + the crate-level dev/reviewer/lead notes — the durable record of the
nine-sprint construction.*

### Developer (`zero.proto-dev`)

(1) *A byte-frozen file's coverage is closeable from OUTSIDE — integration-via-public-API* — the uncovered
Display/Debug/accessor lines inside the anchored files can't take a `#[cfg(test)]` mod (it would re-hash
the anchor); exercise them through the pub API from `tests/`. Freezing and covering are compatible. (2)
*The instrumented run is ground truth; your line-enumeration is only a plan* — the manifest missed the
sibling `UnsafeSegmentStack::Debug`; the measurement surfaced it, and closing it is the gate working
(surface + close, never rationalize the plan). (3) *The gate is scoped to library lines, not in-file test
scaffolding* — a `#[cfg(test)]` panic that never fires is not the TCB. (4) *Reachable-100% + a documented
defensive arm is the honest target* — the structurally-unreachable 3A guard (`list_entry:147`) can't be
hit; literal-100% would mean removing the guard or a coverage-gaming hook (both degrade the crate); the
metric serves the security posture, never the reverse.

### Reviewer (`zero.proto-review`) — 009 + crate-level

**009 method.** (1) *Cover a frozen surface from outside through the public API* (if a target isn't
pub-reachable, that's itself a finding). (2) *Measure the coverage yourself, don't trust the figure* — the
manual `-Cinstrument-coverage` path reproduced it (number became the reviewer's, not relied-upon; this also
discharged the 008 disk reliance). (3) *Scope the sole-uncovered-line check to src/*; classify every
uncovered line (documented-unreachable-library vs test-scaffolding) by reading where it lives; prove by
arithmetic no third file slipped. (4) *Distinguish unreachable-by-construction (keep+document) from
reachable-but-untested (needs a test)*; verify unreachability from source.

**Crate-level (001–009).** The spine of the reviewer role was **independent re-derivation from the source +
a settled post-build run, never the report** — read current source at re-pinned lines, run my own matrix /
suite / fuzz (read-only) / instrumented coverage / rlib probes, re-derive every number. That caught what a
green summary hides: a make-or-break that was itself a subset-shadow (005), a stale artifact answering a
question only the fresh one can (6B), a "pure extract" that wasn't per-caller (008), a framing that
contradicted the spine it served (007). The rule that made verdicts useful: **reject on method, not just
counters** (a green count is necessary, never sufficient); target the **live path** (the observable only
the unit emits from the production entry); and under an infra blocker, fall back to source inspection and
**say so** — never imply a proof you didn't run. The independence (reply only upstream) is the structural
guarantee.

### Lead (`zero.proto-lead`) — 009 + the nine-sprint coordination note

**009 process.** The closeout scoped tightly: the coverage gate on the **library** surface (test
scaffolding doesn't gate); **reachable-100% + a documented arm**, not a literal-100% chase that would
delete a defensive guard (the operator's ruling, which I recommended); integration-via-pub-API so the
anchors stay byte-identical; the 008 disk-deferred reliance closed by the reviewer's *own* re-run, not
hand-waved.

**What the lead role held across 001–009.** (1) *The sprint contract as the single source of truth* — nine
contracts, each the scope/gates/non-goals every fan-out reads; the present-tense plan, the postmortem at
close. (2) *The separation of duties is the trust mechanism, not bureaucracy* — the dev builds and proves
on the live system; the reviewer **independently re-derives** and never coordinates laterally; I arbitrate
against the named gates, refuse partial greens, and never accept or implement myself. That separation is
what made the green verdicts worth anything. (3) *Surface genuine forks to the operator with a lean, decide
the in-group calls myself* — the no_std lens, the reserved-bit ruling, the reassembler cap, the cell
representation, the dedup scope, the coverage fork, the disk all went up; the representation/coverage/
granularity calls I settled. (4) *Pin the mechanism before ruling a fork* (the `payload24!` reading
dissolved the carrier-macro dispute). (5) *Concede when the human punctures the frame* — I carried "key
blocked on the keysym table" for six sprints and surfaced it as a blocker, when the structure-not-values
ruling I held already meant the keysym is a carried u32; the operator corrected it and I dropped it. The
lesson: apply the established invariant to the new case *before* propagating a doc's framing. (6) *Bake a
twice-hit operational stray into the close* — the corpus hygiene (cmin-last, two-corpora, two-party
read-only), the macro-cfg pre-empt, disk-awareness; each became standing discipline rather than a
re-learned mistake. (7) *Scoped commits* — nine clean sprint commits, the pre-existing `net.md`/
`filesystem.md` excluded throughout, the strays (the 0-byte file, the `.profraw`) removed.

**What remains (not codec work).** The operator's full code review (notes handled as they come); the
provisional-numbers ratification + the §8-sibling marking (the designer's design-of-record); downstream
consumer integration (box/proxy/head linking their feature slices — the gating exists for them; the crate
is structure, the consumers own policy/effect/vocabulary). The proto codec is complete and closed.
