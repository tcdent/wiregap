# Sprint 007 — key (the final verb)

> The seventh sprint of the `proto` crate: `key` 0x12 — the structured key event, and the **last verb**.
> The crate completes on this slice: **22 of 22**. It is a clean recurrence — the body reuses the
> ErrorBody shape (fixed prefix + verbatim tail), the 004 `Modifiers`, the carry-raw enum pattern, and
> the verbatim tail; the only new type is a `KeyKind` carrier. No new decoder class, no new `Reject`.
>
> **Frame correction (records the supersession).** Sprints 001–006 framed `key` as "blocked on the
> `>0x10FFFF` keysym table." That framing was wrong and is superseded: **`KeySym` is a carried `u32` —
> the library validates structure, not values** (the structural-only ruling, in its purest form — the
> crate carries a `u32` it never interprets). *Which* keysyms are valid and how named keys map (X11 vs
> HID, the `>0x10FFFF` set) is the head↔box **shared vocabulary**, never a crate decision. The PM/designer
> corrected this 2026-06-24 and fixed `library.md` §8. The 004/005/006 retrospectives carry the old
> framing as dated history; the correction lives here and in the corrected §8 — the history is not
> rewritten.
>
> Team: `proto` (lead `zero.proto-lead`, dev `zero.proto-dev`, review `zero.proto-review`).
> Grounded in [`model.md`](../../proto/model.md) §8 (Input events), [`library.md`](../../proto/library.md)
> §8 (corrected), and the 001–006 contracts.

---

## Scope Boundary

**This sprint owns:** `key` 0x12 — head→box, terminal block. Body
`[KeySym u32][mods u16][kind u8][reserved u8][text: verbatim UTF-8 tail]` (model §8). All structural:

- **`KeySym`** — a plain carried `u32` (no newtype, no enum, no validation, no table; every value valid,
  like `dx`/`dy`/`offset` raw POD). The `0` / `1–0x10FFFF` / `>0x10FFFF` threshold is the *consumer's*
  interpretation.
- **`mods`** — the **004 `Modifiers`** reused exactly (model §8 confirms the identical `Shift`(0)…`Fn`(5),
  bits 6–15 reserved → drop). `key` is in the same module, so it calls the existing `Modifiers` codec — no
  re-implementation.
- **`kind`** — a new carry-raw enum `KeyKind{Press, Repeat, Release, Other(ReservedKeyKind u8≥3)}` (the
  Button/MouseKind enforce-by-construction pattern; the 7th carrier, the only new type). Numbering
  provisional (`Press`=0/`Repeat`=1/`Release`=2).
- **`reserved`** u8 — drop (decode reads + ignores byte 7; encode writes 0).
- **`text`** — verbatim UTF-8 tail `Vec<u8>` (not `String`; like `paste`; empty = none; verbatim into the
  box, never logged — model §313).

`IMPLEMENTED` 21 → 22; `CATALOG` 22. Terminal-gated (`cfg(feature = "terminal")`).

**This sprint does NOT own:** the keysym vocabulary (X11 vs HID / the named-key set — the head↔box call);
session multiplexing and consumer-integration (later efforts, not the verb catalog). No keysym
validation/table in the crate (the frame correction).

## Problem

`key` 0x12 is the last stub — the only unimplemented verb (21/22). The body is structurally trivial and
entirely covered by established patterns; the only thing that ever made it look hard was the superseded
"keysym table" framing (see the header). With KeySym as a carried `u32`, `key` is a clean recurrence.

## Goal

`key` decodes through the one crate-owned grammar: KeySym carried raw (the crate never inspects its
value), `mods` through the reused `Modifiers`, `kind` carry-raw, `reserved` dropped, `text` verbatim. The
crate reaches **22/22** — the L2 verb catalog complete — and the structure-not-values spine is vindicated
(the last verb is the purest carry-raw case). The five protected surfaces stay byte-identical.

## Final State

- `terminal/input.rs`: `Key{keysym: u32, mods: Modifiers, kind: KeyKind, text: Vec<u8>}` + its `Wire`
  impl (VERB=0x12; encode `keysym` + `mods` + `[kind, 0]` + `text`; decode `split_first_chunk::<8>` →
  `ShortBody{8}` if `<8`, then the prefix by destructure + `from_le_bytes`, the tail verbatim) +
  `KeyKind` + `ReservedKeyKind` (private inner). `Modifiers` reused, not re-implemented.
