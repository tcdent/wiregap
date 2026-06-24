# Sprint 001 — Codec Core Foundation

> The first sprint of the `proto` crate: the codec heart every consumer links. It builds the
> three decoders, the safe/unsafe segment type split, the message-dispatch geometry, and the
> self-validation loop — seeded with one message per decoder kind, proving the protocol round-trips
> and rejects the adversarial. No service catalog, no reassembly. This document is the contract:
> the single source of truth for what this sprint owns, what it does not, and what evidence closes it.
>
> Team: `proto` (lead `zero.proto-lead`, dev `zero.proto-dev`, review `zero.proto-review`).
> Grounded in [`library.md`](../../proto/library.md), [`model.md`](../../proto/model.md),
> [`file.md`](../../proto/file.md), [`display.md`](../../proto/display.md), [`net.md`](../../proto/net.md).

---

## Scope Boundary

**This sprint owns** the codec core — the parts of [`library.md`](../../proto/library.md) §2, §3, §6
that have no open operator blocker and that every later sprint links:

1. The greenfield crate skeleton (`proto`, edition 2024, std-only, zero deps, module layout by service).
2. The decoder vocabulary ([`model.md`](../../proto/model.md) §6): the fixed-width-record **primitive**
   (known-width prefix fields), the verbatim-tail decoder, and the segment-stack `0x1F` scan. The
   repeated-fixed-record **tail** (a run of records to the frame end) is deferred — see Non-Goals.
3. The segment stack and `SafeSegment`/`UnsafeSegment` type split ([`file.md`](../../proto/file.md) §0),
   including the **compile-time** logger-accepts-only-safe guarantee and structural traversal safety.
4. The message-dispatch geometry: read the verb `u32`, dispatch to its decoder, the body shape
   `[fixed prefix?][one frame-bounded tail]` (laws 1, 10, 11), and **totality** — any input yields a
   typed message or a typed rejection, never a panic, never a read past the frame.
5. The self-validation loop skeleton ([`library.md`](../../proto/library.md) §6): round-trip, adversarial
   fuzz, conformance-vector mechanism — seeded with the representative message set below.

**This sprint does NOT own** (named, with the sibling sprint that owns it):

- **The full 22-verb catalog** — only the dispatch *mechanism* plus the seed set lands here. Each
  remaining verb's message type is built in its **per-service sprint** (terminal/display, terminal/input,
  file, net). The thin service modules created here host only the seed and stubs.
- **The continuation-bit reassembler** — owned by a later **reassembly sprint**, gated on the cap value
  (operator-reserved). Body shape here is "one frame-bounded tail" → single-frame is the whole core.
- **Feature-gating** — owned by a later **packaging sprint**, gated on the granularity decision. No
  `[features]` in 001; modules are laid so gates drop in without restructure.
- **The L1 frame envelope** (session id + continuation flag) — owned by the reassembly/transport-seam
  sprint. 001 takes L2 `[verb][body]` over a single owned frame slice (see Method).
- **The `key` verb / input codec** — blocked: the `> 0x10FFFF` named-key integer table is unpinned
  ([`model.md`](../../proto/model.md) §11). Owned by the **terminal/input sprint**, after the table is
  assigned. `mouse` is fully pinned but is deferred with the rest of the catalog; it is not in the seed.

## Problem

The `proto` crate does not exist. The wire contract is fully specified across four normative docs, but
there is no Rust that any consumer (box, proxy, head) can link, and no proof the contract is sound. The
protocol is only safe if every party encodes and decodes it identically and parsing is total over
hostile input; nothing yet establishes either. Before any service catalog can be built, the codec heart
— the three decoders, the segment type split, the dispatch geometry, and the loop that proves itself —
must exist and be independently validated.

## Goal

A standalone `proto` crate that compiles `#![no_std]` + `alloc` with zero (non-dev) dependencies and
**proves its own correctness as a closed loop** over a representative seed of the catalog: every seed message round-trips
byte-identically, arbitrary bytes never panic or over-read, hand-authored golden vectors match both
directions, a real filename physically cannot reach a log line (a compile error), and traversal is
structurally unsayable. The heart is built and proven so the per-service sprints have a sound,
audited foundation to extend.

> The crate is the contract; the conformance vectors are its signature
> ([`library.md`](../../proto/library.md) §0). One crate makes "they speak the same protocol" a
> structural fact, not a hope.

## Final State

