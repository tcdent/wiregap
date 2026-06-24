# Sprint 003 — File Service

> The third sprint of the `proto` crate: completing the **file** service — `file-write`, `file-data`,
> `list`, `list-entry`, `request-file` (`file-chunk`/`file-read`/`file-error` are 001 seeds). It builds
> the headline obligation deferred from 002: **`list-entry`'s variable combined-record decoder**
> (`[fixed 24B metadata][unsafe name, 0x1F-terminated]`, repeated to frame end). Built on the 001/002
> codec, inheriting their gates (G0–G6) and invariants.
>
> Team: `proto` (lead `zero.proto-lead`, dev `zero.proto-dev`, review `zero.proto-review`).
> Grounded in [`file.md`](../../proto/file.md), [`model.md`](../../proto/model.md) §8 (file block), and
> the 001/002 contracts.

---

## Scope Boundary

**This sprint owns** the file consumer surface ([`file.md`](../../proto/file.md)):

1. **`list-entry` (0x25)** — the **variable combined-record decoder**: repeated `[fixed 24B metadata]
   [unsafe name, 0x1F-terminated]` to frame end. The headline obligation deferred from 002; a *different,
   harder* decoder than 002's fixed-N tail (variable-length records, record-internal `0x1F` **terminator**).
2. **`file-write` (0x20)** — box→proxy, a **safe** segment stack destination (reuses the 001 safe scan).
3. **`file-data` (0x23)** — proxy→box, **verbatim** read bytes (reuses the 001 verbatim tail).
4. **`list` (0x24)** — box→proxy / head→box, an **unsafe** segment stack directory (reuses the 001 unsafe scan).
5. **`request-file` (0x26)** — head→box, an **unsafe** segment stack source (reuses the 001 unsafe scan).

`IMPLEMENTED` grows 7 → 12. `file-chunk` 0x21 / `file-read` 0x22 / `file-error` 0x27 are already 001 seeds.

**This sprint does NOT own** (named, with the sibling sprint that owns it):

- The **net** service, **reassembly**, the **L1 envelope**, **feature-gating** — later sprints
  (reassembler cap operator-reserved).
