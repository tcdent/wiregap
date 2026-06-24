# Sprint 004 — Input Service

> The fourth sprint of the `proto` crate: the head→box **input** service — `resize`, `viewport`,
> `mouse`, `paste`, `focus`. The **safe direction** (box-parsed; model §8 "not a trusted surface") — but
> the crate still owns the typed grammar (single-source, anti-drift: the head encodes, the box decodes
> through this crate). No new decoder *class*; the one genuinely-new wrinkle is **signed POD (`i16`)**.
> `key` 0x12 stays out (the `> 0x10FFFF` keysym table is unpinned). Built on the 001/002/003 codec.
>
> Team: `proto` (lead `zero.proto-lead`, dev `zero.proto-dev`, review `zero.proto-review`).
> Grounded in [`model.md`](../../proto/model.md) §8 (Input events) and the 001/002/003 contracts.

---

## Scope Boundary

**This sprint owns** the input consumer surface (head→box; the **box** decodes via `decode_message`):

1. **`mouse` (0x15)** — a 12B fixed body `[button u8][kind u8][mods u16][row u16][col u16][dx i16][dy i16]`,
   with the carriers (the headline new representation): `Button`/`MouseKind` carry-raw enums, `Modifiers`
   reserved-bit-drop bitfield, `dx`/`dy` **signed `i16` POD**.
2. **`resize` (0x13)** — `Dimensions{rows u16, cols u16}`, **reusing** the 002 grammar (the §8
   grammar-reuse: create-pty + resize share `Dimensions`).
3. **`viewport` (0x14)** — `[offset u32]`, a fixed `u32`.
4. **`paste` (0x16)** — `[text verbatim tail]`, `Vec<u8>` verbatim.
5. **`focus` (0x17)** — `[gained u8]`, carry-raw `u8`.

`IMPLEMENTED` grows 12 → 17. `key` 0x12 stays a **stub** (keysym table unpinned).

**This sprint does NOT own** (named, with the sibling sprint that owns it):

- **`key` (0x12)** — blocked on the unpinned named-key keysym table ([`model.md`](../../proto/model.md)
  §11); the input service's last verb, in a later sprint once the table lands.
- **Net** (net-payload reassembly — the next genuinely-new mechanism, cap operator-reserved), **the L1
  envelope, feature-gating, control** (open-session) — later sprints.

## Problem

The terminal surface is half-built: the box→head display exists (002), but the head cannot send input
to the box. The 5 input verbs are pinned (model §8 Input events) except `key` (keysym table). The
representation work is pattern-recurrence (carry-raw enums, reserved-bit drop, verbatim, grammar reuse)
plus the first **signed** fields (`mouse` scroll deltas).

## Goal

The head encodes and the box decodes all 5 input verbs through the one crate-owned grammar. The mouse
carriers are bijective over their full ranges (`Button`/`MouseKind` over u8 incl. `Other`; `dx`/`dy`
over the full `i16` **including negatives**; `Modifiers` dropping reserved bits canonically). `resize`
reuses `Dimensions` (no re-implementation, `dimensions.rs` byte-identical). The 001/002/003 protected
surfaces stay byte-identical.

## Final State

- `terminal/input.rs`: `Resize(Dimensions)`, `Viewport{offset:u32}`, `Mouse{...}`, `Paste(Vec<u8>)`,
  `Focus{gained:u8}`, plus `Button`/`MouseKind`/`Modifiers` (+ `ReservedButton`/`ReservedMouseKind`).
- Dispatch wired for 0x13/0x14/0x15/0x16/0x17 (each routes *through* its decoder); `IMPLEMENTED` = 17;
  `key` 0x12 still a stub; `CATALOG` = 22.
- **No new `Reject` variant** — all bodies are fixed-geometry (`ShortBody`/`TrailingBytes`) + carry-raw
  + verbatim; `Reject` stays structural-only (the coherence tell holds).
- The protected anchors — `segment.rs` `b3275467…`, `cell.rs` `8bf962e2…`, `display.rs` `a24b83a2…`, and
  the new `dimensions.rs` (captured at 4A) — byte-identical.

## In Scope — ordered slices (review seams)

