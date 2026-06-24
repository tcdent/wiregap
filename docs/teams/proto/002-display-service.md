# Sprint 002 — Display Service

> The second sprint of the `proto` crate: the box→head **display** service — the cell stream that paints
> a terminal — plus `create-pty`. It builds the **repeated-fixed-record tail decoder** (the headline
> obligation deferred from 001), the cell-op grammar, and the shared `Dimensions` body. Built on the 001
> codec core, inheriting its gates (G0–G6) and invariants (structural totality, carry-raw
> enforce-by-construction, reserved-bit drop, `#![no_std]`+`alloc`+`forbid(unsafe_code)`).
>
> Team: `proto` (lead `zero.proto-lead`, dev `zero.proto-dev`, review `zero.proto-review`).
> Grounded in [`display.md`](../../proto/display.md), [`library.md`](../../proto/library.md) §4/§7,
> [`model.md`](../../proto/model.md) §6/§8, and the 001 contract ([`001-codec-core.md`](./001-codec-core.md)).

---

## Scope Boundary

**This sprint owns** the display consumer surface ([`library.md`](../../proto/library.md) §7: the head
decodes display; the box encodes it; the proxy relays it inert):

1. The **repeated-fixed-record tail decoder** ([`model.md`](../../proto/model.md) §6 — the decoder
   deferred from 001): a run of fixed-width records to the frame end, with a frame-determined count and
   **partial-trailing-record rejection**. Generic over a const record width; display instantiates N=4.
2. **`display` (0x11)** — the box→head cell stream: `Display(Vec<Cell>)`, the cell-op grammar
   ([`display.md`](../../proto/display.md) §1–2).
3. **`create-pty` (0x10)** — head→box, body `Dimensions` (a fixed `[rows u16][cols u16]`).
4. The shared **`Dimensions`** grammar (the §8 grammar-reuse decision: `create-pty` now, `resize` 0x13
   reuses it in the input sprint).

**This sprint does NOT own** (named, with the sibling sprint that owns it):

