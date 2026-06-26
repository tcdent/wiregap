# Sprint 011 — Segment-Scan Match Refactor

> A behavior-preserving refactor for review clarity (operator direction): switch the per-byte alphabet
> validation in `scan_safe`/`scan_unsafe` (`segment.rs`) from the current mechanism
> (`is_ascii_alphanumeric()` + range/alphabet checks) to a **match statement** — the alphabet explicit in
> the arms, easier to review. It stays model §6's byte-scan ("every byte checked against the alphabet,
> verify-never-transform"), just match-expressed. **The behavior is byte-identical** — every one of the
> 256 byte values accepts/rejects exactly as before. `segment.rs` is a protected anchor, so this is the
> **first intentional anchor re-freeze**: legitimate *only* because the behavior is proven identical.
>
> Team: `proto` (lead `zero.proto-lead`, dev `zero.proto-dev`, review `zero.proto-review`).
> Grounded in `segment.rs`, [`model.md`](../../proto/model.md) §6, [`file.md`](../../proto/file.md) §0.

---

## Scope Boundary

**This sprint owns** (one slice, `segment.rs` only):

- **Switch `scan_safe`'s per-byte check to a match** — safe alphabet `[A-Za-z0-9._-]`, ≥1 alphanumeric per
  segment: `match byte { b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => has_alnum=true; b'.' | b'_' | b'-' =>
  {} ; _ => Err(BadSegmentByte{index,offset,byte}) }`, then the existing `text.push(byte as char)`.
