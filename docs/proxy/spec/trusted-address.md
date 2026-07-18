# Trusted Address — Proxy-Side Spec

### The `Address` type, assembly, match, and policy over the committed wire representation

> **Spec, not exploration** — settles the proxy-side half of the trusted-address
> representation `cloud/approach.md` §7 requires. Sprint `docs/teams/proxy/002-*.md`,
> Slice 2; status: **awaiting SA1–SA7 gate; judgment calls routed, not yet ratified — not
> frozen.**
>
> **Authority order (per the sprint contract's SA1 amendment):** where `approach.md` §7
> prose diverges from the committed artifact, the committed artifact governs. The hardest
> anchor is `proto/src/segment.rs` (committed clean, `a7ccd6c`); `net.md` / `model.md` /
> `file.md` are its normative prose, **working-tree-modified, uncommitted** at authoring
> (sha-256 prefixes pinned in §8) — a commit dependency this spec's banner carries, like
> Sprint 1's `terminal.md` §5 dependency.

---

## 0. The requirement (verbatim anchor — SA1)

From `cloud/approach.md` §7: the trusted address (scheme, domain, path) gets its **"own
safe representation"**, **"spec-complete for the address it must carry, and incapable of
expressing a dangerous byte"**; it is **"the only box-originated data the proxy puts into
its own representation, matches against the route (§2, §6), and writes to a log."** The
payload half of §7 (never represented, never logged, transient) is restated in
`architecture.md` §4/§6–§7 and is not re-specified here.

## 1. What is already committed (the gap analysis)

The **wire half exists** and this spec does not redesign it (`net.md` §1 over
`segment.rs`):

- `net-address` = a fixed `scheme` field — **u32 enum, `0` = https, all other values
  reserved** — then a **safe segment stack**: first segment the domain (dots within it),
  remaining segments the path. The box supplies only components; the proxy supplies `://`
  and `/`.
- The committed scan (`segment.rs::scan_safe`, lines 169–208) admits exactly: ≥1 segment;
  every segment non-empty, every byte in `[A-Za-z0-9._-]`, ≥1 alphanumeric per segment;
  explicit rejection arms for an empty stack, an empty segment (leading/trailing/doubled
  `0x1F`), a literal `.` or `..` segment (`Reject::TraversalSegment`), and any
  out-of-alphabet byte. **Verifies, never transforms** — no normalization, no
  case-folding, no unescaping exists in the code path.
- Encoding is canonical: separator-not-terminator, so **each stack has exactly one wire
  encoding** (`model.md` §6); the message is frame-bounded with no box-supplied length.
- The type system carries the log guarantee: only `SafeSegment` exposes `as_str` (the
  safe-to-log capability); a real/unsafe name **physically cannot reach a log line**.

Consequently already unrepresentable, by committed construction: `:` (port), `@`
(userinfo), `?` (query-in-address), `#` (fragment), `%` (percent-encoding), `/`
(injection into a segment), NUL/controls/ESC, non-ASCII, `.`/`..` segments, empty
segments. **What is missing — this spec's subject — is the consumer side:** the type, the
assembly, the match, the reserved-value policy, caps, and the register-spelling rule.

## 2. The `Address` type (normative — SA6)

```
Address { scheme: Scheme::Https, domain: SafeSegment, path: [SafeSegment] }
```

constructed **only** from a structurally-valid `net-address` message: `scheme` per §4,
`domain` = stack segment 0, `path` = segments 1…n. There is no other constructor. **The
proxy never parses a URL string** — not from the box, not from anywhere on the box-facing
path; the representation is built from scanned components the proxy structures
(constructive, never parse-then-validate; the RFC-3986-vs-WHATWG parser-divergence swamp
is unentered, not survived).

## 3. Assembly (normative)

For match and log, the address assembles as:

```
assemble(A) = "https" ++ "://" ++ domain ++ ( "/" ++ path[i]  for each i in order )
```

- **Deterministic and injective.** If `assemble(A) == assemble(B)`: the alphabet contains
  neither `:` nor `/`, so the first `://` and every `/` in the assembled form are
  proxy-supplied structure; splitting the suffix on `/` recovers exactly the original
  segment list → `A == B`. Two distinct addresses cannot collide in the log or the match.
- **Log-safe by construction.** Every assembled byte is in `[A-Za-z0-9._-] ∪ {:, /}` ⊂
  printable ASCII (0x21–0x7E): no ESC (0x1B), no 8-bit C1 (0x9B), no control byte —
  assembled output cannot form a terminal escape even piped raw to a terminal. This is the
  §0 "incapable of expressing a dangerous byte" property, discharged at the type +
  alphabet level.
- Assembly serves **exactly two consumers**: the register match (§5) and the audit log
  line. The outbound HTTP request construction is the originator's/recognizer's spec
  (ledger L4/L5, the cloud effort's) — out of scope here.

## 4. Scheme and reserved-value policy (normative — consumer-owned)

The committed convention (proto sprint 006 lineage; `library.md`): the proto library
rejects on **structure** only; reserved-**value** policy belongs to the consumer. This
spec is that policy for `net-address.scheme`:

> **Admit `scheme == 0` (https) exactly. Every other u32 value — including every reserved
> value — denies the whole request, legibly.**

Extending a value later (were a second scheme ever warranted) is a deliberate, reviewed
spec revision after a legible fail-closed — the promotion pattern at the representation
level, never a default-open reserve.

## 5. Match and the register-spelling rule (normative)

- The route register's **route key is the canonical assembled string** (e.g.
  `https://api.anthropic.com/v1/messages`), stored verbatim in operator config.
- **Match:** route lookup succeeds iff `assemble(A)` equals a register key
  **byte-for-byte**. No case folding, no dot-segment resolution, no percent-decoding
  (nothing to decode — `%` is unrepresentable), no trailing-slash tolerance (a trailing
  `/` is not in the image of `assemble` at all). No match → the whole request denies,
  and the denial's log line carries `assemble(A)` — safe by §3.
- A component-wise implementation (scheme + segment-list equality against a pre-split
  key) is permitted and equivalent **by the §3 injectivity** — the assembled-form
  equality above is the normative definition either way.
- **Register well-formedness at load (JT-3, routed):** a key that is not in the image of
  `assemble` (uppercase is *in* the image; a trailing `/`, `//`, `?`, `%`, or empty
  segment is not) can never match any box request — a dead entry. Recommended: a
  load-time total check (prefix `https://`, split on `/`, each piece non-empty, in
  alphabet, ≥1 alnum) that rejects dead keys at config load, fail-closed and legible.
  Operator-authored input; adversary class: none.

## 6. Caps

The committed structural cap: a `net-address` is **one frame-bounded message** with no
box-supplied length (`model.md` §6) — the wire frame bounds the whole stack; the
continuation-bit reassembly exception is the *payload's*, not the address's (`net.md`
§2). The scan imposes no per-segment or per-stack count beyond that, so proxy-side caps
are operational sanity, not containment: **provisional, marked for review** — assembled
length ≤ 1024 bytes; segment count ≤ 64. A cap violation denies, legibly (never
truncates — deny-don't-rewrite).

## 7. Adversarial closure (SA3)

Every attempt is against the committed scan + this spec's consumer rules; "unsayable"
rows cite the `segment.rs` rejection arm that fires.

| # | Adversary's attempt | Denied by | Anchor |
|---|---|---|---|
| 1 | Port smuggling — `domain:8443` | `:` out of alphabet → `Reject::BadSegmentByte` | `segment.rs:190` |
| 2 | Userinfo — `evil@good.com` | `@` out of alphabet | `segment.rs:190` |
| 3 | Traversal — `..` / `.` as a path segment | Explicit `Reject::TraversalSegment` arm; independently, `..` has no alphanumeric | `segment.rs:178` |
| 4 | Dot-segment resolution games — `a/./b` | `.` segment rejected (row 3); no resolver exists to confuse — match is byte-equality | `segment.rs:178`; §5 |
| 5 | Percent-encoding — `%2e%2e`, `%2f` | `%` out of alphabet; no decoder exists downstream | `segment.rs:190`; §5 |
| 6 | Query/fragment in the address — `?action=delete`, `#x` | `?`/`#` out of alphabet; query is payload by classification, never address | `segment.rs:190`; `approach.md` §7 |
| 7 | Scheme swap — http, ws, file, gopher | u32 ≠ 0 → whole-request deny (every reserved value too) | §4 |
| 8 | Empty-segment injection — `//`, leading/trailing `/` | `Reject::EmptySegment`; and `/` itself is out of alphabet | `segment.rs:175,190` |
| 9 | Control/ESC/UTF-8/C1 bytes toward the log | Out of alphabet at scan; assembled form ⊂ printable ASCII | `segment.rs:190`; §3 |
| 10 | Case shadow — `Example.com` for registered `example.com` | Sayable, **matches nothing** → deny, legibly (the residual RT-1 — a correct deny that can read as a bug) | §5; RT-1 |
| 11 | Look-alike domain — punycode `xn--…`, digit labels, `a..b` | Sayable within the alphabet, matches nothing unless the operator registered that exact spelling — the register is the reviewed surface | §5; RT-3, RT-4 |
| 12 | Two stacks colliding into one assembled/logged form | Impossible — §3 injectivity (no `:`/`/` in the alphabet) + one wire encoding per stack | §3; `model.md` §6 |
| 13 | Oversize address / resource exhaustion | Frame bound (committed); provisional caps; deny never truncate | §6 |
| 14 | Register swap mid-request (reload race) | Match runs against one loaded register snapshot; reload semantics are L12 (open), interaction stated | §5; L12 |

## 8. Residuals — named and bounded (SA4)

- **RT-1 — case sensitivity (required item).** Byte-exact match is mechanically forced
  (scan never transforms + `approach.md` §3 never-rewrite), so `Example.com` ≠
  `example.com`: an operator registers one spelling, the box emits another, and the
  result is a **correct deny that can present as a bug**. Bound: the denial is legible
  and its log line shows the exact non-matching spelling (both spellings log-safe);
  the remedy is the box emitting the registered spelling, or the operator deliberately
  registering the variant — never a proxy fold. Unlike DNS semantics, by design.
- **RT-2 — dead register keys.** A key outside `assemble`'s image never matches; silent
  until traffic fails legibly. Bound: JT-3's load-time check (routed); without it, the
  failure mode is still deny-and-surface, never admit.
- **RT-3 — homograph/punycode curation.** `xn--…` labels are sayable LDH bytes; the proxy
  matches them exactly and never decodes IDN. Bound: the register is the operator's
  reviewed surface — a punycode key is visibly punycode in config and in every log line;
  no normalization exists for an adversary to exploit.
- **RT-4 — domain shape beyond the alphabet is unvalidated** (`a..b`, all-digit labels,
  253+-byte names are sayable). Bound: exact match — a shape the operator never
  registered matches nothing; extra validation is JT-1 (recommended against — surface
  without containment gain).
- **RT-5 — currency.** `net.md`/`model.md`/`file.md` are working-tree-modified,
  uncommitted (pins below); `segment.rs` is committed clean. Commit dependency carried in
  the banner; re-pin at gate.

Currency pins (2026-07-18): `segment.rs` @ `a7ccd6c` (committed); working-tree sha-256
prefixes — `net.md` `02c8ca7a58202bda4e57`, `model.md` `4a468afae792e6e3d3f3`, `file.md`
`5461602d24cf8b4e8b5e`.

## 9. Judgment calls — surfaced and routed, not settled (SA7)

Named `JT-*` to avoid the Slice-1/contract `J4` name collision (flagged at planning):

- **JT-1** — domain-segment validation beyond alphabet + exact match (label rules, length,
  no `..` inside the domain segment). Rec: **no** — exact match against the operator's
  canonical key is the whole guarantee; added validation is surface without containment
  gain. *(= the sprint contract RQ4 "domain-segment validation" bullet.)*
- **JT-2** — scheme policy: https-only now; any extension is a deliberate spec revision.
  Rec: **https-only**. *(= contract RQ4 "reserved-scheme-value" bullet.)*
- **JT-3** — register load-time well-formedness check (dead-key lint, §5). New
  convention, operator-input-only, fail-closed at load. Rec: **yes**.
- **Provisional scalars (marked, non-blocking):** assembled-length cap 1024; segment
  count cap 64.

## 10. The `approach.md` §7 seam — stated and routed (RQ3), not fixed

`approach.md` §7 (working tree) says the proxy supplies **"the structure — `://`, `.`,
`/`"** and calls the representation **"a *distinct* system, because a URL is not a
path."** The committed artifact diverges on both points: the **box supplies the domain's
dots** (they are ordinary alphabet bytes inside segment 0 — `net.md` §1, `segment.rs`),
and the address **reuses the same stack and alphabet** as the file leaf, distinct only by
the scheme prefix and the first-segment-is-domain position convention. `approach.md`
self-describes as research-not-specification; `net.md`/`model.md` are the wire's
normative spec; per the contract's SA1 amendment the committed form governs this spec.
The `approach.md` wording fix is its owner's, routed via the PM — stated here with both
citations, resolved by neither.