- **`list-entry`'s repeated-*combined*-record decoder** — `[fixed metadata][variable 0x1F-terminated
  name]` is variable-length, a **different, harder decoder** than the fixed-N tail built here. Owned by
  the **file sprint**. A 002 green must **not** be read as proving it.
- **The §6-open cell payload semantics** — attribute bits (0x05), cursor shape (0x08), default-color
  theming, a future style-reset op ([`display.md`](../../proto/display.md) §6 Open). The crate carries
  these payloads **raw**; their interpretation is the head's, so they do **not** gate 002.
- **The input service** (`key`/`mouse`/`paste`/`focus`/`resize`/`viewport`) — later; `resize` 0x13 will
  reuse `Dimensions`. `key` stays blocked (keysym table).
- **Reassembly, the L1 envelope, feature-gating, net, file** — later sprints (cap operator-reserved).

## Problem

The 001 codec core proved itself over five seed verbs but deliberately deferred the **repeated-fixed-record
tail decoder** — both retrospectives named it the biggest unbuilt obligation. No consumer-facing service
exists yet: the head cannot decode a display payload to paint a terminal, and the box has no typed
`create-pty`. The display cell stream is the pure form of the deferred decoder (a run of `u32` cells to
the frame end), so it is the natural first service.

## Goal

The head can decode a `display` cell stream into a typed sequence of cell ops, and the box can encode one
— both through the **one** crate-owned cell-op layout, so box-encode and head-decode cannot drift
([`library.md`](../../proto/library.md) §0). The repeated-fixed-record tail decoder exists, is total over
hostile input (including every length residue), and rejects a partial trailing record. `create-pty`
carries a shared `Dimensions` body. All 001 gates and invariants still hold; the 001 scan surface is
byte-identical.

## Final State

- The generic repeated-fixed-record tail decoder exists: an N-byte-record run to frame end; **empty tail
  → `Ok` (a no-op repaint)**; a partial trailing record → `Reject::PartialRecord` (structural).
- `Display(Vec<Cell>)` decodes/encodes through the crate-owned cell-op grammar; `Cell` is the typed op
  set (see Decided Forks #1), payloads carried raw, reserved ops carried by construction.
- `create-pty` (0x10) carries `Dimensions { rows: u16, cols: u16 }`; `Dimensions` is reusable.
- `verb.rs` (the wire-numbers surface) gains the **cell op-set** (0x00–0x08, decided per display.md §6)
  and the 0x10/0x11 dispatch; `IMPLEMENTED` (the renamed `SEED`) = 7 verbs.
- All 001 gates (G0–G6) green on the post-002 tree; the **segment scan surface is byte-identical** to 001.

## In Scope — ordered slices (review seams)

The developer produces the **2A pre-carve creation manifest** (the verbs 0x10/0x11 + the 9-op cell
sub-grammar table — already drafted in co-formulation) and **stops for review before any carve**.

- **2A — the repeated-fixed-record tail decoder.** The generic const-N decoder (`as_chunks::<N>` /
  `chunks_exact` fallback — dev verifies stability on 1.94), the `PartialRecord { record_len, remainder }`
  reject, **empty = Ok**. Gets a focused G1/G2 gate *before* the cell grammar rides on it. *Gates:* G1
  field-edges (0/1/N records Ok + partial-tail reject) + G2 source-discipline.
- **2B — the cell-op grammar + `display` 0x11.** The `Cell` enum (typed op; ratified field-splits
  decoded, §6-open payloads carried whole-raw; `Cell::Reserved` enforce-by-construction); `Display(Vec<Cell>)`
  decode/encode over the 2A tail decoder; the op-set consts in the wire-numbers surface. *Gates:* G1
  (round-trip over the full `Cell` type — the carry-raw canonical invariant), G2 source-discipline.
- **2C — `create-pty` 0x10 + `Dimensions` + self-validation consolidation.** `Dimensions{u16,u16}` body;
  the 0x10/0x11 dispatch arms + `IMPLEMENTED` rename; hand-authored non-circular vectors (a valid cell
  stream, a **partial-tail reject vector**, a reserved-op carry/ignore vector, a `create-pty` vector); the
  kitted G2 fuzz extended to the cell-stream + tail decoder; full G0. *Gates:* G2 (full), G3, G0, G6.d.

## Non-Goals

- `list-entry`'s variable combined-record decoder (file sprint) — the fixed-N decoder does **not** cover
  it; do not claim it.
- Interpreting §6-open cell payloads (attribute/cursor/color bits) — carried raw; the head interprets.
- Ratifying the cell payload semantics or verb-ids (provisional; see Escalations).
- **Do not reject a structurally-valid message:** an empty cell stream is `Ok`; a known verb with nonzero
  reserved upper bytes still dispatches; an unknown cell op is **carried (`Cell::Reserved`), never
  rejected** ([`display.md`](../../proto/display.md) §1 `_` ignored).

## Starting Surface

002 is **additive** on the accepted 001 surface. The developer's 2A pre-carve manifest names every touch:
- `reject.rs`: + `PartialRecord` (additive variant). `wire.rs`: + `decode_record_tail` (the generic
  decoder). `message.rs`: + 0x10/0x11 arms + `Message` variants (before the `UnsupportedVerb` catch-all).
- `verb.rs`: + the cell op-set consts + `SEED`→**`IMPLEMENTED`** (5→7). This is the one accepted **const
  rename**; values unchanged.
- `terminal/`: + `display` and `dimensions` modules (currently only `pane_state`). `lib.rs`: + exports.

Inventory shape (same as 001): `verb id | name | direction | body shape | decoder kind | Rust type |
module | 002 action`, plus the cell op sub-grammar table.

## Method

Inherits the 001 method (owned field-by-field decode; bounded accessors only — no panic-index / `split_at`
/ `unwrap`/`expect` on input; the scan/decoders verify, never transform; build coherence; non-circular
hand-authored vectors; stop-for-review at each seam). Additions:

- **The record width is a compile-time const**, the loop boundary is the slice end, the partial-tail falls
  out of `as_chunks` returning the remainder — **never a box-authored count** (law 11). Alloc stays
  frame-bounded (`Vec<Cell>` ≤ frame_len/4).
- **Carry, don't re-derive:** the cell op is the top byte; ratified field-splits (draw scalar/width;
  `RowCol` 12|12 for goto/cursor-pos/set-dims; 24-bit RGB) are decoded; §6-open payloads are carried as a
  whole raw value. **Never interpret payload bits** the head owns.
- **Single-source layout (anti-drift):** the cell-op layout lives once in the crate so the box's encode
  and the head's decode cannot drift (library §0). This is why `Cell` is typed (Decided Fork #1).
- **Live-path validation (cascading through the seams):** the display decoder's live caller is the
  **head** (the proxy relays display without decoding, display.md §4); `create-pty`'s live caller is the
  **box**. Acceptance validates the path the consumer actually executes, not a harness-only path. 2A:
  `decode_record_tail` proven, no parallel loop. 2B: `Display::decode` routes through it (proven —
  `allow(dead_code)` removed, `PartialRecord` the observable tell). **2C (binding):** `decode_message`'s
  0x11 arm must **route through `Display::decode`** (not re-implement), `Display` wired into `Message`,
  and the kitted fuzz must show coverage reaching the **cell-stream decoder through the 0x11 dispatch** —
  the full production path `frame → decode_message → 0x11 → Display::decode → decode_record_tail → cells`,
  not `Display::decode` in isolation.

## Acceptance

All **001 gates G0–G6 carry forward** and must be green on the post-002 tree. The new/refined obligations:

- **G1 — round-trip + the new geometry.** `decode(encode(m))==m` + canonical re-encode over: the 5 seed
  (unchanged) + `create-pty` + `display`. Cell-level: the carry-raw **canonical invariant over the full
  `Cell` type** (each variant's payload constrained so a non-canonical cell is unrepresentable —
  enforce-by-construction, per Fork #1). Tail field-edges: 0 records (**Ok**), 1, N; **partial tail
  (`frame_len % 4 != 0`) → typed `PartialRecord`**.
- **G2 — totality.** Arbitrary byte lengths (all residues mod 4) → `Ok|Err`, never panic/over-read; the
  partial-tail is a clean reject. The kitted fuzz corpus includes all-mod-4-residues + reserved-op cells;
  coverage must show entry into the **cell-stream decoder** (not near-zero); the alloc cap holds.
- **G3 — non-circular vectors.** Hand-authored byte-by-byte from display.md (review re-derives a sample;
  watch the endianness — op is the **high-order** LE byte): a valid multi-op cell stream (bidirectional),
  a **partial-tail reject vector**, a reserved-op carry/ignore vector, a `create-pty` vector.
- **G4 / G5** — unchanged (no new segment or logger surface in 002; the 001 guarantees stand).
- **G6** — `#![no_std]`+`alloc`+`forbid(unsafe_code)` intact; the standing `x86_64-unknown-none` build
  green each seam.
