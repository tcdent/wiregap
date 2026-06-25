# Sprint 010 — L1/L2 Boundary Correction (excise the envelope)

> A **priority correction** sprint: the operator's code review found that Sprint 005's envelope crossed
> the **L1/L2 boundary** — L1 transport responsibilities (framing, session id, continuation flag) were
> absorbed into the L2 crate. **model.md is the authority and does NOT change; the crate conforms back to
> it.** The crate sheds L1: it becomes `[verb][body]` (model §1, one flat layer), the reassembler keeps
> `push(&[u8], continuation: bool)` with the bool delivered **by the transport**, and the session id +
> framing + the continuation flag are the transport's (below the copy boundary). This **unblocks the
> transport team**, which delivers demuxed, per-session, in-order `[verb][body]` + a per-session
> continuation bool to the reassembler.
>
> *The lead's 005 ruling "the crate owns the L1 envelope layout" is dropped* — it treated an §8-open
> decision as a free team choice when model §1/§2/§3 already constrained it.
>
> Team: `proto` (lead `zero.proto-lead`, dev `zero.proto-dev`, review `zero.proto-review`).
> Authority: [`model.md`](../../proto/model.md) §1/§2/§3/§5, laws 1–2. (The designer owns the doc seams
> 9D/9E/9F — net.md/file.md/library.md — and routes boundary questions via the PM.)

---

## Scope Boundary

**This sprint owns** (the 3 crate seams; one coupled slice):

- **10A (designer's 9A) — excise the L1 envelope.** Delete `src/envelope.rs` (Frame, decode_frame,
  encode_frame + its tests); excise the L1-layout block from `verb.rs` (`ENVELOPE_LEN`,
  `FLAG_CONTINUATION_BIT` + the layout comment); excise the `mod envelope` + the `pub use envelope::{…}`
  re-exports from `lib.rs`. Rework the in-crate callers (only tests/fuzz/examples called `decode_frame`)
  to feed `Reassembler::push` directly. **Re-gate `take_prefix`** to `#[cfg(any(feature = "net", feature
  = "terminal"))]` — the excision orphans it from its core consumer (envelope), so it is dead-code in a
  core-only/file-only build under `-D warnings` (its real consumers are net + terminal; file doesn't use
  it).
- **10B (9B) — verify no session id in the crate.** `grep session_id` over the crate → zero. The session
  is the transport's (§2). (`OPEN_SESSION` the verb const + `open-session` the empty-body verb **stay** —
  an L2 verb; the box increments+opens and the new id arrives via the transport's L1 frame, not the crate
  body; confirm its body remains empty.)
- **10C (9C) — continuation is a push parameter, not a crate flag.** `Reassembler::push(&[u8],
  continuation: bool)` stays; the bool is delivered by the transport (§3); the crate defines **no** flag
  field and reads **no** flag. grep-gated.

**This sprint does NOT own** (named, with the reason):

- **model.md** — the authority; it does not change, the crate conforms to it.
- **The doc seams (9D/9E/9F)** — net.md/file.md/library.md corrections are the **designer's**.
- **Any L2 behavior change** — `decode_message`, the reassembler's `push` (reject-before-grow/fail-closed/
  cap), and all 22 verbs stay **byte-identical** (only the L1 layer leaves).
- **Re-debating the operator-ratified** — net is independent unidirectional emissions (no protocol session
  or correlation); completion is the transport session's stream ending (no close verb). Settled.
- **Removing any verb** — IMPLEMENTED stays 22 (the envelope was framing, never a verb).

## Problem

