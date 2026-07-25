# `scan_safe` — Total-Function Proof (the shared safe-alphabet scan)

### The floor both the leaf-validator and the address recognizer stand on

> **Total-function proof, not a mechanism spec** — proves the committed `scan_safe` is a
> total function over a finite, enumerable domain (spine §15–§16), the shared machinery the
> byte-map *pattern* survives as (the byte-map *name* retired with the raw-byte terminal
> filter — `display.md` §4/§6). Sprint `docs/teams/proxy/004-*.md`, Slice 1; status:
> **awaiting the SA1–SA7 gate.** This is a proof *of committed code*, not new authoring —
> the artifact is `proto/src/segment.rs::scan_safe` @ `a7ccd6c` (committed clean).
>
> **Budget:** this member opens the reviewability ledger (§8). Counted **once**;
> `leaf-validator.md` and `address-recognizer.md` reference it without re-counting.

---

## 0. What is proven, and against what (SA1)

`scan_safe(input: &[u8]) -> Result<SafeSegmentStack, Reject>` is the shared scan that both
tier-1 members reduce to: it takes a frame-bounded byte slice and either produces a verified
stack of safe segments or rejects. The requirement it must meet, from the committed sources
(`file.md` §0, `model.md` §6, quoted): the safe alphabet is **`[A-Za-z0-9._-]` with ≥1
alphanumeric per segment**, `0x1F`-delimited, and the scan **"verifies, it never transforms
— there is no normalization, no case-folding, no unescaping"** (`segment.rs` module doc).
Traversal (`.`, `..`, `/`) is **unrepresentable, not caught**.

The proof obligation (SA2): every possible `&[u8]` input maps to exactly one of
`Ok(stack)` or `Err(reject)`, with no unhandled case and no panic — a total function over a
finite, enumerable domain.

## 1. The domain partition — exhaustive over 256 byte values

Resolution proceeds at two levels; both are finite and fully enumerated.

### 1.1 The byte level — all 256 values, partitioned (SA2)

Within a segment (a `part` between delimiters), every byte falls in exactly one class. The
partition is total and the three classes are disjoint and exhaustive:

| Class | Byte values | Count | `match` arm → effect |
|---|---|---|---|
| **Alphanumeric** | `A`–`Z` (0x41–0x5A), `a`–`z` (0x61–0x7A), `0`–`9` (0x30–0x39) | 62 | `has_alnum = true`; byte kept verbatim |
| **Punctuation (in-alphabet, non-alnum)** | `.` (0x2E), `_` (0x5F), `-` (0x2D) | 3 | accepted; does *not* set `has_alnum` |
| **Out-of-alphabet** | every other byte | 190 | `Reject::BadSegmentByte { index, offset, byte }` |

62 + 3 + 190 = **255**. The 256th value is `0x1F` (US, the delimiter): it is consumed by
`input.split(|&b| b == US)` as the segment separator and **structurally cannot appear inside
a `part`** — so it never reaches the inner `match`. The `match`'s `_` arm would reject it
were it ever present, but the split guarantees it is not (a belt observation, not a reachable
path). 255 in-segment values + 1 delimiter = the full 256.

### 1.2 The structural level — over the split (SA2)

Above the byte classification, five structural outcomes partition every input, in the order
the code tests them (order matters — the first match wins):

| # | Condition | Outcome |
|---|---|---|
| 1 | `input` empty | `Err(EmptyStack)` |
| 2 | any `part` empty (leading / trailing / doubled `0x1F`) | `Err(EmptySegment { index })` |
| 3 | a `part` equal to `.` or `..` | `Err(TraversalSegment { index })` |
| 4 | a `part` with an out-of-alphabet byte | `Err(BadSegmentByte { … })` (from §1.1) |
| 5 | a `part` with no alphanumeric byte | `Err(SafeSegmentNotAlnum { index })` |
| — | otherwise (every `part` non-empty, in-alphabet, ≥1 alnum, none `.`/`..`) | `Ok(SafeSegmentStack)` |

These six outcomes are exhaustive: every `&[u8]` either is empty (1), splits into parts one
of which is empty (2), or splits into all-non-empty parts each of which resolves to exactly
one of (3)/(4)/(5)/accept by the byte partition of §1.1. There is no seventh case and no
input for which control reaches the end of a branch without a return or a decided value.

## 2. Totality — every input decided, no panic (SA2)