- `verb.rs`: `IMPLEMENTED` += `KEY` (under `cfg(terminal)`) → 22; `KEY` annotation stub→impl; the
  `KEY_KIND_*` field codes (pub, un-cfg'd, like the mouse field codes).
- `message.rs`: `+Message::Key` + the verb/decode/encode arms under `cfg(terminal)`.
- `lib.rs`: `+pub use Key/KeyKind` under `cfg(terminal)`.
- A `0x12` seed in `fuzz/corpus/decode` (the per-verb-seed pattern); the corpus re-minimized.
- The five anchors (segment/cell/display/dimensions/errors) byte-identical.

## In Scope — single slice (7A)

`key` is one coherent unit; no new mechanism to stage (unlike 005). **7A — `key` 0x12:** the `Key` decoder
+ `KeyKind` carrier in `input.rs`; the dispatch arm under `cfg(terminal)`; `IMPLEMENTED` 22; the
`KEY_KIND_*` field codes; the value-span / kind / mods / rsv / text vectors; the `0x12` corpus seed; the
`==22` coverage; the 006 build-matrix + per-slice coherence re-run. The developer produces the 7A
pre-carve manifest and **stops for review before the carve.** This completes the crate.

## Non-Goals

- No keysym value validation / table (the frame correction — KeySym is carried raw).
- No new decoder class, no new `Reject` (the ErrorBody shape + carry-raw + verbatim tail recur).
- No session multiplexing / consumer-integration.
- No rewrite of the 004/005/006 historical retros (the correction lives in this contract + library §8).

## Starting Surface

007 is additive + a real decode change in `terminal/input.rs` (its content changes — `input.rs` is **not**
an anchor). Touches: `input.rs` (+Key/KeyKind), `verb.rs` (IMPLEMENTED 22 + field codes), `message.rs`
(+Key arms cfg terminal), `lib.rs` (+exports cfg terminal), `fuzz/corpus/decode` (+the 0x12 seed,
re-minimized), `tests/*` (+the key vectors + round-trip). The five anchors byte-identical. `Modifiers`
reused via its existing in-module codec (not re-implemented, not edited).

## Method

Inherits the established method. The discipline specific to 007:

- **KeySym carries raw — the crate never inspects its value (the make-or-break).** Decode binds `keysym`
  with a plain `u32::from_le_bytes` and has **no** match / range-check / validation on it; encode writes it
  back. This is the proof the frame correction is real — the crate carries any `u32`, the table is the
  consumer's. A reject or a value-check on the keysym re-introduces the crate-level table gate the
  correction removed.
- **Recurrence by reuse.** `mods` calls the 004 `Modifiers` codec (the route-through is observable via the
  reserved-drop canonicalization — a key vector with `mods=0xFFFF` re-encodes to the canonical 6-bit mods;
  since `Modifiers` has no reject, the canonicalization *is* the reuse tell, analogous to 005's
  net-error→ErrorBody). `kind` is the carry-raw enum (Button/MouseKind) pattern. `reserved` drops
  (canonical re-encode). `text` is the verbatim tail (paste). The decode is the ErrorBody shape (fixed
  prefix + verbatim tail).
- **Terminal-gated.** `key` is `cfg(feature = "terminal")` (the 0x1_ block); the terminal `IMPLEMENTED`
  group grows 8→9. The 006 build matrix (8 configs × 2 targets, `-D warnings`) + the per-slice lib-unit
  coherence **re-run**: `key` is in terminal / head / box, **not** the proxy — so the proxy still excludes
  `key` (`decode_message(0x12)` → `UnsupportedVerb` on the proxy slice, confirmed by its coherence).
- **Corpus seed + hygiene.** A `0x12` seed added to `fuzz/corpus/decode` (so the fuzz reaches `Key`
  promptly and the replay-coverage exercises its edges). This grows the decode corpus → `cargo fuzz cmin`
  **last**, on-disk == committed; review's reproduction stays read-only (`-runs=0`) / temp-copy so the
  committed corpus is the validated artifact (no reconcile). The reassemble corpus is untouched (`key` is
  not reassembled).

## Acceptance

All 001–006 gates carry forward at all-features. New/refined:

- **The KeySym carry-raw make-or-break.** Value-span vectors — `0`, `1`, a `1..=0x10FFFF` codepoint,
  `0x110000` (first past Unicode max), a `>0x10FFFF` named-key value, `0xFFFFFFFF` — **all round-trip
  byte-identical, no reject**; **and** source inspection: the decode shows `keysym` as a raw field with
  zero value logic (no match/range-check). A reject or value-check on the keysym = REJECT.
- **KeyKind carry-raw.** `Other` is a private newtype; the in-crate from-u8 ctor routes 0/1/2 to the named
  variants (Other cannot hold 0/1/2). A vector with an `Other` kind (e.g. 5) round-trips carrying the raw
  u8; 0/1/2 round-trip. Numbering provisional (do not block on it).
- **mods reuse / rsv drop / text verbatim.** `mods=0xFFFF` → canonical 6-bit re-encode (the reuse tell);
  `rsv=0xFF` → canonical 0 (reserved-drop, not byte-identical); empty text and non-UTF-8 text both
  round-trip byte-identical.
- **Geometry + live-path.** `<8` → `ShortBody{needed:8, got}`; `≥8` → prefix + verbatim tail (no
  `TrailingBytes` — the tail absorbs the remainder). The live-path: a `Key` from `decode_message` + a
  `ShortBody` from a short `0x12` body (surfaces from the 0x12 arm).
- **Terminal-gated.** The 006 build matrix + per-slice coherence re-run; the proxy slice excludes `key`
  (`UnsupportedVerb`). All-features coverage `==22` (`vectors_cover_every_implemented_verb` +
  `implemented_set_matches_dispatch` → 22), holding immediately (the key vector lands with the verb — no
  narrow/restore).
- **Corpus.** The `0x12` seed lands; the decode fuzz reaches `Key` via dispatch; cmin-last, on-disk ==
  committed (the reassemble corpus untouched). Both fuzz targets clean.
- **Behavior preservation.** The five anchors byte-identical; `Modifiers` reused (not edited). No new
  `Reject` variant.

## Decided Forks (lead rulings — engineering, co-formulated; dev + review concur)

1. **KeySym = plain carried `u32`** (no newtype/enum/validation/table) — the structural-only ruling in its
   purest form (the make-or-break above).
2. **mods = the 004 `Modifiers` reused** (no re-impl; the canonicalization is the reuse tell).
3. **kind = `KeyKind{Press, Repeat, Release, Other(ReservedKeyKind u8≥3)}`** — carry-raw
   enforce-by-construction (the 7th carrier, the only new type). Numbering provisional (Press=0/Repeat=1/
   Release=2).
4. **reserved u8 = drop; text = verbatim tail** (Vec<u8>, not String).
5. **Decode = the ErrorBody shape** (fixed 8-byte prefix + verbatim tail) — no new decoder class, no new
   `Reject`.
6. **Terminal-gated; single-slice 7A** — one recurrence verb + the carrier; no mechanism to stage.
7. **A `0x12` corpus seed** is added (the per-verb-seed pattern; the clean fuzz-reaches-Key guarantee) →
   cmin-last hygiene.
8. **Record hygiene:** the supersession is recorded here; the 004/005/006 historical retros are left as
   dated history (not rewritten — consistent with the 005 anchor-miscount correction in 006).

## Escalations (operator-reserved — routed via the PM)

- **None gating 007.** The `KeyKind` numbering is provisional (operator reviews with the other numbers,
  not a blocker). The keysym vocabulary (X11 vs HID / the named-key set) is the head↔box consumers' call,
  not a crate decision — explicitly *not* an escalation (the frame correction).

## Review Questions

1. Ratify the `KeyKind` numbering (0/1/2) with the other provisional numbers. (Operator — review, not a gate.)
2. Confirm the 7A creation manifest + seam carve before any code. (Review, at the 7A pre-carve stop.) —
   **Confirmed; sprint accepted; the crate is complete (22/22).**

---

## Postmortem — Sprint 007 retrospectives (and the crate close)

*Sprint 007 closed 2026-06-24: accepted by independent review. **The crate is complete — 22 of 22
verbs.*** `key` 0x12, the final verb, is the purest carry-raw case (a plain `u32` carried with zero value
logic), vindicating the structure-not-values spine; the frame correction is proven at the source. A clean
single-slice recurrence — no new decoder class, no new `Reject`; the only new type was the `KeyKind`
carrier (the 7th). Builds on the 001–006 postmortems; this entry also carries crate-level retrospectives
(the reviewer's Parts B/C and the lead's coordination retro), the durable record of how the wire was held
from drifting across seven sprints.*

### Developer (`zero.proto-dev`)

**What moved the plan.** (1) *KeySym carry-raw is the spine's purest case — and it vindicated the spine.*
`key` carries a `u32` the decode never interprets; the keysym vocabulary is the head↔box's, not a crate
decision — so the crate is never blocked on a consumer-owned vocabulary being pinned. "`key` blocked until
the keysym table lands" was a six-sprint misconception; the crate completed without the table ever being
pinned. When a field looks like it needs a crate-side table of allowed values, that's the structural-only
smell — carry the value raw. (2) *Recurrence-by-reuse is the mark of a mature codec — the last verb was the
easiest.* `key` introduced no new mechanism (Modifiers/carry-raw-enum/verbatim-tail/ErrorBody-shape all
reused); when the final verb is the simplest, the abstraction set was right. (3) *A derive-from-incomplete
test must become a completeness assertion* — finishing 22/22 made `CATALOG \ IMPLEMENTED` empty, so
`unknown_low_byte_is_unsupported_verb` transitioned to `CATALOG ⊆ IMPLEMENTED` + out-of-catalog →
`UnsupportedVerb`.

### Reviewer (`zero.proto-review`) — 007 + crate-level

**007 method.** (1) *A carry-raw invariant is proven at the SOURCE, not by vectors* — the vectors sample 6
of 2³² values; the load-bearing proof is reading the decode and confirming `keysym` binds via
`from_le_bytes` with zero value logic (the carry-raw twin of 005's "a bound is only a defense if a test
drives past it"). (2) *The final verb is recurrence-by-reuse — validate the REUSE at each seam* (the
call-through, the canonicalization/reject tell); a re-implementation that happens to round-trip is the
defect (a second grammar that will drift). (3) *The completion-driven test transition* — at a completeness
milestone, a "something is missing" test is re-pointed (to the completeness invariant + an out-of-set
probe), not deleted; verify no live coverage is lost.

**Crate-level — what held the wire from drifting (001–007).** Four load-bearing disciplines: (1) *the
structure-not-values spine* (carry-raw enforce-by-construction, reserved-drop, the one resource bound) —
kept a value table out of the trusted codec (the keysym-table save). (2) *Source-anchored gates* (`==`
coherence, build-coherence, content-hash anchors) — made "the build is the source of truth" real; the
per-sprint narrow/restore was only safe because the `==` is source-anchored. (3) *The per-seam live-path
cascade* — each unit wired at the next seam, proven by an observable only it emits; kept "built" from
meaning "built but dead." (4) *The protected anchors* — five frozen surfaces recomputed byte-identical;
the declaration-level cfg discipline (006) extended it so gating couldn't edit a gated module's content.
*The meta-discipline:* re-derive every claim from source + a settled post-build run, never the report
(this caught the stale-rlib probe, the unreached cap, the dead-code targets); independence (reply only
upstream) is what makes the re-derivation worth anything. **The durable boundary the follow-ons inherit:**
the crate is the structure; the consumers are the policy/effect/vocabulary — every "should the crate
validate/route/allow X" has the spine's answer (carry the structure, the consumer owns the decision).

### Lead (`zero.proto-lead`) — 007 + coordination retro

**007 process.** The final sprint was a clean recurrence, framed and closed in one slice — the strongest
sign the contract/gate machinery had matured. The honest entry: **I carried the "keysym table blocks
`key`" frame for six sprints and surfaced it upward as a blocker**, when the structure-not-values ruling I
held in memory already implied the keysym is a carried `u32` and the table is the consumers' vocabulary.
The human/PM punctured the frame; I conceded it and dropped it. *Lesson:* apply the established invariant
to the new case **before** accepting a doc's framing — a "blocker" inherited from stale doc wording
(library §8/§11) must be checked against the spine, not propagated. I treated "`key` is blocked" as a
past-shaped fact when it was a wrong assumption the spine had already resolved; the anchor was there to be
read.

**Coordination — what the lead role contributed across 001–007.** The sprint contract as the single
source of truth (scope, gates, non-goals — the thing every fan-out reads); one narrow task to one owner;
the **separation of duties** held (the dev builds and proves on the live system; the reviewer
independently re-derives from source and never coordinates laterally — that independence is the whole
value of the verdict); arbitrating each report against the named gates, not whether the work seemed good;
refusing to advance on partial greens or a contradiction (a green build vs an unreached make-or-break);
surfacing genuine forks to the operator (no_std, the reserved-bit policy, the reassembler cap) while
deciding the in-group/representation calls myself; conceding the better-grounded design when shown one
(the 005 cap 8→32 MiB; the frame correction); and scoping every commit to its sprint (excluding the
pre-existing net.md/filesystem.md throughout). The recurring discipline that paid off most: **pre-record a
forced relaxation and its re-tighten** (the subset→`==` narrow/restore), and **bake a twice-hit
operational stray into the close** (the corpus hygiene — cmin-last, two-corpora, two-party read-only) so
it stopped recurring.

**What's next (NOT codec — the operator sequences).** Operator ratification of the provisional numbers
(verb-ids / op-set / error+field codes incl. KeyKind / envelope layout / REASSEMBLY_CAP → final; a
re-bless of the source-anchored vectors when a number moves, not new code); L1 session multiplexing
(composing N of the crate's single-session reassemblers + the count bound the crate deliberately does not
own); consumer integration (box/proxy/head linking their feature slices — the gating exists for them; each
inherits its minimal trusted surface). The crate is the structure; the consumers own policy/effect/
vocabulary — that boundary is the crate's lasting contract.
