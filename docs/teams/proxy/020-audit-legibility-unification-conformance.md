# Sprint 020 — Audit: Legibility, Interface Unification, Spec Conformance

> Proxy team, sprint 20. Full audit pass of what this team has built, before the
> operator reads the project. Three areas: legibility, unification of interfaces
> where parallel patterns emerged that should be one system, and conformance to
> spec. Operator's framing, relayed by the PM: "design patterns reveal themselves
> as the product develops... it is likely that things were maintained in parallel
> across sprints that, seen whole, should be one thing." Drafted by
> zero.proxy-lead from candidate lists independently produced by zero.proxy-dev
> and zero.proxy-review, each checked against current source rather than recalled
> from memory of prior sprints.

---

## Scope Boundary

This sprint owns the `proxy` crate, `docs/proxy`, and this sprint-doc series —
what this team built and maintains directly. It does not own `directpath`,
`reactor`, or `driver` (the nic team's surface), nor `docs/proto` (a separate
design-of-record with its own owner, confirmed by `architecture.md`'s own header
crediting "the designer" as a distinct role from this team). Two items found
during planning cross that boundary; both are named as escalations below, not
resolved here.

**The sprint's governing discipline, earned during planning rather than assumed
in the abstract**: every unification candidate must be shown **parallel** — the
same job, solved by independent implementations — before a slice is planned
around it. A candidate that turns out to be **layered** (one thing correctly
built on another, each adding only its own concern) is recorded as a **negative
result**, not quietly dropped, so a future audit does not re-propose it. This
rule was applied three times during planning and changed the outcome twice:

- The lead's own first-guess candidate (`register.rs`/`vocab.rs`/`route.rs`)
  was retracted within the hour — a deliberate three-layer split (types / data
  / matcher), not duplication. Both dev and review independently reached the
  same conclusion from the code side before the retraction landed.
- Review's five denial types (`AddressDenial`, `Denial`, `LeafDenial`,
  `OpenDenial`, `RouteDenial`) looked like the same thing five times; each
  wraps the layer beneath it and adds only its own arms. Layered, not parallel.
- Review's own FS-step-machine candidate split down the middle under ten
  minutes of checking: `ReadStep`/`DirStep` are genuinely parallel (identical
  three-arm shape — item, `Eof`, `Error`), `WriteStep` is not (no `Eof` at
  all, by construction — a write has no exhaustion, only session-end;
  three distinct failure arms with three different payloads). The rule moved
  the slice's own scope, on a candidate its proposer raised.

## Problem

Nineteen sprints of accretion, each individually reviewed and accepted, leave
patterns visible only in aggregate: a module that grew four unrelated concerns
one slice at a time, a vocabulary restated in ten documents because none of
them defines it, a design constraint (`serve` cannot self-declare its own
running bytes) that every capture sprint has independently worked around
rather than named as a standing property. None of this is a defect in any
single sprint's own gate — every item below was individually correct against
the slice that introduced it — the gap is that nobody has looked at the whole
since Sprint 5.

## Goal

Close the cheap, confidently-scoped items in one pass; convert standing
"provisional" markers into reviewed decisions; name the two genuine forks this
audit surfaced precisely enough for the PM/operator to rule on them without
further investigation; and leave a written negative-results list so the next
audit does not re-spend time on what this one already checked.

## Final State