- **No unhandled case.** The `match byte` is exhaustive by construction — Rust requires it,
  and the three arms (§1.1) cover all `u8` values (the `_` arm makes it total). The outer
  structural tests (§1.2) end in a return or fall through to `Ok`. Every path terminates in
  `Ok` or `Err`.
- **No panic.** The only operations are slice `split`/`iter` (total over any slice),
  byte comparison, `Vec::push`, and `String::push(byte as char)`. None can panic on any
  `&[u8]`: `split` handles the empty and no-delimiter cases; indexing is via iterators, never
  a `[i]` that could exceed bounds; `String::with_capacity`/`push` cannot fail on ASCII.
- **Termination.** Two nested bounded `for` loops over a finite slice; no recursion, no
  unbounded loop. Terminates for every input.

Therefore `scan_safe` is a total function `&[u8] → SafeSegmentStack | Reject`. ∎

## 3. Verify-never-transform (SA6 — constructive)

`scan_safe` is *constructive* in the spine's sense: a dangerous byte is **unrepresentable in
the output type**, not scrubbed from it. Two facts establish it:

1. **No transformation.** The only byte-level operation on an accepted byte is
   `text.push(byte as char)` — an exact, lossless widening (every accepted byte is
   `< 0x80`, ASCII, so `as char` is identity on the codepoint value; grounded in §6). No
   case-fold, no unescape, no normalization exists in the path. The output bytes equal the
   input bytes of each accepted segment, exactly.
2. **The type carries the guarantee.** The result is `SafeSegment(Box<str>)` with a private
   interior and no public constructor other than this scan (`segment.rs` §0). Only
   `SafeSegment` exposes `as_str` (the loggable capability); a byte outside `[A-Za-z0-9._-]`
   *cannot be inside a `SafeSegment`*, so `/`, `0x1F`, control bytes, and `.`/`..` segments
   are structurally absent from any value the type can hold — traversal and injection are
   **unsayable, not caught** (`file.md` §0, `trusted-address.md` §1).

## 4. Adversarial closure (SA3)

Each attempt maps to the arm that denies it; all are on the committed code path.

| # | Adversary's attempt | Denied by | Arm |
|---|---|---|---|
| 1 | Traversal segment `..` / `.` | `Err(TraversalSegment)` — and independently, both lack an alphanumeric (would also trip §5) | §1.2 row 3 |
| 2 | Path separator `/` inside a segment | out-of-alphabet → `BadSegmentByte` | §1.1 row 3 |
| 3 | Embedded `0x1F` to forge a segment boundary | *is* the delimiter — splits, cannot appear in a `part`; a doubled/leading/trailing one yields an empty part → `EmptySegment` | §1.1 note; §1.2 row 2 |
| 4 | Control/ESC (`0x1B`), NUL (`0x00`), high-bit/UTF-8 bytes | out-of-alphabet → `BadSegmentByte` | §1.1 row 3 |
| 5 | All-punctuation segment (`...`, `-_-`) — sayable but no alnum | `Err(SafeSegmentNotAlnum)` | §1.2 row 5 |
| 6 | Empty input, or leading/trailing/doubled delimiter | `EmptyStack` / `EmptySegment` | §1.2 rows 1–2 |
| 7 | Case-shadow (`ABC` vs `abc`) reaching the output changed | impossible — verify-never-transform (§3); the bytes pass through identically, no fold | §3 |

Every adversarial byte or shape resolves to a `Reject`, and nothing outside the alphabet can
reside in the output type — closure is by construction, not by a filter that could miss one.

## 5. Residuals — named and bounded (SA4)

- **R-1 — `scan_unsafe` is a sibling, not covered here.** The unsafe scan (real names,
  display-only, never logged) is a *different* total function with a *different* alphabet
  (any byte but `/`, `0x1F`, NUL). It is out of scope for this member (the security core is
  the *safe* scan — the loggable one on the trusted-representation path). Named so no reader
  assumes this proof covers both; `scan_unsafe`'s own totality is its own obligation if ever
  a security-core member (it is not today — it fails safe, `filesystem.md` §3).
- **R-2 — Interior UTF-8 of an accepted segment is trivially valid** — the accepted alphabet
  is pure ASCII, so `Box<str>` construction cannot fail; no residual, stated to close the
  question a reader might raise about `into_boxed_str`.