- **`Reject` stays structural-only** — `PartialRecord` is a geometry reject; no value-policy variant.
- **Behavior preservation (two-part, per review's refinement).** (a) **Must not change:** the **segment
  scan surface** and the existing verb-id/error-code **values**. *The load-bearing proof is the
  `segment.rs` **source byte-identical*** — standing baseline `sha256
  b3275467be8ce9a196834a618401bfcd917eae7157c5b85de59b95686c9bd4ac` (9539 B, the 1C-accepted scan
  surface). The `segment_rejects` **crate-hash suffix** (`…3a35aab1ae88aee4`) is a target-*identity*
  sanity (stable across content edits), **not** content-integrity; and **not** the whole `segment_rejects`
  test-binary content sha256, which **legitimately rehashes** when `reject.rs` gains a variant (the
  binary links `Reject::{eq,Debug,Display}`). (b) 001 behaviors stay green (segment_rejects 11/11, the 5
  seed round-trips) and every 001-file edit is **purely additive** — no existing value/behavior changed.
  A new `Reject` variant rehashing reject-dependent binaries is **expected**; a changed **scan surface**
  (segment.rs content / crate-hash suffix) is a **REJECT**.

## Decided Forks (lead rulings — engineering, grounded + co-formulated)

1. **Cell representation — typed `Cell` op enum** (`Display(Vec<Cell>)`), bound by review's three rules:
   (a) **type only the op + ratified field-splits**, carry §6-open payloads whole-raw (so unratified
   attribute/cursor codes do not gate 002); (b) **unknown ops carried, never rejected** (`Cell::Reserved`);
   (c) **enforce-by-construction** so the carry-raw canonical invariant holds over the full `Cell` type
   (Reserved holds op ≥ 0x09 only; `Scalar`/`Width`/`RowCol`/`Rgb`/`AttrBits`/`CursorStyleBits` constrained
   to their **bit-widths** via private/from-packed construction). **Make-or-break check (review):** each
   sub-field carries its **full bit-width range RAW** — `Width` is a 3-bit raw (0–7), **not** a
   `{Zero,Normal,Wide}` enum; cursor shape is a 2-bit raw, not `{Hidden,Block,Underline,Bar}`; `Scalar`
   is the full 21-bit raw incl. `> 0x10FFFF` (the head renders the replacement glyph — the crate carries,
   never validates Unicode range); `set-attributes` carries the whole 24-bit payload (bits 9–23 included).
   Enum-izing any sub-field to its *defined value-set* would make an undefined-but-structurally-valid cell
   non-representable — the 001 subset-shadow one layer down. Acceptance verifies the canonical invariant
   over each field's **full bit-width range** (RowCol 4095/4095, Scalar 0x1FFFFF, Width 7, AttrBits
   0xFFFFFF, cursor shape 3), and the arbitrary-cell fuzz self-defends. *Rationale:* the box **encodes**
   display and the head
   **decodes** it through this crate (library §0/§7) — the cell-op layout must live in the crate **once**
   or the two consumers drift; raw `Vec<u32>` would scatter the layout across box and head. The typed
   surface **is** defined wire grammar (the minimize-surface lens trims undefined/ambient surface, not the
   protocol grammar). *Op set:* 0x00–0x08 (display.md §6 **decided**) → consts in the wire-numbers surface,
   vector-lockable. **Operator-confirmed** (2026-06-23): *"the shared spec seam between encode and decode
   is the point."*
