# The Address Recognizer — Total-Function Proof (the cloud route gate)

### The total function over `(scheme) × (safe segment stack)` → register-match-or-deny

> **Total-function proof, not a mechanism spec** — proves the cloud route/address recognizer
> is a total function over a finite domain (spine §15–§16). Sprint
> `docs/teams/proxy/004-*.md`, Slice 3; status: **awaiting the SA1–SA7 gate.**
> **Consolidation:** the per-segment scan is `scan-safe.md` (Slice 1); the `Address` type,
> assembly, injectivity, scheme policy, and byte-exact match are `trusted-address.md`
> (Sprint 2, accepted-not-frozen); the wire disposition is `box-proxy.md`'s net-address row
> (Sprint 3). All cited, none reopened. This member adds the total-function enumeration.
>
> **Budget:** the recognizer's own logic is counted here (estimated from spec — not yet
> committed code, unlike `scan_safe`); `scan_safe` is **not re-counted**. Ledger in §8.

---

## 0. What is proven, and against what (SA1)

The address recognizer is the cloud subsystem's route gate: it takes a box-authored
`net-address` — a `scheme` plus a safe segment stack — and either matches it to a route in
the operator's register (admit, hold for the payload) or denies the whole request. It is the
`scheme==0` / byte-exact-match discipline of `trusted-address.md`, proven here as a total
function.

The requirement (SA1), from `trusted-address.md` §4–§5, quoted: **"Admit `scheme == 0`
(https) exactly. Every other u32 value — including every reserved value — denies the whole
request, legibly."** And the match: **"route lookup succeeds iff `assemble(A)` equals a
register key byte-for-byte. No case folding, no dot-segment resolution, no percent-decoding
... no match → the whole request denies."** This is `box-proxy.md`'s net-address row
disposition ("Scheme==0 only + byte-exact match ... deny → `net-error(2)` + log"), cited.

## 1. The total function

```
recognize_address(scheme: u32, stack: SafeSegmentStack, register: {key}) -> Admit(route) | Deny(reason)

  if scheme != 0                          => Deny(BadScheme)        # §4 scheme policy
  let addr = assemble(scheme, stack)                               # deterministic + injective — trusted-address.md §3, cited
  match register.lookup_exact(addr):                               # byte-exact — §5
      Some(route) => Admit(route)
      None        => Deny(NoRoute)
```

- **Input domain:** `(scheme: u32) × (stack: SafeSegmentStack)`. The `stack` is already the
  total output of `scan_safe` (Slice 1) over the box's segment bytes; the `scheme` is a
  fixed-width `u32` read (`model.md` §6's fixed-prefix decoder — a `split_first_chunk::<4>` +
  `from_le_bytes`, total over any frame-bounded body ≥ 4 bytes; a shorter body is a
  structural reject upstream, `box-proxy.md` JC-3a).
- **Output:** exactly one of `Admit(route)` or `Deny(reason)` for every domain point.

## 2. Totality — composition of total steps (SA2)

1. **Scheme policy is total over `u32`.** `scheme != 0` partitions `u32` into two exhaustive,
   disjoint classes; only `0` proceeds, every other value (all ~4.29 billion, including every
   reserved one) denies. No value is unhandled.
2. **`assemble` is a total, deterministic, injective function** on
   `(scheme=0, SafeSegmentStack)` — proven in `trusted-address.md` §3 (the alphabet contains
   neither `:` nor `/`, so the proxy-supplied `://` and `/` uniquely bound the segments;
   splitting the assembled suffix recovers the original list). **Cited, not re-derived** (per
   the slice charge). Totality: it is string concatenation over a non-empty stack — always
   defined, never panics.
3. **`lookup_exact` is total.** The register is a finite set of byte-string keys; exact byte
   equality of `addr` against it yields `Some(route)` or `None` — a total decision, no
   normalization step that could diverge (`trusted-address.md` §5: no fold, no resolution, no
   decode).
4. **Composition.** `scan_safe` total (Slice 1) → the `scheme` branch total → `assemble`
   total → `lookup_exact` total, sequenced with exhaustive branches. Every
   `(scheme, stack)` reaches exactly one terminal outcome.

Therefore `recognize_address` is a total function `(u32 × SafeSegmentStack) → route | Deny`. ∎

## 3. What this member adds vs. inherits (the honest boundary)

| Property | Where enforced | New here? |
|---|---|---|
| Per-segment alphabet / ≥1-alnum / traversal-unsayable | `scan_safe` (Slice 1) | inherited |
| Port/userinfo/fragment/`%`/query unrepresentable | `scan_safe` out-of-alphabet (Slice 1) | inherited |
| `Address` type, one constructor, no URL-string parse | `trusted-address.md` §2 | cited |
| `assemble` determinism + injectivity | `trusted-address.md` §3 | **cited, not re-derived** |
| Scheme `== 0`-only policy | `trusted-address.md` §4 (accepted-not-frozen) | cited |
| Byte-exact match, no normalization | `trusted-address.md` §5 | cited |
| **Totality of the composed recognizer** | this proof (§2) | **new — the enumeration is this member's ground** |

The recognizer's *mechanism* is entirely Sprint-2/Sprint-3 accepted ground; what this member
adds is the proof that the composition is a total function — the same consolidation shape as
Slice 2.

## 4. Adversarial closure (SA3)

