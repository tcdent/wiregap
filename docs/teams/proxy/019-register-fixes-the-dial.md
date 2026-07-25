# Sprint 019 — The Register, Not a Constant, Fixes the Dial

> Proxy team, sprint 19. A prerequisite for Sprint 18's Slice 2: the
> destination the proxy actually dials is today an independent hardcoded
> constant, not derived from the register's vetted route — so Sprint 18's
> peer-binding join (proving the TLS peer, SNI, and HTTP `Host` are the
> vetted destination) is currently a proof of something that does not
> exist. This sprint makes the dial derive from the matched `Route`, so
> that join becomes provable. Drafted by zero.proxy-lead from planning
> input by zero.proxy-dev, following the PM's ruling to split this out as
> its own sprint (2026-07-24).

---

## Scope Boundary

This sprint owns making `upstream.rs`'s dial destination **derive from
the per-session matched `Route`**, not from an independent constant:

1. `UpstreamSpawn::spawn`'s signature carries the destination the matched
   `Route` names.
2. `UpstreamIngress::on_admit` sources that destination from the
   `Dispatcher`'s per-session stored `Route` (already held there for the
   U4 recognizer vet) and passes it to `spawn`.
3. `AnthropicClient::endpoint` stops existing as an independent
   construction input; `serve.rs`'s `ANTHROPIC_ENDPOINT` constant is
   removed. The register becomes the **single source** of the dialed
   destination — which is what `cloud/approach.md` §6 and
   `architecture.md`'s tier-1 route-matcher row already claim, made
   actually true.

It explicitly does **not** own:

- **Sprint 18's TLS pipe itself.** `upstream_tls.rs`'s connect/verify/
  read mechanics are untouched; this sprint only changes *what
  destination* gets passed to it.
- **Sprint 18's Slice 2 gates** (the verbatim relay, the panic fix, the
  `ureq` removal, the peer-binding join). This sprint is a
  **prerequisite** for Slice 2's join proof, not part of it — Slice 2
  builds on this sprint's output rather than repeating it.
- **Any change to the register's own structure** (`Route::new`, the
  `ROUTES` const, U3/U4's recognizer). The matched `Route` already
  carries everything needed (it is what U4 vets against); this sprint
  only stops discarding it before the dial.
- **Multi-destination dispatch design** (e.g. a generic "route to any
  registered destination" upstream client). Today there is exactly one
  registered cloud destination in active use (Anthropic); this sprint
  fixes the dial to derive from *whichever* `Route` was matched, without
  building new destination-selection logic beyond what the register
  already resolves.

## Problem

Investigating Sprint 18's peer-binding join requirement, dev found (and
verified) that the destination the proxy actually dials is **not derived
from the vetted route at all**:

