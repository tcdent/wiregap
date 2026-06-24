# Sprint 006 — Control (open-session) & Feature-Gating

> The sixth sprint of the `proto` crate: the last structural sprint. It adds `open-session` 0x00 — the
> control verb that opens a session (the smallest verb, an empty body) — and **feature-gates the crate by
> service** (control / terminal / file / net), so each consumer compiles only its slice. The payoff is a
> **security** one: the trusted proxy's slice excludes the terminal grammar, shrinking its auditable
> surface (the operator's minimize-the-trusted-surface lens, realized structurally). No new decoder class.
> After 006 the crate is **21 of 22** verbs; only `key` 0x12 remains, blocked on the `>0x10FFFF` named-key
> keysym table — a follow-on sprint once the operator pins the table.
>
> Team: `proto` (lead `zero.proto-lead`, dev `zero.proto-dev`, review `zero.proto-review`).
> Grounded in [`model.md`](../../proto/model.md) §2/§8, [`library.md`](../../proto/library.md) §7/§8,
> and the 001–005 contracts.

---

## Scope Boundary

**This sprint owns:**

1. **`open-session` 0x00** — head→box, control block. An **empty-body unit verb**: `Message::OpenSession`,
   decode is `if !body.is_empty() → TrailingBytes{extra} else Ok(OpenSession)` (reuse the structural
   reject — **no new `Reject` variant**), encode writes nothing. The box is the sole session allocator
   (§2); the verb carries no parameters — it instructs the box to increment+open, and the new id arrives
   by emission via the 005 L1 envelope `session_id`. Lands in a new `src/control.rs` (the control block's
   home + the seed of the `control` feature). `IMPLEMENTED` 20 → 21.
2. **Feature-gating by service** — per-service cargo features `control` / `terminal` / `file` / `net`,
   mapping to the verb blocks (0x0_/0x1_/0x2_/0x3_) + the service modules + the §7 consumer surfaces. Each
   consumer compiles only its slice; **default = all** (a plain build and the self-validation loop get the
   whole protocol; consumers opt down via `default-features=false` + their slice).

**This sprint does NOT own** (named, with its owner):

- **`key` 0x12** — the last verb, blocked on the operator-reserved `>0x10FFFF` named-key keysym table; a
  follow-on sprint once the table is pinned.
- **The session-allocation logic** (increment + emit) — the **box's** (the consumer), not the crate; the
  crate's `open-session` is purely the empty-body verb.
- **Gating the integration test files** (`tests/*.rs`) — they reference gated types and stay all-features
  (see Method); 006 does not gate them.
- **Speculative gating of `segment`/`errors`** — kept core (see Decided Forks); revisit only if a consumer
  without both file and net ever emerges.

## Problem

The crate is monolithic: every consumer links the whole protocol. The trusted **proxy** (which deep-parses
only net + file addressing and relays display/input inert — §7) currently compiles the entire terminal and
input grammar it never uses, enlarging its auditable surface against the spine's minimize-the-trusted-
surface posture. And the control block has one unbuilt verb, `open-session` 0x00, the session-allocation
seed that pairs with the `session_id` the 005 envelope pinned.

## Goal

`open-session` decodes as the empty-body unit verb (reusing `TrailingBytes`). The crate gates by service so
the proxy compiles `file`+`net` *without* the terminal grammar (a smaller trusted surface), the head
compiles `control`+`terminal`+`file`, and the box compiles all. The full self-validation loop still
exercises the whole protocol at all-features; each consumer slice builds clean and its per-slice coherence
(its gated `IMPLEMENTED` matches its gated dispatch arms) holds. The five protected surfaces stay
byte-identical.

## Final State

- `src/control.rs`: `OpenSession` (unit) + its `Wire` impl; gated `#[cfg(feature = "control")]` at the
  module declaration.
- `verb.rs` (core): `IMPLEMENTED` becomes a `&[u32]` slice with `#[cfg(feature = …)]` on its **elements**
  (all-features → 21; a gated build → the subset). `CATALOG` stays all-22.
- `message.rs` (core): the `Message` enum + `decode_message` dispatch; the per-service arms carry
  `#[cfg(feature = …)]` (a gated-out verb falls to the catch-all → `UnsupportedVerb`).
- `Cargo.toml`: `[features]` `control`/`terminal`/`file`/`net`, `default = ["control","terminal","file","net"]`.
- Per-service module declarations (`lib.rs`/`mod.rs`) carry the cfg; the **module contents are untouched**.
- The five protected surfaces — segment/cell/display/dimensions/errors — byte-identical.

## In Scope — ordered slices (review seams)

The developer produces the **6A and 6B pre-carve manifests** and **stops for review before each carve**.

- **6A — `open-session`.** `control.rs` + the `OpenSession` unit verb + its `Wire`; the dispatch arm 0x00;
  `IMPLEMENTED` 20 → 21 (still a fixed form here, or moved to `&[u32]` in 6B — dev's call). *Gates:*
  round-trip (empty body), the `TrailingBytes` reject, the live-path (an `OpenSession` `TrailingBytes`
  observable from `decode_message`), `==21` coverage. Trivial recurrence.
- **6B — feature-gating.** `Cargo.toml [features]`; the cfg points (module declarations, dispatch arms,
  `IMPLEMENTED` → `&[u32]` per-element cfg); the **build matrix** + the **per-slice coherence**. *Gates:*
  the build-matrix gate (below). The genuinely-new build-matrix work.

## Non-Goals

- `key` 0x12 / the keysym table (follow-on); the box's session-allocation logic.
- **Content-level cfg** — a cfg *inside* a module that changes its decode behavior per feature. The cfg is
  **declaration-level only** (whole modules + dispatch arms + the `IMPLEMENTED` set). A content-level cfg
  is a REJECT (it would break the canonical-validates-the-slices property — see Method).
- Gating `tests/*.rs`; speculatively gating `segment`/`errors`.
- Editing any service-module **content** (anchored or not) — 006 edits only the declaration/wiring files
  (`lib.rs`/`mod.rs`/`message.rs`/`verb.rs`/`Cargo.toml`) + the new `control.rs`.

## Starting Surface

006 is additive + cfg-wiring. New: `src/control.rs`. Touches (declaration/wiring only): `Cargo.toml`
(`[features]`), `lib.rs`/`mod.rs` (cfg on the `pub mod` declarations), `message.rs` (the enum + dispatch
arms, cfg'd), `verb.rs` (`IMPLEMENTED` → `&[u32]` per-element cfg; +the 0x00 entry). The **five anchors**
(segment b3275467 / cell 8bf962e2 / display a24b83a2 / dimensions 7e339e0c / errors a0893df6) stay
byte-identical. (The 005 commit message said "six" but listed these five — a miscount; the count is FIVE.)

Inventory shape: `feature | modules gated | dispatch arms gated | IMPLEMENTED elements | consumer slices
that enable it`, plus the per-consumer build matrix.

## Method

Inherits the established method. The genuinely-new discipline for feature-gating:

- **Declaration-level cfg only — this is what makes the all-features validation sufficient for the slices.**
  The cfg includes/excludes whole modules, dispatch arms, and `IMPLEMENTED` elements — it **never** changes
  a module's decode behavior per feature. So every verb's decode behavior is **feature-invariant**: the
  same code is included or excluded, never altered. Therefore the all-features tests/vectors/fuzz validate
  the slice behavior too (a slice runs a strict subset of the same code). A content-level cfg would break
  this — each feature-set would need its own behavioral re-validation, and a slice could behave differently
  than the canonical. (The `IMPLEMENTED` `&[u32]` per-element cfg is declaration-level *metadata* — the set
  of implemented verbs per slice — not a decode-behavior change.)
- **The build matrix is the no-cross-leak proof.** A gated-out service's modules are excluded by their
  `#[cfg]` declarations; a cross-reference (e.g. `file.rs` touching a `net` type) would **fail** the
  file-only build. So each slice building clean *proves* no service leaks across its gate. **The
  service-alone builds are the make-or-break of the no-leak proof, NOT the consumer slices:** the consumer
  slices have services *co-present* (e.g. `proxy=file,net` has both), so a leak between two co-present
  services compiles clean and is missed — no consumer slice has net *without* file, so a `net→file` leak
  escapes. Only **each service alone** (control-only / terminal-only / file-only / net-only) + `core-only`
  fails on any cross-reference out of that service (net-only catches `net→file`; terminal-only catches
  `terminal→file`). So the matrix MUST include each service alone + core-only (the no-leak proof) **and**
  the consumer slices (the real-world configs) — both targets. A no-leak gate running only the consumer
  slices is a REJECT (it cannot catch a co-present leak).
- **Build-clean is necessary but NOT sufficient — run the per-slice coherence (the 6B make-or-break).** A
  cfg-coherence bug (`IMPLEMENTED` includes a verb whose dispatch arm is gated out, or vice versa) compiles
  clean but breaks coherence. The coherence (`IMPLEMENTED` == the dispatch arms, per slice; + a gated-out
  verb → `UnsupportedVerb`) lives as **lib-unit tests** in `src/` and is **run per feature-set** via
  `cargo test --lib --no-default-features --features <slice>` — they compile per-slice (referencing only
  core + the cfg'd-in surface). This extends the source-anchored `==` to per-feature-set and catches the
  mismatch build-clean misses (incl. an element-cfg/arm-cfg typo — a verb whose `IMPLEMENTED` element is
  `feature="terminal"` but whose arm is `feature="file"`). **The coherence test must reference only CORE
  symbols** (`verb::IMPLEMENTED`, `verb::CATALOG`, `decode_message`, `Reject::UnsupportedVerb`) and must
  **never construct or name a gated type** (a `Mouse`, a `NetPayload`, …) — or it won't compile on a slice
  that excludes that type. (Reconciles the dev's lib-build-only lean with review's per-slice-run
  requirement: lib-unit coherence runs per slice; the integration tests/vectors/fuzz stay all-features.)
- **The security payoff (G6).** The proxy's slice (`file`+`net`, no `terminal`) links *less* code: the
  terminal/input decoders cannot be a vulnerability surface in the trusted proxy. The no-cross-leak makes
  the shrink real (the gated-out code is genuinely not linked). Gating is a security feature here, not
  packaging. A gated-out verb is `UnsupportedVerb` by construction (law 5).
- **Anchors via declarations.** The cfg goes on the `pub mod` declaration line, never inside the file; the
  five anchored files stay byte-identical.

## Acceptance

All **001–005 gates carry forward** at **all-features** (the canonical self-validation). New/refined:

- **`open-session` (6A).** Round-trip (`decode(encode(OpenSession)) == OpenSession`; the wire is just
  `[verb 0x00]`, 4 bytes); the empty-body golden vector + a `TrailingBytes` reject vector
  (`[0x00,0,0,0,0xXX] → TrailingBytes{1}`); the live-path (`OpenSession` `TrailingBytes` from
  `decode_message`); `==21` coverage. No new `Reject` variant.
- **All-features = canonical (unchanged).** `IMPLEMENTED` = 21, all vectors, both fuzz targets, G0–G6
  green on the full crate.
- **The build matrix (6B).** Each consumer slice builds clean — lib, **both targets**, no warnings:
  `proxy = file,net` / `head = control,terminal,file` / `box = all` / a `core-only` minimal. The proxy
  slice **excludes `terminal`** (the trusted-surface shrink). Build-clean *is* the no-cross-leak proof.
- **The per-slice coherence run (the 6B make-or-break).** Per feature-set,
  `cargo test --lib --no-default-features --features <slice>` runs the lib-unit coherence: the cfg'd
  `IMPLEMENTED` == the cfg'd dispatch arms, and a gated-out verb → `UnsupportedVerb`. Not just `cargo build`.
- **No new fuzz/corpus.** `open-session`'s empty-body decode is covered by the existing decode fuzz (a 0x00
  frame → `TrailingBytes`); feature-gating is cfg-only. The committed corpora (decode 401, reassemble 532)
  stay byte-identical; both fuzz targets run all-features.
- **Behavior preservation (content-hash).** The five anchors byte-identical; **no service-module content
  edited** (006 edits only declaration/wiring files + `control.rs`). A changed anchor = REJECT.

## Decided Forks (lead rulings — engineering, co-formulated; dev + review concur)

1. **`open-session` 0x00** — empty-body unit verb (above), in new `src/control.rs`. No new `Reject`.
2. **Granularity = per-service** (`control`/`terminal`/`file`/`net`) → verb blocks + modules + §7 surfaces.
   Per-direction rejected (direction is doc-not-enforcement; the box spans all); all-in rejected (no
   surface minimization).
3. **Core vs gated.** *Core:* `verb` (catalog + consts + `IMPLEMENTED`), `reject`, the `Wire` trait,
   `envelope` (universal routing), `message` (enum + dispatch; per-service arms cfg'd), `segment`, `errors`.
   *Gated:* `control` → `control.rs`; `terminal` → `terminal/*` + `wire::decode_record_tail`; `file` →
   `file.rs` + `list_entry.rs`; `net` → `net.rs` + `reassembler.rs`. **`reassembler` is net-gated** (sole
   consumer is net-payload; the head skips it — the clean L1 split: `envelope` = core/route-everything,
   `reassembler` = net/reassemble-only). *(net-gating is correct while the head's display-repaint
   reassembly — library §5's other exception — stays unbuilt; if that is ever built the reassembler gains
   a terminal consumer and the gate must be revisited (core, or terminal+net). A note for that sprint, not
   a 006 concern.)* **`errors` + `segment` stay core** — every real consumer has file
   or net (so needs both); gating them buys nothing real, and per the don't-gate-speculatively lesson we
   don't gate ahead of a consumer that forces it.
4. **`IMPLEMENTED` → `&[u32]`** with `#[cfg]` on its elements (a fixed `[u32;21]` can't hold cfg'd
   elements); all-features → 21, gated → the subset; `CATALOG` stays all-22. In `verb.rs` (core;
   declaration-level metadata, not decode behavior).
5. **default = all** — plain build + the self-validation loop get the whole protocol; consumers opt down.
6. **Per-slice coherence = lib-unit tests** (in `src/`), run via `cargo test --lib --features <slice>`; the
   integration tests/vectors/fuzz stay all-features. (Reconciles lib-build-only vs per-slice-run.)
7. **Anchors = FIVE** (segment/cell/display/dimensions/errors); the 005 "six" was a miscount, corrected
   here. The binding "no service-module content edited" covers the unanchored modules.

## Escalations (operator-reserved — routed via the PM)

- **None gating 006.** Feature-gating granularity is the team's per library §8 (recorded here for the
  operator's review at green-light). Verb-ids / the L1 envelope layout / `REASSEMBLY_CAP` stay provisional.
- **`key` 0x12 — the keysym table** is the one operator-reserved item blocking the final 22/22; 006 is the
  **last unblocked structural sprint**, so once it closes, the keysym table is the sole gate to completion.

## Review Questions

1. Confirm the per-service granularity + the core/gated split + the consumer build matrix (operator may
   review the gating convention at green-light). (Operator/PM — review, not a gate.)
2. Confirm the 6A and 6B creation manifests + seam carves before any code. (Review, at each pre-carve stop.) —
   **Confirmed; sprint accepted across 6A/6B.**

---

## Postmortem — Sprint 006 retrospectives

*Sprint 006 closed 2026-06-24: all gates accepted by independent review across 6A/6B. `open-session` 0x00
(the empty-body control verb) is in, and per-service feature-gating (control/terminal/file/net) is built,
gated, and proven — the trusted proxy's slice carries **no terminal/control grammar** (proven by an
isolated-rlib symbol probe), so the minimize-the-trusted-surface lens is realized structurally. This was
the first sprint whose make-or-break was a **build-matrix / security property**, not a decoder. The
protocol **structure is complete**: all three decoders, the safe/unsafe segment split, every service, the
L1 envelope + bounded reassembler, and per-service gating — across 001–006. **21 of 22** verbs; only `key`
0x12 remains, operator-blocked on the keysym table. Builds on the 001–005 postmortems; durable lessons.*

### Developer (`zero.proto-dev`)

**What moved the plan.** (1) *Feature-gating IS a security feature* — the per-service split lets the proxy
compile *without* the terminal grammar, so it cannot be attacked through a cell/display/input decoder it
doesn't contain (surface-min = attack-surface-min, G6); proven by the isolated-rlib probe (Mouse/Cell/
OpenSession absent in the file,net proxy build; FileChunk/NetPayload present). (2) *Declaration-level cfg
makes the all-features build validate every slice* — the cfg lives only on declarations (mod / re-exports
/ Message variants+arms / IMPLEMENTED elements), never inside decode logic, so the all-features build
compiles the *exact* decode code each slice runs; the slices need only a clean build + a coherence check,
not a re-test. (3) *The service-alone build is the no-leak proof; consumer slices hide cross-leaks* — a
net→file leak compiles clean wherever both are co-present (proxy=file,net), so only the lone-service build
catches it. (4) *A core-only per-slice coherence test catches what build-clean misses* — two cfg'd surfaces
that must agree (IMPLEMENTED metadata vs the dispatch arms) aren't checked by a clean build; the lib-unit
test naming only core symbols asserts their agreement per slice.

**What failed / surprised.** The empty-enum uninhabited-type guard — cfg'ing all of an enum's variants
makes it uninhabited in some build, but rustc treats `&EmptyEnum` as inhabited (E0004 on `match *self {}`).
The fix: a `cfg(not(any-service))` arm confined to the no-service build — the deref-match proving the place
uninhabited (no panic) for a value-returning match, a non-diverging unit arm where trailing code must stay
reachable.

**What remains.** 21/22 — the protocol structure is complete. Only `key` 0x12, blocked on the `>0x10FFFF`
keysym table (model §11); when pinned, `key` is the final verb → 22/22. It will likely reuse the patterns:
a carry-raw keysym carrier (`>0x10FFFF` carried raw, like Button/MouseKind) + the 004 Modifiers, under the
terminal feature. Beyond: the keysym pinning; then `key`; possibly L1 session multiplexing (the envelope
established `session_id`); and consumer-integration (box/proxy/head linking their slices — the gating
exists *for* those binaries). Provisional: all ids/op-set/codes + envelope layout + REASSEMBLY_CAP; the
feature names are the team's.

### Reviewer (`zero.proto-review`)

**What 006 added to the method.** (1) *Feature-gating is a security gate, and the service-alone builds are
its make-or-break* — when the gate is "no leak across a boundary," the make-or-break is the *isolated*
build (each unit alone), not the realistic composite (which can't fail on a cross-reference between two
co-present sides); a no-leak gate running only the realistic configs is incomplete (REJECT). (2) *The
security property needs a direct artifact probe, not just a green build* — prove "what's linked / what's
absent" against the rlib (nm/symbols), cross-checked against a build that *should* contain it. (3) *Probe
the right artifact — the stale-rlib trap* — `target/debug/deps` holds one rlib per config; "the latest
artifact" can be a preceding test's all-features rebuild (a confident WRONG answer); isolate each slice in
its own `CARGO_TARGET_DIR` and probe *that* one (005's prove-the-run-came-from-this-binary, applied to a
multi-artifact build). (4) *Declaration-level cfg is what makes the all-features run validate the slices*
(a content-level cfg forks behavior and forfeits it — REJECT). (5) *A per-slice coherence check must be
core-symbols-only* so it compiles on the slice it checks; a check under a partial config can only name the
always-present surface, asserting via the core entry point (dispatch), never by constructing a gated value.
(6) *The uninhabited-type guard is a degenerate-config compile artifact* — verify its cfg scope (no-service
only), that it's *off* the dispatch (the catch-all stays core/always-present so no guard masks a verb), and
prefer the uninhabited match over a panic.

**What the `key` 0x12 follow-on inherits.** `key` is the last verb (→ 22/22), operator-blocked on the
`>0x10FFFF` keysym table — a wire-numbers ratification routed via the PM, not a team call; the follow-on
starts only when the table is pinned. `key` is terminal (cfg `terminal`): a new IMPLEMENTED element +
dispatch arm + Message variant + a terminal decoder — the 006 machinery is the template (IMPLEMENTED 21→22,
the per-slice coherence + build matrix re-run, +1 canonical vector, a fuzz consideration if a new decode
shape). Watch the **carry-raw vs reject fork** for an out-of-set keysym (likely carry-raw — an out-of-set
value is still a representable u32 — but the table pin decides; route it, don't settle it). The anchors stay
five; `key` lands in `terminal/input.rs` (content changes — a real decode change, not a declaration-only
edit).

### Lead (`zero.proto-lead`)

**Process lessons.** (1) *When the make-or-break is a property of the build, not a decode, fold the
isolating config into the contract before the carve.* Review's catch — the consumer slices can't prove
no-leak, only the service-alone builds can — went into the Acceptance pre-carve; the gate was complete by
construction, not patched after evidence. (2) *A check that must run per-config names only the
always-present surface and lives where it compiles.* The F reconciliation (per-slice coherence as a
core-only lib-unit test, run via `cargo test --lib --features <slice>`, the integration tests left
all-features) resolved the dev's lib-build-only vs review's per-slice-run without gating `tests/*.rs`. (3)
*Verify a factual count in the record; don't echo it.* I wrote "six anchors" in the 005 commit/postmortem
while listing five — a miscount the dev's retro echoed and review caught; the count is five, corrected in
this contract. The list is authoritative over the word. (4) *A surfaced compile wrinkle (the empty-enum
guard) is routed to review to confirm it's a degenerate-config artifact, not a behavior change* — the dev
flagged it; review validated it off the dispatch path. (5) *Concede to the better-grounded design* recurs
(005's cap; here, review's service-alone matrix and the dev's both-core for segment/errors).

**What's next inherits (process).** The protocol structure is complete; the standing gates + the
declaration-level-cfg discipline + the service-alone no-leak matrix + the core-only per-slice coherence
carry forward; numbers stay provisional. **The keysym table is the sole gate to 22/22** — the one
operator-reserved item, surfaced via the PM; the `key` 0x12 follow-on is unblocked the moment it lands.
Beyond `key`: consumer-integration (the binaries linking their gated slices — the gating exists for them)
and possible L1 session multiplexing on the envelope's `session_id`. The PM sequences once the table lands.