- The **input** service (`key` blocked on the keysym table; `resize` reuses 002's `Dimensions`).
- **`type_flags` semantics beyond the three types** ([`file.md`](../../proto/file.md) §6 open) — the crate
  carries the reserved range raw; meaning is the consumer's.

## Problem

002 built the fixed-N tail decoder but explicitly deferred `list-entry`'s **variable** combined-record
decoder — both 001 and 002 retrospectives named it the next unproven obligation. The file service is
half-built: three verbs are 001 seeds, but the box cannot push a destination (`file-write`), the proxy
cannot stream a read (`file-data`) or a directory listing (`list`/`list-entry`), and the head cannot
trigger an extraction (`request-file`).

## Goal

The file service is complete: the box/proxy/head encode and decode all eight file verbs through the one
crate-owned layout. The `list-entry` variable combined-record decoder exists, is total over hostile
framing (truncated metadata, unterminated names, any byte length), and rejects a missing terminator —
distinguishing the record-internal `0x1F` **terminator** from the segment stack's **separator**. The 001
scan surface and the 002 cell/display surface stay byte-identical.

## Final State

- The `list-entry` decoder: per record, fixed 24B metadata + a `0x1F`-terminated unsafe name; repeated to
  frame end; **empty body → `Ok`** (empty directory); rejects below.
- `FileType { File=0, Dir=1, Symlink=2, Other(u8≥3) }` carry-raw enforce-by-construction; `type_flags`
  high 3 bytes dropped; `size`/`mtime_*` raw POD.
- `file-write`/`list`/`request-file` route through the 001 segment scans; `file-data` through the 001
  verbatim tail — **reused, not re-implemented**.
- Dispatch wired for 0x20/0x23/0x24/0x25/0x26; `IMPLEMENTED` = 12; `CATALOG` = 22.
- 001 `segment.rs` (anchor `b3275467…`) + the 002 `cell.rs`/`display.rs` surfaces byte-identical.

## In Scope — ordered slices (review seams)

The developer produces the **3A pre-carve creation manifest** (the 5 verbs + the `list-entry` record
sub-grammar + the `FileType` carrier) and **stops for review before any carve**.

- **3A — the `list-entry` variable combined-record decoder + `FileType`.** The new terminator-bounded
  record decoder (below) + the `FileType` carrier. Gated standalone (like 2A) before the wiring.
  *Gates:* G1 field-edges (0/1/N records Ok + the reject cases) + G2 source-discipline.
- **3B — the reused-decoder verbs + dispatch.** `file-write`/`file-data`/`list`/`request-file` as new
  message types over the **reused** 001 decoders; dispatch arms 0x20/0x23/0x24/0x25/0x26 (each routes
  *through* its decoder); `IMPLEMENTED` = 12. *Gate:* round-trip + dispatch + reuse-not-reimplement.
- **3C — vectors + extended fuzz + G0.** Hand-authored non-circular vectors (a valid listing +
  the **unterminated-name reject vector** + a truncated-metadata reject vector + a `file-write`/`list`
  vector); the kitted fuzz extended to the `list-entry` decoder **via dispatch**; full G0 (`IMPLEMENTED`=12).
  **Re-tighten the coverage anchor to `covered == IMPLEMENTED`** (12) — it was narrowed to `covered ⊆
  IMPLEMENTED` (no orphan vectors) for 3B, because the dispatch arms grow `IMPLEMENTED` to 12 in 3B while
  the 5 file vectors land here in 3C; the `==` re-tighten *fails unless* the 5 vectors land, so it forces
  them (self-correcting). *Gates:* G2 (full), G3, G0, G6.d.

## Non-Goals

- The net/input/reassembly/envelope/feature-gating sprints.
- Interpreting `type_flags` reserved values or the §6-open bits (carried raw).
- **Do not edit `segment.rs`** (reuse the 001 public scans) — and do not touch `cell.rs`/`display.rs`.
- **Do not reject a structurally-valid message:** an empty `list-entry` body is `Ok` (empty directory).
- **Do not re-implement** a 001 decoder for the reused verbs (carry-not-rediscover / live-path).
- **No `unreachable!`/panic on input-derived values** — the stack-of-1 name validation's impossible arm is
  a defensive typed reject (see Decided Forks #1).

## Starting Surface

003 is **additive**; `segment.rs` is **untouched** (the 001 safe/unsafe scans are reused via their public
API, so the `b3275467…` anchor holds). The 3A manifest names every touch: `reject.rs` +`UnterminatedRecord`
(appended); `message.rs` +5 dispatch arms/variants (each routes through its decoder); `verb.rs`
`IMPLEMENTED` 7→12; the `list-entry` record grammar + `FileType` in a new module; `lib.rs` +exports. Module
layout (a `file/` dir vs `file.rs` + sibling) is the dev's call **provided 001 code is not behavior-changed**
(the file-seed round-trips stay green; if 001 code moves, it is a behavior-preserving move, tests green).

Inventory shape (as 001/002): `verb id | name | direction | body shape | decoder kind | type | module |
003 action`, plus the `list-entry` record sub-grammar table.

## Method

Inherits the 001/002 method (owned field-by-field decode; bounded accessors only; verify-never-transform;
build coherence; non-circular hand-authored vectors; the per-seam live-path cascade; stop-for-review).
Additions:

- **The `list-entry` decoder is a NEW terminator-bounded record decoder** — distinct from 002's fixed-N
  tail (variable records) and from the 001 separator-stack scan. Per record: `split_first_chunk::<24>` for
  the metadata (`None` → reject), then a **bounded** scan for the `0x1F` terminator within the remaining
  slice (`iter().position` / `split-once` — never an unbounded over-read), the bytes before it the name,
  consume the terminator; repeat while bytes remain; **0 bytes left → `Ok(records)`**. Boundaries come
  ONLY from the fixed 24B, the `0x1F` terminator, and the frame — **never a box-authored count** (law 11).
- **Name alphabet validation reuses the single-source 001 scan** (Decided Forks #1): the decoder validates
  the extracted name slice (which contains no `0x1F`) via `scan_unsafe` as a stack-of-1, so the unsafe
  alphabet lives in one place and `segment.rs` is untouched. The impossible multi-segment arm is a
  **defensive typed reject, never `unreachable!`**; an empty name → `EmptyStack` (the empty-name reject).
- **Protected surfaces (content-hash):** `segment.rs` `b3275467be8ce9a196834a618401bfcd917eae7157c5b85de59b95686c9bd4ac`
  stays byte-identical; capture `sha256(cell.rs)` and `sha256(display.rs)` at 3A as additional protected
  anchors (003 must not touch the 002 surface either).
- **Live-path cascade:** the `list-entry` decoder (3A) → the 0x25 dispatch arm (3B) → the kitted fuzz
  reaching it **via `decode_message`** (3C). Observable proof: `UnterminatedRecord` from
  `decode_message([0x25..][unterminated])` proves the full dispatch→decoder path (a reject only the new
  decoder emits). The reused-verb arms likewise route *through* the 001 decoders (no re-implementation).

## Acceptance

All **001/002 gates G0–G6 carry forward**, green on the post-003 tree. New/refined obligations:

- **G1 — round-trip + the new decoder.** Round-trip over the 12 `IMPLEMENTED` verbs. `list-entry`
  field-edges: **0 records → Ok** (empty directory), 1, N; reject cases as typed `Err`: **truncated
  metadata** (<24B for a started record) → `ShortBody`; **unterminated name** (no `0x1F` before frame end)
  → `UnterminatedRecord` (the key new reject); **empty name** → `EmptyStack`; **bad byte** (`/`,NUL) →
  `BadSegmentByte`; **traversal** (`.`/`..`) → `TraversalSegment`. `FileType` round-trips over the **full
  u8 range** incl. `Other(3..=255)` (not a 3-value enum); `type_flags` high bytes drop (re-encode zero).
- **G2 — totality (variable records).** Every malformed framing — mid-metadata, mid-name, unterminated
  trailing name, any length — → `Ok|Err`, never panic/over-read. The terminator scan is **bounded** (no
  over-read seeking `0x1F`). Alloc frame-bounded (each record ≥ 25B → ≤ frame/25 records; names ≤ frame),
  the 131072 cap holds. Source-discipline: no `unreachable!`/`unwrap`/`split_at`/index on input.
- **G3 — non-circular vectors.** Hand-authored byte-by-byte from file.md (review re-derives a sample —
  the 24B LE metadata field offsets + the `0x1F` terminator placement): a valid 1–2 record listing
  (bidirectional), the **unterminated-name reject** vector (the terminator-semantics regression guard),
  a truncated-metadata reject vector, and a `file-write`/`list` vector.
- **G4 / G5** — the 001 scans are reused, the safe/unsafe guarantees stand, **and extend to listing
  names**: the `ListEntry` name field is the **typed `UnsafeSegment`** returned by `scan_unsafe` — **not
  raw bytes** — so the logger-only-accepts-`SafeSegment` guarantee covers a real listing name (it is
  display-only, physically cannot reach a log line). Storing the name as `Vec<u8>` would lose this.
- **G6** — `#![no_std]`+`alloc`+`forbid(unsafe_code)` intact; the standing `x86_64-unknown-none` build
  each seam.
- **`Reject` stays structural-only** — `UnterminatedRecord` is a geometry reject; no value-policy variant.
- **Behavior preservation (content-hash).** `segment.rs` **byte-identical** (anchor `b3275467…`), plus the
  002 `cell.rs`/`display.rs` anchors captured at 3A. Additive edits (`reject.rs` +variant, `message.rs`
  +arms, `verb.rs` `IMPLEMENTED`) legitimately rehash their own binaries; a changed protected-surface
  content hash is a **REJECT**. A behavior-preserving *move* of 001 code (if the layout changes) keeps the
  file-seed round-trips green and is not a behavior change.

## Decided Forks (lead rulings — engineering, co-formulated)

1. **`list-entry` decoder + the scan-reuse seam.** A **new terminator-bounded record decoder** owns the
   framing (find `0x1F`, extract 24B metadata + name slice, consume the terminator, repeat to frame end) —
   distinct from 002's fixed-N tail and from the 001 separator-stack. The name's **alphabet** is validated
   by **reusing `scan_unsafe`** on the extracted (`0x1F`-free) name slice as a stack-of-1, keeping the
   unsafe alphabet single-source and **`segment.rs` byte-identical** (no edit). The impossible
   multi-segment arm is a **defensive typed reject, never `unreachable!`** (totality); empty name →
   `EmptyStack`. *This addresses the dev's "don't edit segment.rs" and review's "don't let separator
   semantics leak" — the decoder owns the terminator; `scan_unsafe` only validates a `0x1F`-free slice.*
   **Review-confirmed (2026-06-23): the stack-of-1 reuse satisfies the bar — no method defect** (the
   0x1F-free slice makes `scan_unsafe`'s split structurally inert, so separator semantics cannot leak;
   the alphabet stays single-source; the anchor stays frozen). **No `segment.rs` refactor** (it would
   re-baseline `b3275467…`, re-opening a surface frozen since 1B, for only a cosmetic gain). The
   `ListEntry` name field is the **typed `UnsafeSegment`** from `scan_unsafe` (G4-preserving — see G4/G5);
   storing raw bytes is a defect. Two binding 3A checks: (i) the slice to `scan_unsafe` is `0x1F`-free
   (take the bytes before the first `0x1F`); (ii) the impossible multi-segment arm is a defensive typed
   reject, never `unreachable!`. New reject: `Reject::UnterminatedRecord` (appended last, structural).
   Empty name → `EmptyStack` (a `NameEmpty` variant is the dev's free choice for clearer diagnostics —
   structural either way).
2. **Empty name → REJECT.** An immediate `0x1F` after the metadata → a typed structural reject
   (`EmptyStack` via the stack-of-1 reuse). Grounds: 001 already rejects an empty segment for both
   alphabets; a real directory-entry name is ≥1 byte; file.md §0's "no min length" governs the *alphabet*,
   not a zero-length name. (Review concurs.)
3. **`type_flags` — carry-raw low byte + reserved-drop high bytes.** `FileType { File, Dir, Symlink,
   Other(u8≥3) }` via the 001/002 enforce-by-construction private newtype (`Other` can never hold 0–2);
   high 3 bytes dropped (002 reserved-bit drop); `size`/`mtime_*` raw POD.
4. **`list` rest-of-frame, no leading count** — resolved by law 11 (a count is a box-authored second
   source of truth); records run to frame end, like 002's cell stream. (file.md §6-open resolved.)
5. **Reused decoders** — `file-write`→safe stack, `list`/`request-file`→unsafe stack, `file-data`→verbatim,
   `file-error`→001 `ErrorBody`. New message **types** only; **route through** the 001 decoders, never
   re-implement.
6. **`SEED` → `IMPLEMENTED` = 12** (already renamed in 002); adds 0x20/0x23/0x24/0x25/0x26.

## Escalations (operator-reserved — routed via the PM)

- **Verb-ids + error codes — provisional** (model §11). The file block ids (0x20–0x27) stay provisional
  pending operator ratification via the wire-numbers surface; vectors re-blessed if a number moves.

## Review Questions

1. **Confirm the scan-reuse ruling** (Fork #1): `scan_unsafe` stack-of-1 reuse with a defensive reject
   (anchor preserved) vs a behavior-preserving `segment.rs` refactor (anchor re-baselined). (Review, at
   the 3A pre-carve stop.)
2. Confirm the 3A creation manifest + seam carve before any code. (Review.) — **Confirmed; sprint
   accepted across 3A/3B/3C.**

---

## Postmortem — Sprint 003 retrospectives

*Sprint 003 closed 2026-06-23: all gates G0–G6 accepted by independent review across 3A/3B/3C (fuzz
reproduced reaching `decode_list_entries` via dispatch at cov 587, vectors re-derived byte-by-byte, the
three protected anchors recomputed byte-identical). The file service is complete (8 verbs); the variable
combined-record decoder — the headline obligation carried since 001/002 — is built, gated, proven on the
live dispatch path, vector-locked, fuzz-covered, and the obligation is retired. Builds on the 001/002
postmortems; durable lessons, not status.*

### Developer (`zero.proto-dev`)

**What moved the plan.** (1) *Reuse an accepted decoder via its public API* → the frozen anchor stays
byte-identical **and** the G4 guarantee is inherited (the list-entry name comes back as a typed
`UnsafeSegment`, never raw bytes — a raw name would have been a real G4 defect, invisible to a
behavior-only check). (2) *Separator-vs-terminator is decided by what FOLLOWS the element* — a stack
segment is separator-framed (the frame ends the last), a record-internal name is terminator-framed
(every name ends with `0x1F` because the next record follows; an unterminated name is `UnterminatedRecord`).
(3) *A count-anchored bidirectional `== IMPLEMENTED` coverage gate self-corrects* — it forced the +2 file
vectors (file-data/request-file) that a hand-typed list omitted. (4) *The build-isolated → wire-dispatch
→ prove-via-dispatch live-path cascade held a 3rd time.*

**003 inherits forward.** File service COMPLETE, the variable-combined-record obligation RETIRED, the
decoder vocabulary whole. Remaining: net 0x31/0x32/0x33 + the reassembler; `key` 0x12 still blocked
(keysym table).

### Reviewer (`zero.proto-review`)

**What 003 added to the method.** (1) *Reuse preserves the TYPE/INVARIANT, not just the reject* — when
validating a reuse, ask what type/guarantee the primitive carries (the typed `UnsafeSegment` → G4
extends to listing names), not only what it rejects; a raw-bytes name would reject identically while
silently dropping the guarantee. (2) *The frozen content-hashed anchor is worth protecting — reuse over
refactor when the cleaner factoring re-opens it*; capture new anchors as surfaces freeze (003 added
cell.rs/display.rs to segment.rs). (3) *Terminator-vs-separator — the discriminating gate is the reject
only the correct framing produces* (the unterminated-name reject), frozen as a vector; the new decoder
owns the new framing, the reused primitive sees only the framing-stripped slice. (4) *Source-anchored
**bidirectional** coverage self-corrects an incomplete reviewer* — anchor to a source const, make it `==`
not subset; a forced one-slice relaxation is legitimate only if `==` is restored at a fail-closed later
seam. (5) *A dead defensive arm, correctly uncovered, is a PASS* — its being uncovered proves it is
dead-by-construction; distinguish a fail-closed/encode-side miss (correct) from a decode-path gap (a hole);
the bar is 100% branch on the decode path + every miss explained by source.

**Next reviewer inherits.** The per-seam live-path cascade + observable-reject proof; reuse preserves the
type/invariant; the protected-anchor set (segment.rs `b3275467` + cell.rs `8bf962e2` + display.rs
`a24b83a2`, recompute all); source-anchored bidirectional coverage; the settled carry-raw/reserved-drop/
structural-only/defensive-not-panic invariants. **No unbuilt-decoder debt remains** — the next genuinely-
new hard mechanism is **net-payload reassembly** (the one trusted-side, continuation-bit, bounded,
fail-closed buffer; its cap value is operator-reserved). *One line:* anchor the gate to the source, make
it bidirectional and fail-closed, and it reviews what the reviewer forgets.

### Lead (`zero.proto-lead`)

**Process lessons.** (1) *Third clean sprint on the established loop* — co-formulate → draft → seam-by-seam
with per-seam independent acceptance → close → scoped commit. (2) *The scan-reuse fork (dev vs review) I
arbitrated toward reuse* (anchor-preserving, single-source, G4-preserving) with the defensive-reject fix,
surfaced for review's gate-confirm; they confirmed. The deciding insight was that reuse preserves the
type/invariant **and** the frozen anchor, where a refactor would re-open a surface frozen since 1B for a
cosmetic gain. (3) *A flagged one-slice gate relaxation is legitimate when it fail-closed re-tightens* —
the dev flagged the coverage `==`→`⊆` narrowing; I recorded the 3C re-tighten in the contract; the
reviewer confirmed it forced + self-corrected. Record the re-tighten so a relaxation can't be forgotten.
(4) *Convergence keeps recurring* — dev and reviewer independently reach the same patterns (the bit-width
carriers in 002, the terminator framing + the typed name here); trust it. (5) *Surface the operator's one
reserved item early* — the reassembler cap gates the net track; the PM is surfacing it in parallel so it
won't be a late blocker.

**Next inherits (process).** The contract is the source of truth; the standing gates carry forward;
numbers stay provisional pending operator ratification; the file service is complete; the net track's cap
value is the one operator-reserved item to land before net-payload reassembly. 004 = input service
(co-formulating now); `key` stays blocked on the keysym table.