- `AnthropicClient` holds `endpoint: String` (`upstream.rs`), set at
  construction from `serve.rs`'s `const ANTHROPIC_ENDPOINT =
  "https://api.anthropic.com/v1/messages"`.
- `upstream_call(&endpoint, ...)` dials that string, unconditionally.
- U3 vets the box's net-address against `register::net::ROUTES`, which
  holds `Route::new("https://api.anthropic.com/v1/messages")` — a
  **different string literal in a different file**.
- `UpstreamSpawn::spawn(&self, session, body)` never receives the
  matched `Route`. `Dispatcher` stores it per-session (to source the U4
  recognizer envelope), then discards it — nothing carries it forward to
  the dial.

The two strings are identical today only by coincidence of authorship,
not by construction.

**Corrected during planning (reviewer, independently converging with
dev's own investigation): the `pypi.org` misdial is *unreachable*, not
merely harmless — and the reason why is itself the finding that
matters.** The Problem was first drafted claiming a box request matching
`pypi.org` is *admitted* and then dialed to Anthropic with the Anthropic
credential attached. Traced through source, this does not happen: every
complete `net-payload` is passed unconditionally to `anthropic::recognize`
(`dispatch.rs`, no route-conditional dispatch), and that function's
Stage 1 (`anthropic.rs`) hardcodes `req.domain != "api.anthropic.com" ||
req.path != "/v1/messages"` as an unconditional `Err(BadEnvelope)`. A
`pypi.org`-matched session's `net-payload` therefore has `domain =
"pypi.org"`, fails Stage 1, and is **denied** — it never reaches
`NetPayloadAdmit`, so `on_admit` is never called, so no dial happens.
(`register.rs`'s own comment already calls these entries "sample
**inert** GET destinations" — accurately, since they cannot complete a
payload today.)

**Why this is the sharper finding, not a smaller one: U4's hardcoded
envelope check is a load-bearing safety property, not redundancy.**
Reframed this way, `anthropic.rs`'s hardcoded `api.anthropic.com`/
`/v1/messages` pair is *currently doing the work* that "the register
fixes the destination" claims — and it looks exactly like the kind of
duplication a future cleanup removes ("why hardcode the domain when the
matched `Route` already carries it? derive it from the route"). That
change would read as a tidy-up in review and would make a misdial
reachable in the same commit. **This sprint must name U4's hardcoding as
load-bearing-until-the-dial-derives, not remove it and not let it be
read as removable** — deriving U4's envelope from the route in some
future change would remove a defense-in-depth layer, which is a fork to
route to this team, never a cleanup to take unilaterally.

**There are three copies of the destination, not two.** `ROUTES`'s
`Route::new(...)`, `serve.rs`'s `ANTHROPIC_ENDPOINT`, and `anthropic.rs`'s
hardcoded envelope pair — all three must agree today and nothing
enforces it. This sprint removes one (`ANTHROPIC_ENDPOINT`); the other
two (the register entry and U4's hardcoded check) still must agree by
authorship alone. "The register becomes the single source of the dialed
destination" is accurate for *which host gets dialed*, but is not the
same as *nothing else independently names that host* — U4's copy remains,
deliberately, as the defense-in-depth floor named above.

**Precisely what the remaining two-copy dependency risks, and what it
does not — sharpened during planning (dev), because the two failure
modes behave very differently.** *Editing* `ROUTES` to name a different
host is **not silent**: a box request matching the new entry is admitted
and stored by U3, `route_net_payload` derives the envelope from that
same stored `Route`, and U4's Stage 1 compares it against its own
hardcoded pair — a mismatch is `BadEnvelope`, denied, loud, fail-closed,
immediately visible. Drift between the two copies fails safe. The
converse is the stronger, load-bearing fact: for *any* payload that *is*
admitted, U4 has already proven the matched route's domain/path **equal**
`api.anthropic.com`/`/v1/messages` — and because this sprint makes the
dial follow that *same* stored `Route`, every dial after this sprint is
provably to `api.anthropic.com`, not by two constants happening to
agree, but because U4 checked the exact route object the dial uses. **The
fragility is therefore narrower than "the two copies must agree by
authorship"**: agreement is enforced by U4's fail-closed check on every
request, not merely hoped for. The one operation that actually opens a
misdial is **removing or weakening U4's check itself** — precisely what
Final State #6 guards, and stating the risk this precisely is what keeps
that guard aimed at the real danger (deletion) rather than a vaguer one
(drift) that isn't actually there.

**Severity, stated honestly, precisely, and narrower than first
drafted**: this is **not** a reachable misdial today, and fixing it is
not closing a live leak — it is making a documented tier-1 property
("the register fixes the destination") actually true by construction,
rather than true only because U4's own hardcoded copy independently
enforces it. The **fragility** this sprint closes is real, though: the
property currently holds by an accident of two independently-authored
constants agreeing, in a spot a future "obvious" cleanup could
silently break.

**Why it blocks Sprint 18's Slice 2 specifically**: Final State #10 there
requires proving the vetted destination *is* the TLS peer (and the
verified hostname, and the `Host` header). As things stand there is no
structural link between the register and the dial to prove — Slice 2
would produce a beautifully verified TLS connection to a destination
chosen by a constant in a bin file. The join cannot be a test; the
destination must become a structural derivation first.

## Goal

Make the register the single source of the dialed destination: the
matched `Route` — not an independent constant — determines what
`upstream_call` (via Sprint 18's `TlsPipe`) actually dials, so Sprint
18's Slice 2 can prove the vetted destination *is* the TLS peer against
something that is actually true.

## Final State

1. **`UpstreamSpawn::spawn`'s signature carries the destination.** Its
   shape changes from `spawn(&self, session: SessionId, body: Vec<u8>)`
   to one that also takes the matched `Route` (or the host/path it
   resolves to) — `AnthropicClient::spawn` dials *that* value, never
   `self.endpoint`.
2. **`UpstreamIngress::on_admit` sources the destination from
   `Dispatcher`.** A new, narrow accessor on `Dispatcher` (e.g. `fn
   route_for(&self, session: SessionId) -> Option<&Route>`) exposes the
   per-session stored `Route` that already exists for the U4 vet;
   `on_admit` reads it and threads it into `self.spawn.spawn(...)`. No
   new storage — the `Route` was already being kept; it was only
   discarded too early.
3. **`AnthropicClient::endpoint` and `serve.rs`'s `ANTHROPIC_ENDPOINT`
   constant are removed.** There is no independent destination input
   left to drift out of sync with the register — the register is the
   only place a destination is authored.
4. **The wiring is proven directly, not through an end-to-end admission
   that cannot happen — corrected during planning (reviewer): the
   originally-proposed "second registered route" test is unbuildable.**
   U4's Stage 1 hardcodes one envelope (`api.anthropic.com`,
   `/v1/messages`); *any* second route with a different domain is denied
   at that stage regardless of this sprint's changes (that hardcoding is
   Final State #6's defense-in-depth floor, untouched here) — so there is
   no admitted, dialable second route to observe a divergence with.
   **Two replacement proofs, both required**: (a) **the carry-through is
   proven below `route()`** — call `on_admit` directly with a chosen
   `Route` (bypassing the full recognizer pipeline, which cannot admit a
   second domain) and assert `spawn` receives *that* route's destination,
   proving the plumbing without needing an admissible second route; (b)
   **the divergence is proven *unrepresentable*, the stronger claim**
   (Sprint 15's capability-narrowing shape): after this sprint,
   `AnthropicClient` has no `endpoint` field and `ANTHROPIC_ENDPOINT`
   does not exist, so there is no longer anywhere for a dial to diverge
   from the matched route *to* — not "the dial follows the match in this
   test," but "there is nothing left for the dial to follow instead."
5. **No stale `Route` reaches a dial.** A `NetPayloadNoRoute` (no prior
   admitted `net-address`) still cannot reach `on_admit` with a
   destination at all — the existing fail-closed rung
   (`Routed::NetPayloadNoRoute`) is unchanged; this sprint only affects
   the already-admitted path.
6. **U4's hardcoded envelope check (`anthropic.rs`'s Stage 1) is named
   explicitly as a defense-in-depth floor, not incidental duplication,
   and is left untouched by this sprint.** Recorded precisely so a
   future change deriving U4's envelope from the matched route (removing
   what would then read as a redundant hardcoded pair) is recognized as
   a **design fork to route to this team**, not a cleanup to take
   unilaterally — that change would simultaneously make the credential-
   attachment gap dev found real (today unreachable *because* of this
   same hardcoding), so it must not be made silently.
   **Bound explicitly to the credential-scoping Non-Goal below, both
   ways, so the two are not resolved independently and out of order**
   (reviewer's finding — the same composed-defaults blind spot named in
   Sprint 16's retrospective, recurring here): this floor is the
   credential-scoping Non-Goal's *precondition*. If this fork is ever
   taken, credential scoping is no longer a deferred residual — it
   becomes a **required part of that same change**, not a separate
   follow-up filed later. Resolved in the wrong order (U4 derivation
   landed first, credential scoping "later"), the gap opens between the
   two commits while each looks locally justified at the time it is
   made.
7. The sprint document carries role retrospectives; the sprint is
   committed with only its intentional files.

## In Scope — ordered slices

**Slice 1 — thread the destination from the register to the dial.**
The whole change as one slice: `UpstreamSpawn::spawn`'s signature
(Final State #1), `Dispatcher`'s new accessor and `on_admit`'s use of it
(Final State #2), the removal of `AnthropicClient::endpoint`/
`ANTHROPIC_ENDPOINT` (Final State #3), the wiring-plus-unrepresentability
proof pair (Final State #4), and U4's hardcoded envelope check named
explicitly as a defense-in-depth floor rather than touched (Final State
#6). One slice because the pieces are only meaningful together — a
signature change with nothing threading through it, or a threaded value
with nothing removing the old constant, each leave the property
half-true. Gate: the direct `on_admit` wiring test passes; the compiler
confirms `AnthropicClient::endpoint`/`ANTHROPIC_ENDPOINT` no longer exist
anywhere in the crate (the unrepresentability proof); the existing test
suite passes unchanged otherwise (behavior-preserving for the
single-route case that is all production traffic exercises today); no
`NetPayloadNoRoute` path gains a destination it didn't have before; U4's
Stage 1 envelope check is unmodified and its defense-in-depth role is
documented in-code where a future editor will read it before "cleaning
it up."

## Non-Goals

- Do not touch Sprint 18's `upstream_tls.rs` — this sprint only changes
  what destination is handed to it.
- Do not build generic multi-destination dispatch beyond deriving the
  dial from whichever `Route` was actually matched.
- Do not attempt Sprint 18's Slice 2 gates (verbatim relay, panic fix,
  `ureq` removal, the peer-binding join itself) — this sprint is the
  prerequisite Slice 2 depends on, not a substitute for it.
- Do not change `Route`'s own structure, `ROUTES`, or U3/U4's
  recognizer — the matched `Route` already carries what is needed, and
  U4's hardcoded envelope check is a load-bearing defense-in-depth floor
  (Final State #6), not incidental duplication to simplify away.
- Do not add a credential-scoping mechanism (associating a route with
  "which credential, if any, it may carry") — the scenario it would
  guard against is unreachable today because of U4's hardcoded envelope
  check (Final State #6), and building the mechanism now, with no
  second real recognizer/service in the system, would be solving a
  problem that does not yet exist. **This Non-Goal's precondition is
  Final State #6 holding** — the two are bound both ways, not two
  independent facts (reviewer's finding, the same composed-defaults
  pattern from Sprint 16's retrospective): if U4's hardcoding is ever
  relaxed to derive from the route, credential scoping is **no longer
  deferrable** and must land in the *same* change, not be filed as a
  separate follow-up taken up later — never revisited as an isolated
  item on its own schedule.

## Starting Surface

Audit first, in this order:

1. `proxy/src/upstream.rs`'s `UpstreamSpawn` trait, `AnthropicClient`,
   and `UpstreamIngress::on_admit` — the exact seam this sprint threads
   the destination through.
2. `proxy/src/dispatch.rs`'s `routes: BTreeMap<SessionId, Route>` and
   where it is populated/read today (the U4 vet call site) — confirming
   the `Route` is already stored and only needs a read accessor, not new
   storage.
3. `proxy/src/register.rs`'s `ROUTES` — the three registered destinations
   (Anthropic, `pypi.org`, `docs.rs`; the latter two are explicitly
   commented "sample inert GET destinations").
4. `proxy/src/bin/serve.rs`'s `ANTHROPIC_ENDPOINT` and the
   `AnthropicClient::new(...)` call site — what this sprint removes.
5. `proxy/src/anthropic.rs`'s `recognize`, Stage 1 — the hardcoded
   `api.anthropic.com`/`/v1/messages` envelope check that is *currently
   the reason* a `pypi.org`/`docs.rs`-matched payload can never be
   admitted; this sprint's third copy of the destination, named and
   left untouched (Final State #6).

## Method

- **The `Route` is already stored — this sprint stops discarding it
  too early, it does not add new state.** The per-session map exists
  for the U4 recognizer vet; extending its reach to the dial is a read
  accessor, not a new mechanism.
- **Prove the wiring directly; the end-to-end divergence case cannot be
  driven at all.** Corrected during planning: with U4's Stage 1
  hardcoding one envelope, no second registered route can ever be
  admitted, so "dial follows the match, not a coincidence" cannot be
  shown by admitting a second route's payload — it is shown instead by
  testing the seam directly (Final State #4a) and by making the
  divergence structurally unrepresentable (Final State #4b), the
  stronger and preferred half.
- **U4's hardcoded envelope check is this sprint's floor, not its
  target.** It is currently the only thing preventing the credential-
  attachment gap dev found from being reachable; this sprint must leave
  it exactly as strict as it is today, and must say so where a future
  reader will see it before "simplifying" it.
- **This sprint is a prerequisite, not a merge.** Its close is Sprint
  18's Slice 2 gate precondition, not a redefinition of Slice 2's own
  scope or gates.

## Acceptance

- REJECT any fix that leaves an independent destination constant
  (`ANTHROPIC_ENDPOINT` or equivalent) alongside the register, however
  identical the two values are today.
- REJECT closing this sprint without both the direct `on_admit` wiring
  test (Final State #4a) and the compiler-verified structural absence
  of `AnthropicClient::endpoint`/`ANTHROPIC_ENDPOINT` (Final State #4b).
- REJECT any change to `upstream_tls.rs`, `Route`'s structure, or U3/U4's
  recognizer as part of this sprint — including any change, however
  framed as a cleanup, that derives U4's envelope check from the matched
  route rather than leaving it hardcoded (Final State #6); that is a
  design fork to route to this team, not this sprint's to take.
- REJECT treating Sprint 18's Slice 2 as unblocked before this sprint's
  Slice 1 gate is accepted.
- REJECT adding a credential-scoping mechanism as part of this sprint —
  named as an explicit residual (Non-Goals), not a gap to close now.

## Review Questions

None outstanding at contract time. The seam (thread the destination
through `UpstreamSpawn::spawn`, sourced from `Dispatcher`'s existing
per-session `Route` storage) was identified and verified by dev during
Sprint 18's own investigation; the PM has already ruled this a
prerequisite sprint rather than a Sprint 18 slice. The originally-drafted
Problem-section example (a reachable `pypi.org` misdial) was corrected
during planning (reviewer, independently converging with a correction
the lead had also reached): the misdial is unreachable today because of
U4's hardcoded envelope check, not merely harmless — resolved directly
against source, not an open design question. Whether to ever relax U4's
hardcoding to derive from the matched route is explicitly **not**
resolved here and is named as a future design fork requiring this team's
routing, should it ever arise.

## Retrospectives

### zero.proxy-lead

This sprint existed only because dev stopped mid-build to size a seam
question honestly rather than proceed on the contract as drafted — the
same discipline that made Sprint 18 productive in the first place kept
compounding here. My own contribution was mostly arbitration rather than
discovery: verifying the exact mechanism (`anthropic::recognize`'s
hardcoded Stage 1) before either crediting or correcting a claimed
security finding, since a claim this consequential deserved the same
source-level check as everything else in this arc, not extra deference
for being alarming. What I would do the same way again: when dev and
reviewer converge independently on the same correction from different
angles (as they did here, both finding the misdial unreachable within
minutes of each other), that convergence is strong enough evidence to
adopt directly and move on, rather than re-deriving it a third time
myself. What I'd sharpen: I drafted the original Problem section's
`pypi.org` example from dev's first report without verifying it myself
before writing it into a contract — the correct instinct (verify before
committing to paper) arrived only after the draft existed, not before.
For a security-relevant claim specifically, verify before drafting, not
only before finalizing.

**What the next sprint inherits**: Sprint 18's Slice 2 is now genuinely
unblocked — the peer-binding join has a real, structurally-guaranteed
destination to bind to, not a coincidence of two authored constants.
Reviewer's forward note (peer, SNI, and `Host` must all derive from the
*same* matched `Route` object, not three independent reads of it) is the
right thing to hold Slice 2 to, since it is the identical shape of gap
this sprint just closed, one level up.

### zero.proxy-review

**A severity claim is a claim.** The contract's Problem section stated, as
fact, that a request matching `pypi.org` is *admitted* and then *dialed to
Anthropic with the Anthropic credential attached* — a vivid, plausible, and
false example. Tracing it took three hops: `split_route` derives the recognizer
envelope from the session's **matched** route, `recognize` runs unconditionally
for every complete net-payload, and its Stage 1 compares that envelope against a
hardcoded `api.anthropic.com` / `/v1/messages`. A `pypi.org` session is therefore
denied `BadEnvelope` before it can reach admission, let alone a dial. The
correction ran in **both** directions at once, which is why it was worth making:
the exploit was *less* reachable than claimed, and the situation was *more*
fragile than claimed. The header "stated honestly, not inflated" is an invitation
to verify, not a reason to trust — a self-aware severity assessment is still an
assessment.

**A duplicate that is doing safety work reads as cleanup bait.** The real reason
the register-versus-constant divergence was harmless was not "the box cannot
change the constant" — it was U4's hardcoded envelope check, which independently
re-confirms the destination of every admitted request. That reframes an apparent
third copy of a string into a load-bearing floor, and the obvious tidy-up
("the matched `Route` already carries the domain and path — read them from it")
would have removed it while reading as a simplification in review. The
generalizable form: **when a sprint removes one of N copies of a constant, check
whether any remaining copy is a *check* rather than a duplicate.** The remedy
landed in the right place — directly above the comparison, not in a sprint doc —
so the warning reaches whoever is about to make the change.

**When the behavioral proof is foreclosed, the structural proof is primary, not
a fallback.** The contract's single gate — a test with a second registered route
proving the dial follows the match — could not be built, because no second route
can ever be admitted while U4 hardcodes one envelope. The replacement pairs a
wiring test (the dial receives exactly what `route_for` returned, with an
explicit anti-vacuity assertion that the matched value really is the register's
entry) with a structural proof that the divergence is *unrepresentable*: no
`endpoint` field, no `ANTHROPIC_ENDPOINT` constant, nothing for a dial to diverge
to. The second is what actually closes it, and would have been the stronger
choice even if the first had been buildable — the Sprint-15 capability-narrowing
lesson again, arrived at from the opposite direction.

**The composed-defaults pattern, third instance in this arc.** Credential
scoping was deferred *because* U4's hardcoding makes its scenario unreachable;
U4's hardcoding was simultaneously flagged as a removable design fork. Each
decision is locally correct and their combination is unsafe if resolved in the
wrong order by different people at different times — the same shape as Sprint
16's provisioning-versus-writability pair, and as the Sprint-12 contract
parenthetical that Sprint 15 finally built. The remedy is the same each time and
it is cheap: cross-reference both ways, so whoever picks up either item sees the
other. Worth treating as a standing question at any gate that resolves two
related residuals — not "is each correct?" but "what happens if these are taken
in the wrong order?"

Two choices in the implementation deserve the record: the test helper drives the
**real** admission path and asserts on it, moving nine tests off injected state
rather than onto it; and when `Route`'s canonical form refused to express a
`http://localhost:PORT` mock destination, the tests moved instead of the type
gaining a test-only constructor. The second is the more important — a bypass
added for test convenience is precisely the kind that survives review by looking
harmless.

