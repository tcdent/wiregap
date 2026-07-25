# The Leaf-Validator — Total-Function Proof (the write-path textual floor)

### The array-level total function that confines what the box can *say*

> **Total-function proof, not a mechanism spec** — proves the filesystem write-path
> leaf-validator is a total function over the leaf-array domain (spine §15–§16). Sprint
> `docs/teams/proxy/004-*.md`, Slice 2; status: **awaiting the SA1–SA7 gate.**
> **Consolidation, not re-derivation:** the per-leaf scan is `scan-safe.md` (Slice 1); the
> confine-write mechanism is `filesystem-write-path.md` (Sprint 2, accepted) — both cited,
> neither reopened. This member adds only the array-level enumeration proof.
>
> **Budget:** the array-level logic is counted here; `scan_safe` is **not re-counted**
> (Slice 1, once). Ledger updated in §8.

---

## 0. What is proven, the seam, and against what (SA1)

The leaf-validator is the **textual floor** of the write path: it confines *what the box can
say* as a destination — the first of the two structural floors (`filesystem.md` §1; the
second, the filesystem-level open-discipline floor confining *where bytes land*, is
`filesystem-write-path.md` §1, a distinct total function proven there and cited here, not
re-derived).

The seam is clean and worth stating exactly: the leaf-validator **produces** the validated
element array `E = [e1 … en]` (n ≥ 1) that `filesystem-write-path.md` §1.2 **consumes** as
its input. This spec proves the production is total; that spec proves the consumption is
total. Composed, the write path is total end to end, with no gap at the seam.

The requirement (SA1), from `filesystem.md` §1, quoted: the destination is **"not a path
string — it is an array of leaves. The proxy supplies every separator and the quarantine
root; the box supplies only leaf names."** Each leaf is valid iff **"every character is in
`[A-Za-z0-9._-]` and it contains at least one alphanumeric"**; and the proxy **"owns a sane
element-count bound and a require-at-least-one-element rule — frame well-formedness, not
intent-reasoning."**

## 1. The total function

```
leaf_validate(dest: &[u8]) -> Admit(SafeSegmentStack) | Deny(reason)

  match scan_safe(dest):                       # Slice 1 — per-leaf + structural
      Err(r)                       => Deny(r)
      Ok(stack) if stack.len() > MAX_ELEMENTS  => Deny(TooManyElements)   # the one new arm
      Ok(stack)                    => Admit(stack)
```

- **Input domain:** every finite `&[u8]` the box supplies as a destination — the same domain
  `scan_safe` is total over (Slice 1).
- **Output:** exactly one of `Admit(stack)` or `Deny(reason)` for every input.
- The **separators and the quarantine root are proxy-supplied**, never in `dest` — the box
  contributes only leaf-name bytes (`filesystem.md` §1). So `dest` is a `0x1F`-delimited
  safe segment stack and nothing else; `/` is not even representable in it (out of alphabet,
  Slice 1 §1.1).

## 2. Totality — composition of two total functions (SA2)

The composition is total because each step is total and the branches are exhaustive:

1. **`scan_safe` is total** (`scan-safe.md`, proven): every `&[u8]` → `Ok(stack)` or
   `Err(reject)`, no panic, terminates. Cited, not re-derived.
2. **The count guard is a total comparison**: `stack.len()` is a `usize` (always defined for
   a constructed stack), `> MAX_ELEMENTS` is total over `usize`. The two arms (`>` and `≤`)
   are exhaustive and disjoint.
3. **The `match` is exhaustive**: `Err` → `Deny`; `Ok` splits into the two count arms. Every
   input reaches exactly one of the three terminal outcomes.

Therefore `leaf_validate` is a total function `&[u8] → SafeSegmentStack | Deny`. ∎

## 3. What this member adds vs. inherits (the honest boundary)

Explicit, so the proof claims only its own new ground:

| Requirement | Where it is enforced | New here? |
|---|---|---|
| Per-leaf alphabet `[A-Za-z0-9._-]` | `scan_safe` byte partition (Slice 1 §1.1) | inherited |
| ≥1 alphanumeric per leaf | `scan_safe` `SafeSegmentNotAlnum` arm (Slice 1 §1.2) | inherited |
| Traversal (`.`/`..`/`/`) unsayable | `scan_safe` `TraversalSegment` + out-of-alphabet (Slice 1) | inherited |
| **≥1 element** (require-at-least-one) | `scan_safe` `EmptyStack` reject — an empty `dest` cannot produce a stack (Slice 1 §1.2 row 1) | **inherited** (already guaranteed; not a separate check) |
| **Element-count upper bound** (a *sane bound*) | the `stack.len() > MAX_ELEMENTS` arm above | **new — the only genuinely new logic in this member** |
| Symlink-escape unreachable (where bytes *land*) | `filesystem-write-path.md` §1 open-discipline floor | cited (the *other* floor, downstream of the seam) |

The require-at-least-one-element rule is **not a second check** — it falls out of
`scan_safe`'s `EmptyStack` reject (an empty destination has no stack), so the array level
adds only the *upper* bound. Stating this prevents double-counting the lower bound as new
logic.

