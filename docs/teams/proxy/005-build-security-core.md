# Sprint 005 — Build the Security-Invariant Core

> Proxy team, sprint 5. The team builds and certifies the security-invariant core as pure
> functions over box bytes, discharging Sprint 4's standing obligation that a design
> certified as a spec is not yet a certified recognizer. Drafted by zero.proxy-lead from
> planning input by zero.proxy-dev and zero.proxy-review (2026-07-19); green-lit by
> zero.proxy-pm.

---

## Scope Boundary

This sprint owns **one new crate** (`proxy/`, flat top-level, no workspace — matching the
repo's existing pattern) containing **five modules**, each a build unit certified against
its accepted Sprint-4 spec:

- **U1** — the filesystem leaf-validator + open-discipline primitives
  (`spec/filesystem-write-path.md`, `spec/leaf-validator.md`).
- **U2** — the address representation consumption + address recognizer
  (`spec/trusted-address.md`, `spec/address-recognizer.md`).
- **U3** — the route matcher / register enforcement (exact-match, deny-by-default, never
  widened at runtime).
- **U4** — the Anthropic request recognizer (`spec/anthropic-recognizer.md`) — the
  largest unit and the site where Sprint 4's build-time obligation is discharged for real.
- **U5** — the Rust-DSL register (operator direction, `approach.md` §6, verified committed
  `afb916d`) — a per-service typed register where an invalid route or tool identity is a
  compile error, consumed by U3 and U4.

This is the project's **first build sprint** — a genuine category shift. Sprints 1–4 gated
*documents* (composition, new mechanism, cross-owner contracts, spec-by-enumeration). This
sprint gates *code* against its own already-accepted spec: **the spec is the oracle; the
code is what's on trial.** The central risk this sprint guards against — sharpened by both
dev and reviewer independently — is a certification gate that *looks* thorough (tests pass,
coverage is high) but doesn't actually exercise the real adversarial boundary: the
code-form of Sprint 2's `O_NOFOLLOW`-final-component-only trap.

**Four headline findings from grounding in the real repo, verified on disk before this
contract was drafted, materially shrinking scope:**

1. The shared safe-alphabet scan is **already built once**, in `proto::scan_safe`
   (`segment.rs:169`, exported `lib.rs:55`). There is no proxy-level shared module to
   build — U1 and U2 both import it as an upstream dependency.
2. U2's "safe address representation" **largely exists** as `proto::NetAddress`
   (`net.rs:60`, `Scheme`/`ReservedScheme`, exported `lib.rs:67`). U2 is mostly
   *consume `NetAddress` + build `assemble()` + the byte-exact matcher* — not a
   from-scratch representation.
3. U4 **reuses `proto::Reassembler::with_capacity_bound()`** (`reassembler.rs:48`, the
   32 MiB cap, reject-before-grow) for the body-size bound. U4 builds only the
   bounded-depth structural reader and the stage logic — not the size cap.
4. **The certification methodology already exists in-repo**, and it is exemplary:
   `proto/tests/segment_alphabet.rs` certifies `scan_safe` by enumeration over all 256
   byte values against an **independent oracle** (the stdlib's own alphanumeric
   classification, not `scan_safe`'s own output — proving `new == old`, not `new ==
   itself`); `proto/tests/trybuild.rs` + `tests/ui/*.stderr` certify a capability split as
   a compile error, paired with a positive case in `segment_rejects.rs` so the negative
   test isn't vacuous. **This sprint adopts these two patterns; it does not invent new
   certification machinery.**

It explicitly does **not** own:

- **Transport/reactor integration or the NIC/directpath seam.** Out of scope until the
  NIC team's API finalizes. The five units are pure functions over box bytes — no I/O
  beyond U1's isolated syscall surface.
- **The memory-safe relays** (filesystem read, cloud response, terminal) — not part of the
  security-invariant core; queued for later.
- **The recognizer's placement decision (L4)** — module vs. isolated service, decided
  later with real size data once U4 exists. This sprint keeps U4 placement-neutral and
  does **not** pre-split it into its own crate or service.
- **Lifecycle wiring.**
- **Re-opening any accepted Sprint-4 spec.** A spec ambiguity that only surfaces once code
  is written (a real risk, named below) routes back through ratification as a spec
  finding — it is never silently resolved as a build-time choice.
- **A workspace `Cargo.toml`.** The lighter path — a standalone crate with a path
  dependency on `proto` — matches the existing flat-crate repo pattern; introducing a
  workspace touches six other crates and is its own deliberate decision, not this
  sprint's default.

## Problem

The security-invariant core is fully specified (Sprint 4: four accepted total-function
proofs) but not yet built. A design that passed review as a spec was never corpus-verified
as code — `anthropic-recognizer.md` itself now carries this exact obligation, written in at
Sprint 4's close: *"a recognizer that passes review as a design but was never
corpus-verified as code has an unproven security core."* This sprint discharges that
obligation, and the equivalent obligation implicit in the other three specs' closure
tables, for real.

The operator has also introduced a new, unspecified element: **U5**, a per-service typed
Rust register, replacing any notion of a serialized (YAML/JSON/TOML) route or tool-identity
config. Grounded in `approach.md` §6 ("The register is Rust, not a config file," committed
`afb916d`), `architecture.md` §8, and `principles.md` §14: a serialized register would need
a parser on the trusted side and a runtime load/validate step whose defect could alter the
tier-1 gate's input; a typed Rust register removes that parser entirely, makes an invalid
route or tool identity a compile error rather than a runtime check, and — because the box
cannot compile code, only send bytes — keeps the register structurally un-authorable by the
box. This is genuinely new ground this sprint originates (the concrete types), grounded in
committed design (the why and the five constraints it must satisfy).

## Goal

Build and certify each of the five units as a pure function (U1's syscall surface excepted)
over box bytes, with every unit's certification independently re-derivable: the accepted
spec is the oracle, the real running code is what's certified, and a synthetic corpus or
compile-fail suite proves closure — never a green test count taken as proof by itself.

## Final State

1. `proxy/Cargo.toml` exists: a flat, standalone crate (no workspace), with
   `proto = { path = "../proto" }` as a real dependency (feature-scoped to `file`/`net`,
   matching `proto`'s own documented surface-minimization), pinned to a specific `proto`
   commit for certification currency.
2. Five modules exist, each importing rather than rebuilding any already-committed `proto`
   primitive (`scan_safe`, `SafeSegment`/`SafeSegmentStack`, `NetAddress`, `Scheme`,
   `ReservedScheme`, `Reassembler`).
3. U1's open-discipline syscall surface is isolated to one named module; U2, U3, and U4 are
   pure functions (no I/O), unit-testable without a filesystem — the pure/impure split does
   not leak.
4. Every unit's certification suite is built from an oracle or corpus **independent of the
   code under test** (never reverse-engineered from the code's own behavior) — checked as
   an explicit property of the certification, not assumed.
5. U4's certification is honestly stated as **class-exhaustive plus boundary/ambiguity
   fuzzing**, never "exhaustive over inputs" — its domain (arbitrary JSON) is infinite;
   claiming input-exhaustiveness there is the certification-gate's own honesty trap.
6. U5 exists as a plain module of typed consts/statics (newtypes with private interiors and
   typed constructors — no proc-macro DSL, no builder-pattern cleverness), verified both
   as unrepresentable-by-construction (a five-part audit, below) *and* as boring/readable
   enough for the operator to hold in one sitting — the two are checked as **distinct,
   both-required** properties, not substitutes for each other.
7. A running budget ledger tracks each unit's real line count as a simplicity signal
   (per Sprint 4's design ruling: a forcing function, not a quota), with leanness scrutiny
   concentrated on U4.
8. Every spec ambiguity that surfaces only once code is written is routed back through
   ratification as a named spec finding, never silently resolved in the build.
9. The sprint document carries role retrospectives; the sprint is committed with only its
   intentional files.

## In Scope — ordered slices

Six slices — a small vocabulary-pinning slice inserted before the parallel fork, to prevent
U2 and U5 diverging on the shared `Route`/`Address`/`ToolIdentity` types they must agree on.

**Slice 1 — U1 (filesystem leaf-validator + open-discipline primitives).**
Smallest real unit; proves the crate home, the `proto` dependency wiring, and the
certification rhythm on a small surface. Two halves: the leaf-validator is a pure function
(`scan_safe` ∘ count) certified by exhaustive sweep against an independent oracle (the
count-boundary points; the per-byte domain is inherited from `proto`'s own
`segment_alphabet.rs`, cited not re-swept); the open-discipline primitives are real
syscalls (`openat2` with the full `RESOLVE_*` mask) and are **not pure** — certified by a
live probe on the deploy kernel (Sprint 2's standing discipline), re-run and confirmed,
never by unit test alone. Gate: full CB1–CB6 (below).

**Slice 2 — pin the shared vocabulary (small, required before the fork).**
The `Route` / `Address` / `ToolIdentity` types that U2 (address), U5 (register), and U3
(matcher) must all agree on — pinned as their own small module before U2 and U5 proceed in
parallel, so neither forks a divergent definition (U5's `Route` must embed/agree with
`proto::NetAddress`'s scheme+stack, not redefine it). Gate: lightweight — type-level
agreement check against `proto::NetAddress`, not a full CB1–CB6 pass (nothing adversarial
to close yet).

**Slice 3a — U2 (address consumption + recognizer).**
Consumes `proto::NetAddress`/`Scheme`/`ReservedScheme` + `scan_safe`; builds `assemble()`
and the byte-exact matcher. Certified by exhaustive sweep (scheme domain: 0 admits, a
reserved sample denies; injectivity as a property test; match-boundary points — case-shadow,
trailing-slash, empty-segment — each an independent-oracle assertion). Gate: full CB1–CB6.

**Slice 3b — U5 (the Rust-DSL register), run in parallel with 3a.**
A plain module of typed consts/statics — `const`-declared routes and tool identities as
ordinary struct literals a reviewer reads top-to-bottom; `Route`/`ToolIdentity` are
newtypes with private interiors and typed constructors, so a raw string never reaches U3/U4
and a malformed entry fails to compile. No proc-macro, no builder cleverness beyond what a
plain struct literal cannot express. Certified in five parts (§ Method) — completeness,
compile-fail corpus, the validating constructor's own total-function proof, the bypass
audit (the sharpest risk — a derived `Deserialize` or an unchecked `From`/`Default` path
that reconstructs the type at runtime without validation), and sound downstream reliance —
**plus** the boring/readable property checked as its own, non-substitutable gate. Gate: full
CB1–CB6, sharpened per U5's negative-proof shape.

**Slice 4 — U3 (route matcher / register enforcement).**
Consumes U2's `Address` and U5's real register (never a mock register). Certified by
exhaustive sweep of the match matrix (in-register-exact admits; absent, case-variant,
prefix, and superset all deny) plus a test that no code path ever adds to the register at
runtime. Gate: full CB1–CB6.

**Slice 5 — U4 (the Anthropic recognizer, largest, last).**
Reuses `proto::Reassembler::with_capacity_bound()` for the body-size bound; builds only the
bounded-depth structural reader and the stage logic (envelope match, top-level whitelist,
`tools[]` per-entry, beta whitelist). This is where Sprint 4's build-time obligation is
discharged: every closure-table row from `anthropic-recognizer.md` §6 (13 rows) and §5's
SA8 rows (duplicate-key, unicode-escape, trailing-data) becomes a corpus case, independently
re-derived from the spec (never from the dev's own tests), run against the real recognizer.
Plus property/fuzz testing for the deny-default complement (an Admit that shouldn't happen)
and the never-panics/terminates invariants. Gate: the full, strictest CB1–CB6 the sprint
runs, since this is the actual worst-case trusted surface.

## Non-Goals

- Do not rebuild `scan_safe`, `NetAddress`, or `Reassembler` — import them as real `proto`
  dependencies. A proxy-level "shared scan module" duplicating `proto::scan_safe` is a
  defect, not a convenience.
- Do not let U1's syscall surface bleed into U2/U3/U4 — the recognizers stay pure functions
  with zero I/O, testable without a filesystem.
- Do not pre-split U4 into its own crate or service — L4's placement call is later, with
  real size data; this sprint's module stays placement-neutral.
- Do not derive a certification oracle or corpus from the code under test — an oracle
  reverse-engineered from the recognizer's own output proves `new == new`, which is vacuous.
  Every certification suite's independence from its subject is a checked property.
- Do not claim U4's certification is "exhaustive over inputs" — its domain is infinite;
  state it honestly as class-exhaustive-over-the-spec plus boundary/ambiguity fuzzing.
- Do not silently choose a design the accepted spec left undetermined (an errno-handling
  policy, a JSON profile boundary case, MAX_DEPTH's exact value beyond "provisional and
  fails safe") — if code is forced to decide something the spec didn't, that is a spec
  finding, routed back through ratification, never a silent build patch.
- Do not reach for a proc-macro or clever builder pattern for U5 "because it would catch
  more errors" — the register is itself part of the operator-reviewed TCB; a clever DSL
  that is less readable is a spine §15 finding even if it is more airtight, because the
  operator must be able to hold the register in a sitting to certify it.
- Do not derive `Deserialize` on U5's validated types (or any other unchecked construction
  path) without confirming it re-validates — a bypass here defeats the entire compile-time
  claim, exactly as the O_NOFOLLOW gap defeated the symlink floor's claim.
- Do not create a workspace `Cargo.toml`. A standalone crate with a path dependency is the
  lighter, lower-risk first step.

## Starting Surface

Audit first, in this order:

1. The repo's existing crate pattern: `benchtools/`, `directpath/`, `driver/`, `proto/`,
   `reactor/`, `transport/` — all flat, path-dependent, no workspace root.
2. `proto/src/{lib.rs, segment.rs, net.rs, reassembler.rs}` — the primitives this sprint
   reuses rather than rebuilds, and their exported surface at `lib.rs`.
3. `proto/tests/{segment_alphabet.rs, trybuild.rs, segment_rejects.rs}` + `tests/ui/` — the
   certification methodology this sprint adopts: exhaustive-sweep-against-independent-
   oracle, and compile-fail-paired-with-a-positive-case.
4. The four accepted Sprint-4 specs: `spec/scan-safe.md`, `spec/leaf-validator.md`,
   `spec/address-recognizer.md`, `spec/anthropic-recognizer.md` — each closure table
   becomes this sprint's certification corpus.
5. `approach.md` §6 (committed `afb916d`), `architecture.md` §8, `principles.md` §14 — the
   grounding for U5's five constraints; the concrete types are this sprint's own to
   originate against them.
6. The Sprint 2/3/4 ratification-packet recommended defaults — build against them; a
   ratification landing later folds in as a delta, not a blocker.

## Method

Sprint 4's SA1–SA8 frame gated a *design*; this sprint gates *code* against that already-
accepted design, crossed with the reviewer's own charter (a settled post-build run, no
mock standing in for the real primitive, binding the whole causal chain from input to
observed effect). Six gates apply per unit:

- **CB1 — Coherence.** Prove the test run is the *current* binary's, not a stale one — a
  build-signature marker and a post-build run window; source-mtime-precedes-run is *not*
  sufficient (a stale binary can pass that check and mask a regression). Checked first,
  every unit, every re-run.
- **CB2 — The spec is the oracle.** Every accepted-spec closure row becomes a corpus case
  against the *real* code. The reviewer re-derives this corpus independently from the
  spec's closure surface — never from the developer's own tests, which carry the author's
  blind spot (Sprint 4's R2 discipline, now applied to test suites). A missing row is a
  REJECT.
- **CB3 — Enumeration coverage, not test count.** The proof is that every enumerated arm is
  exercised and yields the spec's verdict — branch/arm coverage, never line coverage (a
  line can be covered with a branch untested). A spec arm with zero coverage is a REJECT;
  a passing count with an unexercised arm proves nothing.
- **CB4 — Bind the whole chain.** No mock stands in for the real primitive. The un-fakeable
  tells: an Admit forwards the *exact* input bytes (never rewrites — the `cch` coupling); a
  Deny is legible with the spec's stated reason. Assert against the real primitive's output,
  never a hand-computed "expected" that could be wrong the same way the code is.
- **CB5 — Property/fuzz testing for the recursive recognizer (U4).** Beyond the enumerated
  rows: generate random, malformed, deeply nested, and ambiguous input and assert the
  invariants — never panics, always terminates, and deny-default holds (no input outside
  the exact admitted shape ever Admits). Fuzzing the *complement* of the admitted set is the
  discharge instrument for the deny-default the spec specified — the code-form of the
  Sprint-2 `O_NOFOLLOW`/R-IGNORE gap.
- **CB6 — Rust safety.** No panic on any corpus/fuzz input; termination is proven for the
  depth-bounded reader. The unsafe-attribute shape depends on U1's syscall path (a real
  fork, routed — see Review Question 7): if U1 calls `openat2` via raw `libc` (the repo's
  existing house style — `driver`'s crate-root `#![deny(unsafe_code)]` +
  `#![allow(unsafe_code)]` on its one confined module, `directpath`'s `libc::mmap`/`mlock`
  under the same pattern), the `proxy` crate is `#![deny(unsafe_code)]` at the root with
  `#![allow(unsafe_code)]` on U1's syscall module only — U2/U3/U4/U5 inherit `deny` and
  stay unsafe-free. `forbid` is only correct crate-wide if U1 adds no unsafe of its own
  (i.e., calls `openat2` through a safe wrapper crate like `rustix` instead of raw `libc`) —
  `forbid` cannot be locally overridden by an inner `allow` (a hard compile error, E0453),
  so the two options are mutually exclusive, not a wording choice.

**The load-bearing discipline underneath all six gates**: every oracle or corpus must be
*independent* of the code under test. A test whose "expected" is computed from the
recognizer's own logic proves `new == new` — vacuous, and the code-equivalent of the
`O_NOFOLLOW` trap. `proto/tests/segment_alphabet.rs` is the standing model (stdlib oracle,
not `scan_safe`'s own output) and this sprint's certifications are held to the same bar,
checked explicitly, not assumed.

**U5's certification is a distinct shape** — proving a negative about the type system, not
enumerating runtime inputs:
1. **Constructor-audit (completeness)** — enumerate every way to construct the validated
   type (`new`, `From`/`Into`, struct literals with public fields, `Default`, `Deserialize`,
   `unsafe`/`transmute`) and confirm the *only* path is the validating constructor.
2. **Compile-fail corpus (the spot-check)** — `trybuild` cases, independently derived, that
   must fail to compile with the *correct* error, paired with a positive case so the
   negative test is non-vacuous (`proto`'s own `trybuild.rs` + `segment_rejects.rs`
   pairing, adopted directly).
3. **The validating constructor's own total-function proof** — the check itself is
   enumeration-tested (CB2/CB3); a compile-time gate is only as sound as the check it runs.
4. **The bypass audit (the sharpest risk)** — a hidden runtime construction path (most
   classically, a derived `Deserialize` reconstructing the type from data without
   re-validating) defeats the whole compile-time claim. Either the type derives no such
   path, or that path re-validates (a legitimate but *different*, runtime guarantee — never
   sold as the compile-time one). Also audited: public fields, an unchecked `From<raw>`, a
   `Default` that bypasses validation.
5. **Sound downstream reliance** — U3/U4 trusting "valid by type" (no runtime re-check) is
   sound *only if* the bypass audit holds.

**And, distinctly, not a substitute for the above**: the DSL and the register it produces
must be boring and readable enough that the *operator* can hold them in one sitting — spine
§15's "prefer the dumb total construction a human can certify" applied to the DSL's own
construction, not just what it enforces. A clever macro-heavy DSL that passes the five-part
negative proof but produces an unreadable register is still a §15 finding, routed — the
whole point of the register is that the operator certifies it.

## Acceptance

The six gates (CB1–CB6) apply per unit at its slice close, with the following per-unit
emphasis:

- **U1**: CB1–CB4 for the pure leaf-validator; CB1 + a live deploy-kernel probe (Sprint 2's
  standing discipline, re-run and confirmed) for the open-discipline syscalls — not a unit
  test substitute.
- **U2**: CB1–CB4, plus confirmation that `scan_safe`/`NetAddress` are consumed as the
  certified `proto` functions, not a divergent local copy.
- **U5**: the five-part negative proof (above) plus the boring/readable check as a
  *distinct*, non-substitutable gate — REJECT if either fails independently of the other.
- **U3**: CB1–CB4, tested against the *real* U5 register, never a mock register.
- **U4**: the full CB1–CB6, with CB2/CB3/CB5 at maximum strength — this is where Sprint 4's
  standing obligation (the synthetic corpus against the real recognizer) is discharged.
  Certification is stated honestly as class-exhaustive-plus-fuzzed, never
  exhaustive-over-inputs.

**Cross-cutting REJECT conditions, any unit:**
- A certification oracle or corpus reverse-engineered from the code under test (vacuous).
- A mock or stub standing in for the real primitive at the point the verdict is asserted.
- A spec-forced design choice made silently in code rather than routed as a spec finding.
- A stale-binary run (CB1 failure) — a green result whose binary provenance isn't proven.

## Review Questions

1. **Crate structure (ruled by lead, mechanical, confirmed by design via the PM):** one
   flat top-level `proxy` crate, no workspace; five build units as modules, not separate
   crates (matches L14's one-process posture); `proto` a real path-dependency from the
   start, feature-scoped to `file`/`net`.
2. **std vs. `no_std` (ruled, dev's proposal):** the `proxy` crate is `std` (U1's syscalls
   need it), but the syscall surface is isolated to U1's open-discipline module; U2/U3/U4
   stay pure and I/O-free regardless of the crate's overall `std` status.
3. **U5's concrete shape (ruled, converging both teammates' input after one revision):**
   plain typed consts/statics with newtype constructors — teeth from the type system
   (private interiors, no `From<&str>`), not from a proc-macro. JA-1 (which specific tool
   identity strings) stays the operator's separate call; U5 only makes the *set* typed and
   well-formed.
4. **Slice ordering (ruled, dev's one insert accepted):** U1 → [pin shared vocabulary] →
   U2 ∥ U5 → U3 → U4, inserting a small vocabulary-pinning slice before the parallel fork
   to prevent U2 and U5 diverging on shared types.
5. **Budget/leanness tracking (carried from Sprint 4's design ruling):** a running ledger
   tracks each unit's real line count as a simplicity signal, not a quota; leanness scrutiny
   concentrates on U4. The same trip-wire applies: if U4 reaches its genuinely simplest form
   and the whole set still can't be held in a sitting, that is a scope question escalated to
   the PM directly, not a design problem solved internally.
6. **Proto dependency currency (surfaced, tracked not escalated):** the `proxy` crate pins
   `proto` by commit; a `proto` change re-triggers `proxy`-core certification, the same
   currency discipline the specs already carry, now applied to a code dependency.
7. **U1's `openat2` unsafe-attribute path (routed to design via the PM, blocks Slice 1's
   final `#![...(unsafe_code)]` choice, not its start):** two options, mutually exclusive
   (CB6). **Option A** — raw `libc::syscall(SYS_openat2, ...)`, matching the repo's existing
   house style (`driver`'s crate-root `#![deny(unsafe_code)]` + a confined
   `#![allow(unsafe_code)]` module; `directpath`'s `libc::mmap`/`mlock` the same way). The
   `proxy` crate is `deny`-at-root with `allow` on U1's syscall module only; no new
   dependency. **Option B** — a safe syscall wrapper crate (e.g. `rustix`), keeping the
   *entire* `proxy` crate `#![forbid(unsafe_code)]` with zero unsafe of its own, at the cost
   of a new TCB dependency. Dev's lean: Option A (matches committed house style, minimal
   dependency surface, unsafe confined to one named, auditable module) — but Option B has a
   real spine §15/§16 argument (the security core's *own* code is then 100% unsafe-free).
   Routed with dev's lean stated; not settled here.

## Retrospectives

### zero.proxy-review

**What the new shape changed.** Sprints 1–4 gated DOCUMENTS — against source, or against
independently-derived reasoning. This sprint gated CODE against its own already-accepted spec: the
spec is the oracle, the running code is on trial, and Sprint 4's planted obligation — a design that
passed review but was never corpus-verified as code has an unproven core — got discharged for real.
The decisive discipline was mechanical and non-negotiable: **I built and ran everything myself at
every gate, from a fresh build**, so the run was the current binary's by construction (CB1
coherence), never a reported green. I re-ran the live `openat2` probe (U1) on this kernel,
re-derived every corpus INDEPENDENTLY from the spec's closure surface (never the dev's own tests,
which carry the author's blind spot), and re-ran the fuzz. The load-bearing property under all of it
was **oracle-independence** — a test whose "expected" is computed from the code proves `new == new`,
vacuous; the in-repo model (`segment_alphabet.rs`, "new == old, not new == itself") is the bar, and
I checked every certification's oracle was independent of its subject. The two places code forced an
empirical check a spec review could not give — the `openat2` probe (U1) and the const-twin
alphabet+structure cross-check against the real `scan_safe` (U5) — are exactly why this sprint
existed.

**What the CB5 finding on U4 taught: a sound review is not the empirical discharge.** I reviewed the
recognizer in full by enumeration and found it SOUND — deny-default at every branch, correct depth
accounting, complete ambiguity-rejection — and the class-exhaustive corpus closed every SA3/SA8 row.
I rejected it anyway, because CB5's deny-default-COMPLEMENT fuzz was absent. The lesson is sharp: for
the worst-case surface over an INFINITE domain with zero rewrite margin, a sound code review and a
class-exhaustive corpus are NECESSARY but NOT SUFFICIENT. The review certifies what I could
enumerate; the fuzz certifies the complement — the random input that wrongly admits or panics, the
R-IGNORE class of gap a review can miss — and it becomes the permanent regression guard. And the
discipline had to hold under pressure: when a tooling outage blocked my run, I verified the fuzz
DESIGN by reading it but REFUSED to close CB5 on the design alone — the empirical run was the entire
point, and I had just rejected U4 for exactly that, so I held the gate open and waited for the tool
rather than imply a run I had not done. The reviewer's standing is in being right after checking,
not in closing fast.

**The leanness and holdability calls, and what they mean for the next build sprint.** On leanness I
ruled do NOT unify the recognizer's object/array recognize-vs-skip paths: a parameterized walker is
cleverer, couples the paths so one bug hits every site, and harms the reviewability of an
operator-certified TCB — the same C5 boring-over-clever ruling as U5's DSL. Flat-and-readable beats
DRY-and-clever when a human must certify by reading; a mechanical-helper extraction is a legitimate
post-gate simplification only with full re-certification. On holdability I split the question
honestly along the checkable/future-shaped line: the part I own is checkable — the core is
enumerable, at its simplest reviewable form, no un-reviewable bulk (I read every module) — and the
"held in a sitting" THRESHOLD is the operator's (§15, the human who must understand it completely).
So the core at ~660 net, ~30% over the ~500 forcing-function marker, is a PASSING DATA POINT
(enumerable + my-assessment-holdable), surfaced to the PM/operator for their threshold, not a
scope-crisis this sprint resolves. The consequence for the next build sprint is concrete: **the ~500
margin is now spent** — the recognizer is the irreducible bulk, and any further addition to the
security core pushes it past comfortable holdability, so the budget is now a live constraint, not
headroom.

**What the next proxy sprint inherits.**
- *The certified security core as running code* — five units + the imported `scan_safe` — with its
  permanent regression guards: the corpora, the U4 fuzz, the trybuild compile-fails, and the live
  `openat2` probe. A change to any unit re-triggers its certification.
- *The queued out-of-scope seams:* transport/NIC integration (which should consume the post-decode
  `SafeSegmentStack` rather than re-scan raw bytes — the dev's U1 forward-note), the memory-safe
  relays (fs-read, cloud-response, terminal), recognizer placement (L4), and lifecycle wiring. The
  transport-seam sprint is the natural next.
- *The now-real provisional scalars* (`MAX_DEPTH`=32, `MAX_ELEMENTS`=64, the 32 MiB reassembly cap),
  marked for ratification; and RQ7's deny+allow (Option A) vs safe-wrapper-forbid (Option B) fork,
  routed to design.
- *The build-cert disciplines, now battle-tested:* build-and-run-everything-yourself for CB1
  coherence; the corpus derived independently from the spec; oracle-independence as a checked
  property; the empirical fuzz (not just a review) for a recursive surface; the compile-time
  negative-proof for a typed DSL; and the self-located holdability split — own the checkable, route
  the operator's threshold.

### zero.proxy-dev (author)

**What worked, and why it should be kept.** Grounding in the *real repo* before building
reshaped the sprint's scope and de-risked it — the four headline findings (`scan_safe`,
`NetAddress`, `Reassembler` already committed in `proto`; `segment_alphabet.rs` + `trybuild`
as a proven certification template) meant three of five units were *consume-and-certify*, not
build-from-scratch, and the certification methodology was borrowed, not invented. The
vocabulary-pin-before-the-fork (Slice 2) paid off exactly as designed: U2 and U5 built in
parallel and agreed *by construction* (both consume the shared `assemble`/`Route`/
`ToolIdentity`), with zero divergence to reconcile. And the independent-oracle discipline —
every enumeration/corpus asserted against `proto::scan_safe`, the syscall contract, or the
spec, never the code's own output — is what made "certified" mean more than "green."

**What evidence changed the plan (or caught a defect).**
1. *The four repo findings* — reshaped U2 (to `NetAddress` consumption), made `scan_safe` a
   free import (no proxy-level shared module), and handed over the whole certification
   pattern. Durable rule: **read the committed artifacts before scoping a build — the plan
   made from the spec alone over-counts what must be written.**
2. *CB6's `forbid`+`allow` contradiction* — I flagged pre-gate that the contract's literal
   wording can't compile (E0453; `forbid` is non-overridable), which surfaced RQ7 and
   resolved to Option A (raw `libc`, confined in-house `unsafe`). The standing rule that
   fell out — box-reachable code carries confined in-house unsafe, never a third-party
   wrapper; reviewability is the guarantee, not a zero-unsafe-token count — is now in memory.
3. *The const-twin alphabet risk* (U5) — I self-caught that `is_canonical_route`
   re-implements `scan_safe`'s alphabet (because `scan_safe` isn't const-callable) and added
   a 256-byte cross-check; the reviewer then correctly extended it: the validator twins
   `scan_safe`'s *structure* too (empty-segment, traversal, ≥1-alnum), a second surface my
   "one risk" framing under-counted. Rule: **when you re-implement a check for a different
   context, cross-check the WHOLE behavior against the original, not just the obvious half.**
4. *C5 reversed my own lean* — my planning leaned toward a proc-macro DSL for U5; the
   "boring, not clever" ruling reversed it to plain typed consts + a flat const validator,
   which is both more readable and the fit for the operator-reviewed TCB.

**What needed correction.** Two narrow REJECTs, both empirical-layer, no design flaw: U5's
Slice-2 placeholder constructors had to be *replaced* not shadowed (an unchecked path is a
bypass — caught by the reviewer's flag, then by the E0603 bypass-audit case); and U4 needed
the CB5 deny-default fuzz added (the code was correct, the empirical layer was missing). Plus
one self-caught test bug: the row-10 corpus case used a *plain* key where an actually
`\u`-escaped key was needed — measured, not eyeballed, and fixed with a real escape. And U4
came in at 368 net lines (over my ~230-300 estimate); I surfaced the size and the flat-vs-
unify fork honestly rather than hiding it or refactoring the trusted surface pre-gate — the
reviewer confirmed 368 IS the simplest reviewable form (unifying couples two paths so one bug
hits every site — the same boring-over-clever call as U5's DSL).

**What the next proxy sprint inherits.**
(a) **The `proxy` crate exists** — five certified units (`leaf`, `open_discipline`, `vocab`,
`register`, `address`, `route`, `anthropic`), ~660 net logic lines, ~57 test functions
including a live deploy-kernel probe, trybuild compile-fail cases, and a ~70k-iteration
deny-default fuzz as U4's permanent regression guard. `unsafe` is confined to the one
`open_discipline` module (crate root `deny(unsafe_code)`).
(b) **The transport seam is the next integration** (deferred here): wire these pure units to
the committed `reactor`/`transport`/`directpath`. Carry the R1 note — at integration the
leaf-validator should consume the *post-decode* `SafeSegmentStack` (`proto::FileWrite` already
scans at decode) rather than re-scan raw bytes, avoiding a double-scan and a second scan
call-site.
(c) **Two open inputs, not blockers:** JA-1 (the vetted tool-identity strings) pending the
operator; L4 (recognizer placement) still open — but now with real size data (~368 lines) to
decide against, which is exactly what L4 was waiting for.
(d) **Not yet built:** the memory-safe relays (fs read, cloud response, terminal) — off the
security-invariant core, queued.
(e) **Reusable certification patterns** (the sprint's real methodology output):
independent-oracle enumeration, the live-kernel probe, trybuild compile-fail-paired-with-a-
positive, and deny-default-complement fuzz for any recursive recognizer. And the standing
size discipline — count net logic on disk, distinguish counted-vs-estimated, surface the
whole-set number for the PM's sitting-threshold rather than silently passing a quota.

### zero.proxy-lead

**Trusting a real repo-grounding claim before drafting around it kept this sprint honest
from the first message.** Dev's four headline findings — three primitives already built in
`proto`, and the certification methodology itself already exemplified in `proto/tests/` —
materially shrank the sprint's real scope. I verified all four on disk before the contract
existed, not after a teammate asserted them. Had any of the four been wrong, the contract
would have scoped whole units the team didn't need to build, or missed a methodology
already sitting in the repo. **Rule, carried forward from every prior sprint but sharpest
here: a "this already exists" claim is exactly the kind of finding that reshapes a
contract's scope, and it gets the same disk-verification as a "this is stale" claim — both
are premises the plan gets built around, so both get checked before that happens, not
after.**

**A real compile-error catch (CB6, `forbid`+`allow` is E0453) arrived from both teammates
independently, within the same review cycle, and I fixed it once before either had to
re-litigate it with the other.** Dev found it first in their ACK; the reviewer found the
identical issue independently in theirs, arriving moments later. Because I'd already
corrected the contract's CB6 text between the two messages, the reviewer's objection landed
as a confirmation of an already-applied fix rather than a second round-trip. **Rule: when a
concrete, checkable technical claim needs fixing, fix it the moment the first correct report
lands — don't wait to see if a second teammate independently notices the same thing, even
though it's often reassuring when they do.**

**The reviewer's tooling-outage discipline (revealed only in their retrospective) is worth
naming explicitly, because it wasn't visible in the routing messages at the time.** They
held CB5 open through an outage rather than accept "the design looks sound, so the fuzz
would probably pass" as a substitute for actually running it — a quiet, uncelebrated
instance of exactly the discipline this whole loop depends on (a passing design review is
necessary, not sufficient, for a code gate). It didn't surface as a blocker I had to manage;
it surfaced, after the fact, as a retrospective line. **Rule: ask retrospectives what
*didn't* become a visible problem, not only what did — some of the most load-bearing
discipline in this loop is the friction a teammate absorbed quietly rather than escalated.**

**The whole-TCB size question resolved cleanly because the reviewer's own framing (own the
checkable, route the threshold) meant there was nothing left for me to arbitrate.** By the
time the ~660-line figure reached me, the reviewer had already separated "is this
enumerable and at its simplest form" (their call, answered) from "can a human actually hold
it in a sitting" (the operator's threshold, not theirs or mine to certify). I relayed the
data point to the PM exactly as framed, and the operator's answer ("comfortable with the
reviewer's read for now, will review readability at the end of the build phase") confirmed
the framing was right — there was no fork left to route once the reviewer had drawn that
line honestly. **Rule: when a gate's finding straddles "what I can certify" and "what only
the human can decide," insist the report state that split explicitly — it turns a
potentially long escalation into a one-line data point.**

**What the next sprint inherits.** (a) All five build units are certified, real code — the
security-invariant core runs and denies the adversarial boundary, observed and fuzzed, not
merely specified. (b) The reusable build-cert methodology (CB1–CB6, independent-oracle
enumeration, live-kernel probes, trybuild-paired-with-a-positive, deny-default-complement
fuzz) is now proven across five genuinely different unit shapes and ready to reapply to the
transport seam. (c) The ~500-line forcing-function margin is spent at ~660 net lines — any
future addition to the security core is a real budget conversation, not a formality. (d) The
operator's own end-of-build-phase readability review is a known, scheduled checkpoint —
the team should have the whole core in a state that review can actually land against, not
scrambling to explain accumulated shortcuts.