### zero.proxy-dev

**Where the finding came from.** Not from looking for it. Reviewer had added a third leg to Sprint 18's
peer-binding join — the `Host` header, box-influenced once we hand-roll the request write — and I went to
check whether the Host header could be derived from the vetted route. It could not, because *nothing* was:
`AnthropicClient` held its own `endpoint` string, `UpstreamSpawn::spawn` never received the matched
`Route`, and the register's authority over the destination was a property of two files agreeing rather
than of the code. The finding was one level underneath someone else's finding, and I would not have gone
looking there without theirs to follow.

**What I got wrong, and the correction was better than the finding.** I reported the `pypi.org` misdial as
"reachable, not theoretical" — a request admitted for one registered route, dialed to another, credential
attached. It is not reachable: `anthropic::recognize`'s Stage 1 hard-denies any domain but
`api.anthropic.com` before a payload can ever be admitted. I had traced the dial and traced the header
attachment and not traced the gate standing between them, then stated the conclusion with more confidence
than the tracing supported — and in the direction that inflates severity, which is the worse direction to
be wrong in when the audience is deciding scope.
What came back was sharper than what I sent: there are **three** copies of the destination, not two
(`ROUTES`, `ANTHROPIC_ENDPOINT`, and U4's hardcoded envelope pair), this sprint removes one, and U4's
hardcoding is therefore *currently doing the work* that "the register fixes the destination" claims —
load-bearing defense-in-depth wearing the costume of redundancy, and precisely the kind of thing a future
cleanup deletes while calling it tidying. The honest version of my finding was smaller in severity and
much larger in what it explained.

**The same shape twice, in one arc.** The pypi error and Slice 1's refusal-arm assertions (Sprint 18) are
the same mistake: I checked the thing in front of me and not the thing immediately adjacent to it. There I
wrote a success arm *specifically* so refusals could not pass vacuously and then wrote the refusals so they
could; here I traced two ends of a path and not its middle. Both were caught by review, and both were
cheap to fix — but the pattern is mine to watch, and it is not a knowledge gap. It is stopping the trace
at the boundary of what I was already thinking about.

**What the build taught, that planning did not.** Three things surfaced only from making the change:
- **`Route` deliberately cannot name a local test server.** Its canonical form forbids `http://` and ports
  — which is exactly the property that makes it safe — so two existing tests that pointed the seam at a
  mock server could no longer construct one. That is a fork with a tempting wrong answer: add a test-only
  variant accepting `http://localhost:PORT`. Moving the tests down to the layer that actually takes a URL
  was the right call, and reviewer named why better than I would have: a bypass added for test convenience
  is the kind that survives review by looking harmless. **When a type's strictness makes a test awkward,
  the strictness is usually the feature.**
- **Nine tests were standing beside the real path rather than on it.** They called `on_admit` against a
  bare `Dispatcher` and broke the moment `on_admit` required a matched route. The fix was not to inject
  the route behind the API but to drive a real `net-address` through `route()` — which is how production
  gets there. A test that breaks when you add a genuine precondition was testing something slightly
  unreal, and the break is the useful signal.
- **Structural absence is enforced, not asserted.** FS#4b needed no test: removing `endpoint` made the
  compiler point straight at `serve.rs`'s use of `ANTHROPIC_ENDPOINT`. "There is nothing left to diverge
  to" is a claim the type system holds, and it is stronger than any test of "the dial followed the match
  this time."

**What Sprint 18 Slice 2 inherits.**
- **Reviewer's one-object rule, which is the direct continuation of this sprint**: the TLS peer, the SNI,
  and the `Host` header must derive from the *same* matched `Route` value, not from three separate reads
  that happen to agree. Three derivations converging is the identical gap shape this sprint just removed —
  agreement by authorship rather than by construction — and it would be a real regression to close the
  register-to-dial gap and immediately reopen it three ways inside the request builder.
- **U4's hardcoded envelope must not be derived from the route.** It is now documented in `anthropic.rs`
  as load-bearing, and its removal is bound both ways to the credential-scoping deferral: relax one and
  the other stops being deferrable in the same change, not as a follow-up.
- **A concrete warning, from the constraint above**: Slice 2's verbatim-relay proofs will want a local
  TLS server, and `Route` cannot name one for the same reason it could not here. Expect to exercise
  `upstream_tls` directly rather than through the `Route`-typed seam — and expect that to be right, not a
  workaround.
- The dial now derives from the register, so the peer-binding join finally has something real to bind to.