2. **Repeated-fixed-record tail decoder** — generic const-N (`as_chunks::<N>`, `chunks_exact` fallback);
   N=4 for display; **empty tail → `Ok`** (a no-op repaint — *not* the segment stack's `EmptyStack` rule,
   which requires ≥1 because an empty address is meaningless); partial tail → `Reject::PartialRecord
   { record_len, remainder }` (structural, distinct from `TrailingBytes`). Built generic, gated on
   display's N=4 instantiation; `list-entry` is a different decoder (Non-Goals).
3. **`Dimensions` = `{ rows: u16, cols: u16 }`** — a fixed `[u16][u16]` free verb body, carried raw (the
   box clamps oversize, like goto's out-of-grid clamp; no value reject). **Distinct** from the cell-op
   `RowCol` (12|12 packed, payload-constrained) — a free body is not 24-bit-constrained, so "generous
   where free" (model §4); do not force-unify across the wire layers. Reusable: `create-pty` now, `resize`
   later (grammar-reuse §8 decision: yes).
4. **`SEED` → `IMPLEMENTED`** — the constant names the set of verbs with decode arms (grows each service
   sprint), anchoring `seed_set_matches_dispatch` / G0 coverage. A const **rename** (value-preserving);
   `IMPLEMENTED` = 7 (0x18, 0x21, 0x22, 0x27, 0x30, 0x10, 0x11).

## Escalations (operator-reserved — routed via the PM)

- **Cell representation (Fork #1) — RESOLVED: typed `Cell`** (operator, 2026-06-23, with 002 green-lit).
  The operator ratified the anti-drift rationale verbatim: *"typed Cell — the shared spec seam between
  encode and decode is the point."* (The raw-`Vec<u32>` alternative is closed.)
- **Verb-ids + the cell op-set + error codes — provisional.** The op-set (0x00–0x08) is display.md-decided
  and **vector-lockable**; the §6-open payload semantics are carried raw and do not gate. Verb-ids stay
  provisional pending operator ratification via the wire-numbers surface; vectors re-blessed if a number
  moves.

## Review Questions

1. **Confirm the cell-representation choice** (typed `Cell` vs raw `Vec<u32>` — Fork #1 / Escalations).
   (Operator.) — **Resolved: typed Cell, operator-confirmed 2026-06-23.**
2. Confirm the 2A creation manifest + seam carve before any code. (Review, at the 2A pre-carve stop.) —
   **Confirmed; sprint accepted across 2A/2B/2C.**

---

## Postmortem — Sprint 002 retrospectives

*Sprint 002 closed 2026-06-23: all gates G0–G6 accepted by independent review across seams 2A/2B/2C
(incl. an independent fuzz reproduction at cov 436, byte-by-byte vector re-derivation, and the cascading
live-path proven end-to-end). The repeated-fixed-record tail decoder deferred from 001 is now built,
gated, and proven on the live display path. Numbers remain provisional pending operator ratification.
Builds on the 001 postmortem; durable lessons, not status.*

### Developer (`zero.proto-dev`)

**What moved the plan.** (1) *The live path cascades through seams* — "tested in isolation" ≠ "on the
live path." `Display::decode` routing through the 2A decoder at 2B *looked* sufficient, but the production
path `frame → decode_message → 0x11 → Display → tail → cells` didn't exist until 2C wired the dispatch
arm; verify coverage reaches code through the **real entry point**, not a test-only call. (2) *Build the
queued obligation generic, gate it standalone, then let a consumer ride it* — the fixed-N tail decoder
landed in 2A with its own gate before the cell grammar's complexity piled on in 2B. (3) *Typed
single-source pays off across a two-consumer wire format* — box encodes display, head decodes it, both
through one crate-owned layout (library §0, no drift); minimize-surface trims undefined surface, not the
defined grammar. (4) *Decode the ratified field-split, carry the value raw* — carriers are bit-width
newtypes, never value-enums, so display.md §6-open codes stay the head's concern and never gate the crate.

**What failed / surprised.** (a) The 2B→2C live-path gap was the sharpest correction — I under-read
"live path" as the verb-body decoder; review's cascading condition caught it. (b) Behavior-preservation
hashing needed reconciling — a "byte-identical test binary" is confounded by linked `Reject::{eq,Debug}`
that grow with any variant; anchor preservation to the **source** (segment.rs sha256 + crate-hash), not a
downstream binary.

**003 inherits.** The reusable fixed-N tail decoder, the cell op-set in the wire-numbers surface, the
`IMPLEMENTED`-grows-each-sprint coherence anchor, the shared `Dimensions` grammar (`resize` reuses it),
the fuzz+replay-coverage pattern. **Still owned by the file sprint (not covered here):** `list-entry`'s
**variable** combined-record `[metadata][0x1F-terminated name]` — a different, harder decoder whose
record-internal `0x1F` *terminator* differs from the segment stack's *separator*. Still blocked/provisional:
`key` 0x12 (keysym table); all verb-ids + the cell op-set + error codes pending operator ratification.

### Reviewer (`zero.proto-review`)

**What 002 added to the method.** (1) *The live path cascades through seams — make it a binding per-seam
condition.* A unit proven in isolation is not proven on the production path; at each seam the question is
"does the thing the next layer calls actually route through the proven unit, with no parallel
re-implementation?" The **observable proof** is a reject only the lower unit emits (`PartialRecord` from
`Display::decode`, then from `decode_message`) — an inline re-implementation could not produce it. Carry
the condition forward; never let a unit sit tested-but-dead below the production entry point. (2) *The
bit-width-raw carrier check* — when the crate types a wire value, each split sub-field must carry its
**full bit-width range raw**, never an enum over its defined value-set (undefined-but-in-width values must
round-trip). Verify two ways: a decode/encode **bijection trace** (widths tile the payload exactly) **and**
tests over the undefined-but-in-width corners. Enforce-by-construction has two required shapes: a public
width-checked `new()->Option` for encode-side carriers, and a decode-only private carrier for the
reserved/unknown value. (3) *Content-hash > metadata-suffix* (a correction I owned mid-sprint) — cargo's
test-binary filename suffix is a target *identity*, stable across content edits; the load-bearing proof
is a **content hash of the protected source** + the mtime edit-set + green baselines. The whole-test-binary
content sha256 legitimately changes on an additive edit, so rejecting on it is a false positive. (4)
*Fuzz coverage must reach the unit via the production dispatch* — a coverage figure that doesn't climb
into the new unit through the real entry point is near-zero *for that unit*; explain every miss by source
(decode-path vs encode-side/peripheral).

**003's reviewer inherits.** The per-seam live-path cascade (route-through + observable-reject proof +
remove `allow(dead_code)` when the consumer lands); the typed-wire-grammar gate (full-bit-width round-trip
+ bijection trace + undefined-but-in-width tests; enforce-by-construction; never a value-set enum);
content-hash behavior preservation; fuzz-via-dispatch. *Next carried-forward unproven obligation:*
`list-entry`'s variable combined-record decoder (file sprint). *One line:* in a sliced build, "proven" is
per-seam and per-layer — the catch is always asking what the green did **not** exercise: the production
path, the undefined-but-valid value, the protected source, the dispatched entry.

### Lead (`zero.proto-lead`)

**Process lessons.** (1) *The 001 process held with no friction* — co-formulate (dev + review) → draft →
team review → seam-by-seam carve with per-seam independent acceptance → close. The one genuine fork
(cell-rep typed vs raw) I arbitrated toward typed on the anti-drift/single-source grounds (over review's
auditable-surface lean), surfaced it to the operator as a flagged call, and the operator ratified it on
that exact reasoning — the surface-vs-anti-drift weighting was theirs to confirm, and the lens here was
"the crate's reason to exist," not gratuitous surface. (2) *Convergence is a strong signal* — the dev and
reviewer independently arrived at the bit-width-raw carriers; when both reach the same design unprompted,
trust it. (3) *The loop self-corrected cleanly* — the dev surfaced the behavior-preservation hash ambiguity,
I tightened the contract, and the reviewer owned a correction to their own earlier framing; none of it
needed escalation. (4) *Carry each seam's binding condition forward* — review's per-seam live-path cascade
worked because every seam's condition explicitly bound the next; the lead's job is to relay those as build
inputs so they're built-to, not just checked-at-acceptance.

**003 inherits (process).** The contract is the source of truth; standing gates (G6.d bare-metal, G2
coverage+alloc-cap+fuzz-via-dispatch, content-hash behavior preservation) carry forward; numbers stay
provisional until operator ratification via the wire-numbers surface; the file service is the natural next
chunk (it owns the deferred `list-entry` variable combined-record decoder), with net/reassembly behind it
(cap operator-reserved) — but the chunk is the operator's sequencing call.