- `proto/` crate exists: edition 2024, **`#![no_std]` + `extern crate alloc`**, `#![forbid(unsafe_code)]`,
  **zero non-dev dependencies** (lib links only `core` + `alloc`); test-only dev-dependencies (fuzz,
  `trybuild`) permitted and may use `std`.
- The parsing vocabulary is bounds-checked with no box-supplied length: the **verbatim-tail** and
  **segment-stack scan** tail decoders are complete; the **fixed-width-record primitive**
  (`split_first_chunk::<N>` + `from_le_bytes` over a known-width prefix) is complete. The
  **repeated-fixed-record tail** decoder is deferred — no seed verb uses it (see Non-Goals).
- `SafeSegment` and `UnsafeSegment` are distinct types; the only `Unsafe → Safe` path re-runs the safe
  scan; the logger's signature accepts only `SafeSegment`; feeding it an `UnsafeSegment` **fails to
  compile**.
- A single total verb-`u32` match dispatches to the right decoder; an unknown verb is a typed
  `Reject::UnsupportedVerb` (law 5), not a panic.
- The seed messages (below) each have an owned Rust type implementing the `Wire` trait, round-trip, and
  carry a hand-authored golden vector.
- The self-validation harness runs under `cargo test` and emits the named gate evidence (G0–G5).

## In Scope — ordered slices (review seams)

Lands as one sprint, carved as three reviewable seams. The developer produces the **creation manifest**
(read-only seam map, shape under Starting Surface) and **stops for review before any carve**.

- **1A — Segment primitive + type split + logger bound.** The `0x1F` verifying scan; the safe alphabet
  `[A-Za-z0-9._-]` (≥1 alphanumeric) and the unsafe structural rule (any byte but `/`, `0x1F`, NUL;
  never `.`/`..`); `SafeSegment`/`UnsafeSegment` as distinct types; the logger bound; the `trybuild`
  compile-fail test. *Method:* the scan verifies, never transforms; traversal is structural.
  *Gates:* G4, G5.
- **1B — The decoder vocabulary + geometry + dispatch + seed verbs.** The fixed-width-record
  primitive, the verbatim-tail decoder (segment-stack scan landed in 1A); the `[fixed prefix?][one
  frame-bounded tail]` shape; all 22 verb-id consts + the total verb-`u32` match yielding the crate-wide
  `Message` enum; owned field-by-field decode with validation at every non-POD field; the 5 seed message
  types. Per Fork #8: **carry-raw** enum reserved value-ranges, **drop** spare reserved bits (decode to
  the defined structure). *Method:* `split_first_chunk` + `from_le_bytes`; no panic-indexing, no
  `split_at`, no `unwrap`/`expect` on input-derived values. *Gates:* **G1** (round-trip over the seed)
  and **G2's source-discipline half** (bounded accessors, no-panic-index, frame-bounded alloc).