## 4. Adversarial closure (SA3)

| # | Adversary's attempt | Denied by | Anchor |
|---|---|---|---|
| 1 | Traversal (`..`, `.`, embedded `/`) | `scan_safe` — unsayable, not caught | Slice 1 §4 rows 1–2 |
| 2 | Empty destination (zero leaves) | `scan_safe` `EmptyStack` → `Deny` | Slice 1 §1.2 row 1 |
| 3 | Control/NUL/UTF-8/`0x1F`-forge in a leaf | `scan_safe` out-of-alphabet / `EmptySegment` | Slice 1 §4 rows 3–4 |
| 4 | Pathologically deep path (count exhaustion) | `stack.len() > MAX_ELEMENTS` → `Deny(TooManyElements)` | §1, the new arm |
| 5 | Absolute path / injected separator | `/` out of alphabet; separators are proxy-supplied, never in `dest` | §1; Slice 1 §1.1 |
| 6 | Symlink swapped at a path component after validation | not this floor's job — the open-discipline floor's `RESOLVE_NO_SYMLINKS` closes it, TOCTOU-free | `filesystem-write-path.md` §1, §3 row 5 (cited) |

Rows 1–5 are the textual floor's own closure; row 6 is explicitly the *downstream* floor's,
cited to show the seam is covered without this spec claiming it.

## 5. Residuals — named and bounded (SA4)

- **R-1 — `MAX_ELEMENTS` is a provisional scalar.** The *shape* (a bounded upper count,
  fail-closed on exceed) is settled; the *number* is provisional, marked for review, not a
  blocking judgment call (contract Non-Goals: a scalar is assign-mark-proceed). Bound: any
  value ≥ the deepest legitimate quarantine path is safe; too-low only over-denies (fails
  safe), never over-admits.
- **R-2 — This proves the textual floor only.** Symlink-escape and where-bytes-land are the
  open-discipline floor's (`filesystem-write-path.md` §1), a separate total function; the
  §0 seam and §4 row 6 make the division explicit so no reader credits this floor with the
  other's guarantee.
- **R-3 — Binds to `scan_safe` @ `a7ccd6c`** via Slice 1; a change there re-triggers both
  proofs.

## 6. Primitive-semantics grounding (SA5)

| Claim | Grounding |
|---|---|
| `scan_safe` is total and produces a ≥1-element stack | `scan-safe.md` (Slice 1), proven; `EmptyStack`/`EmptySegment` guarantee non-empty stack of non-empty segments |
| `SafeSegmentStack::len()` is always defined | committed `segment.rs`: `len(self) -> usize` over the inner `Vec`; a constructed stack has a defined length |
| `usize > MAX_ELEMENTS` is a total comparison | primitive integer comparison, total and panic-free over `usize` |
| Separators/root are proxy-supplied, absent from `dest` | `filesystem.md` §1 (quoted §0); `/` is out-of-alphabet in `scan_safe` regardless |

## 7. Judgment calls (SA7) — none new

The only scalar (`MAX_ELEMENTS`) is provisional-and-marked, not a fork (R-1). The alphabet,
the ≥1-alnum and ≥1-element rules, and the count-bound *shape* are all Sprint-1/Sprint-2
accepted ground. Nothing is chosen here that a source has not already settled.

## 8. Budget ledger (updated)

| Member | Slice | Lines | Counted | Simplest-form note |
|---|---|---|---|---|
| `scan_safe` (shared floor) | 1 | 35 | once | proven in Slice 1; referenced here, not re-counted |
| **leaf-validator (array level)** | 2 | **~4** | + | the `match` wrapper + the one `len() > MAX` arm; `scan_safe` excluded. No simpler form enforces a count bound over the validated stack. |
| address recognizer | 3 | *tbd* | + | excl. `scan_safe` |
| Anthropic recognizer | 4 | *tbd* | + | incl. bounded JSON reader inside; the leanness-scrutiny piece |

Recorded as a simplicity data point, not a quota (design's ruling, `scan-safe.md` §8): the
array level is ~4 net lines of genuinely new logic over the Slice-1 floor — as small as a
bounded-count check gets. Whole-set human-holdable check runs at Slice 4's gate.

## 9. Currency pins (2026-07-19)

`scan-safe.md` (Slice 1, gate-accepted); `filesystem-write-path.md` @ `bf56dc6` (committed
clean — the authorized offset amendment landed in that commit, verified on disk this
session; **not** working-tree as an earlier draft of this pin assumed) — cited for the
confine-write mechanism and the seam it consumes; `filesystem.md` §1 — **re-pinned 2026-07-25** `dec2453` → `7f7de0b`
(2026-07-18), the array-of-leaves requirement; committed `segment.rs` @ `a7ccd6c` for
`SafeSegmentStack::len` (**still current**). Re-pin at the gate.

On the `filesystem.md` re-pin: the intervening commit is the §4 no-offset reconciliation
to `proto/file.md` §1. This spec cites **§1**, which that commit did not touch — so the
identity moved while the cited content did not.
