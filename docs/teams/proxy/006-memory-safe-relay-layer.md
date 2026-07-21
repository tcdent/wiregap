# Sprint 006 — The Memory-Safe Relay Layer (Phase 2)

> Proxy team, sprint 6, the first sprint of Phase 2. The team builds the memory-safe
> plumbing that surrounds the certified security core: the wire codec/dispatch layer and
> three tier-2/3 relays. Drafted by zero.proxy-lead from planning input by zero.proxy-dev
> and zero.proxy-review (2026-07-19); green-lit by zero.proxy-pm.

---

## Scope Boundary

This sprint owns **one new tier-2 module** (`proxy::dispatch`, the codec/demux seam) plus
**three relays**, all built as pure or minimally-impure safe Rust, sibling to — never
touching — the certified tier-1 security core from Sprint 5:

- **S1 — codec + dispatch.** `proto`'s L2 codec wired as the wire-protocol parser, the
  `reactor`'s frame delivery, and a verb-demux that routes gated channels into the tier-1
  core and free/relay channels to the relays below.
- **S2 — the filesystem read service** (`file-read` → `file-data`).
- **S3 — the cloud response relay** (`net-response`/`net-error` → box, verbatim).
- **S4 — the terminal relay** (`display`/input, box↔head), built transport-agnostic per
  the operator's flexible-transport direction.