1. **A canonical review-vocabulary reference exists in `docs/proxy`,
   covering the full family in each case** (corrected during Slice 1: `JC`
   has **four** members — `JC-1`/`JC-2`/`JC-3`/`JC-4`, the fourth being log
   discipline, `contract/box-proxy.md:77` — not three; most sprints cited
   only needed the first three, but the reference covers the whole family).
   `MB1`–`MB6`, `IB1`–`IB3`, `CB1`–`CB6`, `JC-1`–`JC-4`, `U1`–`U5`, the
   `L`-ledger numbers, and the `FS#N`/`R`-number conventions are each
   **defined once, in one place — canonically new for four families that had
   no home (`CB`, `MB`, `IB`, `U`), and cited rather than re-defined for two
   that already did** (`JC-1`–`JC-4` in `contract/box-proxy.md`; the
   `L`-numbers in `architecture.md` §11's ledger) — narrowed from "define
   each exactly once" during Slice 1 (dev's finding, ratified) because
   re-defining an already-canonical family would recreate the exact
   duplication this sprint exists to remove. Every sprint doc that uses this
   vocabulary cites the reference rather than restating it (review's
   finding: this vocabulary is currently restated across sprint docs `006`,
   `007`, `008`, `009`, `010`, `011`, `012`, `013`, `015`, `016`, and defined
   canonically in none of them — the actual gate bar exists only as
   repetition, which can drift between restatements — confirmed during
   Slice 1: `MB6`'s allow-list drifted from its Sprint 6 origin
   (`open_discipline` alone) after Sprint 8 added `privilege`, and several
   post-Sprint-9 restatements call the drifted value "unchanged", true
   against Sprint 9 and false against Sprint 6. Resolved against current
   source, not any restatement — `lib.rs:1`'s `#![deny(unsafe_code)]` is
   re-allowed only by `open_discipline.rs:16` and `privilege.rs:16` — and
   the drift is recorded in the reference rather than silently resolved
   toward whichever restatement happened to be read first).
2. **A canonical capture-coherence procedure exists in `docs/proxy`.** The
   recipe knowledge Sprint 17 (and Sprint 5, Sprint 14 before it) each
   partially rediscovered — per-binary feature sets and why invocations must
   stay separate (feature unification changes what a shared library compiles
   to), target-directory dependence for `benchtools`-linked binaries,
   path-dependence in the absence of `--remap-path-prefix`, and the four-value
   coherence chain (build-time hash, `/proc`-during-run hash, on-disk-after
   hash, a reviewer's independent fresh rebuild) — lives in **one** document,
   not scattered prose across three sprint docs. Review manufactured a false
   coherence mismatch on Sprint 17's own gate from exactly this scatter; this
   closes the recipe gap that caused it.
3. **The cold-docs conformance pass has run and its findings are recorded.**
   Every doc-versus-build drift this team has found to date (`proxy-head.md`'s
   proxy-authored-toward-the-head line, the §5/L9 config-loader row,
   `terminal.md` §5's stale tier-4 IOMMU claim) was found **incidentally**, by
   a sprint that happened to touch that area for an unrelated reason — never
   by deliberately looking. This Final State inverts that: list every
   `docs/proxy` section against the last sprint that actually touched its
   subject matter, and read the **coldest** sections first (review's finding:
   what a sprint most recently changed is precisely what has already been
   checked — the undetected drift, if any, is where no recent sprint has had
   reason to look). Output is a list of confirmed-current sections and any
   newly-found drift, each drift item triaged per Final State #9's shape
   below (fixed here if trivial and in-scope, or named as its own
   escalation if it crosses this team's boundary or the design-of-record's
   authority).
4. **The credential's custody is legible in one place, and this is the
   high-touch half of the split, not the small-sounding one.**
   `CredentialSource` (the descriptor, currently `vocab.rs:163`) and
   `Credential` (the value, plus its entire env-resolution policy, currently
   `upstream.rs:298-350`) — ~1400 lines apart — are co-located, so "where
   does the credential come from, and what happens when the variable is
   empty" is answerable by reading one place, not two. This is the one
   secret in the system (`architecture.md` §8 names its custody explicitly),
   and it should be readable in one sitting. **Blast radius, checked against
   source (dev's finding): `Credential` has 22 external references — more
   than every other symbol this Final State and #5 touch, combined**
   (`UpstreamFrame` 6, `UpstreamHandle` 6, `UpstreamSpawn` 5,
   `AnthropicClient` 4, `UPSTREAM_CHANNEL_CAP` 3, `UpstreamIngress` 2,
   `run_adapter_with_upstream` 1). The module-split proper (#5) is
   comparatively contained; this move is where the gate's attention belongs.
5. **`upstream.rs` (1474 lines, the crate's largest module) is split along
   its own seams — three modules with a real dependency edge, not four
   independent peers** (corrected during planning, dev's finding: the dial
   is not independent of the ingress adapter — `AnthropicClient::spawn`
   constructs the `sync_channel` the seam runs on and sends `UpstreamFrame`
   values directly; `stream_response` is written entirely in terms of
   `UpstreamFrame`. That is correct design — the channel vocabulary *is*
   the seam between the two halves, not an accident — so the honest target
   shape is: the seam types plus the loop-side ingress adapter
   (`UpstreamFrame`/`UpstreamHandle`/`UpstreamSpawn`/`UpstreamIngress`/
   `run_adapter_with_upstream`); the credential type and policy (Final
   State #4); and the dial that implements the seam
   (`AnthropicClient`/`Destination`/`upstream_call`/`call_over`/
   `write_request`, plus response streaming and transport-error mapping).
   "Four concerns" is an accurate description of what the file currently
   *holds*; three modules with a dependency edge is the accurate target
   *shape*. Behavior-preserving: same public API where anything outside the
   module depends on it, same test coverage, nothing byte-different at the
   wire.
6. **`lifecycle::body_fingerprint` moves to where it belongs**, if Final
   State #5's split creates a natural home (a diagnostics module) — filed in
   `lifecycle.rs` today only because that file was open during Sprint 17,
   with nothing to do with the bring-up ladder it currently sits beside. If
   no natural home falls out of #5, this item is dropped rather than forcing
   a module into existence for one function.
7. **`ReadStep`/`DirStep` are unified behind one generic streaming-step
   contract; `WriteStep` stays separate, explicitly, with the reason
   recorded.** `ReadStep` and `DirStep` are proven parallel (identical
   three-arm shape: an item, `Eof`, `Error(ErrorBody)`) — one contract
   currently written twice. `WriteStep` is proven **not** parallel: no `Eof`
   at all (a write has no exhaustion — completion is session-end, `file.md`
   §1, Sprint 9's Fork-B), and three distinct failure arms with three
   different payloads, answering "what happened to this chunk" rather than
   "what is the next item in a stream". The value is forward-looking, stated
   honestly: unifying two three-arm enums saves little code today; the reason
   it is worth doing is that the next streaming source conforms to one named
   contract rather than inventing a third variant.
8. **Thirteen "provisional, marked for review" items are individually
   reviewed and closed out — confirmed as-is, adjusted, or (for the two
   spec-only items) the spec corrected — each with a one-line recorded
   reason, and every "provisional" marker removed once reviewed.** Eleven
   are live scalars in `src/` (all still live, grepped fresh, excluding
   bins): `register.rs:64` (readable-root default), `fs_read.rs:30` (read
   chunk size), `anthropic.rs:19` (max structural nesting depth),
   `leaf.rs:17-18` (bounded upper count, `spec/leaf-validator.md` R-1),
   `dispatch.rs:193` (`REPLY_QUEUE_CAP`), `upstream.rs:39`
   (`ANTHROPIC_VERSION`), `upstream.rs:44` (`UPSTREAM_TIMEOUT`, 120s),
   `upstream.rs:53` (`UPSTREAM_CHANNEL_CAP`, 64), `head.rs:47` (head
   ingress cap), `head.rs:53` (head frame cap, 1 MiB), `fs_write.rs:27-28`
   (quarantine per-file cap, 32 MiB). Marking these provisional and
   proceeding was correct each time (the team's own rule against blocking a
   sprint on a scalar); the rule only discharges its debt if the review
   actually happens, and across 19 sprints it has not. Two need individual
   attention rather than a rubber stamp: `ANTHROPIC_VERSION` is a vendor API
   contract string that drifts with the vendor with nothing to notice a
   drift (a process/comment fix, not necessarily a value change);
   `register.rs:64` is *provisional configuration inside the compiled-in
   register itself*, which sits oddly against §8's "the operator edits
   these lists" framing — flag this one's disposition alongside the doc
   rather than deciding it in isolation.

   **A twelfth and thirteenth exist outside `src/`, found during Slice 1's
   cold-docs pass because the original grep was structurally blind to
   spec-declared-but-unimplemented items** (dev's finding): `spec/trusted-
   address.md` §6 declares, verbatim, "provisional, marked for review —
   assembled length <= 1024 bytes; segment count <= 64", stating a cap
   violation "denies, legibly". **Neither cap exists in the code** — the
   only related constant anywhere is `proto`'s `REASSEMBLY_CAP` (32 MiB),
   which bounds payload reassembly, a different thing entirely. §6 itself
   already states the actual containment bound is the wire frame (a
   `net-address` is one frame-bounded message with no box-supplied length)
   and that these two caps would be "operational sanity, not containment"
   — so nothing is unsafe either way, but the spec currently states
   enforced behavior that is not enforced. **Ruled: amend §6 to record that
   these two caps were considered and dropped because the frame bound
   already provides containment, rather than implementing two new checks
   that add no security value purely to match a stale, never-implemented
   line** — consistent with this crate's standing preference for
   structural, off-by-construction guarantees over redundant
   configuration-shaped ones. `spec/trusted-address.md` is this team's own
   file (`docs/proxy/spec/`), so this is an in-scope correction, not an
   escalation.
9. **Two spec-conformance escalations are named precisely and routed to the
   PM — not resolved by this sprint:**
   - `docs/proto/model.md`'s Law 8, "the box is the sole allocator of
     sessions" (lines 47, 376, 392), is confirmed stale two ways: both
     endpoints have originated since Task 15 (2026-07-21), and Sprint 12's
     terminal-input ruling made it stale a second way. Review flagged this at
     Sprint 12's RQ3; it was routed to the designer and has not landed.
     `docs/proto` is outside this team's scope boundary.
   - ~~`architecture.md` §8 enumerates three configured things...~~
     **RESOLVED by the PM, 2026-07-25**: `KernelProtections` (the
     noexec/nosuid/nodev mount check, `lifecycle.rs:54`'s fourth rung) is a
     validated *environment precondition*, not configuration — §8's
     three-item enumeration is correct as written and needs no fourth
     bullet. The designer will add a one-line reconciling note when
     `architecture.md`'s own gate opens; no action for this sprint.
10. **Two genuine forks are named for a PM/operator ruling, not resolved by
    this sprint:**
    - **`serve` cannot self-declare its own running bytes, and the obvious
      fix is barred by an existing TCB decision.** All four bench binaries
      print a `benchtools::Provenance` header; `serve` — the one binary that
      matters most — prints none, because `benchtools` is explicitly "off by
      default, the production lib never links it" (`Cargo.toml:26`,
      deliberate TCB-minimization). Sprint 17's capture hit this directly:
      `serve`'s bin-sha had to be taken *externally* (`sha256` of
      `/proc/<pid>/exe`), yielding 3 of 4 coherence-chain values instead of 4.
      Three paths forward, each trading something real: link bench tooling
      into the production binary (grows the TCB to gain self-attestation);
      reimplement self-hashing inside `proxy` itself (a second, independent
      implementation of what `benchtools` exists to own — the standing
      unification discipline argues against inventing a second one on
      purpose); or ratify that `serve` is *permanently* externally-attested
      only, and document that as a standing property rather than a gap.
      Dev and review offer no recommendation — this trades TCB surface
      against evidence strength, which is the human's call. The lead's lean,
      offered as input only: option three, consistent with this project's
      standing TCB-minimization discipline (`box-reachable-core-no-third-
      party-unsafe`) — but this is explicitly the PM's/operator's decision,
      not a default to act on.
    - **`vocab.rs` has four hand-rolled copies of one newtype pattern**
      (`Route`, `RootPath`, `CredentialSource`, `ToolIdentity` — each a
      `pub struct X(&'static str)` + `const fn new` with an `assert!` +
      `const fn as_str` + a separate free-standing validator), written across
      different sprints. This is one level *below* the layering boundary
      already retracted for the module trio — the layering there is right;
      the repetition here is inside the type layer itself. Scoped as
      **decide whether to unify, not unify**: the `C1` compile-error
      guarantee and the `U5` bypass audit both rest on "`new()` is the only
      constructor", any unification must preserve that provably, and a macro
      trading four readable blocks for one clever one runs against this
      project's own `C5` ("boring, not clever"). This sprint's obligation is
      a recorded decision with reasoning, not a mandated change.
11. **The box-facing emission path is investigated, not unified, this
    sprint** — review's strongest unification candidate and the one most
    exposed to reviewer's-own-blind-spot risk, named as such: it accreted one
    gate at a time across Sprints 9, 10, 11, 12, and 15
    (`push_reply_frame`, `enqueue_net_response`, `enqueue_net_error`,
    `route_head_input`, `notify_deny`, `close_reply`,
    `ReplySource::{File,Dir,Queue}`, `pump_replies`), each accepted correctly
    against the slice that introduced it, never examined as a whole. The
    investigation's job is to determine — against this sprint's own
    parallel-vs-layered discipline — whether this is genuinely one emission
    system wearing several names, or a correctly layered accretion (like the
    denial types turned out to be). One specific asymmetry must be ruled on
    either way: three of the paths above take `Enqueue` and backpressure;
    `notify_deny` does not, and can drop — established previously as correct
    by *reachability*, not by design symmetry. The investigation states
    plainly whether that reachability argument still holds or has become
    stale history. **If the investigation concludes genuine unification is
    warranted, the restructuring itself is out of this sprint's scope** —
    `dispatch.rs` alone is 1182 lines and this would be a substantial,
    separately-gated change; this sprint produces the finding and a
    recommendation, a follow-on sprint (if the PM greenlights one) does the
    work.
12. **A written negative-results list exists**, so a future audit does not
    re-propose what this one already checked: the module trio
    (`register.rs`/`vocab.rs`/`route.rs`), the five denial types, `Provenance`
    and `body_fingerprint` (already single-implementation), `cloud_relay`/
    `terminal_relay`/`fs_read`/`fs_write` (checked, genuinely different jobs
    despite superficially parallel names), `WriteStep` (not parallel with
    `ReadStep`/`DirStep`, stays separate), and the **absence of a shared
    diagnostic convention** across `serve.rs`/`s17_box.rs`/`s17_head.rs`
    (each binary invents its own `println!`/`eprintln!` line format) —
    real, but checked and judged low-value relative to this sprint's other
    items, and explicitly descoped rather than never considered. Recording
    "checked, judged low value, descoped" as its own category, distinct from
    "checked, proven layered" — a future audit should not re-derive either.
    **The box-facing emission path (Final State #11), Slice 5's own
    candidate, is added here too** — layered, not parallel, shown rather
    than asserted: `push_reply_frame` is the sole enqueue primitive
    (`REPLY_QUEUE_CAP` and `Enqueue::Full` each have exactly one production
    site in `src/`, both inside it), all four wrapper entry points
    (`enqueue_net_response`, `route_head_input`, `enqueue_net_error`,
    `notify_deny`) delegate to it and add only their own concern, and three
    of the originally-named eight items (`close_reply`, `pump_replies`,
    `ReplySource`'s File/Dir-vs-Queue split) turn out not to be emission
    entry points at all. No unification, no follow-on sprint for the
    unification question. Reviewer's own candidate did not survive its own
    pre-committed bar — recorded here precisely because that is what makes
    it a clean result rather than a quietly-dropped one.
13. **The sprint document carries role retrospectives; the sprint is
    committed with only its intentional files.**
14. **A deferred finding, named and ranked rather than silently dropped:
    ordinal references used as durable identifiers are a defect class, and
    this sprint hit both of its failure modes.** First seen as `RT-N`
    residual numbering colliding across specs (Slice 1's bounded
    completion check: `trusted-address.md`'s `RT-4` and
    `address-recognizer.md`'s `RT-4` are genuinely distinct residuals that
    happen to share a number, confirmed not a stale reference — the
    collision itself nearly produced a false positive in the very check
    this slice established). Seen a second time, a different symptom of
    the same root, when Slice 5's own residual finding forced this
    contract's slices to be renumbered mid-sprint: a stale `REJECT Slice 6`
    clause silently kept pointing at the vocab.rs decision after it became
    Slice 7 (review's own semantic-renumbering check, written *after* the
    first instance, caught the second one live — existence-checking alone
    would have passed it, since Slice 6 existed, just as the wrong slice).
    **The shared root, stated so it points at a fix rather than only a
    vigilance practice** (review's refinement): an ordinal encodes
    *position in a list*, not identity — collide it *across* a namespace
    (two independently-numbered lists sharing a number) and it is
    ambiguous; insert *within* a namespace and every reference after the
    insertion point silently re-targets. A stable identifier (a name or
    slug, not a number) has neither failure mode. The semantic check
    (Final State #12/this note) is the **mitigation** for ordinals that
    already exist; the **dissolution** is to stop minting new ones —
    whoever scopes this should weigh a stable-identifier scheme against
    retrofitting the existing residuals, which is itself the same class of
    problem (every citation to the old ordinal would need updating).
    **Honest limit on how this was caught, on the record** (review): the
    second instance was found because the rule had just been written and
    the renumbering was fresh in the message stream — attention that
    happened to be pointed the right way, not a repeatable mechanism. The
    repeatable version is running the check as a step whenever slices are
    reordered, not relying on a reviewer remembering to look. Not actioned
    this sprint (the stopping-line discipline above applies); named here
    so a later pass inherits the root cause, not just the two symptoms.
15. **A residual found underneath Slice 5's negative result: `notify_deny`
    (`dispatch.rs:1037`) discards `push_reply_frame`'s returned `Enqueue`,
    unlike its three sibling wrappers, and the reachability argument that
    makes today's drop safe is unwritten and untested.** The property:
    no session can receive a deny verdict while its reply queue is already
    at `REPLY_QUEUE_CAP` (reviewer's own Sprint 13 reasoning, never
    committed to a comment or a test). Nine verdicts route into
    `notify_deny` and nothing in `route()` excludes a session with queued
    replies; one concrete shape was traced (a streaming response's queue
    at cap, then a further malformed frame denies the same session) but
    whether the transport can actually deliver that second frame while a
    response streams was left explicitly unanswered. **Disposition, ruled
    by review**: write the property down at `notify_deny` **and** add the
    missing test (drive a session's queue to `REPLY_QUEUE_CAP`, then fire
    a deny on it) **as one combined item, not two** — a comment asserting
    the property without a test is the identical failure in better prose,
    and the test is what makes the comment durable, since it fails when the
    reachability property changes. A third option — making `notify_deny`
    propagate like its siblings, removing the dependence entirely — is
    **named, not scoped**: the siblings backpressure their *ingress*
    (stop reading, retry); `notify_deny` has no ingress to stop, since the
    deny is generated synchronously inside `route()` from an
    already-delivered frame, so "propagate" requires answering
    *backpressure to whom* — the real answer is that `route()` would need
    a backpressure-aware return and the composed adapter would need to
    retain and retry a deny, a seam change, not a local fix. Whoever picks
    this up starts from that question, not from an assumed small edit.

## In Scope — ordered slices

**Slice 1 — canonical references and the cold-docs pass (docs only).**
Final States #1, #2, #3. Lowest risk (no production code touched), highest
immediate legibility value, and its findings (particularly #3's cold-docs
pass) may surface additional conformance items that reprioritize later
slices — run this first for that reason as well as risk. Gate: both new
reference documents exist and are cited (not restated) from at least one
sprint doc each as a smoke check; the cold-docs list and its findings are
recorded, with any newly-found drift triaged (fixed here if trivial and
in-scope, or named as its own escalation per Final State #9's shape).

**Slice 2 — the provisional-item review.**
Final State #8, thirteen items (grown from eleven during Slice 1's
cold-docs pass, which found two more declared in `spec/trusted-address.md`
§6 that a `src/`-only grep could not see). Sequenced **before** the module
split, not after, on a concrete interaction dev found: three of the eleven
`src/` scalars (`upstream.rs:39` `ANTHROPIC_VERSION`, `:44`
`UPSTREAM_TIMEOUT`, `:53` `UPSTREAM_CHANNEL_CAP`) live inside the module
Slice 3 is about to split and relocate. Disposing them here, against stable
references, before they scatter across new module boundaries, avoids two
costs: Final State #8's own line references going stale mid-sprint, and
Slice 3 having to carry "provisional" markers through a reorganization
intact — exactly the kind of incidental loss a reorganization slice is
worst at. Gate: all thirteen items individually dispositioned with a
recorded one-line reason each; `ANTHROPIC_VERSION` and `register.rs:64`
given the individual attention named in Final State #8; the two
`trusted-address.md` §6 items closed by amending the spec (not by adding
code); no scalar changes value without a stated reason.

**Slice 3 — the credential/upstream.rs split.**
Final States #4, #5, #6. The largest code-touching slice in this sprint,
and the one whose safety-relevance is highest (the credential's custody —
see Final State #4's blast-radius note: `Credential`'s 22 external
references are the high-touch change here, not the three-module split
itself, which is comparatively contained). Behavior-preserving throughout,
proven against a named list of invariants, not a general impression:
  - `Credential`'s field stays **private**; the redacted `Debug` impl
    survives unchanged.
  - `Credential::from_env_value` stays **total** over all three cases
    (unset, set-but-empty, non-UTF-8) — same behavior, new location.
  - `CredentialSource` keeps its private field, with `env()` (or its
    current equivalent) as the **sole** constructor.
  - The strongest proof for a pure move is that the diff is
    **relocation-only, with no logic change** — a green test suite proves
    the *tested* behavior survived, not that nothing else moved; review
    checks the diff shape directly, not only the test outcome.
  - **The `trybuild` bypass-audit test
    (`credential_source_no_literal_bypass`) is a regenerable artifact and
    the sharpest risk in this slice**: it asserts a compile *failure*, and
    its expected `.stderr` embeds paths and module names that moving
    `CredentialSource` will likely change. If the `.stderr` changes, the
    gate requires confirming the captured error is **still the same class**
    (private-constructor inexpressibility, `E0603`-shaped) — not merely
    that `trybuild` is green again. Regenerating a negative-proof's
    expected output without checking what it now proves is exactly how a
    security property is silently weakened while every visible signal
    stays green. **Closed at Slice 3's gate** (review, independently):
    every changed line in both `.stderr` files is a module-path token or
    its mechanical consequence (a caret-column shift exactly matching the
    path-length difference); no error code or error-text line appears in
    either diff, meaning both are byte-identical — the diff is
    self-evidencing, since a changed error *kind* would necessarily show
    up as a changed code or message line, not just a path token.
  - **`AnthropicClient::spawn` reading `Credential`'s field directly
    (`self.credential.0`) could not survive the split** — legal only
    because dial and credential shared a module; moving the dial out
    forced the read into a new `pub(crate) expose()` accessor. **Record
    this precisely, not as "no widening occurred"** (review's correction):
    the field itself stays private, but the *crate-visible* surface that
    can reach the secret did widen — module-private access became
    crate-visible access. What makes it acceptable is not the absence of
    widening but its shape: forced by the split rather than chosen,
    bounded to one documented caller
    (`upstream_dial.rs:100`, the `x-api-key` header write), and net more
    auditable than what it replaced (one grep answers "what can reach the
    secret" where before the answer was "read the whole module"). State
    it as *minimal, justified, bounded widening*, not as its absence —
    the bound is the part that can rot, and only a claim that names it
    stays checkable. **Standing check, not a one-time approval**:
    `expose()`'s own doc comment states a contract ("anything else calling
    this is a finding") that nothing verifies mechanically. Any future
    slice touching `credential.rs` or `upstream_dial.rs` re-confirms
    `expose()` still has exactly one caller before closing — the same
    discipline this sprint applies to a provisional scalar with no
    review, applied to a documented-but-unchecked access contract.
Full test suite green and `clippy` clean are necessary, not sufficient.
Gate: full independent review (function and hygiene both) against the
invariant list above, not a checkpoint — this is production
security-relevant code being moved and re-seamed.

**Slice 4 — the streaming-step unification.**
Final State #7. Small, isolated, and already fully scoped by the
parallel-vs-layered check done during planning (independently re-verified
by dev against current source: confirmed to hold exactly). Gate: existing
`fs_read.rs` test coverage passes unchanged against the unified shape;
`WriteStep` verifiably untouched.

**Slice 5 — the box-facing emission-path investigation.**
Final State #11. Investigation and recommendation only. Stated flatly, not
conditionally: **no production code changes land in this slice, regardless
of the finding** — any recommendation to unify scopes as a follow-on
sprint's work, full stop; a "trivially layered, nothing to do" finding is
the only case where the finding and the (non-)action coincide, and that
coincidence is not a license for any other outcome to include a change.
Gate: a finding of **parallel** must be **shown** parallel — the same job,
solved by independent implementations — to the identical bar this sprint's
governing rule imposes everywhere else, not asserted in prose as "evidence
either way." The `notify_deny` backpressure-asymmetry ruling must
additionally state whether its grounding **survives** whatever the
investigation contemplates: the asymmetry was established as correct by
*reachability* (a deny fires on a session with no streaming replies
queued, so `Full` cannot arise there) — a structural fact about the
*current* layout. If the investigation contemplates unification, the
ruling must say whether that reachability property would survive it, not
only whether the asymmetry is correct today.

**Slice 6 — the `notify_deny` reachability fix.**
Final State #15, found underneath Slice 5's negative result. One combined
item, not two: write the reachability property down at `notify_deny`
naming what would invalidate it, **and** add the missing test (drive a
session's reply queue to `REPLY_QUEUE_CAP`, then fire a deny on it,
assert the intended behavior) — ruled as inseparable by review, since a
comment without a test is the same undocumented-and-unexercised failure
in better prose. No behavior change: this documents and tests the
existing drop, it does not alter it. The third disposition (making
`notify_deny` propagate like its siblings) is named, not scoped, per
Final State #15 — it needs a design answer to "backpressure to whom"
before anyone can size it, and is explicitly out of this slice. Gate:
the property is stated at the call site; the new test fails if the
reachability property is later violated; full suite green, `clippy`
clean.

**Slice 7 — the vocab.rs newtype decision.**
Final State #10 (second bullet). A ruling with recorded reasoning, not an
implementation. Gate: the decision and its reasoning are recorded in this
document (or a follow-on's, if unification is ratified and scoped as
future work); `new()`-is-sole-constructor is explicitly re-affirmed as a
property any eventual unification must preserve provably.

**Not a slice — routed to the PM directly, at green-light and as needed:**
Final State #9 (both escalations) and Final State #10's first bullet
(`serve`'s self-declaration fork). These are named, not scheduled — the
PM/operator's ruling, if it lands during this sprint, may become its own
small in-scope item; if not, this sprint closes with them recorded as open
escalations, not failures.

## Non-Goals

- Do not unify the five denial types, the `register.rs`/`vocab.rs`/`route.rs`
  trio, `Provenance`/`body_fingerprint`, or `cloud_relay`/`terminal_relay`/
  `fs_read`/`fs_write` — each checked and confirmed either already
  single-implementation or genuinely layered, not parallel. Re-proposing any
  of these without new evidence is exactly what Final State #12's negative-
  results list exists to prevent.
- Do not unify `WriteStep` with `ReadStep`/`DirStep` — proven not parallel
  (no `Eof`, three distinct failure-arm payloads, answers a different
  question).
- Do not implement the box-facing emission-path unification even if Slice
  5's investigation concludes it is warranted — that is a follow-on sprint's
  work, scoped and gated separately, given `dispatch.rs`'s size and the
  security-relevance of what it emits to the box.
- Do not implement any change to `docs/proto/model.md` or resolve its Law-8
  staleness — outside this team's scope boundary; route to the PM.
- ~~Do not resolve `architecture.md` §8's three-vs-four divergence
  unilaterally~~ — **RESOLVED by the PM 2026-07-25**: no fourth bullet
  needed, `KernelProtections` is a precondition not configuration. This
  Non-Goal is discharged, not active.
- Do not decide the `serve` self-declaration TCB tradeoff (link bench
  tooling, reimplement self-hashing, or ratify external-attestation-only) —
  a human/operator call on risk tolerance and TCB surface, not a technical
  question this team resolves by itself.
- Do not implement `vocab.rs`'s newtype unification without a separate,
  explicit ratification of the resulting design against `C1`
  (compile-error-on-malformed) and the `U5` bypass audit's
  sole-constructor property.
- Do not change any of the eleven provisional scalars' *values* without a
  stated, recorded reason — this slice reviews and disposes, it does not
  retune for its own sake.
- Do not treat this audit as a re-litigation of any already-accepted
  sprint's design — every item here is a legibility, organization, or
  documentation question about *already-correct* work, not a claim that a
  prior sprint's decision was wrong.

## Starting Surface

Audit first, in this order:

1. `docs/proxy/` (all files) and the sprint-doc series
   (`docs/teams/proxy/001`–`019`) — the vocabulary-restatement survey (Final
   State #1) and the cold-docs pass (Final State #3) both start here.
2. `proxy/src/upstream.rs` — read start to finish against its own public
   surface (the four seams named in Final State #5) before touching
   anything; `proxy/src/vocab.rs:163` (`CredentialSource`) as the other half
   of Final State #4.
3. `proxy/src/fs_read.rs` (`ReadStep`, `DirStep`) and `proxy/src/fs_write.rs`
   (`WriteStep`) — confirm the parallel/layered finding from planning still
   holds against current source before Slice 4 begins.
4. `proxy/src/dispatch.rs` and `proxy/src/head.rs` — the box-facing emission
   path's eight-odd entry points (Final State #11), read as a whole rather
   than per-call-site.
5. `proxy/src/vocab.rs`'s four newtypes (`Route`, `RootPath`,
   `CredentialSource`, `ToolIdentity`) side by side — Final State #10's
   second bullet.
6. `docs/proxy/architecture.md` §8 against `proxy/src/lifecycle.rs:54`'s
   `AUTHORITIES` array — Final State #9's second escalation.

## Method

- **The parallel-vs-layered discipline governs every unification candidate
  in this sprint, without exception**, including ones raised by the person
  proposing them: show the same job solved by independent implementations
  before scoping a slice around it; a candidate found to be layered is
  recorded in Final State #12's negative-results list, not dropped silently.
  This rule already changed this sprint's own scope twice during planning
  (the module trio, the FS-step split) before a single slice was written —
  keep applying it with the same rigor once slices are underway.
- **Behavior preservation is proven, not asserted, for every reorganization
  slice** (Slices 3 and 4): full test suite green, `clippy` clean on every
  relevant feature configuration, and — for Slice 3 specifically, given its
  security-relevance — independent review confirms nothing changed except
  location and module boundaries against the named invariant list, the same
  standard this team already holds itself to for prior refactors.
- **A "provisional" marker is closed by a recorded decision, not by silence.**
  Slice 2's disposition of each of the thirteen items states its reasoning
  in one line, even where the answer is "confirmed as-is" — the point is
  converting an open flag into a decision, not necessarily changing a value.
- **Correcting a cold doc means checking references at two granularities,
  not one** (review's refinement, sharpened once already during Slice 1's
  own gate — the first version scoped "what cites it" to the *document*,
  which missed the *entry*-level case that turned out to be the more
  common of the two). Level one, cross-document: a correction can leave a
  sibling document asserting the pre-correction claim (`address-
  recognizer.md`'s RT-4 originally still called withdrawn caps
  "provisional" and cited the very section that withdrew them), and —
  easy to miss because it is self-referential — the correction is itself
  the act that stales any currency pin *to* the corrected file
  (`box-proxy.md`'s pin on `trusted-address.md` went stale the moment
  Slice 1 edited it). Level two, same-file: a corrected *entry* can still
  be referenced elsewhere in its *own* document by an untouched sentence
  that describes its old status (a second `JT-3` mention, `address-
  recognizer.md` §7 citing its own now-withdrawn `RT-4` at a status that
  no longer applies) — nothing here cites another document, so the
  document-level check alone does not catch it. Four of this slice's
  findings were the second kind, not the rarer case. **The bounded
  completion check catches both in one pass**: for each identifier
  corrected in a slice, grep it across every touched document — this
  finds a same-file stale reference and a cross-file one identically,
  since neither the grep nor the check cares which file the match lands
  in. Every remaining cold-docs correction in this sprint runs that check
  before it is considered closed.
- **A sweep's stopping line is a property, not a feeling — and the
  property is checkable, which is what makes it defensible.** Slice 1's
  own scope grew considerably past its original three Final States through
  a repeated pattern: an already-ruled decision (a withdrawn cap, a
  resolved-not-open finding, a stale currency pin), applied to an
  already-identified instance of it, where the alternative was shipping
  known-stale content inside a slice whose entire purpose is removing
  stale content — which is self-undermining, not merely untidy. Each such
  extension was ratified on that specific test, not on "cheap and already
  understood" alone (a criterion with no natural bound). The stop is
  reached when every instance the sweep actually surfaced is dispositioned
  — at which point continuing would mean starting a *new* sweep for a
  *new* class of thing, which is genuinely new scope and routes as such.
  Review's own gate on this slice found the boundary had been drawn
  correctly but applied incompletely (a sweep can under-cover within its
  own stated boundary, distinct from the boundary being wrong) — closed by
  a **bounded completion check**: for each identifier corrected in a
  slice, grep it across every touched document and confirm no remaining
  reference describes its pre-correction status. This terminates (a finite
  list against a finite set of files) and is exactly the discipline that
  distinguishes "the entry is fixed" from "every reference to the entry is
  fixed" — the gap that produced four of this slice's findings.
- **Cross-boundary findings are named precisely and routed, never resolved
  unilaterally.** Two items in this sprint (the `docs/proto` staleness, the
  `architecture.md` §8 divergence) touch a design-of-record this team does
  not own; both are stated as escalations with the lead's/dev's non-binding
  lean where one exists, and both route through the PM.
- **A genuine value tradeoff is named as a fork, not defaulted.** The
  `serve` self-declaration question trades TCB surface against evidence
  strength — a human's call on risk tolerance, offered with the lead's lean
  but not acted on without the PM's/operator's ruling.
- **Investigation slices (5) do not silently become implementation slices.**
  If Slice 5 concludes unification is warranted, the deliverable is the
  finding and a recommendation for a follow-on sprint, not the restructuring
  itself — the same discipline that kept this sprint's own scope from
  drifting during planning applies to slices that could grow past their
  gate.

## Acceptance

- REJECT any unification (implemented or merely proposed as a slice) whose
  parallel-vs-layered check is missing or asserted rather than shown —
  every candidate in Final State #12's negative-results list was checked
  by reading the actual arms/shapes involved, not by name-similarity.
- REJECT re-proposing any item on the negative-results list without new
  evidence not considered during this sprint's planning.
- REJECT Slice 3 or Slice 4 closing on "the tests still pass" alone without
  independent review confirming the reorganization changed nothing but
  location — a passing test suite is necessary, not sufficient, for a
  behavior-preservation claim on security-relevant code.
- REJECT Slice 3 closing on a regenerated `trybuild` `.stderr` without
  confirming the captured error is still the same class
  (private-constructor inexpressibility) — a green `trybuild` run after
  regenerating its expected output proves the test suite is happy, not that
  the negative proof still proves what it did before.
- REJECT unifying `WriteStep` with `ReadStep`/`DirStep`, or treating the
  streaming-step contract as covering write completion — a write's
  completion is session-end (`file.md` §1), not exhaustion, and forcing an
  `Eof` arm onto it would misrepresent a settled design decision (Sprint 9's
  Fork-B).
- REJECT Slice 5 producing a restructuring of `dispatch.rs`/`head.rs` rather
  than a finding — the investigation's scope is explicitly bounded against
  growing into unplanned implementation.
- REJECT Slice 6 (the `notify_deny` reachability fix) landing the comment
  without the test — a written-but-unchecked property reproduces the exact
  defect this item exists to close, only in better prose. Both or neither.
- REJECT Slice 7 landing a `vocab.rs` newtype change without an explicit,
  separate statement that `new()` remains the sole constructor and that `C1`
  (compile-error on malformed) still holds — a macro or generic that
  quietly reopens either property is a regression on this crate's own
  bypass-audit discipline, not a cleanup.
- REJECT this sprint resolving the `docs/proto` Law-8 staleness or the
  `architecture.md` §8 divergence directly — both are named escalations to
  the PM, and doing so unilaterally would be exactly the "correct the doc
  to match code" anti-pattern this team already treats as a standing error.
- REJECT this sprint deciding the `serve` self-declaration TCB tradeoff
  without an explicit PM/operator ruling — three genuinely different
  options exist, none of which this team is positioned to pick alone.
- REJECT closing Slice 2 with any of the thirteen items still marked
  "provisional" without a recorded reason, or with a value changed without
  one.
- REJECT implementing the two `spec/trusted-address.md` §6 caps as new code
  to make the spec's existing wording true — the spec's own text already
  states the frame bound provides containment and these would be
  "operational sanity" only; the correct fix is amending the spec to record
  they were dropped, not adding validation that adds no security value.
- REJECT treating this sprint's findings as a critique of any prior
  sprint's gate — every item here was correctly accepted against the scope
  it was reviewed under; the finding is about the aggregate, not any single
  past decision.

## Review Questions

Two genuinely open, both named above and routed to the PM rather than
settled here:

1. **The `serve` self-declaration fork** (Final State #10, first bullet):
   link `benchtools` into the production binary, reimplement self-hashing
   inside `proxy`, or ratify permanent external-attestation-only. The lead's
   non-binding lean is the third option, consistent with this project's
   standing TCB-minimization discipline — offered as input, not a default.
   **A two-sided datum from Sprint 17, since option three is the only one
   actually exercised** (review's finding, offered deliberately as both
   halves rather than the half that supports the lean): *supporting* it —
   external attestation held at the strongest bar this project has applied,
   a `serve` rebuild from a cleaned target matched the recorded hash
   exactly, independently, and the preserved artifact was separately
   re-hashed and confirmed. *Against* it — the same capture showed exactly
   how the external path degrades: once `s17_head`'s source moved after the
   capture (a since-accepted, unrelated fix), its preserved binary became
   the *sole* anchor for that binary, permanently un-reproducible from
   source. Option three is proven sufficient at capture time and proven
   fragile afterward — both are real, and both should weigh on the ruling.
2. ~~**`architecture.md` §8's three-vs-four divergence**~~ **RESOLVED by the
   PM, 2026-07-25**: `KernelProtections` is a validated environment
   precondition, not a fourth configured thing — `architecture.md` §8 needs
   no fourth bullet; the code's four-rung ladder is correct as built. The
   designer will add a one-line reconciling note to `architecture.md` when
   its own gate opens. No further action for this sprint.

One item is confirmed and escalated, not a question needing the PM's
judgment so much as a heads-up that it is still open: `docs/proto/model.md`
Law 8's staleness (Final State #9, first bullet), previously routed at
Sprint 12 RQ3 and still unresolved.

## Retrospectives

### zero.proxy-lead

**What this sprint actually was, versus the three-line directive it arrived
as.** "Audit legibility, unification, conformance" reads as a survey. What it
required was routing two independently-produced candidate lists into a shared
governing discipline before a single slice existed, then holding that
discipline against everyone who touched it afterward — including its own
authors, including me. The PM's green-light fixed the *scope*; the discipline
is what kept seven slices, several of which grew considerably past their
original Final States through legitimate extension, from becoming an
unbounded audit that never closed.

**Where I drew my own line, and where I didn't move it.** I read source,
ran every gate myself before treating a report as fact, and edited the
sprint contract directly throughout — that is mine to do. I made exactly one
edit to code-adjacent content outside the contract this whole sprint (the
archived-evidence-artifact annotation, back in Sprint 17, made only because
review named it explicitly as mine and I would not have made it unprompted).
Every actual fix — every doc correction, every comment, every test — routed
through dev, verified independently before I treated it as done. That
discipline held for seven slices and a great many rounds of back-and-forth,
and it is the reason I can say the evidence in this document is trustworthy
rather than merely voluminous.

**The stopping-line discipline is the thing I am proudest of ruling, and the
riskiest one to get right.** Slice 1 alone grew from three Final States to
nine files and a bounded completion check across four rounds of legitimate
extension. Each time, the test I actually applied was narrower than "cheap
and already understood" — it was "the same already-ruled decision, applied
to an already-identified instance, where the alternative is shipping known
stale content inside a slice whose entire purpose is removing it." That test
has a natural stopping point; "cheap and understood" does not. Naming the
property rather than trusting the feeling is what let me rule quickly and
correctly across many rounds without either stonewalling real value or
letting the slice run forever.

**Two mistakes worth recording exactly, since both were caught before they
mattered.** I nearly re-gated already-committed, already-accepted work
(Sprint 18 Slice 2) because I processed a stale status artifact as if it
were new — reviewer caught it immediately with source, not memory, and I
retracted without re-deriving anything. And I nearly over-claimed credit for
an evidence-artifact annotation dev had already made independently, before
I'd finished checking whether they had. Both were small, both were caught
before they reached a decision, and both are why "verify independently"
means verifying my *own* prior claims too, not only the reports I receive.

**What made this sprint's evidence trustworthy was not my process — it was
the team's demonstrated behavior under it.** Dev caught their own vacuous
checks repeatedly and increasingly *before* reporting them, not after review
found them — the arc's clearest signal that a discipline had actually
transferred rather than merely being imposed. Review pre-committed the bar
that would satisfy them on their *own* candidate before seeing dev's finding,
then accepted a negative result on it cleanly. Both caught concurrent-edit
collisions in this very document without being asked to look. None of that
is guaranteed by a sprint contract, however carefully written; it is the
actual conduct of the people executing it, and it is the reason this
sprint's negative results are worth as much as its positive ones.

**What the next sprint should inherit, beyond what dev and review already
named.** Ordinal identifiers (FS#14) are a standing tax this codebase pays
repeatedly and should stop minting. A written property with no check
(`expose()`'s single-caller contract, `notify_deny`'s reachability argument
before this sprint) decays invisibly and needs a name, a bound, and — where
at all cheap — a mechanical re-check, not just documentation. And when
several people write into one shared document concurrently, a structural
check after the fact is cheap insurance against a malformed result nobody
would see from inside their own edit — true of this sprint's own
retrospectives section, caught this time only because someone happened to
notice the file had moved underneath them.

### zero.proxy-review

**The residual closed this sprint was one I created.** `notify_deny` is the
only one of `push_reply_frame`'s four callers that discards the returned
`Enqueue`; the other three either backpressure their producer or propagate the
condition upward. I accepted that discard in an earlier sprint because the
*behavior* was correct — and it was. What I did not ask is what has to be true
for it to be correct, and whether that is written anywhere. It rested on an
unwritten reachability property, and an unwritten premise is not a weaker
version of a written one, it is a different object: nothing fails when it stops
holding. Closing it took a dedicated slice plus a re-gate. **The bar this
upgrades: at a gate, "does this behave correctly?" is not the question. The
question is "what must be true for this to be correct, and is that written at
the site where someone would break it?"** A discarded return, a silent default,
an unchecked precondition — each is a place where a premise is load-bearing and
invisible, and each passes a behavior review cleanly.

**The strength of a non-vacuity control lives in the caller, not in the test.**
The Slice 6 test fires a real deny on a full queue and asserts the drop, with a
control arm firing the identical deny on a fresh session to prove the path is
live. What makes that airtight is not in the test at all: `route` calls
`notify_deny` **unconditionally** after `route_inner`, with no branch between
the verdict and the notification. That is what rules out the session's own
history having diverted the path before the notification, which is the residual
the control alone leaves open. The transferable form: **a control is only as
strong as the branch structure between the trigger and the observed effect, and
that structure lives in the function under test's caller — which the test never
mentions.** Read the caller before crediting the control.

**Entailment beats reproduction only inside a boundary, and the unbounded rule
is most attractive exactly where it is most dangerous.** Discharging the
mutation-testing claim by reading the assertions — `expect_front` and
`expect_back` are distinct, so `pop_front`+`push_back` and `pop_back`+`push_back`
each provably fail one — was the stronger form of verification, because the run
could only re-observe what the source already forces. But the general rule needs
both halves stated: **premises read from the implementation (not a doc about
it), and a conclusion the source actually determines.** This team has one
counterexample for each half, and both are mine. Sprint 14: I reasoned about a
negative control from `block`'s documented contract and concluded it would hang;
the implementation re-polls, so it does not — a valid chain rooted in a stale
leaf, which is the more insidious failure because the reasoning looks sound all
the way down. Sprint 18: `TlsPipe::connect` drives the handshake to completion
inside `connect`, which no static audit of that module establishes — the source
underdetermines the answer, so there is nothing to entail from. Reproduction is
expensive and entailment feels rigorous, so the pull toward it is strongest on
hardware and live-path claims, which is precisely the category where the source
does not settle the question. When either half fails, reproduction is not the
weaker form — it is the only one.

**FS#14's root: ordinals used as durable identifiers.** `RT-N`, `Slice-N`, and
`task16_stub_reply_frame` are the same defect, not three coincidences. An
ordinal encodes *position*, not identity, so it fails in two directions:
it **collides across namespaces** (`RT-4` denotes different things in two
specs) and it **goes stale under insertion** (inserting a new Slice 6 left an
acceptance clause pointing at what had become Slice 7, and left the new Slice 6
with no acceptance clause at all — the guard and the guarded drifted apart
silently). The mitigation for ordinals already minted is a semantic check
whenever slices are reordered; the dissolution is to stop minting new ones.
Renaming the existing ones is *not* the fix — it invalidates every citation,
which is the same problem in a new costume. **The honest caveat, which matters
more than the finding:** I caught the stale clause because I had just written
the rule and the renumbering was fresh in the message stream. That is attention
happening to point the right way, not a repeatable mechanism. The repeatable
version is the check running as a step when slices are reordered, not a reviewer
remembering to look. Whoever inherits this should inherit the root, not two
instances to pattern-match against.

**A negative result recorded at the site is a deliverable.** Slice 7 declined to
unify the four validated newtypes and wrote the reasoning next to the types
rather than in this document, so the next person tempted by it stands in front
of it. The decisive argument was not line count — it was that the U5 bypass
audit stays verifiable by *reading* an impl block, and behind a macro becomes
"true by macro expansion" for the crate's most security-relevant compile-time
guarantee. That is where "boring, not clever" stops being taste and becomes what
keeps the audit cheap. Worth noting that the candidate's own premise moved
mid-sprint: Slice 3 relocated `CredentialSource` to another module and `Route`
grew `split`, so the pattern was 3.5 times across two modules by the time it was
judged. **A unification candidate must be re-checked against current source at
the moment of the ruling, not against the shape that prompted the audit.**

**Every slice this sprint tested the same thing: whether a written rule reaches
the person who would break it.** The `notify_deny` property was correct and
unwritten. The Slice 7 ruling was written, but at three of the four sites it
governs — and the fourth held the type whose divergent naming is doing the
ruling's argumentative work, so a well-meaning rename "for consistency" would
have deleted a premise *and* manufactured the exact condition the ruling lists as
grounds to reopen. The remedy landed at the two functions, each with its own
distinct reason rather than one warning repeated, because a warning that reads
as boilerplate teaches a reader that warnings here are skippable. **Legibility is
not whether a rule is recorded; it is whether it is recorded where the reader
will be standing.**

**On my own accuracy.** Two slips are already recorded in Sprint 17's
retrospective — recommending `/proc/PID/cmdline` without carrying across the
permission reasoning that made it work, and confirming two annotations were
present (true) before narrating *who wrote them* (unverified). Both are the same
shape: **the verified part and the stated part were not the same part.** That
shape recurred in this sprint's final slice from the other direction — three
consecutive descriptions of the same artifact each caught something real and
none was true of the whole, which is how a three-site change gets reported as a
one-site change without anyone being careless. This is the actual argument for
re-deriving from source rather than from a report, and it is not about
distrusting the reporter: **descriptions drift by summarization, including my
own, and the drift is invisible from inside the description.**

**What the next audit inherits.** Ask what must be true rather than what
behaves. Read the caller before crediting a control. Bound the
entailment-over-reproduction preference by both of its halves. Treat ordinals as
positions, never as names, and do not rename the existing ones. Check whether a
remaining copy of a constant is a *check* rather than a duplicate before removing
it. And when a rule is written down, ask where the person who would break it is
standing — that question, not the writing, is what made the difference in four of
this sprint's seven slices.

### zero.proxy-dev

**The audit's own governing rule did most of the work, including against me.** "A unification candidate
must be *shown* parallel, not asserted" retired three candidates before a slice was scoped, and then
retired a fourth mid-sprint — my own. In Slice 7 I went in expecting to weigh a macro against four
hand-rolled newtypes, and the side-by-side showed **the premise had moved under the candidate**: Slice 3
had already relocated `CredentialSource` and its validator to `credential.rs`, and `Route` had grown
`split()`, so "the same shape four times in one file" was really the same shape 3.5 times across two
modules. The rule's value is not that it rejects bad candidates; it is that it forces you to look at the
current tree rather than the tree you remember.

**Three findings that only aggregate review could produce.** Each was individually correct in the sprint
that introduced it, which is exactly why nineteen sprints of gates never caught them:

- **The caps multiply.** Every bounded queue was individually bounded and individually documented as
  such; nobody had written down the product. `HEAD_CHANNEL_CAP` × `HEAD_FRAME_MAX` × two channels is a
  number that emerged from three separate decisions rather than one anybody made.
- **`notify_deny` is the exception to an otherwise-layered emission path.** The path *is* one system —
  `push_reply_frame` is the sole enqueue primitive, shown by the cap being checked in exactly one place
  and `Enqueue::Full` being constructed in exactly one place. Three of four wrappers act on the return;
  `notify_deny` discards it. The property making that safe was **unwritten and untested** until Slice 6.
- **Two provisional scalars lived in a spec, not in `src/`.** My original eleven came from grepping
  `src/` for "provisional". A grep is shaped by where you point it, and `spec/trusted-address.md` §6
  declared two caps that were never implemented — invisible to that method by construction.

**What failed, and the pattern underneath it — this is the part worth keeping.**

The individual misses were ordinary: a line-oriented `grep` that accepted the LF it existed to reject; a
first assertion that would have passed if `notify_deny` never ran at all; a `cargo doc` baseline measured
against a stale `target/doc` on one side of the comparison only; backticks in a message body that bash
command-substituted. Each cheap, each caught.

The one worth naming is Slice 2's cap arithmetic, because the *shape* of the error was new. I stated a
symmetric `× 2` product and **hedged on reachability** — carefully, explicitly, in the doc. The
arithmetic itself was wrong: `HEAD_FRAME_MAX` bounds what gets *written*, not what gets *queued*, because
`encode_head_frame`'s filter runs in the writer thread after `rx.recv()`. So I was **confident about the
half I had not checked and cautious about the half I knew I had not** — and the hedge made the whole
claim read as carefully bounded while the error sat in the asserted half.

**A hedge on the wrong axis is worse than no hedge**, because it signals diligence that was not applied
there. The underlying miss is the familiar one — I checked *that* the cap is enforced, not *where*
relative to the queue — but the hedge is what made it survive my own review. The correction is not "hedge
more"; it is to ask which half of a claim the caveat is actually protecting, and whether the *other* half
was checked at all.

**What caught things, ranked by how much I trust it now.**

1. **Mutation testing.** The only technique that proves an assertion *can* fail. In Slice 6 it turned
   "the test passes" into "the test catches drop-oldest **and** replace-newest, both verified, source
   restored byte-identical after each". Everything else establishes that a check ran.
2. **Non-vacuity controls.** Fire the identical input where the condition does *not* hold, and prove the
   outcome differs. Slice 6's first assertion was green and proved nothing; the control made the drop
   attributable to queue-fullness rather than to a dead code path.
3. **Exercising a new check against inputs it must REJECT before trusting it.** Caught the `grep`/LF bug
   in Slice 1 — only the LF case exposed it, and LF is the single character that gate exists for.
4. **Clearing derived state on *both* sides of a comparison.** My `cargo doc` delta was wrong because I
   regenerated one side and not the other. A comparison is only as good as its least-refreshed half.
5. **Guards written in an earlier sprint.** Sprint 18's "the child ran no test — this proved nothing"
   assertion caught Slice 3's module move invalidating a hardcoded test path. It existed only because
   vacuous passes had bitten me twice before, and it paid for itself in a sprint I did not write it for.

**A method that generalised further than its finding.** Review's note in Slice 1 — when a cold doc is
corrected, check what *cites* it — has two halves, and the second is much sharper: check for currency
pins that **your own correction just invalidated**. Checking for assertions that depended on the old text
is intuitive. Checking for pins whose staleness you *caused* is not, because the staling is a consequence
of the fix rather than something the fix reveals. Run as a full sweep rather than against the named
instances, it found four more sites across three files.

**What the next sprint inherits.**

- **Two canonical references now exist** (`review-vocabulary.md`, `capture-coherence.md`). Sprint docs
  **cite** them; they do not restate. Two vocabulary families (`JC-1..4`, the `L`-ledger) were already
  canonical elsewhere and are cited, not re-defined — re-defining them would have created the exact
  duplication the sprint existed to remove.
- **`expose()` carries a standing check, not a one-time approval**: exactly one caller, re-confirmed by
  any slice touching `credential.rs` or `upstream_dial.rs`. It is a *bounded* widening — the framing
  matters, and "no widening occurred" would be false.
- **The `RULING` on the four newtypes names what would reopen it** — several more such types *and*
  genuinely shared constructor/accessor names. "The names now match" is explicitly not a trigger, because
  a rename-for-consistency would manufacture the very condition the trigger describes.
- **`notify_deny`'s option 3 is named and not scoped**: making it propagate like its siblings needs an
  answer to "backpressure to whom" first — `route` would need a backpressure-aware return and the
  composed adapter a retain-and-retry. A seam change, not a local fix.
- **Two content flags stay raised and unresolved** (`architecture.md`'s four intervening commits vs
  `proxy-head.md`'s unsectioned reliance; `approach.md`/`anthropic-api.md`'s real content changes vs
  `anthropic-recognizer.md`'s specific citations). Written into their docs as *unverified* rather than
  implied-verified.
- **The RT-numbering collision** — two specs each maintain an independently-numbered `RT-N` list, so a
  bare cross-doc `RT-4` is ambiguous. It nearly produced a false positive in the completion check that
  found it. Named, deliberately not chased.
- **A currency pin that names *what* it depends on can be verified; one that names only the artifact can
  only be re-pinned.** That distinction decided which Slice-1 pins closed and which became flags, and it
  costs nothing to write pins the good way up front.
- **Intra-doc links resolve differently in module vs item docs** — module docs see items *defined* in the
  module, item docs additionally see its imports. That is precisely the class of link a module split
  breaks, and it is why Slice 3's five broken links clustered where they did.