The developer produces the **4A pre-carve creation manifest** (the 5 verbs + the mouse field sub-grammar
+ the carriers) and **stops for review before any carve**.

- **4A — `mouse` + the carriers** (the headline new representation, isolated first). `Button`/`MouseKind`
  carry-raw enforce-by-construction; `Modifiers` reserved-bit-drop; `dx`/`dy` signed `i16`; `Mouse` the
  12B fixed-only body. *Gates:* G1 carrier round-trips over full ranges (**incl. negative `i16`**) + G2
  source-discipline.
- **4B — the 4 simpler verbs + dispatch.** `resize` (reuse `Dimensions`), `viewport`, `paste` (verbatim),
  `focus`; the 0x13–0x17 dispatch arms (each routes through its decoder); `IMPLEMENTED` = 17. *Gate:*
  round-trip + dispatch + reuse-not-reimplement (resize through `Dimensions`).
- **4C — vectors + extended fuzz + G0.** Hand-authored non-circular vectors (mouse **with a negative
  delta** + carrier `Other` values + reserved-bit-set mods; a non-UTF-8 paste; resize/viewport/focus +
  reject vectors); the kitted fuzz extended to the input decoders **via dispatch**; full G0 with the
  coverage anchor re-tightened to `covered == IMPLEMENTED(17)`. *Gates:* G2 (full), G3, G0, G6.d.

## Non-Goals

- `key` 0x12 (keysym table) — stub only.
- Net/reassembly/envelope/feature-gating/control sprints.
- A new `Reject` variant (all 004 bodies are fixed-geometry + carry-raw + verbatim).
- **Do not edit** `segment.rs`/`cell.rs`/`display.rs`/`dimensions.rs` (reuse `Dimensions` via its public
  API).