This is the project's **second genuine category shift**, the inverse of the first five
sprints. Sprints 1–5 were all *adversarial* gates: prove a security property holds against
hostile input, where the feared failure is a gate that **does less than claimed** (admits
something it shouldn't). This sprint's code is tier-2/tier-3 — *"parses box bytes but does
not gate… a bug fails the feature, never containment"* (`architecture.md`, tier taxonomy).
Here the feared failure inverts: a relay that quietly **grows gating logic it shouldn't
have** — a content check, a filter, a "just in case" validation — silently becoming an
un-reviewed tier-1 surface. Every prior sprint feared under-doing; this one fears
over-doing.

**Two confirmed findings in our own composition document, found during planning and
verified on disk, both ours to fix directly (not routed externally):**

1. `architecture.md`'s tier-3 "Quarantine writer" row (line 123) still reads "writes
   verbatim bytes... **at an offset** for chunked transfers" — the same stale offset
   phrasing fixed in three other places during Sprint 3 (`filesystem.md` §4,
   `filesystem-write-path.md` §1.2, `model.md` §8's blockquote) but missed in this table
   row. Fix: "...appended in order at the running cursor (the wire carries no offset —
   `file.md` §1)".
2. Ledger **L9** ("config/route-register loader tier placement… open, provisionally tier
   3") is **largely resolved by construction**: Sprint 5's U5 deleted the loader entirely
   — the register is compiled-in `const` TCB source (`proxy::register`, verified: no
   `File`/load/parse/`Deserialize` anywhere), so L9's exact concern ("a register-*load*
   defect alters the tier-1 gate's input") no longer has a loader to apply to. The residual
   — filesystem roots and credential values still needing a runtime mechanism — is a
   *different* question (config mechanics, ledger L12), not a route-register loader
   feeding the gate.

Both ride this sprint's doc-closeout (Slice 5).

It explicitly does **not** own:

- **Any tier-1 security logic.** The three relays and the dispatch layer touch **no**
  tier-1 function except at the dispatch seam itself, which routes gated channels *into*
  the core (`net-address`/`net-payload`/`file-write`+`file-chunk`) — never re-implements or
  duplicates what the core already certifies. A relay that finds itself needing a tier-1
  function is mis-scoped: it is actually a gated channel and belongs in dispatch, not in a
  relay.
- **The real NIC/directpath integration.** Relays are tested against a fake wire (the
  `transport` crate's own test pattern), not the real device — `directpath` remains the
  proxy-NIC fill-in, out of scope until that API finalizes (per Sprint 3's shape-now/
  fill-in split, still standing).
- **Hardening the cloud response relay or the terminal relay "for safety."** Both are
  explicitly free/no-gate directions by design (`approach.md` §5; `terminal.md` §2); adding
  a content check to either is a spine contradiction, not a defensive improvement.
- **Applying the write-path's open-discipline floors to the filesystem read path.** The
  read path is explicitly *fail-safe*, not gated (`filesystem.md` §3; `box-proxy.md`'s
  file-read row) — `O_NOFOLLOW` final-component-only is *correct* there. Over-applying the
  write floors (`RESOLVE_NO_SYMLINKS` and the full mask) to reads is tier-1 logic on a
  fail-safe channel, growing the trusted surface for zero containment gain.
- **Hardcoding the terminal relay's transport as local-only or network-only.** The operator
  has directed the head transport be intentionally flexible ("any protocol that works both
  locally and over the network," captured in `proxy-head.md` during this sprint's
  planning, 2026-07-19) — the relay is built transport-agnostic over a wire seam.

## Problem

The certified security core (Sprint 5) has no surrounding plumbing: no wire codec wired in,
no reactor integration, no way for a gated request to actually reach the core or for a
free-direction response to reach the box. Three tier-2/3 components named in
`architecture.md`'s placement tables are unbuilt: the wire-protocol parser (already
committed as the `proto` crate, needs wiring not building), the filesystem read service, and
the cloud response relay; the terminal relay additionally needed its transport-framing
question resolved before it could be scoped — resolved during this sprint's planning as
intentional flexibility, not a gap.

## Goal

Build the codec/dispatch seam and the three memory-safe relays as correct, robust, non-
gating plumbing around the certified core — reviewed for memory safety, correctness, and
liveness, and explicitly *not* for security logic, since none of this code is supposed to
gate anything.

## Final State

1. `proxy::dispatch` exists: decodes `proto`'s L2 messages from reactor-delivered frames,
   demuxes on the verb, and routes gated channels (`net-address`, `net-payload`,
   `file-write`+`file-chunk`) into the certified tier-1 core, and free/relay channels to
   the three relays below. Wired against a fake wire (transport's test pattern), not the
   real NIC.
2. The filesystem read service exists: streams a named file's bytes back as `file-data`
   frames via `O_NOFOLLOW` through safe `std` (`OpenOptionsExt::custom_flags`) — no new
   `unsafe`. Fails safe by design; carries no gate.
3. The cloud response relay exists: streams `net-response`/`net-error` bytes to the box
   verbatim, frame-by-frame, never reassembled, never inspected.
4. The terminal relay exists: relays `display`/input frames box↔head, decoding only L1/
   session routing and the binding verb (per `library.md` §7 and `proxy-head.md` §1) —
   built transport-agnostic over a wire seam, asserting nothing about the concrete
   transport framing.
5. Every component is memory-safe (deny(unsafe_code) except the fs-read module's confined,
   audited syscall use), fuzzed for robustness (no panic/UB/leak/deadlock on malformed,
   truncated, oversized, or churning input) rather than for adversarial closure, and
   verified to carry **no** content-based decision anywhere in the relay path.
6. The two doc findings (the offset phrasing, L9) are corrected in `architecture.md`.
7. The sprint document carries role retrospectives; the sprint is committed with only its
   intentional files.

## In Scope — ordered slices

**Slice 1 — codec + dispatch (foundational; every relay sits behind it).**
Wire `reactor::Reactor<H,R,S>` (a `Host` + `RxSource` + `TransmitSink`) against a fake wire;
per delivered frame, `proto::decode_message` → `proto::Message`, dispatched on
`Message::verb()`. The session→service binding (first verb binds the session to a service;
all subsequent frames on that session route there) is the sprint's one **security-relevant
wiring invariant** — a mis-binding routes box bytes to the wrong service — gated explicitly
against `box-proxy.md` §1.3, and confirmed to be a pure routing decision by verb, never a
content decision. `proxy::Cargo.toml` adds `reactor` and `transport` as real path
dependencies (Sprint 5 deferred them; this sprint *is* that seam) and expands `proto`'s
feature set beyond `file`/`net` to cover the verbs dispatch must route. **The known
double-scan integration point** (Sprint 5's own forward-looking note: `proto::FileWrite`
already runs `scan_safe` at decode) is resolved here — but resolving it touches tier-1
code, not just the dispatch glue (ruled below, Review Question 2): `proxy::leaf` gains a
new, narrow entry point, `leaf_validate_prescanned(&SafeSegmentStack)` (count-bound only),
and the existing `leaf_validate(&[u8])` is refactored to `scan_safe` then delegate to it —
behavior-preserving, signature unchanged, Sprint 4/5's tests still pass unmodified.
Dispatch calls the new prescanned entry point with the already-decoded stack. The count
bound is a **gate** (it denies `TooManyElements` — tier-1 logic, not memory-safety
plumbing), so it stays a tier-1 function `leaf.rs` exposes — never inlined into the tier-2
dispatch module, which would be the *inward* mirror of the creep this sprint otherwise
guards against. **This one tier-1 addition is gated CB-style**, specifically: CB1
coherence, a count-bound enumeration over the prescanned entry (denies at `MAX_ELEMENTS`,
admits at or below, on `SafeSegmentStack` input directly), and the un-fakeable
type-construction audit — `SafeSegmentStack` is constructible *only* via `scan_safe`
(`segment.rs`), so no box bytes can reach the count-bound un-scanned, meaning dropping
`leaf_validate`'s own re-scan drops no floor. This is a small, targeted re-cert (the
carry is a same-scan — `FileWrite::decode` and `leaf_validate` already ran `scan_safe`
over the identical body — not a full U1 re-run) — **the dispatch glue around it is gated
MB1–MB6**, with special weight on MB3 (the wiring's own security-relevant invariant) and
MB4 (the wiring introduces the sprint's only concurrency). The two bars stay cleanly split
at the seam: tier-1 logic re-certified adversarially, tier-2 glue reviewed for memory
safety.

**Slice 2 — the filesystem read service.**
`proto::{FileRead, FileData, FileError, UnsafeSegmentStack}` plus readable-root
confinement, opened via safe `std` (`O_NOFOLLOW` through `OpenOptionsExt::custom_flags`) —
no new `unsafe`. No dependency on Slice 1's dispatch beyond receiving the routed frame.
Gate: MB1–MB6.

**Slice 3 — the cloud response relay.**
`proto::{NetResponse, NetError}`; upstream bytes stream to the box verbatim, frame-by-
frame, never reassembled by the proxy (`net.md` §3), never inspected. The simplest unit;
no pending dependency. Gate: MB1–MB6.

**Slice 4 — the terminal relay (last; its one-time pending dependency has since landed as
flexibility, not a blocker).**
Relays `display`/input frames box↔head, decoding only the routing information and the
binding verb, per `library.md` §7 and `proxy-head.md` §1 (cited, not re-derived). Built
transport-agnostic over a wire seam — the operator's flexible-transport direction means the
relay must work over *either* a local or a network transport, so nothing in this slice
hardcodes one. `proto` gains the `terminal`/`control` features. Gate: MB1–MB6, plus an
explicit check that no clause or code path asserts the concrete (still-unpinned) transport
framing as settled.

**Slice 5 — integration + doc-closeout.**
The two confirmed doc findings (offset phrasing, L9); any remaining ledger reconciliation;
retrospectives.

## Non-Goals

- Do not add a content-based check, filter, denylist, or "defensive validation" to any
  relay — routing by envelope (session id, verb) is the whole job; a decision based on what
  the content *is* is tier-1 logic that has crept into tier-2/3 code, a mis-tiered REJECT.
- Do not "harden" the cloud response relay or add any content policy to it — the spine
  explicitly discards that worldview for the free direction (`approach.md` §5); a check
  here is not just scope creep, it's a spine contradiction.
- Do not apply the write-path's `RESOLVE_NO_SYMLINKS`/full-mask open discipline to the
  filesystem read path — the read path's `O_NOFOLLOW`-final-component-only is the correct,
  fail-safe bar; importing the write floor here is tier-1 logic for zero gain.
- Do not hardcode the terminal relay to a local-only or network-only transport, and do not
  assert the concrete transport framing as settled anywhere in this sprint's code or
  contract citations — it remains an intentional, operator-directed flexibility.
- Do not drop the fuzzing discipline just because this sprint isn't adversarial — repurpose
  it. The assertion flips from "never wrongly admits" to "never panics, never exhibits UB,
  never leaks, always makes progress" under malformed, truncated, oversized, or churning
  input.
- Do not silently accept a relay's failure mode drifting toward "denied a legitimate request
  because the content looked wrong." A relay's only legitimate failure modes are feature
  failures (wrong bytes, a dropped frame, a crash) — never a content-policy denial.
- If genuine gating logic is ever found to be needed somewhere in this layer, do not smuggle
  it into a relay reviewed only for memory safety — relocate it to the tier-1 core and
  route it through the adversarial-closure bar (Sprint 4/5's discipline), or remove it if,
  on inspection, it isn't actually needed (usually the case, since the box is already
  contained).

## Starting Surface

Audit first, in this order:

1. `docs/proxy/architecture.md`'s tier-2 and tier-3 placement tables in full — this
   sprint's components are named there with their capability-over-box-bytes and grounding
   already stated.
2. The three boundary contracts: `contract/box-proxy.md` (§1.3 for session→service binding,
   §3.2/§3.4 for the routing sets each relay cites), `contract/proxy-head.md` (§0/§1/§3,
   including the just-landed flexible-transport update), `contract/proxy-nic.md` (for
   context only — its fill-in half stays out of scope).
3. The certified tier-1 core, `proxy/src/{leaf,address,route,anthropic,register,vocab,
   open_discipline}.rs` — cited as the seam dispatch routes into, never modified or
   duplicated.
4. `proto/src/{message.rs, file.rs, net.rs}` exports; `reactor/src/{lib.rs, reactor.rs}`
   (`Reactor`/`Host`/`RxSource`/`TransmitSink`); `transport/src/lib.rs`
   (`SessionId`/`SessionTable`/`Delivery`/`Frame`) — the crates this sprint wires together.
5. The two confirmed doc findings (offset phrasing at `architecture.md` line 123; ledger
   L9) — verify independently before treating them as settled.

## Method

This sprint's gates are the inverse of Sprint 5's. Six properties (MB1–MB6, merged from
both teammates' converging frameworks) replace the CB1–CB6 adversarial-closure discipline:

- **MB1 — Coherence** (carries over from CB1 unchanged): the test run is the current
  binary's, proven by a build-signature marker, never a stale-binary green.
- **MB2 — Correctness / relay-fidelity.** The relay moves the *right* bytes to the *right*
  place, unmodified. The oracle is byte-identity (output == input) plus correct routing —
  not an admit/deny oracle. Round-trip/identity tests: feed a frame, assert it emerges
  unmodified at the correct endpoint.
- **MB3 — Tier-fidelity: it does not gate (the sprint's central, inverted gate).** Confirm
  the code contains no content-based decision anywhere in the relay path — no filter, no
  neutralization, no deny-on-content. The unifying tell: a relay routes by *envelope*
  (session id, verb); the moment it branches on what the *content is*, that is tier-1 logic
  that crept in un-reviewed. A relay found to gate is a REJECT-and-relocate: relocate the
  logic to the tier-1 core (reviewed adversarially) if genuinely needed, or remove it
  (usually the case, since the box is already contained).
- **MB4 — Liveness.** No deadlock, livelock, leak, starvation, or unbounded growth — the
  reactor and per-channel handlers introduce concurrency the pure tier-1 functions never
  had. Bounded resources (session table, reassembly cap, buffers); the single-threaded
  reactor (no locks, so no deadlock, but progress must still be verified); RAII/no-fd-leak;
  backpressure that drops or blocks cleanly, never hangs or grows unbounded.
- **MB5 — Memory safety under malformed input.** The no-panic half of Sprint 5's CB5 fuzz
  survives; its deny-default half does not apply (there's nothing to deny-by-default over).
  Fuzz malformed, truncated, oversized, and boundary input; assert no panic, no unbounded
  allocation, always terminates or otherwise makes progress. This is where a silent
  memory-safety bug would hide if happy-path testing were the only bar — the fix is to
  repurpose the fuzzing discipline for robustness, not drop it because the sprint isn't
  adversarial.
- **MB6 — Rust safety** (carries over from CB6): pure safe Rust for all relay logic;
  `deny(unsafe_code)` at the crate root holds; the one confined `allow` module stays
  `open_discipline`'s syscalls — the fs read service's `O_NOFOLLOW` goes through safe `std`
  (`OpenOptionsExt`), adding no new unsafe module.

**Gating the wiring itself** (Slice 1, which has no accepted spec of its own to be an
oracle for) is a hybrid of Sprint 3's composition discipline and MB1–MB4: the *routing/
disposition* oracle is the already-composed boundary contracts (cited, Sprint-3 style — a
wiring that deep-parses `display`, or gates `net` at the relay, or routes a verb to the
wrong service, contradicts a contract exactly as a redefinition-in-code would); the
*concrete glue* (handler dispatch shape, buffer management, session-binding lifecycle) has
no spec oracle and is verified by MB1–MB4 directly.

## Acceptance

MB1–MB6 apply per slice at its close:

- **MB1 — Coherence.** REJECT on a stale-binary run (never proven current).
- **MB2 — Correctness/relay-fidelity.** REJECT if any relay's output diverges from its
  input, or a frame routes to the wrong session/service.
- **MB3 — Tier-fidelity (does not gate).** REJECT + relocate on any content-based
  conditional, filter, "just in case" validation, or neutralization found anywhere in a
  relay or the dispatch glue. Concrete watch list: a conditional on relayed content, any
  transform of relayed bytes (even a "harmless" normalization breaks output==input and, for
  the cloud response, re-imports the confidentiality worldview the spine discards), or a
  relay's failure mode drifting toward "denied on content" rather than "feature failed."
- **MB4 — Liveness.** REJECT on any deadlock, unbounded resource growth, fd/session leak,
  or evidence that one channel can starve another, under load/churn/backpressure testing.
- **MB5 — Memory safety under malformed input.** REJECT on any panic, UB, or hang under
  fuzzed malformed/truncated/oversized/churning input — the fuzz is required, repurposed
  for robustness rather than dropped because this sprint isn't adversarial.
- **MB6 — Rust safety.** REJECT on any new `unsafe` outside the confined
  `open_discipline` module, or a broken `deny(unsafe_code)` crate-root posture.

**Slice 1's additional gate**: the session→service binding is verified against
`box-proxy.md` §1.3 specifically and confirmed to be a pure routing decision by verb, never
influenced by content.

**Slice 4's additional gate**: no clause or code path asserts the concrete (still-flexible)
transport framing as settled; the relay's shape is verified against the currently-committed
`proxy-head.md`, re-pinned at the gate (currency-of-source, Sprint 3's standing discipline).
Additionally: because `proxy-head.md` §0 now names a new residual (non-box network-exposure
when the head is remote), MB5's robustness fuzz for this relay must treat head→box input
frames as potentially network-injected — the same untrusted-shaped fuzz bar as the box
side — not a new gate, just an expanded fuzz source-set for when the transport happens to
be a network hop.

## Review Questions

1. **Dispatch module location (ruled by lead, mechanical):** a new module,
   `proxy::dispatch`, tier-2, clearly a sibling to — never inside — the certified tier-1
   core. The core's *modules* stay exactly as Sprint 5 built them
   (`proxy::{leaf, address, route, anthropic, register, vocab, open_discipline}`); the one
   change this sprint makes to the core's *content* is the narrow, behavior-preserving
   `leaf_validate_prescanned` addition ruled in Review Question 2, not a restructuring.
2. **The double-scan integration point, and its tier-1 addition (ruled by lead, dev's
   proposal approved as scoped):** resolving Sprint 5's own forward-looking note (dispatch
   should consume the post-decode `SafeSegmentStack`, not re-scan raw bytes) requires a new
   tier-1 entry point, `proxy::leaf::leaf_validate_prescanned(&SafeSegmentStack)` (count-
   bound only), with the existing `leaf_validate(&[u8])` refactored to `scan_safe` then
   delegate to it — **behavior-preserving**: unchanged signature, unchanged behavior,
   Sprint 4/5's tests pass unmodified. **In scope for Slice 1**, on dev's correct
   observation that the count bound is tier-1 security logic and does not belong in a
   tier-2 dispatch module (that would itself violate MB3, the gate this sprint adds). Per
   the reviewer's sharpening: the count bound stays a **gate** (`TooManyElements`), so it
   stays in `leaf.rs`, called by dispatch, never inlined into tier-2 code — the addition
   is gated CB-style with three specific pieces: CB1 coherence, a count-bound enumeration
   directly on `SafeSegmentStack` input (denies at `MAX_ELEMENTS`, admits below), and the
   un-fakeable type-construction audit (`SafeSegmentStack` is constructible only via
   `scan_safe`, so dropping `leaf_validate`'s internal re-scan drops no floor) — a small,
   targeted re-cert (the carry is a same-scan), not a full U1 re-run. The dispatch glue
   around it is gated MB1–MB6. The two bars stay cleanly split at the seam.
3. **The two doc findings (ruled by lead, our own composition document, fixed directly):**
   the `architecture.md` line-123 offset phrasing and ledger L9's construction-mooted
   status are both corrected at Slice 5's doc-closeout — no external routing needed, since
   both are findings about our own document and both are already-settled facts (the
   offset-free wire, Sprint 5's compiled-in register) being reconciled into it, not new
   design decisions.
4. **Slice ordering (ruled, both teammates converged):** S1 (codec+dispatch, foundational)
   → S2 (fs read) ∥ S3 (cloud response relay) — both independent, no pending dependency —
   → S4 (terminal relay, sequenced last though its dependency has already landed as
   flexibility, simply because S2/S3 have none) → S5 (integration + doc-closeout).

## Retrospectives

### zero.proxy-review

**The inverted shape, and the evidence it was real.** Sprints 1–5 were all adversarial gates —
prove a security property holds against hostile input, where the feared failure is a gate that does
*less* than claimed. This sprint inverted both axes, and the inversion was concrete, not
rhetorical. The *bar* changed: the correctness oracle became byte-identity / relay-fidelity
(output == input, routed right) instead of admit/deny; the CB5 fuzz survived but its assertion
*flipped* from "never wrongly Admits" to "never panics/UB/leaks, always progresses"; and a whole new
gate appeared — MB3 tier-fidelity (the code must *not* gate), the exact inverse of adversarial
closure. The *risk* changed too: from a gap that admits to creep that gates. The evidence that this
was the right frame showed up as passing tests that would have been nonsense a sprint earlier — the
cloud relay's ESC-sequence relaying *untouched* (the sanitize temptation is the creep), and the
fs-read path deliberately *not* importing the write-path symlink floors (importing them would be
tier-1 logic on a fail-safe channel). One thing did *not* invert: memory-safety on box-reachable
code stays a containment concern even with nothing adversarial in view — a memory bug is the one way
tier-2 plumbing can breach containment — so MB5/MB6 kept the old weight while everything around them
flipped.

**The Slice-1 self-correction, and the method fix it produced.** I accepted Slice-1 dispatch, and
the dev later found a real §1 over-decode: `route()` called `decode_message` unconditionally,
deep-parsing the terminal-service L2 bodies (`display`/`key`/`mouse`) that `proxy-head.md` §1's
decode/relay pin says to relay *opaque*. The humbling part is that it was the exact failure mode I
had named in my own planning input for this sprint ("a wiring that deep-parses `display` contradicts
the contract as a redefinition-in-code would"), and I had both artifacts — the §1 clause and the
`decode_message` call — under my eyes at the gate. The lesson is precise: a green MB suite
(memory-safety + no-panic + the binding invariant) can coexist with a *conformance* defect sitting
in plain sight, because conformance-to-the-composed-contracts is a separate axis from the MB
properties, and re-passing tests do not exercise it. The durable fix, now standing method: at a
**wiring gate, walk each route/relay arm against the decode-vs-relay pin** — confirm every relay
channel is forwarded with *no* body decode — as a first-class step of MB2, weighted the same as the
tier-1 seam and the binding invariant. I applied it at S4: I read every arm and confirmed only
`NET_ADDRESS`/`FILE_WRITE` decode a body, and the new conformance test proved the asymmetry is real
(a malformed `display` body forwards opaque; a malformed `net-address` still deep-parses and
rejects). Re-passing tests are necessary, not sufficient; the arms must be read.

**The MB4 binding-growth catch — liveness gates on stateful code.** The dispatch test first recorded
MB4 green on a false premise: "the dispatcher is a pure function, no concurrency." But the dispatcher
is *stateful* — a per-session binding map that only ever grows — so that green was unearned. The
catch: the map has no eviction path, so it grows `O(sessions-per-epoch)` unbounded, the exact
R-SILENT-MEMBUG this sprint pre-committed to catching, invisible to a happy-path test that binds a
few sessions. The lesson: for stateful tier-2 code, MB4 liveness is not just deadlock-freedom (the
"no concurrency" half is true but partial) — it is **bounded resources over the process lifetime**,
and the gate must test the *state's growth*, not a few calls. The resolution (option B): name the
residual, add the `close_session` eviction hook (structurally ready for a transport session-close
signal that does not exist yet), and a characterization test that *measures* the growth (100
sessions → 100 bindings → evict → 99). A subsidiary lesson came from my own error inside this catch:
I endorsed a "one transport signal closes both cross-crate gaps" consolidation that was wrong —
binding-eviction needs a *transport* session-close signal, net-payload reassembly needs a *proto*
length/end-marker, different layers and owners — and I retracted it the moment the data turned. The
standing being right is in checking, then owning the correction, not in never being challenged.

**What the next proxy sprint inherits.**
- *The four accepted relay-layer units* — dispatch, fs-read, cloud-relay, terminal-relay — as
  memory-safe, correct, non-gating plumbing around the certified core, with the verb-peek dispatch
  now §1-conformant (the proxy runs no terminal/display/input decoder on box bytes).
- *Three named residuals, all hook-ready or tracked, none blocking* — and two of them are
  cross-crate asks the next sprint carries to the PM as **separate** items with **different owners**:
  (1) the MB4 binding-growth awaits a **transport** session-close/give-up signal to drive
  `close_session`; (2) net-payload multi-frame reassembly awaits a **proto**-side length/end-marker;
  (3) JC-1 reserved-verb-byte disposition (proto-side, tracked-not-fixed). Do not let these
  reconverge into one ask — the layer/owner split is the correction that was hard-won here.
- *The provisional scalars* (`READ_CHUNK`, `MAX_ELEMENTS`) stay marked for ratification.
- *The standing review disciplines this sprint added:* arm-by-arm conformance walking at every
  wiring gate (a first-class MB2 step, not an afterthought); robustness-fuzz (assertion: no
  panic/UB/leak/hang) rather than closure-fuzz for tier-2/3; liveness gates on stateful code that
  test the state's growth over time, never accepting a "pure function" framing of stateful code; and
  build-and-run-everything-yourself for MB1 coherence, which caught nothing new here only because it
  is already reflexive.

### zero.proxy-dev (author)

**What worked, and why it should be kept.** The category shift (adversarial gates →
non-gating plumbing) landed cleanly because the MB1–MB6 frame *named the inverted risk*
up front: MB3 "does not gate" was the load-bearing gate all sprint, and every relay's
central proof was that it carries no content decision. Grounding each unit in the real
crates before building (`reactor::pump_all` as the consume-seam, `proto::decode_message`'s
`[verb][body]` format, the `verbatim_verb!` newtypes) meant the wiring was discovered, not
guessed. And the "one verbatim thing in N lines" discipline paid off — the cloud response
relay is 10 lines and the terminal relay 41, each doing exactly one thing.

**What evidence changed the plan (or caught a defect).**
1. *The four planning findings from grounding* — two doc corrections (the quarantine-writer
   offset, L9-mooted-by-U5) and two reuse points (`Reassembler`, the fake-wire test
   pattern). Durable rule, again: **read the committed artifacts and the composition doc's
   own tables before scoping — stale phrasing and already-resolved questions hide there.**
2. *The RQ2 tier-1 addition* — the double-scan resolution forced a small tier-1 function
   (`leaf_validate_prescanned`), whose soundness rests entirely on a type-construction
   audit: `SafeSegmentStack` has one constructor (`scan_safe`). Rule: **when a fast-path
   skips a validated step, the whole soundness is "can the type be built any other way?" —
   prove the constructor discipline (a trybuild bypass case), don't assume it.**
3. *The §1 over-decode finding (the sharpest)* — building the terminal relay per
   `proxy-head.md` §1 revealed that S1's dispatch used `decode_message`, which deep-parses
   *every* body including `Display`'s cell stream — so the proxy was running the head's
   decoder on box bytes, against §1/§7. The fix (verb-peek routing, deep-parse only the
   gated channels) shrank the proxy's box-bytes parse surface to exactly net+file
   addressing. Rule: **a downstream slice is the honest test of an upstream one — S4 could
   not deliver a genuinely opaque relay on top of an over-decoding dispatch, and that
   exposed the S1 gap the S1 gate didn't exercise. Surface it, don't build the wrong thing
   correctly.**

**What failed or needed correction.** Two MB re-gates, both empirical-layer: the MB4
unbounded-binding-map (I recorded MB4 green treating a stateful dispatcher as stateless —
fixed with a named, tested, hook-ready residual mirroring transport's R5); and the S2
whole-file slurp (`read_to_end`, unbounded — fixed by streaming per `READ_CHUNK` + a
regular-file/`O_NONBLOCK` gate against device/FIFO). Plus two self-caught test-construction
errors (a "malformed" net-address that wasn't; over-constructing real `Key`/`Display`
messages when raw verb-keyed frames were both simpler and more faithful to the opaque
relay). And one record-correction: I conflated the two cross-crate gaps ("one signal closes
both") — they have different owners (transport session-close vs. proto length-marker).

**What the next proxy sprint inherits.**
(a) **Three named, hook-ready residuals**, each bounded and mirroring an accepted property:
the dispatcher's `close_session` binding eviction (awaits a **transport** session-close
signal); the net-payload reassembly handler (unbuilt, awaits a **proto**-side net.md
length/end-marker); and both are epoch-bounded meanwhile.
(b) **JC-1 (tier-1 policy)** — reserved verb-byte disposition is not enforced (both decode
paths mask `& 0xff`, dropping the reserved upper bytes unchecked); a nonzero reserved on an
*acted* message should deny. Routed to the lead → likely PM/policy.
(c) **The two cross-team asks** — transport surfaces a session-close signal; proto adds a
net-payload length/end-marker so `Reassembler` can concatenate to length. Both are future
dependencies, not proxy work.
(d) **The relay layer is built** — `proxy::{dispatch, cloud_relay, fs_read, terminal_relay}`
around the certified tier-1 core, all pure safe Rust with `unsafe` confined to
`open_discipline`, tested at the MB (memory-safety/correctness/liveness) bar. Still unwired:
the actual transport/NIC integration (`directpath` fill-in) and the net-payload reassembly
handler (blocked on the proto ask).

### zero.proxy-lead

**Naming the sprint's inversion up front paid for itself immediately, and precisely.** The
contract stated before any code existed that this sprint feared the opposite failure from
every prior one — creep, not a gap — and both teammates independently produced the same
signal for catching it (a content-based decision) before writing a line. That shared,
named frame is what let the fs-read gate go *fast*: when dev deliberately did not import
the write-path's symlink floors, the reviewer could check that decision against a stated
rule rather than reason it out fresh under time pressure. **Rule: when a sprint's failure
mode inverts from the team's habit, say so explicitly in the contract before work starts —
the naming is what lets a teammate recognize the *right* restraint (not adding a check) as
readily as a wrong one.**

**The reviewer's Slice-1 miss, and the way it surfaced, is the clearest evidence this loop
is working as designed — not despite the miss, but because of how it was handled.** The
`proxy-head.md` §1 over-decode sat in code the reviewer had already accepted, under a
property they had personally written into the Sprint-3 contract. It surfaced two slices
later because dev, building the next piece honestly, found the code couldn't do what the
sprint actually needed — and *said so* rather than building around it. The reviewer then
owned the miss without hedging, named the precise mechanism of the failure at their own
gate (MB2's conformance half untested, only its correctness half), and produced a durable
method fix (arm-by-arm conformance walking, promoted to a first-class step) rather than a
one-off apology. Nothing about this required my arbitration — I verified the finding and
ruled on scope, but the finding, the ownership, and the fix all came from the team
directly. **Rule: a self-corrected miss with a named mechanism and a durable fix is a
signal the process is sound, not a defect to manage — the lead's job there is to verify
and route quickly, not to supervise the correction.**

**Routing this sprint's cross-team asks as two, never one, took active vigilance —
including from me.** The transport session-close signal and the proto length/end-marker
look, from a distance, like "the reassembly problem" — and the reviewer's own verdict
briefly reconverged them into one claim before self-retracting. I caught and named the
distinction once (relaying dev's correction) and the reviewer caught it independently
moments later, but the pull toward collapsing two different-owner findings into one tidy
narrative was real and recurring. **Rule: when two findings share a *symptom* (both are
"a missing signal"), actively check whether they share an *owner* before reporting them as
one ask — the tidier story is the one worth being suspicious of.**

**What the next sprint inherits.** (a) The inverted-gate framing (MB1–MB6, watching for
creep) is reusable for any future tier-2/3 work — name the inversion again wherever it
applies, don't assume it's understood. (b) Two distinct, separately-owned cross-team asks
are tracked (transport session-close; proto length/end-marker) — report them as two,
resist the urge to consolidate. (c) JC-1 (reserved-verb-byte disposition) is a real, if
currently benign, tier-1 policy question awaiting routing. (d) The relay layer is built and
certified around the tier-1 core exactly as scoped; the transport/NIC seam and the
net-payload handler are the next sprint's honest starting point, not loose ends this one
left behind.