| # | Adversary's attempt | Denied by | Anchor |
|---|---|---|---|
| 1 | Port / userinfo / fragment / `%`-encoding / query-in-address (`d:8443`, `e@d`, `#x`, `%2e`, `?a=1`) | out-of-alphabet at `scan_safe` — unsayable in a segment | Slice 1 §1.1; `trusted-address.md` §7 rows 1–6 |
| 2 | Scheme swap (http, ws, file, gopher — any `u32 ≠ 0`) | `scheme != 0` → `Deny(BadScheme)`; every reserved value too | §1, §2.1 |
| 3 | Case shadow (`Example.com` for registered `example.com`) | byte-exact, no fold → `Deny(NoRoute)`, legible — a *correct* deny (residual RT-1) | §5; RT-1 |
| 4 | Two distinct addresses colliding into one register match | impossible — `assemble` injective (`trusted-address.md` §3) | §2.2 |
| 5 | Non-canonical would-be match (`//`, trailing `/`, empty segment) | not in the image of `assemble` → matches nothing → deny | §5; `trusted-address.md` §5 |
| 6 | Dead register key (a key outside `assemble`'s image) | can never match any box request; a config-lint concern, fails safe (deny) | RT-2; `trusted-address.md` JT-3 (cited) |
| 7 | Redirect / box-influenced destination at runtime | the register is proxy-owned and never widened at runtime (no redirect-following) | `approach.md` §6 (cited); `box-proxy.md` route-matcher row |

Rows 1 and 3–5 resolve by the committed representation + injectivity; row 2 is the scheme
policy; rows 6–7 are cited config/register invariants, fail-safe.

## 5. Residuals — named and bounded (SA4)

- **RT-1 — case sensitivity (the required, cited residual).** Byte-exact match makes
  `Example.com ≠ example.com`: an operator registers one spelling, the box emits another →
  a **correct deny that can present as a bug** (`trusted-address.md` §8 RT-1, cited). Bound:
  the denial is legible, its log line carries the exact non-matching spelling (both
  log-safe); the remedy is the box emitting the registered spelling or the operator
  registering the variant — never a proxy fold.
- **RT-2 — dead register keys.** A key outside `assemble`'s image never matches; silent
  until traffic fails legibly. Bound: `trusted-address.md`'s JT-3 load-time lint (cited); the
  failure mode is deny-and-surface regardless, never admit.
- **RT-3 — scheme policy is accepted-not-frozen.** `scheme==0`-only rides the Sprint-2
  ratification packet (`trusted-address.md` JT-2); a ruling that admits a second scheme
  changes the cited policy, not this proof's totality (the scheme branch stays exhaustive
  over `u32` whatever the admit-set). Coupling stated, not re-opened.
- **RT-4 — provisional caps** (assembled length, segment count) — the *shape*
  (bounded, fail-closed) settled, the *numbers* provisional (`trusted-address.md` §6),
  assign-mark-proceed.

## 6. Primitive-semantics grounding (SA5)

| Claim | Grounding |
|---|---|
| `scan_safe` produces the stack, total | `scan-safe.md` (Slice 1), proven |
| Fixed-width `u32` scheme decode is total over a body ≥ 4 bytes | `model.md` §6 fixed-prefix decoder (`split_first_chunk::<4>` + `from_le_bytes`); a shorter body is a structural reject upstream |
| `assemble` deterministic + injective | `trusted-address.md` §3 (cited, not re-derived) |
| Exact byte-equality lookup is total, no normalization | `trusted-address.md` §5 (no fold / resolution / decode); byte comparison is total |
| Scheme `!= 0` is a total comparison over `u32` | primitive integer comparison, panic-free |

## 7. Judgment calls (SA7) — none new

Every choice here — scheme policy, match discipline, injectivity, caps — is
`trusted-address.md`'s, cited with its accepted-not-frozen status (RT-3, RT-4). This member
introduces no new fork; it proves the composition total. (Sprint 2's JT-1/JT-2/JT-3 remain
that spec's routed calls; this proof does not re-route or settle them.)

## 8. Budget ledger (updated)

| Member | Slice | Lines | Basis | Simplest-form note |
|---|---|---|---|---|
| `scan_safe` (shared floor) | 1 | 35 | **counted on disk** (`segment.rs`) | committed; referenced, not re-counted |
| leaf-validator (array level) | 2 | ~4 | estimated from spec | `match` wrapper + one `len() > MAX` arm |
| **address recognizer** | 3 | **~18** | estimated from spec | scheme check + `assemble` (concat over proxy-supplied structure) + exact-match lookup; `scan_safe` excluded. No simpler form performs an exact-match gate. |
| Anthropic recognizer | 4 | *tbd* | — | incl. bounded JSON reader inside; the leanness-scrutiny piece |

**Basis distinction (carried forward):** `scan_safe` is *counted on disk* (real committed
code); every other member is *estimated from spec* (not yet implemented) — the estimate
firms to a real count when the build lands. Recorded as a simplicity data point, not a quota
(design's ruling, `scan-safe.md` §8). Whole-set human-holdable check at Slice 4's gate.

## 9. Currency pins (2026-07-19)

`scan-safe.md`, `leaf-validator.md` (Slices 1–2, gate-accepted); `trusted-address.md` @
`c4690bf` (committed Sprint-2 spec; §2–§6 cited — verified on disk this session);
`box-proxy.md` @ `bf56dc6` (Sprint-3 net-address row, verified); committed `segment.rs` @
`a7ccd6c`. The
scheme policy and caps are cited at `trusted-address.md`'s accepted-not-frozen status
(Sprint-2 ratification packet pending). Re-pin at the gate.