- **Do not validate** `paste` UTF-8 (consumer's concern) or `mouse` coords/deltas (carried raw; box clamps).
- **Do not enum a value-set or carry-raw a reserved bitfield** — the three mouse patterns are distinct
  (see Method): carry-raw enums (Button/MouseKind), reserved-drop bitfield (Modifiers), signed POD (dx/dy).

## Starting Surface

004 is **additive**; the 001/002/003 surfaces are untouched. The 4A manifest captures all four protected
anchors (`segment.rs`/`cell.rs`/`display.rs` + the new `dimensions.rs`, since `resize` reuses it). Touches:
new `terminal/input.rs`; `message.rs` +5 arms/variants; `verb.rs` `IMPLEMENTED` 12→17 (+ re-anchor
`implemented_set_matches_dispatch` and the coverage gate to 17); `terminal/mod.rs` + `pub mod input`;
`lib.rs` +exports. **No `reject.rs` change.**

Inventory shape (as prior sprints): `verb id | name | direction | body shape | decoder kind | type |
module | 004 action`, plus the mouse field sub-grammar + the carrier table.

## Method

Inherits the established method (owned field-by-field decode; bounded accessors only; build coherence;
non-circular hand-authored vectors; the per-seam live-path cascade — here **box-side** via
`decode_message`; content-hash behavior preservation; stop-for-review). The mouse body exercises **three
distinct patterns that must not be conflated**:

- **Carry-raw enum** (`Button`, `MouseKind`): private `Reserved*(u8)` newtype, in-crate from-byte ctor
  (`≥6` / `≥4`), getter — `Other(in-defined-range)` unrepresentable; **full-u8 round-trip incl.
  `Other(6..=255 / 4..=255)`; never a value-set enum** (which would reject/drop an undefined value — the
  subset-shadow one layer down, caught by the arbitrary-byte fuzz).
- **Reserved-bit-drop bitfield** (`Modifiers`): a 6-bool struct `{shift, control, alt, super_, caps_lock,
  fn_}` — bits 0–5 typed, **bits 6–15 unrepresentable by construction** (canonical re-encode, reserved
  zero; no masking discipline needed). *Not* a carry-raw enum, *not* a raw u16.
- **Signed POD** (`dx`/`dy` `i16` — the new wrinkle): `i16::from_le_bytes`/`to_le_bytes`, full range, no
  validation. **The make-or-break is the sign bit:** the gate round-trips over `i16::MIN` (LE `00 80`),
  `-1` (LE `FF FF`), `i16::MAX` (LE `FF 7F`); a positive-only test/vector is a **subset-shadow** and is
  rejected. `row`/`col` are raw `u16` (box clamps an out-of-grid coord).
- **Reuse, not re-implement:** `resize` routes through `Dimensions::decode_body`/`encode_body` (002,
  public) — `dimensions.rs` untouched (a new protected anchor); a re-implemented dimensions parse is a
  defect. Observable: a `Dimensions` reject (`ShortBody`/`TrailingBytes`) surfacing from `resize`'s decode.
- **Live-path cascade (box-side):** `decode_message` routes each input verb (0x13–0x17) to its decoder;
  the fuzz reaches each **via dispatch** (cov climbing into the input decoders, not isolation); observable
  via a reject only the new decoder emits (a mouse `TrailingBytes`, the `Dimensions` reject from resize).

## Acceptance

All **001/002/003 gates G0–G6 carry forward**, green on the post-004 tree. New/refined obligations:

- **G1 — round-trip + the carriers.** Round-trip over the 17 `IMPLEMENTED` verbs. Mouse carriers: full-u8
  round-trip for `Button`/`MouseKind` incl. `Other`; **`dx`/`dy` full-`i16` incl. `i16::MIN`/`-1`/
  `i16::MAX`** (the sign-bit gate); `Modifiers` reserved-bit drop canonical (decode reserved-set mods →
  bits 0–5 only, re-encode zero); `row`/`col`/`viewport.offset` full unsigned; `focus.gained` full-u8
  carry-raw. Geometry: mouse 12B / viewport 4B / focus 1B / resize 4B are fixed-only (`ShortBody` if
  short, `TrailingBytes` if long); paste verbatim (empty/1B/max-frame).
- **G2 — totality.** Arbitrary bytes → `Ok|Err`, never panic/over-read. The bodies are frame-bounded
  (fixed + verbatim), the alloc cap holds; the fuzz reaches the input decoders **via `decode_message`
  dispatch**. Source-discipline: no `unwrap`/`expect`/`split_at`/index/panic on input.
- **G3 — non-circular vectors.** Hand-authored byte-by-byte from model §8 (review re-derives a sample,
  watching the **`i16` LE of negatives** — `dx=-1`→`FF FF`, `i16::MIN`→`00 80` — and the carrier `Other`):
  a mouse vector **with a negative delta** + `Other` button/kind + reserved-set mods; a **non-UTF-8 paste**
  (locks `Vec<u8>`); resize/viewport/focus + their reject vectors.
- **G4 / G5** — unchanged (no new segment/logger surface; input `text`/`paste` are verbatim into the box,
  never logged — the safe/unsafe segment rule does not apply, model §8).
- **G6** — `#![no_std]`+`alloc`+`forbid(unsafe_code)` intact; the standing `x86_64-unknown-none` build.
- **`Reject` stays structural-only** — no new variant.
- **Behavior preservation (content-hash).** `segment.rs` `b3275467…` + `cell.rs` `8bf962e2…` + `display.rs`
  `a24b83a2…` byte-identical; **`dimensions.rs` captured at 4A as a new anchor** (resize reuses, must not
  edit). Additive edits (`message.rs`/`verb.rs`/`lib.rs`/`terminal`) legitimately rehash; a changed
  protected-surface hash is a **REJECT**.

## Decided Forks (lead rulings — engineering, co-formulated; dev + review concur)

1. **Mouse carriers.** `Button{None=0,Left,Middle,Right,Back,Forward, Other(ReservedButton u8≥6)}` and
   `MouseKind{Down=0,Up,Move,Scroll, Other(ReservedMouseKind u8≥4)}` — carry-raw enforce-by-construction
   (private inner, in-crate from-byte ctor, getter; `Other` can't hold a defined value), full-u8
   bijective. `Modifiers` — **a 6-bool struct** (reserved bits 6–15 unrepresentable by construction →
   canonical without masking, legible) over the masked-u16 newtype. `row`/`col` raw u16; `dx`/`dy` raw
   `i16` (full range incl. negatives — the sign-bit make-or-break). Mouse is a 12B fixed-only body.
2. **`focus` → `Focus{gained:u8}` raw** — not a Rust bool (a bool is non-canonical for `gained≥2`:
   `2`→`true`→re-encode `1`≠`2`, or needs value-rejection against the structural-only ruling) and not a
   carry-raw enum (overkill for one boolean-ish byte; every u8 is a valid `gained`, carried losslessly,
   consumer interprets `≠0`). (Dev + review concur.)
3. **`resize` → `Resize(Dimensions)`** reusing the 002 grammar via `Dimensions::decode_body`/`encode_body`
   (public) — no re-implementation; `dimensions.rs` untouched (new anchor). Realizes the §8 grammar-reuse.
4. **`viewport` → `Viewport{offset:u32}`** raw; **`paste` → `Paste(Vec<u8>)`** verbatim (not `String` —
   UTF-8 is the consumer's, law-11/structural-totality, like `ErrorBody.detail`).
5. **Module: one `terminal/input.rs`** (the carriers are small + mouse-only; no separate `mouse.rs`).
6. **No new `Reject` variant** — fixed-geometry + carry-raw + verbatim only; `Reject` structural-only.

## Escalations (operator-reserved — routed via the PM)

- **Verb-ids + `MouseKind` numbering — provisional.** model §8 numbers `Button` (None=0..Forward=5) but
  **not** `MouseKind` (Down/Up/Move/Scroll); the contract pins `MouseKind` provisionally (Down=0, Up=1,
  Move=2, Scroll=3) in the wire-numbers surface (`verb.rs`), like the cell op-set + error codes, pending
  operator ratification. The input verb-ids (0x13–0x17) likewise provisional; vectors re-blessed if a
  number moves.

## Review Questions

1. **Ratify the input verb-ids + the `MouseKind` numbering** (0–3) in the wire-numbers surface. (Operator.)
2. Confirm the 4A creation manifest + seam carve before any code. (Review, at the 4A pre-carve stop.) —
   **Confirmed; sprint accepted across 4A/4B/4C.**

---

## Postmortem — Sprint 004 retrospectives

*Sprint 004 closed 2026-06-24: all gates G0–G6 accepted by independent review across 4A/4B/4C (fuzz
reproduced reaching the input decoders via dispatch at cov 698, vectors re-derived byte-by-byte, the four
protected anchors recomputed byte-identical, the committed corpus reconciled first-time). The input
service is complete (resize/viewport/mouse/paste/focus); `key` 0x12 remains (keysym table). 17 of 22
verbs implemented. No new decoder class — the patterns recurred plus the signed-POD wrinkle. Builds on the
001/002/003 postmortems; durable lessons, not status.*

### Developer (`zero.proto-dev`)

**What moved the plan.** (1) *A new value-domain needs a gate over its EXTREMES — positives are a
subset-shadow.* `i16::from_le_bytes` takes every bit pattern, but a positive-only test never exercises the
sign bit; the gate round-trips `i16::MIN`(00 80)/`-1`(FF FF)/`MAX`(FF 7F) and the vector carries a
negative. (2) *One body can mix representation patterns; conflating any two is the defect* — mouse holds
carry-raw enum (Button/MouseKind) + reserved-drop bitfield (Modifiers) + signed POD (dx/dy), each with a
distinct failure mode (subset-shadow / non-canonical / over-rejection); name and assign each field's
pattern. (3) *A struct of named bools makes reserved bits unrepresentable BY THE TYPE* — Modifiers 6-bool
has no field for bits 6–15, so a non-canonical value can't be constructed (stronger than a masked-u16).
(4) *Corpus hygiene: cmin LAST + read-only replay → committed == on-disk first-time* (002/003 needed a
reconcile from post-cmin strays; 004 avoided it).

**What failed / surprised.** The 4B "remove `allow(dead_code)`" didn't apply (Mouse is a *pub* Wire impl
→ reachable, no allow; only `pub(crate)` free-fn decoders need it) — surfaced the correction. The
enumerated-N-vs-`==`-N mismatch recurred; the count-anchored `==` gate self-corrected. Pre-recording the
4C re-tighten (after 3B) kept the 4B↔4C seam tracked.

**005 inherits.** Input service complete bar `key` 0x12 (blocked); IMPLEMENTED 17/22. Remaining: `key`
0x12, `open-session` 0x00 (trivial empty body), net-response 0x31 / net-error 0x32 / **net-payload 0x33**.
The one big unbuilt mechanism is the **net-payload continuation-bit reassembler** (its own sprint; bounded
cap operator-reserved); net-response verbatim, net-error reuses `ErrorBody`. Carries: carry-raw
enforce-by-construction (now 6 carriers), reserved-bit-drop, signed POD, protected-anchor discipline,
IMPLEMENTED-anchored gates, live-path cascade, fuzz+replay-via-dispatch + cmin-last hygiene. All
ids/op-set/codes still provisional.

### Reviewer (`zero.proto-review`)

**What 004 added to the method.** (1) *The signed-POD sign bit is a subset-shadow of its own — the gate
must DEMAND a negative* (assert over `i16::MIN`/`-1`, and a vector with a negative delta); the code is
trivially bijective, but a positive-only *test* proves nothing about the sign bit. (2) *Three patterns in
one body — validate each field by its KIND against spec semantics, not by "it round-trips"*; the defects
are the cross-applications (carry-raw the Modifiers reserved bits → non-canonical; value-set-enum the
Button → over-rejection). (3) *Reserved-unrepresentable BY THE TYPE beats a masking discipline* (the
6-bool struct; ask "can a caller construct a value with reserved bits set?"). (4) *Corpus hygiene first —
cmin-last + read-only replay → on-disk == committed; a reconcile is a smell that an op wrote the corpus
after measurement.*

**005's reviewer inherits.** The four subset-shadow classes to demand coverage of (full type / undefined-
but-in-width / **the sign bit** / source-anchored bidirectional `==`); pattern-by-field discipline;
unrepresentable-by-the-type over masking; corpus-hygiene-first; the per-seam live-path cascade (now 4
sprints; a pub Wire impl needs no `allow` — the observable reject is the proof regardless). **The next
genuinely-new hard mechanism is net-payload reassembly:** the protocol's *one* trusted-side reassembly
(model §3 / net.md §2) — a bounded, fail-closed continuation-bit buffer the **proxy** holds, and the
**first decoder on a genuinely TRUSTED-side path where totality is TCB *defense***, not box-side
robustness. So its gate is stricter — **a payload that never ends or exceeds the cap must FAIL CLOSED**
(the make-or-break) — the **cap value is operator-reserved**, and the **L1 envelope / continuation-bit
framing** (deferred since 001) finally comes into scope.

### Lead (`zero.proto-lead`)

**Process lessons.** (1) *Fourth clean sprint on the established loop;* the 004 representation forks
(Modifiers 6-bool, focus raw u8) settled with the team — convergence again. (2) *Pre-record a recurring
seam pattern once learned.* After 3B's flagged coverage relaxation, I pre-recorded the 4C `==`-re-tighten
in the contract, so 4B's subset narrowing was tracked from the start (no surprise, self-correcting). (3)
*Turn a twice-hit operational stray into a standing pre-commit check.* After the 002/003 corpus reconciles,
I added a cmin-last + verify-on-disk==committed instruction to the 4C clearance; 004 committed clean
first-time. Operational lessons belong in the close discipline, not re-learned each sprint. (4) *The
pub-Wire-impl correction* (no `allow` to remove) — the dev surfaced it, I confirmed and routed to review;
the cascade's binding content (the observable reject) was never the `allow`.

**005 inherits (process).** The contract is the source of truth; the standing gates + the cmin-last
corpus hygiene + the subset-shadow classes carry forward; numbers stay provisional. **The net/reassembly
track is the major remaining work, and its reassembler cap value is the one operator-reserved item that
gates it** — surfaced to the operator via the PM in parallel. Net-response/net-error are buildable now;
net-payload + the reassembler (TCB-defense, fail-closed, the L1 envelope into scope) is materially
different from everything to date and warrants its own careful framing.