- **Switch `scan_unsafe`'s per-byte check to a match** — any byte but `/`, 0x1F, NUL: `match byte { b'/' |
  0 => Err(BadSegmentByte{…}); _ => {} }` (0x1F is the separator — structurally never reaches the per-byte
  loop).
- **An exhaustive-256, non-circular behavior-identity test** (new `tests/segment_alphabet.rs`, via the pub
  API — see Method).

**This sprint does NOT own:**

- **Any behavior change** — byte-identical accept/reject for all 256 values + the segment-level invariants.
- **The alphabet** — unchanged (file.md §0); the match arms *are* the exact current alphabet.
- **A new `Reject`** — the same five (EmptyStack/EmptySegment/TraversalSegment/BadSegmentByte/
  SafeSegmentNotAlnum); the segment loop, the ≥1-alnum / `.`/`..` / empty checks, the `0x1F` split, the
  `SafeSegment` String / `UnsafeSegment` raw-bytes outputs, and the G4 safe/unsafe type split — all
  unchanged.
- **Any other file** — `segment.rs` + the new test only; the other 4 anchors byte-identical.

## Problem

`scan_safe`/`scan_unsafe` validate the segment alphabet with `is_ascii_alphanumeric()` + range/alphabet
checks. The operator wants the alphabet expressed as a **match statement** — the accepted/rejected bytes
explicit in the arms — for easier review. (Per the earlier grounding: the byte-scan is model §6's design
for auditability/totality; the match is its most explicit, most reviewable form.)

## Goal

The per-byte validation is a match with the alphabet explicit in its arms, **byte-identical** in behavior
(every 256-value accept/reject + the exact Reject variant + the segment-level invariants), model-§6
conformant. `segment.rs` re-freezes to a new content hash — the change is content-only, the behavior
proven unchanged.

## In Scope — single slice (11A)

**11A** — the two match refactors in `segment.rs` + the exhaustive-256 non-circular test. The dev produces
the 11A manifest (the exact match arms + the exhaustive-256 design + the new `segment.rs` hash) and **stops
for review before the carve** (review checks the arms' ranges against the spec alphabet byte-for-byte).

## Non-Goals

Any behavior change; any alphabet change; a new Reject; any file beyond `segment.rs` + the new test; the
4 non-segment anchors are byte-identical.

## Starting Surface

`segment.rs` `scan_safe` (the safe per-byte check) + `scan_unsafe` (the unsafe per-byte check); the new
`tests/segment_alphabet.rs`. The 4 non-segment anchors (cell 8bf962e2 / display a24b83a2 / dimensions
7e339e0c / errors a0893df6) stay byte-identical; `segment.rs` (currently b3275467) re-freezes.

## Method

- **The match arms ARE the exact alphabet.** Safe: `A-Z|a-z|0-9` → accept + `has_alnum`; `.`|`_`|`-` →
  accept (no `has_alnum`); else → `BadSegmentByte`. Unsafe: `/`|NUL → `BadSegmentByte`; else → accept
  (0x1F is the delimiter, structurally excluded from the per-byte loop). The `has_alnum` distinction
  (alnum sets it; `.`/`_`/`-` do not) is preserved exactly.
- **The make-or-break: an EXHAUSTIVE-256, NON-CIRCULAR behavior-identity test.** For **all** 256 `u8`
  values, for **both** scans, the match scan's **exact `Result`** (Ok, or the exact Reject variant +
  fields) equals the spec. Three binding properties: **(1) all 256, not a sample** — a sampled test
  (`'A','z','0','.','/'`) misses a single-byte flip, and build-clean compiles a mis-ranged arm; **(2)
  non-circular** — the test's expected is **spec-derived and expressed *differently* from the match's
  arms** (the reference predicate uses `b.is_ascii_alphanumeric() || matches!(b, b'.'|b'_'|b'-')` (safe) /
  `!matches!(b, b'/'|0)` (unsafe), while the match uses explicit *ranges* — so a mis-ranged arm mismatches
  the oracle and is caught; a shared range expression would be circular); **(3) exact Result** — a
  rejected safe byte → `BadSegmentByte{index,offset,byte:b}`, not a different variant. (0x1F is the
  separator — covered via the split path, not the per-byte alphabet.) The test is an **integration test
  in `tests/`** (via the pub `scan_safe`/`scan_unsafe`) so it's non-circular by location and the re-freeze
  is the refactor alone.
- **The segment-level invariants are byte-identical** (test them too — not per-byte): ≥1-alnum
  (all-`.`/`_`/`-` safe segment → `SafeSegmentNotAlnum`); never-`.`/`..` (→ `TraversalSegment`); empty
  segment (→ `EmptySegment`); empty input (→ `EmptyStack`); the `0x1F` split (a stack of 2); the safe
  byte-as-char String + the unsafe raw bytes. PLUS the existing `segment_rejects` + the file/net vectors +
  the **decode fuzz** (corpus 410) replay **unchanged** (the file-write/request-file/list + net-address
  verbs route through these scans — same coverage, 0 crashes, the corpus identical, not regenerated).
- **The anchor re-freeze (the first intentional one) is legitimate ONLY via the behavior proof.**
  `segment.rs` re-freezes to a new sha256 — recorded as the new baseline. The content changes (the match
  form); the behavior does **not** (the exhaustive-256 + the unchanged tests/fuzz prove it). An anchor
  guards *unintended* change; an audited behavior-preserving refactor updates it — the re-freeze path the
  008 contract anticipated, now exercised. It is **not** a license to change behavior. The other 4 anchors
  byte-identical.
- **Still model §6 conformant** — a match still "verifies, never transforms," makes the alphabet explicit,
  keeps the `0x1F` separator / the empty+traversal rejects / the safe-unsafe type split / G4 / the ≥1-alnum.

## Acceptance

- **The exhaustive-256 non-circular test (the make-or-break).** All 256 values, both scans, the exact
  `Result` == a spec-derived reference expressed differently from the match's arms. Any single-byte flip or
  wrong-variant reject = REJECT.
- **The segment-level invariants byte-identical** (≥1-alnum/traversal/empty/empty-stack/0x1F-split/output
  type); the existing `segment_rejects` + the file/net vectors pass unchanged.
- **The decode fuzz** (corpus 410) replays unchanged — same coverage, 0 crashes, the corpus byte-identical
  (not regenerated); the reassemble corpus (563) untouched.
- **`segment.rs` re-frozen** to a new sha256 (recorded as the new baseline); the **other 4 anchors
  byte-identical** (recomputed, unchanged).
- **Scope / conformance.** `segment.rs` + the new test only; no new Reject; IMPLEMENTED 22; the 8-config
  build matrix (× 2 targets, `-D warnings`) + per-slice coherence unchanged (segment is core, behavior
  identical); model §6 conformant. G6.d both targets.

## Decided Forks (lead rulings; dev + review concur)

1. **The match arms = the exact current alphabet** (safe `A-Z|a-z|0-9` + `.`/`_`/`-`; unsafe not-`/`/NUL;
   0x1F the structural delimiter) — byte-identical, the `has_alnum` distinction preserved.
2. **The exhaustive-256 test is non-circular** — the reference predicate is spec-derived and expressed
   differently from the match's arms (std `is_ascii_alphanumeric` vs explicit ranges), all 256, exact
   Result.
3. **The test is a `tests/` integration file** (via the pub API) — non-circular by location; the
   `segment.rs` re-freeze is the refactor alone (no inline test code in the anchor).
4. **The `segment.rs` anchor re-freezes** (content-only; behavior proven identical) — the first intentional
   re-freeze; the other 4 anchors byte-identical.

## Escalations

- **None.** The operator directed the refactor; the alphabet and behavior are unchanged; no fork.

## Review Questions

1. Confirm the 11A manifest — the exact match arms (ranges checked against the spec alphabet byte-for-byte)
   + the exhaustive-256 non-circular design + the new `segment.rs` hash — before the carve. (Review.) —
   **Confirmed; 11A accepted.**
2. The exhaustive-256 (all 256, non-circular reference, exact Result) is the make-or-break, run
   independently at acceptance. (Review.) — **Run independently (a third spec-hand-derived oracle): 0
   mismatches.**

---

## Postmortem — Sprint 011 retrospectives

*Sprint 011 closed 2026-06-25: a behavior-preserving refactor for review clarity (operator direction).
`scan_safe`/`scan_unsafe`'s per-byte alphabet validation is now an explicit **match** (the alphabet in the
arms), provably **byte-identical** — the reviewer ran an independent exhaustive-256 with a *third*
spec-hand-derived oracle: **0 mismatches** over all 256 values, both scans. This is the **first intentional
anchor re-freeze**: `segment.rs` re-froze to `1febd7a128177b61aba124edd0d1d0d7c89cb171f7d93ba1d714c74110372eba`
(was `b3275467…`) — legitimate only because the behavior is proven identical; the other 4 anchors
byte-identical. No new Reject; IMPLEMENTED 22; segment.rs + the new test only; the corpora unchanged.
Durable lessons.*

### Developer (`zero.proto-dev`)

(1) *A refactor whose only goal is review legibility is legitimate work — the arms ARE the spec, auditable
at the site.* The path alphabet is security-relevant; as a stdlib predicate the accepted set is inferred
from a call, as a match it's visible at the site. (2) *An equivalence proof is only as good as the
independence of its oracle, and over a finite domain it should be exhaustive* — the oracle a different
expression of the spec (else circular), and u8 (256) is small enough to exhaust (no escape; a sample
misses the one flipped byte). (3) *The first intentional anchor re-freeze* — a content-hash anchor is a
change-DETECTION gate, not an immutability decree: intentional change + behavior-proof + re-pin the
baseline; "change only deliberately, with proof, and re-pin," not "never change." The 008-anticipated
path, run the first time.

### Reviewer (`zero.proto-review`)

(1) *The third-independent-oracle.* Three expressions are in play — the new match (explicit ranges), the
dev's reference (`is_ascii_alphanumeric()||matches!(.|_|-)`), and the **spec** (file.md §0). `new==itself`
is circular; `new==dev's-reference` is one-level; the gold standard is `new==spec-hand-enumerated` — the
reviewer builds the oracle as a *literal* enumeration of the spec (the 65 safe bytes listed; unsafe =
any-but-{/,NUL}), a third expression independent of both the impl and the dev's predicate (ran over all
256 → 0 mismatches; a bug shared between the impl and the dev's predicate would still mismatch a literal
spec enumeration). (2) *Exhaust a small finite domain* (a byte / small enum / bounded id) — cheap and
definitive; reserve sampling/fuzzing for domains too large to exhaust. (3) *The first intentional re-freeze
is gated entirely on the behavior proof* (recompute + record the new baseline, confirm the siblings
unchanged; the hash moves only because the behavior provably did not — not on the reviewer's say-so). (4)
*A coverage-region shift is not a behavior change* — cov measures source-region execution (a `match`
compiles to different regions than an `if/else`); prove identity by the exhaustive test + the unchanged
suite + the corpus replaying identically, never by the cov number.

### Lead (`zero.proto-lead`)

**Process lessons.** (1) *Don't re-litigate the operator's call; presume competence.* I had told the
operator the byte-scan was already auditable; they then directed the *match* form anyway — for **site-local**
auditability (the alphabet in the arms, not inferred from a stdlib call). That's a legitimate further
improvement on the same axis (review-ease), not a contradiction of my answer; I planned it, didn't argue
it. (2) *A behavior-identity refactor's gate is an exhaustive + independent-oracle proof* — I set the
exhaustive-256/non-circular gate; the reviewer raised it to the third-spec-hand-derived oracle, the
strongest non-circularity. The make-or-break for "did behavior change?" on a finite domain is "exhaust it
against a spec-derived oracle," not a sample. (3) *The first intentional anchor re-freeze is a discipline,
not a loophole* — the re-freeze is accepted only on the proven behavior-identity (the content changed, the
behavior didn't); the new `segment.rs` baseline is recorded for the next sprint. The 008 anchor convention
designed this path; 011 ran it and it holds. (4) *The pre-carve gate, held again despite the green-light*,
confirmed the arms byte-for-byte before the carve — cheap insurance on a security-relevant classification.

**Carry-forward.** `segment.rs`'s anchor baseline is now `1febd7a1…` (recompute against this; the other 4
unchanged). The re-freeze precedent: a protected anchor changes only via an audited, behavior-proven
refactor (the third-oracle + exhaustive-domain proof is the bar). The crate stays complete (22/22) /
deduped / covered / documented / model-conformant — now with the scan alphabet auditable at the site. What
remains is not the crate's: operator/designer ratifications, the transport integration.