- **1C — flag-1 correction + wire-numbers review surface + self-validation harness + hand-authored seed
  vectors.** First, the Fork #8(i) private-newtype correction (so vectors author against a type with no
  non-canonical reps). A **single, orderly location** laying out all verb ids + error codes (ordered,
  commented, at-a-glance reviewable) so the operator can decide changes before ratification (operator
  requirement; must land before sprint close; reconcile with module ownership — services reference the
  one numbering surface, do not fragment it). Then:
  round-trip property over the seed catalog with an explicit **`SEED` constant + `assert_eq!(covered,
  SEED.len())`** coverage anchor (review's minor — count anchored to source); the **kitted adversarial
  fuzz** (cargo-fuzz, ASan+UBSan, corpus seeded from the G3 vectors + generated messages, alloc cap,
  coverage figure) — **G2's full dynamic half consolidates here** because its corpus seeds from the G3
  vectors; the conformance-vector mechanism with **hand-authored, frozen, non-circular** golden vectors
  (incl. the `0xFF` non-UTF-8 `detail` vector). *Gates:* **G2 (full)**, **G3**, and **G0** over the whole
  run; the `x86_64-unknown-none` target build wired as a standing G6.d guard (review's durability rec).

**Seed message set** (≥1 per decoder kind; exercises both segment alphabets and the prefix+tail shape):

| decoder kind | seed verb | why |
|---|---|---|
| fixed-width-record primitive | `pane-state` (`[u32][u32][u32]`) | the one genuinely all-integer POD body; exercises the `split_first_chunk`+`from_le_bytes` primitive — a fixed **prefix**, *not* a repeated-record tail (see Non-Goals) |
| verbatim tail | `file-chunk` | pure verbatim, simplest tail |
| segment stack (safe) | `net-address` | exercises the **safe** alphabet + the logger bound |
| segment stack (unsafe) | `file-read` | exercises the **unsafe** alphabet + the type split |
| fixed prefix + verbatim tail | error body (`[code u32][detail tail]`) | the law-10 `[prefix?][tail]` shape directly |

`key` is **excluded** (unpinned keysym table; deferred-blocked). The other non-seed verbs (17 of the 22,
`mouse` included) are stubbed or deferred to their service sprints.

## Non-Goals

- The full verb catalog message types beyond the seed (each lands in its service sprint).
- **The repeated-fixed-record tail decoder** — a run of fixed records to the frame end, with a
  frame-determined record count and partial-trailing-record rejection (`frame_len % N != 0` → reject).
  No seed verb has a repeated-record tail, so it is built, gated (G1 field-edges incl. the partial-tail
  reject + G2 totality), and proven against a real layout in the **first service sprint that lands such
  a verb** — display (cell stream) or file (`list-entry` records) — not claimed here. 001 builds only
  the fixed-record *primitive* the prefix uses.
- The continuation-bit reassembler, the reassembly buffer, the cap (later sprint; cap value
  operator-reserved).
- Feature-gating, `no_std`, the L1 envelope.
- The `key`/input codec until the named-key table is pinned.
- **Do not delete a fallback just because production does not hit it — trace callers and tests first.**
  (Greenfield: nothing to delete yet, but the discipline holds as the catalog fills.)
- **Do not generate golden vectors by blessing `encode()` output** — the encoder is the thing under
  test; a self-blessed vector is a tautology that proves nothing (see Method / G3).

## Starting Surface

Greenfield: there are no extraction sites, so the shadow→prove→switch *extraction* inventory does not
apply. The equivalent pre-edit artifact is a **creation manifest** the developer produces **before
writing any code**, covering the full catalog so every verb's eventual home and this sprint's action on
it are explicit:

```
verb id | verb name | direction | body shape ([prefix?]/tail kind) | decoder kind | Rust type | module | 001 action (seed | stub | defer)
```

The developer hands the manifest up as the read-only seam map; review confirms it and the seam carve
**before any code is carved**.

## Method

The discipline every seam follows:

- **Owned, field-by-field decode** is the uniform path: `split_first_chunk::<N>` + `from_le_bytes`,
  validating every non-POD field (enum/bool/char/constrained `u32`). **No whole-struct overlay/transmute.**
  This is the ratified reading of [`model.md`](../../proto/model.md) §5 — see Decided Forks #2.
- **Totality is the security property.** Only bounds-checked accessors (`get`, `split_first_chunk`,
  `split_at_checked`); never panic-indexing, `split_at`, or `unwrap`/`expect` on input-derived values.
  Parsing any byte string yields `Ok(Message)` or `Err(Reject)` — never a panic, over-read, or
  input-controlled allocation.
- **The scan verifies, never transforms.** A segment byte is checked against its alphabet or the message
  is rejected; `0x1F` ends a segment; the frame ends the stack. Traversal (`/`, `..`) is unrepresentable
  by construction, not caught.
- **Build coherence (G0).** Every gate's coverage count is `assert_eq!`'d against a source-derived
  constant (e.g. `CATALOG.len()`), so a stale binary or a hand-typed count is detectable. The validated
  run comes from a post-build `cargo test` of the committed tree.
- **Non-circular vectors.** Golden vectors are authored **by hand from the spec layout**, committed, and
  frozen — never produced by running `encode()`. The encoder cannot be both subject and oracle.
- **Stop-for-review at each seam.** The seams (1A/1B/1C) are checkpoints, each a true fork (a new proof
  obligation — the compile-time guarantee, the totality property, the vector non-circularity). The
  developer stops for the corresponding gate check at each; full independent acceptance is at the
  sprint boundary.

## Acceptance

The gates that close the sprint, adopted from `zero.proto-review`'s gate spec. Re-derived by review from
current source and a settled post-build run, never from the developer's report.

- **G0 — Build coherence.** The run log validated comes from a post-build `cargo test` of the committed
  tree; coverage counts are `assert_eq!`'d against source-derived constants. A green from a binary not
  tied to current source = REJECT.
- **G1 — Round-trip (message-anchored, seed-complete).**
  - *coverage:* every seed verb appears ≥1; `assert_eq!(covered, SEED.len())`. `mismatch=0 ∧ checked=0`
    is a REJECT.
  - *field-edges:* per fixed field min/0/max; per tail empty/1B/max-frame; per segment
    stack 0/1/N segments + alphabet-boundary bytes, **including the typed-reject cases** — empty stack,
    empty segment, and a safe segment with 0 alphanumerics ([`file.md`](../../proto/file.md) §0 requires
    ≥1) must each be a typed `Err(reject)`, not an accept.
  - *reserved fields:* a **reserved-nonzero** fixed field **structurally parses (Ok)** — the library does
    not validate reserved/spare values (operator ruling; see Decided Forks #8). It is a totality case
    (G2), not a G1 reject expectation. (Segment-alphabet violations above are structural and still reject.)
  - *eq:* `decode(encode(m)) == m` over all cases on the **full field set**; mismatch=0.
  - *canonical:* `encode(decode(encode(m)))` byte-identical; mismatch=0.
- **G2 — Totality under fuzz (byte-anchored).** `decode(arbitrary) → Ok|Err`, never panic/over-read/
  input-controlled alloc. Credible only with **all** of: cargo-fuzz/libFuzzer; ASan+UBSan,
  `overflow-checks=on`, `debug-assertions=on`; a corpus seeded from the G3 vectors + valid generated
  messages, coverage-guided and committed; a named exec/wall budget with **0 crashes, 0 timeouts, 0 OOM**
  AND a branch-coverage figure for the codec module; a test global allocator capping any single
  allocation at `frame_size * k`, asserted never exceeded; and the source audit (no panic-indexing/
  `split_at`/`unwrap`/`expect` on input). A bare "ran fuzz, no crash" with no coverage and no alloc cap
  = REJECT.
- **G3 — Conformance vectors (external, non-circular).** ≥1 hand-authored golden vector per decoder kind,
  naming its verbs, committed and frozen; `decode(golden) == expected_msg` AND `encode(expected_msg) ==
  golden` per vector. Encoder-blessed vectors = REJECT. The **error-body vector carries a non-UTF-8
  `detail` byte** (e.g. `0xFF`) — it decodes/encodes fine as `Vec<u8>` and is a permanent regression
  guard locking the verbatim-ness (a later change to `String` would break it). See Decided Forks #7.
- **G4 — Logger-only-safe is a compile error (type-level capability).** The lib performs no logging (an
  effect would itself break library §0); it defines a **safe-to-log capability** only `SafeSegment`
  carries, which a consumer's logger requires. Binding evidence:
  - *negative:* a `tests/ui/` `trybuild` case feeds `UnsafeSegment` to the **exact** capability bound a
    real consumer's logger uses (not a strawman) and **fails to compile** against a pinned error.
  - *sealing:* if the capability is a trait it **must be sealed** (unimplementable outside the crate), so
    a consumer cannot `impl` it for `UnsafeSegment` and bypass the guarantee; if it is an inherent
    accessor on `SafeSegment`, the trybuild must feed `UnsafeSegment` to a fn requiring that accessor.
  - *positive:* a normal compiling test exercises the capability on a `SafeSegment`, so the negative is
    not vacuous (a bug where *neither* type carries it would otherwise pass).
  - *audit:* no `From<UnsafeSegment> for SafeSegment`, no unchecked constructor/accessor, no public inner
    field, no `transmute`; the only `Unsafe → Safe` path re-runs the safe scan. A runtime test is
    insufficient by construction.
- **G5 — Traversal safety (structural).** The scan rejects `/`, `0x1F`, NUL, and the `.`/`..` segments;
  the receiver joins under its own root. Boundary bytes and the `.`/`..` rejects tested explicitly.
- **G6 — Auditable surface (no_std + alloc + no unsafe).** The lib's API surface is structurally minimal,
  proven by build, not inference:
  - *G6.a* lib crate root is `#![no_std]` + `extern crate alloc`; the lib builds `no_std`.
  - *G6.b* zero non-dev deps: `[dependencies]` empty; `cargo tree` shows the lib links only `core` +
    `alloc` (test crates are `[dev-dependencies]` only).
  - *G6.c* `#![forbid(unsafe_code)]` on the lib — forecloses transmute-overlay (Fork #2) and the unsafe
    pointer arithmetic an over-read would need (library §0: safe Rust end to end).
  - *G6.d* **evidence is a lib-only `no_std` build** (`cargo build -p proto` / a dedicated `no_std` check
    target), **not** inferred from the `std` `cargo test` run — `cargo test` builds the workspace as
    `std`, so a `std`-reach in the *lib* would be masked. The audited surface is the library target.
  - G6 (bounds *what* API exists) and G2.alloc (bounds *how* the one growable part behaves under hostile
    input) together discharge the operator's minimize-surface lens.

> Verb ids in vectors are **provisional** until the §8 number sets are ratified (Decided Forks #4 /
> Escalations). If a ratified number changes, the affected vectors are re-authored — they gate only 1C.

## Decided Forks (lead rulings — engineering, grounded in the docs + team consensus)

1. **Message representation** — owned structs at the decode edge, built field-by-field, validating every
   non-POD field. Each message is a struct in its service module implementing a small `Wire` trait
   (`const VERB: u32`, `encode`, `decode`); one total verb-`u32` match yields the crate-wide `Message`
   enum; per-service decoders stay directly callable. Not a single flat enum as the primary type (it
   fights the open feature-gating decision); the two-level organization is Rust code structure, not a
   second wire layer (law 1 intact). *(Dev + review concur.)*
2. **`model.md` §5 overlay reading — RATIFIED owned-copy.** §5 *permits* (“can overlay”) but its own
   rationale caps overlay at all-integer POD; §4 (not throughput-bound, stated twice) removes overlay's
   only motivation; §6's `split_first_chunk`+`from_le_bytes` vocabulary *is* "read fields in place" as
   safe per-field copies. Owned field-by-field copy is the uniform path; whole-struct
   overlay/transmute is **not** built. Overlay-as-transmute on a pure-POD body (e.g. `pane-state`) is a
   possible later optimization, explicitly out of 001. Verbatim tails are **owned (copied)** in 001;
   zero-copy tail-borrowing is a deferred, isolated optimization.
3. **Conformance-vector method** — hand-authored from the spec layout, committed, frozen, non-circular.
   The developer authors from the spec; review independently re-derives a sample to confirm
   non-circularity at acceptance.
4. **Envelope ownership** — the crate owns **L2 `[verb][body]`** over a single owned frame slice
   (`&[u8]`); the slice length is the frame boundary and the totality anchor. The L1 envelope (session
   id + continuation flag) is deferred and will wrap L2 without disturbing it. *(Dev + review concur.)*
5. **Crate basics** — name `proto`, edition 2024, **`#![no_std]` + `extern crate alloc`**,
   **`#![forbid(unsafe_code)]`**, zero non-dev deps, **no `[features]`** in 001. `no_std` is the team
   ruling under the operator's *minimize-auditable-surface* lens (dev build-confirmed, review concurs):
   it makes library §0's no-effect boundary (laws 5–7) a **compile error** rather than a re-proved
   discipline — `std::fs/net/process/thread/time/env` cannot link. `alloc` is retained (not pure
   `no_std`) because Fork #2's owned-copy needs it; this is sound *because* law 11 makes every allocation
   frame-bounded (not input-count-controlled) and G2.alloc proves the cap. Gated as **G6**.
6. **Seed set** — the table under In Scope (dev's proposal; satisfies review's per-kind + both-alphabet
   constraints).
7. **Decode-edge representation refinements** (review-confirmed). (a) `ErrorBody.detail` is **`Vec<u8>`
   verbatim, not a validated `String`** — *required* by law 11 + model §8 (detail is a verbatim tail
   rendered inertly; validating UTF-8 would incur a wire obligation and make the error body non-total,
   rejecting a malformed detail that should still decode and relay). (b) `Reject` derives `Debug` +
   `Display` only — **no `core::error::Error` impl** (smaller surface; additive, non-breaking later) —
   and stays a small value-type: variants may carry a bad byte/offset for diagnostics but **no owned
   copies of input payloads**. (c) The logger guarantee is the **type-level sealed capability** of G4.
8. **Reserved-bit policy — the library makes no ruling (operator).** The library does not validate
   reserved or spare field *values*; totality is structural only (verb dispatch, body shape, segment
   alphabet). Reserved-bit policy is the consumer's. This supersedes the tolerant/strict/split debate.
   A reserved/spare-nonzero fixed field **structurally parses → Ok for every verb, uniformly** (a G2
   totality case, never a typed reject).
   - **Boundary (do not over-read the ruling):** *length/shape stays policed.* Extra bytes after a no-tail
     body (e.g. `pane-state`) → `TrailingBytes`; a truncated body → `ShortBody`. Only reserved-bit
     *values within a structurally-valid body* are carried, not judged.
   - **Coherence tell:** under structural-only totality every `Reject` variant is a *structural* failure —
     `{UnsupportedVerb, ShortBody, TrailingBytes, BadSegmentByte, EmptySegment, TraversalSegment}`. No
     value-policy variant survives (the manifest's `BadScheme`/`BadErrorCode` are dropped).
   - *Resolved for 1B (lead ruling, both halves):*
     (i) **enum reserved value-ranges** (`Scheme` 1+, `ErrorCode` 7+, later `type_flags`/`Button`/`kind`)
     → **carry-raw, enforced by construction.** Soundness forbids transmuting an out-of-range discriminant
     (model §5), so represent by match-and-construct — `Scheme { Https, Other(ReservedScheme) }`,
     `ErrorCode { …, Unknown(ReservedCode) }` — where `ReservedScheme`/`ReservedCode` are **private
     newtypes whose only constructor is the in-crate `from_u32`** (rejecting in-range/known values), with
     a public getter for consumers. Decode matches known constants else carries the raw `u32`; encode
     writes it back. Rejecting would be the library making value policy (forbidden by the operator ruling);
     carry-raw surfaces the unknown to the actor as an explicit match arm and is forward-compatible.
     *(Reverses the manifest's `BadScheme`/`BadErrorCode` — `Reject` stays structural-only.)* The
     **private-newtype enforcement is required** (review's 1B G1 ruling): a publicly-constructible
     `Other(u32)`/`Unknown(u32)` admits a non-canonical `m` (e.g. `Unknown(3)`, an in-range value) for
     which `decode(encode(m)) != m`, so G1 would hold only over the decode-image subspace, not the full
     type. The newtype makes the non-canonical value unrepresentable — structural-over-documented, the
     crate's thesis. Lands as the first 1C task (no wire/dispatch/vector ripple); review re-verifies at the
     1C seam.
     (ii) **reserved bits** (pure spare bits/bytes — verb upper-3-bytes, `input_flags` reserved bits,
     etc.) → **drop** (decode to the defined structure; not stored; re-encode emits zero). Chosen over
     the carry/preserve lean on the operator's **minimize-surface** lens: drop satisfies every defined
     gate (G1.canonical holds message-anchored; arbitrary reserved-set bytes are a G2 `Ok`),
     byte-identical-over-reserved is **not** a required property (relay is envelope passthrough, not
     decode/re-encode; no actor re-emits a foreign message), a consumer wanting a reserved-bit policy
     still holds the raw frame, and carrying is additive/non-breaking if a concrete need appears. It is
     also the natural, minimal decode. The principle: a spare *bit* has no meaning (drop); a *field's
     value* always has meaning even in a reserved range (carry). *(Review tested DROP against every
     defined gate and the relay/terminal-sink architecture, found no violation — relay is envelope
     passthrough so a relayed body's reserved bits pass verbatim, and every decoder is a terminal sink so
     no dropped bit is lost downstream — and conceded the carry lean 2026-06-23.)*
   - **Dispatch consequence (load-bearing, binding 1B check):** the verb `u32` is `[id low byte][3
     reserved bytes]`. Dispatch keys on `verb & 0xFF` and **drops the upper 3 bytes regardless of value**;
     a *known* id with nonzero reserved upper bytes **still dispatches** to that verb (re-encoded with
     zero upper bytes). `UnsupportedVerb` keys **only** on an unknown low-byte id. A strict full-`u32`
     match that rejected a reserved-nonzero *known* verb would be a **G2 totality violation** (a
     structurally-valid message wrongly rejected) — so DROP-on-dispatch is required, not optional.

## Escalations (operator-reserved — routed to the PM)

- **Verb-id + error-code ratification** ([`model.md`](../../proto/model.md) §11). **Stay provisional**
  (operator, 2026-06-23) — do not ratify yet. 1C delivers a **single, orderly location** in the codebase
  laying out all verb ids + error codes so the operator can review them at a glance and decide changes
  before ratification; it **must be in place before the sprint closes**. Vectors are authored provisional
  against the drafted §8 numbers and re-blessed if a number moves. Non-blocking.
- **`no_std`** ([`library.md`](../../proto/library.md) §8) — **RESOLVED.** Operator delegated to the team
  under the minimize-auditable-surface lens; team ruling is `#![no_std]` + `alloc` + `forbid(unsafe_code)`
  (Decided Fork #5, gated as G6).
- **Reserved-bit decode policy — RESOLVED (operator), supersedes the split/tolerant/strict debate.**
  The library **does not validate reserved or spare fields** — reserved bits are not a library concern.
  Totality is a *structural* guarantee (verb dispatch, body shape, segment-alphabet correctness), **not**
  the values of reserved/spare fields. The library parses structure and hands typed messages up;
  consumers own any reserved-bit policy. No reserved-bit posture in the decode contract. (model.md §5 may
  be clarified by the spec owner, but gains no library reserved-bit posture.) See Decided Fork #8.
- **Spec defect — §8 verb count — RESOLVED** (designer). 22 is correct; the doc is corrected and the id
  **table is the catalog of record** — `CATALOG` anchors to **22**. By law 1 every id is a verb, `list-entry`
  (0x25) included; **no special-casing** — all 22 ids get codecs (in 001, the 5 seed; the rest stub/defer).

### §8 open-decision classification (the PM's explicit ask)

| §8 decision | classification | disposition |
|---|---|---|
| Message representation | team | decided (Forks #1/#2) |
| Envelope ownership | team | decided (Fork #4: L2-only) |
| Grammar reuse | team | defer — relevant when service modules land |
| Feature-gating granularity | team | defer — no features in 001 |
| Reassembler residence | team | defer — reassembly is a later sprint |
| Reassembler **cap value** | operator | later sprint (net.md §2 mandates bounded; number unset) |
| `no_std` | operator → team | **resolved:** `no_std` + `alloc` + `forbid(unsafe_code)` (Fork #5, G6) |
| Named-key integer table | operator | blocks the terminal/input sprint, not 001 |

## Review Questions

For the operator — open before **1B** (none block the 1A carve):

1. **Ratify the §8 verb-id + error-code number sets** so conformance vectors are not provisional. (Operator.)
2. **Confirm the enum-reserved-value-range representation** (1B fork, Decided Fork #8): defined enum
   fields with reserved value-ranges (`Scheme` 1+, `ErrorCode` 7+) — carry the raw value (working lean,
   consistent with the operator's structural-only ruling) vs reject. To settle at 1B framing. (Operator/team.)

Settled (no longer open): the **§8 verb count** (22, table is canon, all ids are verbs — designer);
the **reserved-bit policy** (operator: the library makes no ruling — structural totality only, consumers
own reserved-bit policy — Decided Fork #8); `no_std`+`alloc`+`forbid(unsafe_code)` (Fork #5/G6); the
acceptance gates G0–G6 match review's spec; the creation manifest / seam carve is review-confirmed.

---

## Postmortem — Sprint 001 retrospectives

*Sprint 001 closed 2026-06-23: all gates G0–G6 accepted by independent review across seams 1A/1B/1C
(incl. an independent fuzz reproduction and byte-by-byte vector re-derivation). Numbers remain
provisional pending operator ratification. Role-scoped, durable lessons — not status.*

### Developer (`zero.proto-dev`)

**What moved the plan.** (1) *Wire-format ambiguity has an oracle — the canonical-encoding
requirement.* §6 separator-vs-terminator was decided by G1 canonical re-encode (only separator gives one
encoding per stack), not taste — surface ambiguity *with* its criterion, don't default it. (2) *"For
every message" means unrepresentable, not documented.* Carry-raw-as-public-enum passed G1 only over the
decode-image; a hand-built `Unknown(3)` breaks round-trip. Close the gap in the type system (private
newtype), not a doc comment. (3) *Totality is structural-only; rejecting a spare value is itself the
bug.* Dispatch masks the low byte and drops reserved upper bytes — a strict full-`u32` match would be a
G2 totality violation and a denial lever. Validate only what the trusted side acts on; carry-raw enum
ranges, drop spare bits; `Reject` has no value-policy variant. (4) *A structural constraint beats a
re-proved discipline.* `no_std`+`forbid` makes laws 5–7 a compile error; binding evidence is the
lib-only/bare-metal build, not `cargo test` (std, masks a lib std-reach).

**What failed / surprised.** The carry-raw public-enum miss was the one real defect — caught at the 1B
seam, *not* by me (the stop-for-review cadence earned its keep). `cargo-fuzz` 0.13's coverage binary
isn't where `llvm-cov` reads it; the durable workaround is replaying the committed corpus through a
`-Cinstrument-coverage` example.

**002 inherits.** Foundation done (3 decoders, type split, dispatch, `Wire`/`Message`, self-validation):
add a verb = `impl Wire` + dispatch arm + vector; keep the std/unsafe fuzz crate separate from the
no_std/forbid lib. **Deferred + must-build-when-needed:** the repeated-record to-frame-end tail
(display/list-entry), gated on partial-tail reject + G2. Settled: reserved-bit policy, standing
bare-metal check, G4/G6 gates. `verb.rs` is the single numbers edit-point; **numbers still provisional —
confirm §8 ratification before freezing more vectors.** Open: `key` 0x12 (keysym table), L1 envelope +
reassembler + operator-reserved cap, feature-gating granularity.

### Reviewer (`zero.proto-review`)

**What acceptance caught, and the transferable lesson.** (1) *The subset-shadow.* G1 was green but the
harness exercised only the decode-image while the type publicly permitted a non-canonical message — the
green proved less than the gate claimed. For any "for every m" gate, ask what the *set* of m is: does
the type permit an m the harness can't construct? Drove the fix to enforce-by-construction. (2)
*Non-circular vectors.* A vector produced by `encode()` is a tautology; re-derive a sample byte-by-byte
from the *authority* (the spec), not the code. (3) *The fuzzer-must-reach-the-code bar.* "0 crashes"
proves nothing at near-zero coverage; require a coverage figure proving entry into the decoders, and make
a structural property (the alloc cap) a checked invariant — a clean run *is* the proof. (4) *Behavior
preservation via artifact identity.* The decisive proof the 1A scan surface was untouched was its
**test-binary hash** byte-identical across seams — pin coherence to an artifact, never a visual diff
read.

**Load-bearing evidence standards.** The settled run must be *yours*, scoped to the change's own binary.
The no_std proof is the lib-only/bare-metal build, not `cargo test`. Structural-over-documented: gate on
the compile-error/type/build that makes a violation unrepresentable. Geometry checked at the boundary
(dispatch on `verb & 0xFF`; `TrailingBytes` only on a pure-fixed body), not by eye.

**Fork method.** Route forks with a recommended default; don't resolve them; distinguish a *gate
violation* from a *preference*. When the operator ruled reserved-bits=DROP against my carry lean, I
tested DROP against every gate + the relay/terminal-sink architecture, found no violation, and conceded.
Standing is in being right *after* checking, not in never being overruled — retract the moment the data
turns.

**002 inherits.** *Standing gates:* the bare-metal no_std build each seam; the coverage-figure +
alloc-cap G2 bar; the subset-shadow question on every "for every m" gate; vectors re-derived from spec.
*Settled invariants to hold new code to:* reserved bits DROP; `Reject` structural-only; enum reserved
value-ranges carry-raw + enforce-by-construction (private newtype, decode-only, `.get()`) — hold every
new carrier (`KeySym`, `type_flags`, `Button`) to that pattern. **Biggest carried-forward unproven
obligation:** the repeated-fixed-record tail decoder (frame-determined count + partial-trailing reject)
is *deferred, not built* — a green on the 5 seed kinds does NOT cover it; 002 must build and gate it
(G1 field-edges incl. partial-tail reject + G2 + a hand-authored vector). *One line:* a green is a
hypothesis; the highest-value catches came from asking what the green did **not** exercise.

### Lead (`zero.proto-lead`)

**Process lessons.** (1) *Form the sprint as a team.* Co-formulating with dev + review before promoting
to the PM surfaced the full G0–G6 acceptance gate and the representation forks *up front*, so they
weren't discovered mid-build. (2) *Concede the operator's frame-punctures fast.* Twice the operator
reframed a team debate that was in the wrong frame — `no_std` → the minimize-auditable-surface lens;
reserved-bits → the library-makes-no-ruling (structural-only). Drop the old frame, don't defend it. (3)
*On a fast-moving fork, flag relays as provisional.* The designer's reserved-bit *split* and my relay of
it crossed the operator's superseding ruling, briefly stranding careful work by both teammates — when an
upstream decision is in flight, tell teammates a relay may be superseded so they don't over-invest. (4)
*Preserve reviewer independence.* Never pre-bless a verdict (separator, flag-1) — route forks for
independent re-derivation; that independence is what caught the subset-shadow. (5) *Gate forks before
routing a slice, and manufacture an anchor when the operator needs to decide later* (the `verb.rs`
wire-numbers surface turned a future ratification into a one-location edit).

**002 inherits (process).** The contract is the source of truth; the standing gates (G6.d bare-metal,
G2 coverage + alloc-cap) carry forward; numbers stay provisional until the operator ratifies via the
wire-numbers surface; and the deferred repeated-record-tail decoder is the headline unbuilt obligation
for the first service sprint that lands such a verb.
