# The Anthropic Recognizer — Total-Function Proof (the recursive egress gate)

### The whole trusted surface for the worst-case destination — a total function over an attacker-authored JSON body, with zero rewrite behind it

> **Total-function proof of a genuinely new mechanism** — the Anthropic Messages body-shape
> recognizer, architecture ledger **L4** (placement, open) and **L5** (exact recognized
> shape, this sprint's target). Sprint `docs/teams/proxy/004-*.md`, Slice 4; status:
> **awaiting the strictest gate the team runs — SA1–SA7 plus SA8 (parse-fidelity).**
>
> **Design-resolved inputs (2026-07-19), treated as settled here:** JA-2 (top-level field
> whitelist = the minimal known-inert set, deny the rest); JA-3 (JSON profile =
> bounded/fail-closed, depth/size as shallow as the whitelisted fields require, tied to the
> `REASSEMBLY_CAP`; numbers provisional); JA-4 (beta-header admit-set = **empty** — deny all
> capability-gating betas by default). **Still open:** JA-1 (the exact vetted client-side
> tool-identity strings) — pending the operator directly; **pinned to the Anthropic
> capability snapshot of 2026-06-20** (`anthropic-api.md`), **fail-closed on drift**.
>
> **Budget:** this is the piece leanness scrutiny concentrates on (design's ruling) — the
> proxy's own code and the piece most at risk of bloat. The bounded JSON reader (Stage 0)
> is counted **inside** this member's number, never as a library. Ledger, §10.

---

## 0. What is proven, the category shift, and the requirement (SA1)

The recognizer takes a box-authored `POST /v1/messages` request and returns **`Admit(forward
the body verbatim)` or `Deny(reason, legible)`** — never a rewrite, never a strip-and-proceed.
It **is** the containment for this hop: there is no emit-side backstop, so *"a recognizer
that is too permissive forwards what it missed"* (`anthropic-api.md` §6). A too-permissive
admit is a **containment breach**, not a feature bug.

**The category shift from Slices 1–3.** Those three are *flat* and the proxy *owns* the
representation — a bad byte is structurally unrepresentable. This recognizer is *recursive*
over an attacker-authored JSON body the proxy does **not** own: **Anthropic's own server will
parse the same bytes.** Two new risk classes follow, each with a dedicated treatment:

1. **The JSON parse itself is trusted surface** (§2). A general, unbounded-grammar parser
   handed the box's bytes before any field logic is exactly the parse-an-arbitrary-grammar
   bet the spine forbids (Sprint 2's SA6) — and uncounted bulk against the reviewability
   budget. The parse must be a *bounded, total structural reader*, small-and-ours, counted.
2. **Parse-disagreement is exploitable** (§5, SA8). Because the bytes are forwarded verbatim
   and re-parsed downstream, any construct the recognizer and Anthropic could interpret
   *differently* (duplicate keys, non-canonical escapes, trailing data) is a gap the box
   drives. The recognizer closes it not by matching Anthropic's parser but by **rejecting
   every parse-ambiguous construct fail-closed**.

The requirement (SA1), from `anthropic-api.md` §6/§8, quoted: **"deny *unless* the request is
proven wholly inert"**; admit **only** the two shapes — bare custom tools and exact-string
vetted client-side identities — with **"no prefix rule, no 'a number may follow,' no version
fuzzing"**; **"recurse to full depth, and default-deny on anything unrecognized."** The verb
(`POST`), domain (`api.anthropic.com`), path (`/v1/messages`) are exact matches, denied
otherwise. Everything else — a server-side `type`, `mcp_servers`, `container`, a
capability-gating beta header, an unknown `type`, a **new version** of a vetted tool, an
unknown top-level field — **fails closed.**

### Build-time obligation (load-bearing — this gate accepts the design, not built code)

This spec's gate certifies the **design**; the recognizer is **not yet implemented.** The
standing obligation that travels with this artifact to whoever builds it: **when the
recognizer is implemented, a synthetic full-field corpus — every SA3 and SA8 row of §5–§6,
both the admissible and the inadmissible shapes — is run against the *real code* as a
permanent regression guard**, proving the built recognizer implements the deny-default this
spec specifies (not merely that a hand-computed "expected" agrees). The corpus is the
build-time evidence that the code *is* the total function proven here; it is required, not
optional, because this member is the entire trusted surface with no rewrite behind it. A
recognizer that passes review as a design but was never corpus-verified as code has an
unproven security core.

## 1. The total function (all stages)

```
recognize_anthropic(envelope, body_bytes, betas) -> Admit | Deny(reason)

  # Stage 1 — envelope, exact-match (reuses the address-recognizer discipline, Slice 3)
  if (verb, domain, path) != (POST, api.anthropic.com, /v1/messages)   => Deny(BadEnvelope)

  # Stage 4 — beta headers, empty admit-set (JA-4)
  if betas is non-empty                                                => Deny(BetaNotInert)

  # Stage 0 — the bounded structural JSON reader (the crux, §2)
  tree = read_bounded(body_bytes)   # Ok(tree) | Deny(ParseReason) — total, depth/size-bounded,
                                     # ambiguity-rejecting; NEVER a general JSON library

  # Stage 2 — top-level field whitelist (JA-2; kills mcp_servers/container by construction)
  for key in tree.top_level_keys:
      if key not in TOPLEVEL_WHITELIST                                 => Deny(UnknownField{key})

  # Stage 3 — tools[] per entry, recursing to full depth
  for entry in tree.tools_or_empty():
      if entry has "type":
          if entry.type not in VETTED_CLIENT_TYPES  (exact string)     => Deny(BadToolType)
      else:  # bare custom tool
          if entry.keys not subset of CUSTOM_TOOL_KEYS                 => Deny(BadCustomTool)
          # interior of input_schema is client-side-inert: bounded (Stage 0) but not capability-policed

  Admit(forward body_bytes verbatim)   # nothing edited — the box's cch hash stays valid
```

`TOPLEVEL_WHITELIST`, `VETTED_CLIENT_TYPES`, `CUSTOM_TOOL_KEYS`, and the depth/size caps are
the design-resolved / operator-pending sets of §9; the *shape* above is settled.

## 2. Stage 0 — the bounded structural JSON reader (the crux)

The reader is the security core of this member and the reason it is not a general parser. It
is a **total function `&[u8] -> Tree | Deny`** with four properties, each making an
otherwise-unbounded surface finite and fail-closed:

1. **Size-bounded.** The body is the reassembled `net-payload`, already capped at
   `REASSEMBLY_CAP` (provisional 32 MiB, reject-before-grow — `library.md` §8). The reader
   never sees an unbounded input; a body at the cap is the largest domain point.
2. **Depth-bounded.** Every structural descent (into an object or array) increments a depth
   counter; exceeding `MAX_DEPTH` (provisional, shallow — as deep as the whitelisted Messages
   shape requires, JA-3) is an immediate `Deny(TooDeep)`. **This is what makes the recursion
   terminate** and the "recurse to full depth" requirement finite: full depth is a bounded
   depth, and anything deeper is denied, not explored.
3. **Restricted profile, deny-default at every branch.** The reader accepts only the JSON
   subset the Messages body needs — objects, arrays, strings, numbers, booleans, null — and
   **denies at the first byte that does not fit the expected structural position.** It is not
   RFC-8259-complete and does not try to be: any construct outside the profile is a `Deny`,
   never a best-effort parse. Deny is the default of every branch; admit is the exception
   reached only by fully-recognized structure.
4. **Ambiguity-rejecting (the SA8 core, detailed in §5).** Any construct whose meaning could
   differ between the reader and Anthropic's parser is denied: a **duplicate key** in an
   object → `Deny(DuplicateKey)` (never silently pick first-or-last); a **non-canonical
   unicode/`\u` escape** or an over-long encoding → `Deny(BadEscape)`; **trailing bytes**
   after the top-level value → `Deny(TrailingData)`; a number outside the accepted numeric
   profile → `Deny(BadNumber)`.

The reader is **small-and-ours and counted** (§10). It is not a general JSON library precisely
because a library (a) parses an unbounded grammar the spine forbids as the first thing attacker
bytes touch, (b) resolves duplicate keys and escapes by its own rules — reintroducing the
parse-disagreement gap — and (c) is uncounted bulk. All three are closed by owning a bounded
reader.

## 3. Stages 1–4 — recognition over the bounded tree

- **Stage 1 — envelope.** `(verb, domain, path)` exact-match against
  `(POST, api.anthropic.com, /v1/messages)` — the exact-match discipline proven in Slice 3,
  reused; any miss denies.
- **Stage 4 — beta headers.** The admit-set is **empty** (JA-4, design-resolved): any
  `anthropic-beta` header denies. Betas gate capability, and the recognizer admits no
  capability, so the safe admit-set is nothing. (Placed early in §1's flow as a cheap total
  reject before parsing.)
- **Stage 2 — top-level field whitelist.** Every top-level key must be in `TOPLEVEL_WHITELIST`
  (JA-2, the minimal known-inert set: `model`, `messages`, `system`, `max_tokens`, the
  sampling/thinking params, `tools`, `tool_choice`, `metadata`, `stream`). `tool_choice` is
  admitted unpoliced because it only *references* which tool the model may use — it never
  *defines or executes* one; the capability line is entirely at the `tools[]` entry level
  (Stage 3), so a reference to an already-recognized tool carries no effect of its own. Any
  other key —
  `mcp_servers`, `container`, or anything unknown — denies the whole request. This is the
  decisive move: the effect-bearing siblings are excluded **by not being admitted keys**, not
  by a denylist that must stay complete.
- **Stage 3 — `tools[]` per entry, to full (bounded) depth.** A `type`-tagged entry admits
  only on **exact string match** to `VETTED_CLIENT_TYPES` (JA-1, operator-pending, snapshot
  2026-06-20) — `text_editor_20250728` matches, `text_editor_20260101` is a different string
  and denies. A bare (no-`type`) entry admits iff its keys are a subset of
  `CUSTOM_TOOL_KEYS` (`name`, `description`, `input_schema`, `strict`, `allowed_callers`);
  the `input_schema` interior is client-side-inert (the box runs it — `anthropic-api.md` §2)
  so it is **not** capability-policed, but it **is** depth/size-bounded by Stage 0. Anything
  else denies.

## 4. Totality and termination (SA2)

- **The parse terminates.** The input is finite (size-bounded, §2.1); every descent increments
  a depth counter capped at `MAX_DEPTH` (§2.2); the reader advances a byte cursor that only
  moves forward. No unbounded loop, no unbounded recursion — the reader visits each byte at
  most a bounded number of times and denies at the depth cap. It terminates on every input.
- **Every input is decided.** Stage 0 returns `Ok(tree)` or `Deny`. Given a tree, Stages 1–4
  are finite walks (a fixed envelope check, a finite key set, a bounded `tools[]` array of
  bounded entries) each ending in a `Deny` or falling through to `Admit`. Deny-default at
  every branch means no unrecognized construct reaches `Admit` — there is no "unhandled"
  path; the absence of a matching admit-arm *is* a deny.
- **No panic.** All operations are byte-cursor advances, integer depth comparisons, exact
  string/set membership, and subset checks over finite key sets — none can panic on any
  `&[u8]`.

Therefore `recognize_anthropic` is a total function over `(envelope × body_bytes × betas)`,
admitting only the proven-inert shape and denying every other point. ∎

## 5. Parse-fidelity (SA8 — the genuinely new gate)

The recognizer forwards verbatim and Anthropic re-parses, so **the guarantee is not "we parse
identically to Anthropic"** — we cannot prove that against a parser we do not control.
The guarantee is **"we admit nothing whose interpretation is parser-dependent."** Ambiguity is
denied at Stage 0, closing the gap by construction:

| Ambiguity vector | The exploit if admitted | Stage-0 disposition |
|---|---|---|
| **Duplicate object keys** (`{"tools": [inert], "tools": [web_fetch]}`) | recognizer inspects one `tools`, Anthropic executes the other | `Deny(DuplicateKey)` — never first-or-last-wins |
| **Non-canonical unicode / `\u` escapes** (`tools`, overlong, surrogate tricks) | a key spelled to read as inert to one parser, capability to the other | `Deny(BadEscape)` — only a canonical, in-profile encoding is accepted |
| **Trailing data after the top value** (`{…}{…}` or `{…}\0junk`) | recognizer stops at the first object; Anthropic reads more | `Deny(TrailingData)` |
| **Number/whitespace edge forms** (leading `+`, hex, `NaN`, exotic whitespace) | profile-dependent acceptance | `Deny(BadNumber)` / out-of-profile deny |
| **Comments / trailing commas / other JSON5-isms** | one parser tolerates, the other rejects or shifts structure | out-of-profile → `Deny` |

The unifying rule: **the restricted profile plus ambiguity-rejection means every body the
recognizer admits has exactly one interpretation, so what the recognizer inspected is what
Anthropic executes.** SA8 is satisfied by refusing ambiguity, not by mirroring a foreign
parser.

## 6. Adversarial closure (SA3 — required rows plus SA8 rows)

| # | Adversary's attempt | Denied by |
|---|---|---|
| 1 | Server-side `type` — `web_fetch_20260318`, `code_execution_*`, `web_search_*` | Stage 3 — not in `VETTED_CLIENT_TYPES` (exact-string), denies |
| 2 | `mcp_servers` array | Stage 2 — not an admitted top-level key |
| 3 | `container` field | Stage 2 — not an admitted top-level key |
| 4 | Capability-gating beta header (`mcp-client-*`, `code-execution-*`) | Stage 4 — empty admit-set (JA-4) |
| 5 | New-version stamp of a vetted tool (`text_editor_20260101`) | Stage 3 — different exact string, denies (fail-closed until re-admit) |
| 6 | Unknown top-level key | Stage 2 — whitelist, denies |
| 7 | Depth bomb (deeply nested arrays/objects) | Stage 0 — `MAX_DEPTH` → `Deny(TooDeep)` |
| 8 | Size bomb / never-ending body | `REASSEMBLY_CAP` reject-before-grow (Stage 0.1) |
| 9 | **Duplicate `tools` key** hiding a server-side tool | Stage 0 — `Deny(DuplicateKey)` (SA8) |
| 10 | **Unicode-escaped key** to smuggle `mcp_servers` as `mcp_servers` | Stage 0 — `Deny(BadEscape)` (SA8) |
| 11 | **Trailing second object** after an inert first | Stage 0 — `Deny(TrailingData)` (SA8) |
| 12 | Server-side tool nested inside a custom tool's `input_schema` | not reachable — `input_schema` interior is inert data the box runs; no `tools[]`-level `type` there to execute; and Stage 3 policing is at the `tools[]` entry level where execution is selected |
| 13 | Effect tool with a bare-custom disguise (a `type`-less entry named `web_fetch`) | admitted as a *custom* tool — and that is correct: a `type`-less entry is client-executed by construction (the API only emits a `tool_use` block; the box runs it), so it carries no server effect regardless of its name (`anthropic-api.md` §2, §6) |

Row 13 is the subtle one and is sound by the source's own line: the danger is *server
execution*, selected by a vetted server `type`; a name without a `type` cannot select it.

## 7. Residuals — named and bounded (SA4)

- **RA-1 — JA-1 admit-list is a dated snapshot, open.** The `VETTED_CLIENT_TYPES` strings are
  pinned to the **2026-06-20** Anthropic snapshot and pending the operator (the
  tool-identity-level standing-authorization register). Bound: **fail-closed on drift** — a
  string not on the pinned set denies; a new vetted version is a deliberate operator re-admit
  (the promotion pattern), never an auto-widen. The recognizer's totality does not depend on
  the set's contents, only on exact-match against whatever the set is.
- **RA-2 — read-only, never rewrite (the `cch` coupling).** The recognizer inspects and
  forwards **verbatim**; it never normalizes or edits the body. The box signs its own body
  (the `cch` xxHash64), so any edit would invalidate the signature — reinforcing verify-and-
  deny (`anthropic-api.md` §6). Stated so no reader infers a canonicalization pass; the
  ambiguity-rejection of §5 is *reject*, never *rewrite-to-canonical*.
- **RA-3 — provisional scalars.** `MAX_DEPTH`, the numeric profile bounds, and `REASSEMBLY_CAP`
  (32 MiB) are provisional numbers; the *shapes* (bounded, fail-closed) are settled (JA-3).
  Assign-mark-proceed; too-shallow only over-denies (fails safe).
- **RA-4 — L4 placement is open, and this proof is placement-neutral.** Whether the recognizer
  is an in-transport module or an isolated service (`anthropic-api.md` §8, ledger L4) does not
  affect its totality — the function is the same wherever it runs. Routed to cloud/design,
  not settled here.
- **RA-5 — the admit-list is a snapshot of a growing surface.** The server-side type strings
  grow (`web_fetch_20250910 → _20260318` in a year); the whitelist-not-denylist design means
  new server types are denied by construction (not on the admit-list), so growth is a
  non-event — but the snapshot date is pinned (RA-1) so a reader knows the vintage.

## 8. Primitive-semantics grounding (SA5)

| Claim | Grounding |
|---|---|
| The body arrives bounded at `REASSEMBLY_CAP` | `library.md` §8: the `Reassembler` is reject-before-grow, cap checked before append (provisional 32 MiB) |
| A bounded-depth recursive descent terminates | standard result: a forward-only cursor over a finite input with a capped recursion depth visits each byte a bounded number of times |
| Exact string match cannot version-fuzz | `anthropic-api.md` §6: "no prefix rule, no 'a number may follow'" — string equality, not pattern |
| Client-side (`type`-less or vetted-`type`) tools cannot cause server effect | `anthropic-api.md` §2: the API emits only a `tool_use` block for a client tool; the box executes it, and box execution is contained (`principles.md` §6/§19) |
| A duplicate key is a real cross-parser hazard | JSON (RFC 8259) does not define duplicate-key semantics; parsers differ (first-wins / last-wins / error) — the reason §5 denies rather than picks |

## 9. Judgment calls (SA7)

- **JA-1 — the exact `VETTED_CLIENT_TYPES` strings — OPEN, routed to the operator** (the
  tool-identity standing-authorization register). Pinned to the 2026-06-20 snapshot,
  fail-closed on drift. The one call this member does not settle.
- **JA-2 — `TOPLEVEL_WHITELIST` — RESOLVED (design, 2026-07-19):** the minimal known-inert
  set (§3); deny the rest, extend via the promotion pattern. Cited as settled.
- **JA-3 — the JSON profile + depth/size caps — RESOLVED (design, 2026-07-19):**
  bounded/fail-closed, as shallow as the whitelisted fields require, tied to `REASSEMBLY_CAP`;
  numbers provisional (RA-3). Cited as settled.
- **JA-4 — beta-header admit-set — RESOLVED (design, 2026-07-19): empty** — deny all
  capability-gating betas (§3, Stage 4). Cited as settled.
- **L4 — recognizer placement** — not this sprint's; the spec is placement-neutral (RA-4),
  routed to cloud/design.

## 10. Budget ledger (final — whole-set check)

| Member | Slice | Lines | Basis | Simplest-form note |
|---|---|---|---|---|
| `scan_safe` (shared floor) | 1 | 35 | counted on disk | committed; counted once |
| leaf-validator (array level) | 2 | ~4 | estimated from spec | `match` + one count arm |
| address recognizer | 3 | ~18 | estimated from spec | scheme + assemble + exact-match |
| **Anthropic recognizer** | 4 | **~230–300** | estimated from spec | **incl. the bounded JSON reader (~150–200 of it) — counted here, never a library.** The reader dominates because it is where deny-by-default and ambiguity-rejection live; the recognition stages (1–4) are thin over the tree. |
| **Whole set** | — | **~290–360** | mixed | held-in-one-sitting: yes — one shared scan + two thin gates + one bounded reader with staged recognition |

**Whole-set enumerability (the real property, design's ruling):** the set is *simplest-form
and human-holdable* — three of four members are a scan and two thin wrappers over it; the
fourth's weight is concentrated in one bounded reader whose every branch is deny-default. No
member carries an unbounded parser, a general library, or an un-enumerable case. The line
counts are simplicity signals, not a quota; the bounded-reader discipline is what keeps the
largest member reviewable. **If a later real build shows the reader cannot be held in one
sitting, that is a scope question for the PM** (design's ruling), not a number to squeeze —
but the design as specified is enumerable.

## 11. Currency pins (2026-07-19)

Anthropic capability surface: **snapshot 2026-06-20** (`anthropic-api.md`, §4/§6 — the vintage
JA-1 is pinned to; fail-closed on drift). `anthropic-api.md` §1/§2/§6/§8 and `approach.md`
§1/§3 (working-tree state, cited for the admit-mechanism and verify-and-deny). `library.md`
§8 for `REASSEMBLY_CAP`. Slices 1–3 (`scan-safe.md`, `leaf-validator.md`,
`address-recognizer.md`) gate-accepted. Re-pin at the gate; the JA-1 set re-pins whenever
the operator ratifies.
