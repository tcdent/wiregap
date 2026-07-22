# Sprint 007 — Net-Payload Continuation Flag (Task 11)

> Proxy team, sprint 7. A small, operator-ratified fix to the net-payload reassembly gap
> Sprint 6 found: no transport signal exists to mark the last chunk of a multi-frame
> request body. Drafted by zero.proxy-lead from planning input by zero.proxy-dev and
> zero.proxy-review (2026-07-21); scope, mechanism, and placement all ratified by the PM/
> operator before this document was written — this sprint records the settled design and
> gates its build, not a fresh planning round.

---

## Scope Boundary

This sprint owns three things, all in the `proto` crate plus the now-unblockable wiring in
our own `proxy::dispatch`:

- **`proto::NetPayload`** converts from a one-line `verbatim_verb!` macro to a hand-written
  struct + `Wire` impl (mirroring the existing `NetAddress` pattern): a per-chunk message
  carrying a 1-byte fixed-width continuation flag as its **header prefix**, followed by the
  verbatim content tail.
- **`docs/proto/net.md` §2** is corrected to specify this body shape and retire the false
  "transport continuation signal" language (no such transport field exists — confirmed
  Sprint 6).
- **`proxy::dispatch`** gains a per-session `Reassembler`, wiring the previously-unbuildable
  `net-payload` → reassemble → `anthropic::recognize` path.

It explicitly does **not** own:

- **Any change to `proto::Reassembler`'s own code or unit tests.** Verified during
  planning: the reassembler already took the right shape (`push(chunk, continuation:
  bool)`); the only defect was *where* `continuation` was documented as coming from (a
  transport signal that doesn't exist). The fix is doc-only there. **Precision, caught
  during Slice 1**: this is zero churn for `reassembler.rs`'s *own* unit tests (they push
  generic bytes and don't care about the flag's source) — it does *not* mean zero churn
  overall. The per-chunk header-flag design (Final State #4) mechanically forces a rewrite
  of the net-payload *flow* integration tests elsewhere in `proto` (`net.rs`'s
  reassemble-then-decode-once test, `vectors.rs`'s golden byte vectors, the round-trip
  test) — those modeled the old whole-buffer-decode-once flow, which the ratified per-chunk
  design has already superseded. Not a fork (Final State #4 already settles it), just a
  more precise scope than "zero test churn" implied.
- **Any transport crate change.** Confirmed proto-only by the PM's brief and unaffected by
  anything found during planning.
- **The recognizer-envelope sourcing question** (how `anthropic::recognize`'s `Request`
  gets its verb/domain/path/betas from the session's matched route, beyond the
  reassembled body) — noted as a thin follow-on, not this sprint's to settle or guess at.

## Problem

Sprint 6 found that `net-payload` multi-frame reassembly cannot be wired because no
continuation signal exists anywhere in the committed system to mark a chunk as the last
one. The operator ratified a fix (Task 11): append a signal to the final chunk. Planning
surfaced a real, load-bearing ambiguity in that brief before any code was written —
independently caught by both dev and reviewer.

**The ambiguity, and why it mattered**: `net-payload` carries the box's arbitrary request
body — no byte value is guaranteed absent from legitimate content. A signal **searched for
in content** (present only on the final chunk, detected by matching a value) is unsound: a
non-final chunk that happens to end in that value would be wrongly read as final
(silently truncating a legitimate request), and the only fix — escaping — would reopen
the exact two-parser parse-ambiguity problem Sprint 5 built an entire gate (SA8) to close
for the Anthropic recognizer. This also violates `model.md`'s own documented boundary
convention: "a boundary comes from... a fixed width... never a count the box authored";
"the delimiter never meets unvalidated content." There is no existing precedent in `proto`
for a content-scanned terminal value on arbitrary bytes.

**The resolution, ratified**: a fixed-width, **structural** continuation flag — one byte,
always at a known position, never searched for in content, so collision with box-authored
content is impossible by construction (the same principle behind every other design
decision this project has made: traversal unsayable, not caught). The PM refined the
placement once further, from an initial suffix-on-the-payload framing to a proper
**message-header prefix field** — which turns out to fit `model.md`'s own documented body
shape (`[fixed-width prefix?][one frame-bounded tail]`) exactly, the same shape every
other verb already uses, and — a genuine bonus found during the redesign — this placement
means `proto::Reassembler` needs no code change at all, only a documentation fix.

## Goal

Wire net-payload reassembly end-to-end: a multi-frame request body accumulates correctly
via a sound, structural per-chunk signal, and reaches the Anthropic recognizer as one
complete, byte-faithful buffer — the same buffer that is (eventually) forwarded upstream.

## Final State

1. `proto::NetPayload` is a struct `{ more: bool, content: Vec<u8> }` with a `Wire` impl:
   encode writes a 1-byte flag (`0x00` = more chunks follow, nonzero = final) then the
   content verbatim; decode splits the fixed 1-byte prefix (an empty body is
   `Reject::ShortBody`) and the rest is content.
2. `docs/proto/net.md` §2 documents this shape as the normative wire contract; the false
   "transport continuation signal" language is removed.
3. `proto::Reassembler`'s doc comment is corrected to state the continuation signal's real
   source (the decoded `NetPayload` header, not the transport) — no code or test changes.
4. `proxy::dispatch` holds a per-session `Reassembler` (keyed alongside the existing
   binding map, inheriting the same bounded-per-epoch/`close_session`-eviction shape as
   Sprint 6's binding-map residual); on a `net-payload` frame, decodes `NetPayload`, calls
   `Reassembler::push(content, more)`, and on `Complete(body)` hands the body to
   `anthropic::recognize`.
5. The Anthropic recognizer's existing corpus and fuzz suite are re-run through the new
   reassembly path (not fed directly), proving the seam preserves reassembly fidelity —
   the exact bytes reassembled are the exact bytes the recognizer vets, with no split.
6. The sprint document carries role retrospectives; the sprint is committed with only its
   intentional files.

## In Scope — ordered slices

**Slice 1 — the proto-side change.**
`NetPayload`'s struct + `Wire` impl; `net.md` §2's correction; `Reassembler`'s doc-only
fix. Gate: MB1 (coherence), MB2 (correct encode/decode round-trip), the header-decode
totality property (a too-short body is a typed reject, never a panic), header-strip
fidelity (no header byte ever leaks into the reassembled content), and the reassembly-
fidelity property below.

**Slice 2 — the dispatch wiring.**
The per-session `Reassembler`, the `net-payload` → reassemble → `anthropic::recognize`
path. Gate: Sprint 6's arm-by-arm MB1–MB6 bar (this is tier-2 wiring, same discipline as
Sprint 6's dispatch), plus confirmation that the recognizer sees the same buffer that
would be forwarded (CB4, across the seam) and that the Anthropic corpus/fuzz re-run
through this path is unchanged in outcome from the direct-feed baseline.

## Non-Goals

- Do not change `proto::Reassembler`'s code or tests — the fix is documentation-only
  there; if a teammate finds a reason to touch its code, that is a new finding to route,
  not something this sprint's scope covers.
- Do not touch the `transport` crate.
- Do not guess at the recognizer-envelope sourcing question (verb/domain/path/betas
  beyond the body) — note it as a thin follow-on for a later sprint.
- Do not let the operator's ~12-line scope signal push the design toward a content-
  scanned (unsound) mechanism to keep the diff smaller — the signal is a soundness
  tripwire as much as a budget, and was honored by flagging the design (landing at
  ~13–14 lines) to the PM rather than rounding it down. The PM/operator has since
  confirmed satisfaction with the shape at this size.

## Starting Surface

Audit first, in this order:

1. `proto/src/reassembler.rs` (full) — the existing, correctly-shaped `push`, whose only
   defect is documentation.
2. `proto/src/net.rs`'s existing `NetAddress` struct + `Wire` impl — the precedent this
   sprint's `NetPayload` mirrors.
3. `docs/proto/model.md` §6/§11 — the fixed-width-prefix body-shape convention this design
   follows, and the "delimiter never meets unvalidated content" rule it must honor.
4. `docs/proto/net.md` §2 — the section this sprint corrects.
5. `proxy/src/dispatch.rs` — the existing `net-payload` routing (currently
   `Routed::ToServiceHandler(Service::Net)` with no handler), and the existing binding-map
   eviction-hook shape (Sprint 6, `close_session`) the new per-session `Reassembler` map
   mirrors.
6. `proxy/src/anthropic.rs` and its existing corpus/fuzz suite (Sprint 5) — re-run through
   the new path, not modified.

## Method

This is the smallest-shaped sprint yet, but it is not exempt from the team's standing
disciplines — a tightly-scoped, already-decided sprint is exactly where a happy-path test
(one clean chunk) could hide a genuine adversarial edge, and both teammates independently
flagged this as the risk worth holding the line on:

- **The design is MB (memory-safety/correctness) plus one load-bearing adversarial
  property: reassembly fidelity.** Because this primitive's output is the Anthropic
  recognizer's input, the property that matters beyond ordinary correctness is that the
  exact bytes reassembled are the exact bytes forwarded and the exact bytes the recognizer
  vets — one buffer, no split, no silent divergence. The header-field placement makes the
  in-content-collision case moot (there is no content-scanned terminator to smuggle), but
  the corpus still covers: header-decode totality (a too-short chunk is a typed reject,
  never a panic), header-strip fidelity (no header byte leaks into content), per-chunk flag
  semantics (a never-ending "more" stream trips `REASSEMBLY_CAP` and rejects, never hangs
  or grows unbounded), and the seam re-run of the existing Anthropic corpus/fuzz through
  the new path.
- **A code-forced spec ambiguity routes back, never gets silently patched** — the standing
  rule from every prior sprint, reapplied: this sprint's own planning is the proof that
  the discipline holds even under a tight, already-decided brief.
- **The reviewer re-derives the adversarial corpus independently**, not from the dev's
  test list, per the team's standing practice since Sprint 2.

## Acceptance

- **MB1 — Coherence.** The test run is the current binary's.
- **MB2 — Correctness.** `NetPayload` encode/decode round-trips exactly; header-decode is
  total (an empty or too-short body is `Reject::ShortBody`, never a panic); no header byte
  ever appears in reassembled content.
- **MB4/MB5 — Bounded, robust.** A never-ending "more" stream still trips
  `REASSEMBLY_CAP` and rejects (the existing defense, unweakened); no panic on malformed
  input.
- **Reassembly fidelity (the one adversarial property).** REJECT if the bytes the
  recognizer vets ever diverge from the bytes reassembled/forwarded — the reviewer's
  corpus (empty, single-chunk, multi-chunk, a missing terminator hitting the cap, and the
  Anthropic corpus/fuzz re-run through this path) must show byte-identity throughout.
- **MB6 — Rust safety.** No new `unsafe`; `deny(unsafe_code)` crate-root posture
  unaffected.

## Review Questions

*(None outstanding — the mechanism, its placement, and its size were all resolved during
planning, before this document was drafted: the PM ratified the header-prefix shape, and
separately confirmed satisfaction with the ~13–14-line size after it was flagged per the
operator's own scope signal.)*

## Retrospectives

### zero.proxy-dev

**What I built.** Slice 1 converted `proto::NetPayload` from the one-line `verbatim_verb!` macro
to a hand-written struct `{ more: bool, content: Vec<u8> }` + `Wire` impl (mirroring `NetAddress`),
placing a fixed 1-byte continuation flag as the message-header prefix (`0x00` = more, nonzero =
final); corrected `net.md` §2 and the `Reassembler` doc; and rewrote the net-payload *flow*
integration tests to the per-chunk decode-then-concat model. Slice 2 wired `proxy::dispatch` end to
end: a per-session `Reassembler` map sharing the `close_session` eviction hook, a `route_net_payload`
handler (decode chunk → push content+flag → on `Complete` invoke the tier-1 Anthropic recognizer),
three new `Routed` dispositions, and a marked-stub recognizer envelope. Final evidence: proto suite
132 tests, proxy suite 95 tests, both 0-failed; clippy clean; 0 `unsafe`.

**What the evidence changed.** Three planning-time course corrections, none of them mine to make
alone:
- *The disambiguation fork.* The ratified brief said "append a signal to the final chunk," which
  read naturally as a content-scanned terminal value. Because `net-payload` carries the box's
  arbitrary request body, a scanned terminator collides with legitimate content (silent truncation)
  and, if escaped, reopens the SA8 two-parser ambiguity Sprint 5 closed. I flagged this rather than
  build it; the reviewer independently caught the same fork. Resolved by the operator to a
  fixed-width **structural** flag — collision-free by construction, not by scanning.
- *The header-placement refinement.* The PM moved the flag from an initial suffix-on-the-payload
  framing to a proper **message-header prefix field**, which fits `model.md` §6's
  `[fixed-width prefix][one frame-bounded tail]` shape exactly and — a bonus surfaced in the
  redesign — meant `proto::Reassembler` needed no code change, only a doc fix.
- *The size flag-and-approval.* The struct+`Wire` landed at ~13–14 lines, over the operator's
  ~12-line scope signal. I treated the signal as a soundness tripwire, not a budget to round down
  into a content-scanned mechanism, and flagged the overage to the PM, who confirmed satisfaction
  with the shape at that size.

**What needed correction mid-build.** Two gate-driven additions I did not anticipate:
- *Corpus-completeness hold (Slice 1).* The reviewer held Slice-1 close for two required tests: a
  **collision-freedom** guard (content equal to both flag values, including at position 0, must
  round-trip and reassemble byte-identically — the discriminating regression guard for the fork we
  spent a planning round resolving) and a **header-decode totality** negative arm (empty body →
  `ShortBody`, never a panic). Both landed; the reviewer confirmed the collision-freedom test is
  genuinely discriminating (fails under a content-scanning implementation).
- *Overflow-coverage ruling (Slice 2).* I flagged the dispatch-level evict-on-overflow branch as an
  uncovered coverage boundary (the 32 MiB `REASSEMBLY_CAP` is impractical to trip with real chunks)
  and offered options rather than widen the API or ship a heavy test unprompted. The lead ruled a
  `#[cfg(test)]`-gated small-cap constructor — invisible to the production build, cheap to exercise.
  Implemented as a unit test *inside* `src/dispatch.rs` (a `#[cfg(test)]` item is invisible to
  integration tests, which link the lib built without `--test`); the proto reject-before-grow
  mechanism stays cited, not re-proven, so the test confirms only that dispatch propagates the reject
  and evicts the poisoned buffer.

**What the next proxy sprint inherits.**
- **The stub-envelope go-live gate (must be *tracked*, not just documented).** `dispatch`'s
  `stub_envelope_request` pins the sole admit-envelope so the reassembled body is the only variable
  the recognizer seam exercises. It is safe *now* — pre-integration, nothing drives it live — but it
  is **not** production route→envelope wiring and must not ship as-is. The lead has flagged this to
  the PM as a go-live tripwire; the real fix is the recognizer-envelope sourcing work below, which
  must land (and retire the stub) before any live path drives net-payload recognition.
- **The remaining cross-crate asks.** Two named gaps outlive this sprint, both waiting on other
  owners: (1) the **transport session-close/give-up signal** that would *drive* `close_session` —
  the MB4 residual, now covering both the binding map and the (tighter, in-flight-only) reassembler
  map; and (2) the **recognizer-envelope sourcing** question (how `recognize`'s verb/domain/path/
  betas come from the session's matched `Route`), the follow-on that dissolves the stub gate above.
- **JC-1 on the now-acted net-payload path.** `net-payload` is now a message the proxy *acts on*
  (it reaches the tier-1 recognizer), so box-proxy.md §2's JC-1 (uniform reserved-deny on acted
  messages) is in scope for it. The continuation flag is deliberately **total** — any nonzero byte
  maps to "final," leaving no reserved disposition to police — which the corpus makes visible
  (`net_payload_any_nonzero_flag_is_final`). If a later sprint wants reserved flag values (e.g. for
  a future third disposition), that is a JC-1 policy decision for the contract owner, not a silent
  widening of the decode.

### zero.proxy-review

**A decided scope is not a decided mechanism.** The brief was tight and operator-ratified —
append a terminal byte, rework the reassembler, wire the handler. But "append a terminal byte" is
a *what*, and it hid a *how* that decided soundness: how the terminal byte is disambiguated from
content. Both dev and I independently caught the same fork at planning, before a line was written —
`net-payload` carries the box's arbitrary request body, so **no** byte value is content-free; a
value-searched in-band terminator is unsound (a legitimate terminal-value byte truncates a benign
body, and the only fix, escaping, reopens the SA8 two-parser ambiguity Sprint 5 spent a whole gate
closing), while a structural fixed-width marker is collision-free by construction. Routing that fork
(default: structural) rather than treating the brief as fully specified is what kept an unsound
primitive from shipping under a green happy-path. The PM's refinement (header-prefix over suffix)
landed it on `model.md`'s own `[fixed-width prefix][frame-bounded tail]` shape — and, a genuine
bonus, meant the reassembler needed no code change at all.

**`checked=0` proves nothing, even in a small sprint.** Slice 1's code was correct and the design
sound, and I still held it: two of the four adversarial-corpus properties I had named —
collision-freedom and header-decode totality — had *unexercised* arms. Collision-freedom was the
load-bearing miss: no test fed a content *containing* a flag value, yet collision-freedom is the
entire reason the sprint exists. A passing test that never exercises the property is not evidence of
it, and the guard has to be **discriminating** — content packed with flag-value bytes (including at
position 0) that round-trips and reassembles byte-identically, which *fails* under a content-scanning
implementation and *passes* under the positional one. Small scope shrinks the code, not the bar; it
is exactly where a clean single-chunk green hides the adversarial edge.

**Name the deferral precisely; do not let it soften.** The recognizer-envelope sourcing was
correctly scoped out as a follow-on — but the Slice-2 implementation put a stub envelope in the
*production* build, so the production net-payload path recognizes every body under a hardcoded
Anthropic envelope. It opens no gate (deny-default under the real recognizer holds — no wrongful
admit) and is safe now because the whole dispatch is pre-integration. The precision that mattered:
this is not a thin follow-on but a **blocking gate-before-go-live** — before net-payload is ever
driven on a real wire, the envelope must come from the session's matched `Route`, or the stub path
be made structurally unreachable. A finding that is real-but-not-yet-live earns a named tripwire with
a recommended default (structural unreachability), routed to the PM — not a soft note that dissolves.

**What gating an already-decided-scope sprint teaches.** The three moments above are the same shape:
a decided scope removes the *design debate*, not the *review*. The mechanism inside the decided
"what", the corpus behind the passing tests, and the deferral behind the marked stub each still
needed independent scrutiny — and each is where the real risk sat, precisely because the smallness
invited a rubber-stamp. The discipline that held was unchanged from the adversarial sprints:
build-and-run-everything-yourself, re-derive the corpus independently, and check each property is
*exercised* and *discriminating*, not merely green.

**What the next sprint inherits.**
- Net-payload reassembly is wired end-to-end and CB4-across-the-seam is proven (reassembled ==
  forwarded == recognized, across six chunkings plus the U4 corpus/fuzz re-run *through* the path).
  The proto continuation-flag primitive has permanent discriminating collision-freedom + totality
  guards.
- *One new gate-before-production tripwire*, routed to the PM: envelope sourcing (verb/domain/path/
  betas from the session's `Route`) is a **blocking** prerequisite before net-payload goes live;
  preferred resolution is structural unreachability of the stub path, not documentation.
- *The two Sprint-6 cross-crate asks still stand as separate items, different owners:* the transport
  session-close signal (to drive `close_session` eviction) and JC-1 reserved-verb disposition
  (proto-side). The net-payload proto end-marker half is now delivered — do not let the remaining two
  reconverge.
- *The standing disciplines, reconfirmed under tight scope:* route the mechanism fork inside a
  decided scope; corpus properties must be exercised **and** discriminating (`checked=0` proves
  nothing); name a real deferral as a tripwire with a default, not a soft follow-on.

### zero.proxy-lead

**A "small, already-decided" brief still has exactly one open question worth stopping
for, and the discipline is finding it fast, not skipping the search.** The PM's brief
specified the *what* (append a terminal byte) with enough confidence that it would have
been easy to route straight to build. Both teammates independently found the *how* was
genuinely undetermined and unsound in its most literal reading — before either had seen
the other's analysis. That convergence, on a sprint I'd have been tempted to plan lightly
given its size, is the clearest evidence yet that "the scope is decided" and "the design
is safe to build" are different claims, and only the team's habit of checking the second
even when the first feels settled caught it. **Rule: sprint size predicts how much
process a plan needs, not how much scrutiny the design deserves — a five-line fix can
hide a real fork exactly as easily as a five-hundred-line one.**

**Holding the line on the operator's own stated tripwire, rather than rounding it away
myself, was the right call and worth naming as a pattern.** The design landed at ~13–14
lines against an explicit "~12, flag me" instruction. It would have been easy to decide
unilaterally that 14 is close enough to 12 not to bother the PM — especially with a
design I already judged sound. I flagged it anyway, because the instruction's value is
precisely that it doesn't trust my own judgment about what counts as "close enough" on a
threshold the operator set for their own reasons. The PM's answer confirmed satisfaction
immediately — the cost of asking was one message; the cost of guessing wrong would have
been silently overriding a boundary someone else drew on purpose. **Rule: an explicit
numeric or scope tripwire from the human is not advisory just because I have an opinion
about the actual number — honor the letter of it, especially when honoring it costs
almost nothing.**

**The reviewer's precision on the stub-envelope finding is what kept a real hazard from
softening into a checklist item.** "Note it as a follow-on" and "name it as a blocking
go-live gate with a preferred structural fix" read almost the same in a status update, but
they produce very different outcomes months later when someone else picks up the
integration work. I routed it to the PM the same way it was given to me — as a tripwire
with a recommended default, not folded into a residuals list where its urgency would have
been lost. **Rule: when a finding's whole value is its urgency, preserve that urgency
verbatim through every hop it takes — a lead's summary is where precision most often
quietly degrades into blandness, and this is exactly the discipline my own retrospective
should model, not just praise in others'.**

**What the next sprint inherits.** (a) The stub-envelope go-live gate is the sharpest
open item — whoever picks up the transport/NIC integration must resolve it before
net-payload runs live, preferably by structural unreachability. (b) Two distinct,
separately-owned cross-crate asks remain (transport session-close signal; JC-1 reserved
disposition) — track them as two, the same discipline Sprint 6 established. (c) A tight,
already-scoped sprint still gets full planning scrutiny on its one genuine unknown — size
is not a proxy for how much the design needs checking.