- **R-3 — The proof binds to `a7ccd6c`.** Any change to `scan_safe` re-triggers this proof;
  the currency pin (§9) is the coupling.

## 6. Primitive-semantics grounding (SA5)

| Claim | Grounding |
|---|---|
| `match byte { … }` is exhaustive / total over `u8` | Rust language rule: a `match` without a catch-all covering all `u8` is a compile error; the `_` arm makes it total. Verified: the committed code compiles. |
| `slice::split` handles empty and delimiter-absent inputs without panic | `core::slice::split` contract: yields the whole slice as one part if no delimiter; yields empty parts around adjacent/edge delimiters. Never panics. |
| `byte as char` is a lossless, exact widening for accepted bytes | Every accepted byte ∈ `[A-Za-z0-9._-]` ⊂ ASCII (`< 0x80`); `u8 as char` on a value `< 0x80` yields the same Unicode scalar value — no `unsafe`, no `from_utf8` fallibility (`segment.rs` inline comment, verified). |
| `0x1F` cannot appear inside a `part` | `input.split(|&b| b == US)` consumes every `0x1F` as a separator; a `part` is a maximal run containing no `0x1F`. |

## 7. Judgment calls (SA7) — none

This member proves committed code; it introduces no new fork. The alphabet, the ≥1-alnum
rule, the reject arms, and the delimiter are all settled (`file.md` §0, `model.md` §6,
committed `segment.rs`). Nothing here is routed because nothing here is chosen.

## 8. Budget ledger (opens here — maintained across Slices 1–4)

Line counts recorded as **data points toward the whole set's standard: each piece in its
simplest reviewable form, and the whole set human-holdable in one sitting** (design's
ruling, 2026-07-19: the ~500 is a *forcing function for simplicity, not a quota to divide
or a cap to squeeze under* — contract Final State 5 / Method / Acceptance). No member is
measured against a per-slice or four-member numeric target; a count is a signal of
simplicity, never a pass/fail sum.

Counting convention: substantive source lines a reviewer reads to hold the function — the
`fn` body including control flow and `match` arms, excluding blank lines and comments.
`scan_safe` is counted **once**; downstream members reference it.

| Member | Slice | Lines | Counted | Simplest-form note |
|---|---|---|---|---|
| `scan_safe` (shared floor) | 1 | **35** | once | `segment.rs` @ `a7ccd6c`, lines 169–208: 40 physical − 5 comment − 0 blank = 35 net logic (counted on disk). Two nested bounded loops + an exhaustive `match`; no simpler form leaves the alphabet enumerable. |
| leaf-validator (array level) | 2 | *tbd* | + | excl. `scan_safe`; array-count + ≥1-element bounds only |
| address recognizer | 3 | *tbd* | + | excl. `scan_safe`; assembly + exact-match + scheme policy |
| Anthropic recognizer | 4 | *tbd* | + | incl. the bounded JSON reader **inside** this count — the piece leanness scrutiny concentrates on (design's ruling: the proxy's own code, most at risk of bloat) |

Rules carried to every slice: `scan_safe` counted once; the bounded JSON reader counts
*inside* the Anthropic number, so a general JSON library cannot hide as uncounted bulk —
the discipline that keeps the reader in its simplest deny-by-default form. Leanness effort
concentrates on the Anthropic recognizer (Slice 4), the proxy's own code and the piece most
at risk; tier-2 plumbing is out of this sprint's leanness scope. The count is **necessary,
not sufficient** — the real property is simplest-reviewable-form plus human-holdable-in-one-
sitting, confirmed as a whole-set property at Slice 4's gate; if the set cannot be held in
one sitting, that is a scope question for the PM, not a number to squeeze under.

## 9. Currency pins (2026-07-19)

`proto/src/segment.rs` @ `a7ccd6c` (committed clean; `scan_safe` lines 169–208) —
**still current, verified 2026-07-25**.

`proto/library.md` §8 (residence) and `file.md` §0 / `model.md` §6 (alphabet) are the
normative prose. **Re-pinned 2026-07-25:** all three were cited as working-tree state and
are now committed @ `c6c2b5e` (2026-07-22), **byte-identical** to when they were read
(sha-256 prefixes `c8cb29b87665e60e`, `5461602d24cf8b4e`, `4a468afae792e6e3`) — a status
upgrade, not a content change, so nothing this proof cites has moved.

The proof binds to the committed function; re-pin at the gate.