Sprint 005's envelope `[session_id u32][flags u32][L2 [verb][body]]` is a **second framing layer beneath
the verb** — model §1 forbids exactly that ("no second framing layer and no sub-discriminator beneath the
verb"). It put the session id in the crate (§2: the id "arrives in the L1 frame" — the transport's) and
the continuation bit in a crate flag field (§3: the continuation bit is "the transport's"). The transport
team is blocked: the crate absorbed the framing/session/continuation responsibilities that are theirs.

## Goal

The crate is `[verb][body]`, model-§1-conformant — the L1 layer (framing, session id, continuation flag)
has provably **left** the crate (enforced by absence — the grep-gates). The reassembler reassembles the
**opaque** L2 bytes with the transport's continuation bool. The transport delivers demuxed, per-session,
in-order `[verb][body]` + the continuation bool (the outward deliverable that unblocks the transport team).
The L2 behavior (`decode_message` + the reassembler + 22 verbs) is byte-identical; the five anchors are
byte-identical.

## Final State

- `src/envelope.rs` deleted; `ENVELOPE_LEN`/`FLAG_CONTINUATION_BIT` gone from `verb.rs`; the `mod
  envelope` + re-exports gone from `lib.rs`.
- No `session_id` anywhere in the crate; no flag field/read; `continuation` only the `push` parameter.
- The reassemble fuzz target + corpus + the multi-frame golden vector reworked **push-direct** (no
  `decode_frame`).
- `take_prefix` re-gated `cfg(any(net, terminal))`.
- `decode_message`, the reassembler, all 22 verbs, the decode corpus (410), `REASSEMBLY_CAP`, the 5
  anchors — unchanged. IMPLEMENTED = 22. ~1,632 → ~1,564 auditable lines.

## In Scope — single slice (10A)

The three seams are **coupled**: excising `envelope.rs` (10A) is *what* removes the session-id read (10B)
and the continuation-flag read (10C), so 10B/10C are the **verification grep-gates** of 10A's excision,
not separate code changes; and the excision **forces** the reassemble rework in the same slice (the
frame-based fuzz + the multi-frame vector call `decode_frame` and won't compile once it's gone). So one
slice: **10A = the excision + the `take_prefix` re-gate + the reassemble rework (fuzz/corpus/vector →
push-direct)**, with the 10B/10C grep-gates + the model-conformance + the L2-preserved checks folded into
its acceptance. The dev produces a 10A pre-carve manifest (the exact excision sites + the reworked
fuzz/corpus/vector + the whole-crate grep-gates) and **stops for review.**

## Non-Goals

model.md change; the doc seams (designer's); any L2 behavior change; re-debating the operator-ratified
(net unidirectional / no close verb); removing a verb.

## Starting Surface

The references audit: `src/envelope.rs` (delete), `verb.rs` (L1 consts), `lib.rs` (mod + re-exports),
`tests/net.rs` + `tests/vectors.rs` (rework the decode_frame/encode_frame calls → push), `fuzz/
fuzz_targets/reassemble.rs` (rework), `fuzz/corpus/reassemble/*` (regenerate), `examples/replay_corpus.rs`
(rework the reassemble mode); the `take_prefix` re-gate (`wire.rs`). The five anchors (segment b3275467 /
cell 8bf962e2 / display a24b83a2 / dimensions 7e339e0c / errors a0893df6) stay byte-identical (the envelope
was never an anchor). `OPEN_SESSION`/`open-session` (empty body) stay.

## Method

- **model.md is the authority; conform to it.** §1 (one flat layer, no second framing layer beneath the
  verb), §2 (the session id is the L1/transport frame's), §3 (the continuation bit is the transport's, the
  reassembly opaque). The crate does not redefine these — it sheds them.
- **The boundary is enforced by ABSENCE — the whole-crate grep-gates are the make-or-break.** Over
  `src/ + tests/ + fuzz/ + examples/`, each yields zero: no `mod envelope`/`decode_frame`/`encode_frame`/
  `struct Frame`/`use envelope`; no `ENVELOPE_LEN`/`FLAG_CONTINUATION_BIT`; no `session_id`; no `flags`
  u32 read / continuation **flag** read (continuation matches only the `Reassembler::push` param in
  `reassembler.rs`). `OPEN_SESSION` (the verb const) is the one allowed `*session*` survivor.
- **DRIVE the excision by the gate's own greps — including comments — so the gate yields zero by
  construction, not by a hand-list that can miss one.** The grep-gates match **doc comments** too: the
  pre-carve review found 3 stale references the hand audit missed — `control.rs:5-6/:15` ("the new session
  id … arrives on the L1 envelope" / "rides the envelope"), `wire.rs:34` (`take_prefix`'s doc "Core:
  envelope (every slice) consumes it" — now false), `reassembler.rs:17` ("proxy keys `session_id` →
  Reassembler"). Reword each (semantics unchanged — open-session carries no id; the transport routes each
  session's bytes; `take_prefix`'s consumers are now net + terminal). These are **comment-only** edits;
  the reassembler's code/behavior stays byte-identical (the comment edit doesn't touch it; `reassembler.rs`
  is not an anchor). Run the acceptance greps over the current source and clean **every** hit before the
  carve completes.
- **L2 behavior is byte-identical.** `decode_message` and `Reassembler::push` (reject-before-grow,
  fail-closed, the cap) are **unchanged** — `continuation` was already a `push` parameter, so only the
  *source* of the bool changes (transport, not `decode_frame`). The decode corpus (410), the L2 vectors,
  and the reassembler's **direct** push-tests (cap-boundary/never-ending/production-wiring) stay unchanged.
- **The reassemble rework preserves the TCB-defense make-or-break.** The reassemble fuzz target drives
  `push(chunk, continuation)` **directly** over arbitrary (chunk, bool) sequences at a small cap — and
  must still **reach cap-exceed + never-ending** (the fail-closed exercised; 0 crash/hang/abort/OOM; the
  capping allocator never aborts). The reassemble corpus regenerates as push-shaped seeds (the old
  frame-format seeds no longer apply); cmin-last, on-disk == committed; the reviewer reproduces read-only/
  temp-copy. The multi-frame golden vector reworks to push-direct, hand-authored non-circular
  (`push([0x33,"PO"],true); push(["ST"],true); push(["!"],false) → Complete([0x33]"POST!") →
  decode_message → NetPayload("POST!")`) — the reassembled output byte-identical, only the framing wrapper
  gone.
- **The `take_prefix` re-gate** (the 008-macro-cfg analog): the excision orphans it from core; re-gate to
  `cfg(any(net, terminal))` so the build matrix stays clean.

## Acceptance

- **The grep-gates (the make-or-break).** Over the whole crate, **zero** matches for: framing
  (`mod envelope`/`decode_frame`/`encode_frame`/`struct Frame`/`use envelope`; `src/envelope.rs` absent),
  the L1 consts (`ENVELOPE_LEN`/`FLAG_CONTINUATION_BIT`), `session_id` (the `OPEN_SESSION` verb const is
  the one allowed survivor; open-session's body stays empty), and the continuation **flag** (`flags` u32
  read / a continuation *field*; `continuation` matches only the `push` param). The reviewer runs them.
- **Model conformance.** The crate is `[verb][body]` (§1); the reassembler is opaque + transport-bool
  (§3); no session/correlation; no close verb (CATALOG stays 22).
- **L2 behavior byte-identical.** `decode_message` + `Reassembler::push` unchanged; the decode corpus
  (410) + the L2 vectors + the reassembler's direct push-tests pass unchanged.
- **The reassemble rework.** The push-direct fuzz still reaches cap-exceed + never-ending (fail-closed
  exercised, 0 crash/hang/abort/OOM); the multi-frame vector push-direct (the reassembled output
  byte-identical); the reassemble corpus regenerated, cmin-last, on-disk == committed.
- **The `take_prefix` re-gate** holds: the 8-config build matrix (× 2 targets, `-D warnings`) all-clean,
  **including core-only and file-only** (where `take_prefix` would be dead without the re-gate).
- **Anchors / IMPLEMENTED / matrix.** The 5 anchors byte-identical; IMPLEMENTED = 22, CATALOG = 22; the
  per-slice coherence re-run clean (core shrinks — envelope + the L1 consts gone; the reassembler stays
  net-gated). The new auditable line count reported (~1,564).

## Decided Forks (lead rulings)

1. **Excise the L1 envelope** — model-§1 conformance; the crate is `[verb][body]`, L1 is the transport's.
2. **One coupled slice (10A)** — the excision, the reassemble rework, and the grep-gates are not
   separable (decode_frame's removal breaks its callers immediately).
3. **Re-gate `take_prefix` to `cfg(any(net, terminal))`** — the excision orphans it from core (load-bearing
   for the build matrix; the 008 macro-cfg analog).
4. **Seam labels 10A/10B/10C** (the designer's 9A/9B/9C — relabeled to avoid the 009-9A collision; same
   seams; the designer's doc seams stay 9D/9E/9F).
5. **L2 behavior byte-identical** — `decode_message` + the reassembler's `push` unchanged (continuation
   was always a param); `OPEN_SESSION`/open-session (empty body) stay; IMPLEMENTED = 22.

## Escalations

- **None.** The operator/designer set the scope; model.md is the authority; boundary questions route via
  the PM to the designer. The operator-ratified rulings (net unidirectional / no close verb) are settled.

## Review Questions

1. Confirm the 10A creation manifest (the exact excision sites + the reworked fuzz/corpus/vector + the
   whole-crate grep-gates + the `take_prefix` re-gate) before the carve. (Review.) — **Confirmed;
   pre-carve caught 3 missed doc-comments; 10A accepted.**
2. The whole-crate grep-gates (absence of envelope/session_id/continuation-flag) are the make-or-break,
   run independently at acceptance. (Review.) — **Run; ZERO by construction.**

---

## Postmortem — Sprint 010 retrospectives

*Sprint 010 closed 2026-06-25: the L1/L2 boundary correction. The operator's full code review found that
Sprint 005's envelope crossed the boundary model.md draws — L1 framing, session id, and the continuation
flag were absorbed into the L2 crate, through five sprints under unanimous team consensus. 10A excised
`envelope.rs` + the L1 consts + the re-exports, reworded 3 stale doc-comments, re-gated `take_prefix`, and
reworked the reassemble fuzz/corpus/vector push-direct. The crate is now **model-§1-conformant**:
`[verb][body]`, one flat layer; the L1 frame (session id + continuation) the **transport's**, below the
copy boundary (§2/§4); the reassembler takes the transport's `(opaque bytes, continuation bool)` (§3).
The make-or-break whole-crate grep-gates yield **zero by construction**; L2 is byte-identical
(decode_message + the reassembler + all 22 verbs; the decode corpus 410 unchanged); the TCB-defense
fail-closed is preserved push-direct (review's 1.12M-exec reproduction). **The transport team unblocks.**
124 tests (−3 the excised envelope unit tests); 5 anchors byte-identical; 1,599 auditable lines.*

### Developer (`zero.proto-dev`)

(1) *A normative spec is the AUTHORITY — team consensus, even unanimous and shipped, is not
spec-conformance.* The 005 envelope crossed model §1/§2/§4; the whole team missed it (the dev
co-formulated "crate owns the L1 layout," ruled, accepted, shipped 5 sprints); the operator's review
caught it. When the crate and the spec disagree, the crate conforms back. (In the
`normative-spec-is-the-authority` memory.) (2) *Drive an excision by the gate's own greps — zero by
construction, comments included* (the hand-list missed 3 stale doc-comments; the grep-driven method
catches them; echoes 009's tool-is-ground-truth). (3) *Excising a module orphans what it was the sole
consumer of — re-gate it* (`take_prefix` was core only because envelope consumed it → re-gate
`cfg(any(net, terminal))`; the 008 macro-cfg shape). (4) *A well-factored boundary made the correction
cheap* — the reassembler always took `(opaque bytes, bool)`; the violation was the **framing wrapper**
around a clean core, so excising it was a harness change with the behavior byte-identical. The 005
`push(bytes, bool)` design was right.

### Reviewer (`zero.proto-review`)

(1) *Validate the contract against the model AUTHORITY, not only the change against the contract.* At 005
the reviewer validated the envelope's internal conformance (total / law-11 / reserved-drop) thoroughly —
all correct *against the 005 contract* — but never checked whether crate-owns-L1 conformed to model §1.
The chain of authority runs **change → contract → MODEL → operator**; when a contract makes a *structural*
choice (who owns a layer, where a boundary sits), that choice must be checked against the model, not
ratified by validating its internals — "a well-built violation reads as correct at the contract level."
(2) *Drive-by-greps — the make-or-break is zero by construction, comments included* (a stale doc naming an
excised concept is itself a boundary leak); classify each survivor (the 3 `\bflags\b` benign L2;
`OPEN_SESSION` the one session survivor). (3) *A boundary-conformance gate proves an ABSENCE* (the
responsibilities left the crate — the whole-crate grep yields zero) **plus** the positive
model-conformance check (what remains is `[verb][body]`); run the absence-greps yourself. (4) *When a
violation wraps a correct primitive, excise the wrapper and re-prove the primitive's make-or-break through
the new entry point* (the TCB-defense fail-closed reproduced push-direct, not assumed to carry over).

### Lead (`zero.proto-lead`)

**The error was mine to own.** In 005 I ruled the §8-open "envelope ownership" fork as *crate-owns-L1* —
treating an open decision as a free team choice when model §1 ("no second framing layer beneath the verb"),
§2 (session id in the L1 frame), and §4 (the copy boundary) already constrained it. I even quoted model
§1 ("everything a transport carries … lives below") in other sprints, yet didn't read it against this
decision. Unanimous co-formulation + a clean implementation + reviewer acceptance felt like conformance
and wasn't. The operator's code review — reading the built crate against the authority — is what caught it.

**Process lessons.** (1) *A layer-boundary decision is a spec-check against the authority, not a design
preference* — in co-formulation, any "who owns X (framing/session/identity/ordering)" question is read
against the model, not polled. (2) *The pre-carve gate earns its place on a correction sprint* — despite
the PM's "carve immediately," I held the pre-carve manifest validation; the reviewer's whole-crate grep
caught 3 stale doc-comments that would have failed acceptance, and the fix (drive the excision by the
gate's own greps) made it zero-by-construction. Fast ≠ skip the risk seam. (3) *Scoped commits across team
boundaries* — the 010 commit excludes the designer's library.md/§8 doc seams and the other team's
`docs/nic/` work; the decode corpus (410) + the 5 anchors stay untouched.

**What remains (not codec, not the team's to start).** The 010 commit (the human's word); operator/designer
ratifications (the provisional numbers; the session/continuation layout is no longer the crate's surface at
all — one fewer provisional to ratify); **transport integration** (the now-unblocked other team owns the
L1 frame, parses it, and hands the crate `(L2 bytes, continuation bool)`).
