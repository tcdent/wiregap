# Sprint 015 — The Head-Browses-Box File Channel

> Proxy team, sprint 15, wiring the head→box file-browse verbs (`list`,
> `request-file`) and their box-authored answers (`list-entry`, `file-error`)
> through the real head connection Sprint 12 built. Resolves the
> `list-entry`/`file-error` discrepancy reviewer flagged at Sprint 12's gate
> (the contract named them as box→head egress; only `display`/`pane-state`
> were wired). Drafted by zero.proxy-lead from planning input by
> zero.proxy-dev and zero.proxy-review (2026-07-24).

---

## Scope Boundary

This sprint owns the head-initiated file-browse channel: the head asking to
list or extract files from the box's own filesystem, and the box's answers
relayed back opaquely.

1. **`request-file` (head→box, 0x26)** — a fire-and-forget relay; the box's
   answer is a normal `file-write` session to quarantine, reusing Sprint 9's
   mechanism verbatim. No new correlation, no new session model.
2. **`list` (head→box, 0x24) / `list-entry` (0x25) / `file-error` (0x27),
   box-authored answers** — a standalone opaque relay, origination-keyed
   (not verb-content-keyed), structurally disjoint from the proxy's own
   confined filesystem service.

It explicitly does **not** own:

- **Reusing or extending `route_head_input`.** Confirmed dictated by the
  existing code, not a design preference: `route_head_input` gates on
  `terminal_displays.contains(&s_display)` and stamps a pty-specific
  correlation — a file-browse request has no display/pty context at all, so
  reusing it would always hit the fail-safe `Dropped` branch. This sprint
  builds a standalone relay instead.
- **Any wire-level correlation token for `list`'s answer.** Settled during
  planning (Method) — `list` is a one-shot, self-describing request answered
  by a genuine data frame, not the PTY's forced two-session-plus-correlation
  shape. Inventing a token here would be over-applying the PTY's solution to
  a case that doesn't need it.
- **Any content decision on relayed file-browse traffic.** The relay stays
  opaque both directions, MB3 unchanged — unsafe names are display-only at
  the head (`file.md` §0/§3), never inspected or logged by the proxy.

## Problem

`terminal_relay::direction_of` only covers the pure terminal verbs (`display`,
`pane-state`, `create-pty`, `key`, `resize`, `viewport`, `mouse`, `paste`,
`focus`) — `list`, `request-file`, `list-entry`, and `file-error` all hit its
`None` arm today, so nothing relays them. This is the exact gap reviewer
flagged at Sprint 12's gate: the contract named `list-entry`/`file-error` as
box→head egress, but only `display`/`pane-state` were ever wired.

Planning surfaced why this is harder than adding rows to a direction table:
`list` (0x24) is explicitly dual-role (`verb.rs`: "box→proxy/head→box"), and
`list-entry` (0x25) / `file-error` (0x27) are directionless-by-verb-alone
(marked "response" / "either"). The **same wire verb** already means two
different things depending on where it originates: a box-initiated `list` of
the *trusted host's* readable root (already built, Sprint 6/9, answered by
the proxy's own confined `fs_read`/`DirReader` via `ReplySource::Dir`) versus
a head-initiated `list` of the *box's own* filesystem, answered by the box
itself and relayed opaquely to the head. A verb-content lookup cannot tell
these apart — the router must key on **origination** (which of the composed
loop's three disjoint flows a frame arrived on), never on decoding the
payload.

This dual-role overlap creates a genuine security floor, not just a routing
puzzle: a head-initiated `list`/`request-file` must **never** reach the
proxy's own confined filesystem service (`route_list`, `route_file_read`, the
quarantine writer). If it could, the head would be using the proxy as a
**confused deputy** to browse the trusted host's filesystem — ground the head
must never reach. This is true *today* only as an emergent property of the
composed loop's control flow (head-socket frames are processed exclusively in
`drain_head_input`, which never calls `dispatcher.route`) — not yet an
*enforced* one, and planning was explicit that emergent-but-unproven is not
good enough for a security floor.

## Goal

Wire `request-file` and the `list`/`list-entry`/`file-error` file-browse
channel through the real head connection, opaque and origination-keyed both
directions, with the confused-deputy floor made **structurally
unrepresentable** (not merely unused) and proven by a real disjointness test
in each direction — not assumed from today's code shape.

## Final State

1. `request-file` (head→box) is relayed opaquely via the standalone file
   relay (below); the box's answer is an ordinary `file-write` session to
   quarantine — Sprint 9's mechanism, reused verbatim, diffed empty against
   its certifying commit. No proxy action is taken on `request-file` itself
   beyond relaying it.
2. A **standalone head→box file-browse relay** (distinct from
   `route_head_input`, which structurally cannot serve this — Problem)
   relays both `request-file` and `list` head→box opaquely via a single
   **one-shot `reply.open(payload)` call** (`relay_head_file`) — **corrected
   from this Final State's original reply-queue-keyed design, superseded
   during Slice 2's build**: `list`'s head→box request, like
   `request-file`'s, is a single fire-and-forget frame, not an ongoing
   stream — it needs no pacing, no backpressure, and therefore no
   `ReplyStream`/queue/key at all (`Reply::open` is infallible; only
   `send`, used for continuing a session, can `WouldBlock`). The pty input
   relay's reply-queue-keyed-by-`s_display` mechanism was never the right
   comparison: it exists for pacing an *ongoing* stream against an
   *existing* display session, neither of which applies here. The
   pre-existing box-*initiated* host-list (`ReplySource::Dir`, proxy-
   authored, answering the box's own request) is unaffected and keeps its
   own reply-queue mechanism unchanged — only the *head*-initiated
   `list`/`request-file` relay is one-shot.
3. **Origination-keyed routing for the security floor, verb-peek for the
   egress split** (tightened at review, second pass): there are **two**
   physical ingress origins, not three — **NIC** (`Event::Delivered`) and
   the **head socket** (`drain_head_input`) — and the confused-deputy floor
   rests on *that* distinction: a head-socket frame never reaches
   `dispatcher.route`/`fs_read` at all, full stop. Within the **NIC**
   origin, a box-initiated `list`/`file-read` of the *trusted host* (routed
   proxy-authored via `pump_replies`) and a box-authored answer to a
   head-initiated `list` (routed to `send_to_head`) are split by a
   **verb-peek** (id only, e.g. `list` 0x24 vs `list-entry` 0x25) — the same
   id-only classification the existing `display`/`pane-state` routing
   already uses, never a body decode. So: the security floor is
   origination-keyed (which of the two ingress origins), and the box→head
   egress classification within the NIC origin is verb-peeked — both
   legitimate, neither in tension with the other.
4. **The confused-deputy floor is structural, not emergent**: the standalone
   head-file relay has **no handle** to `route_list`, `route_file_read`, or
   the quarantine writer — the deputy path is unrepresentable in the relay's
   own code, not merely unreached at runtime (the box-reachable-core
   capability-narrowing discipline applied to this seam).
5. **Two disjointness properties, each proven by a real test, not assumed**:
   (a) a box-emitted `list-entry`/`file-error` answering a head-initiated
   `list` reaches the head and the box-facing reply-queue nowhere; (b) a
   proxy-authored `list-entry`/`file-error` answering a *box-initiated*
   host-list (the trusted-host view) reaches the box and the head nowhere —
   the sharper property, since a leak here would expose the confined host
   filesystem listing the head must never see.
6. The sprint document carries role retrospectives; the sprint is committed
   with only its intentional files.

## In Scope — ordered slices

**Slice 1 — `request-file`, the clean reuse.**
Extend the relay to route `request-file` head→box opaquely; confirm the
box's quarantine-write answer needs no change (Sprint 9 unchanged). Gate:
reuse-by-diff (the quarantine write path empty-diffed against its Sprint-9
certifying commit); confirm the Sprint-9 certification already exercised the
exact write behavior relied on here; REJECT any proxy action taken on
`request-file` beyond relaying it opaquely.

**Slice 2 — the standalone `list` file-browse relay, origination-keyed.**
The new relay (Final State #2–#4) plus the origination-keyed routing across
the two ingress origins (NIC, head-socket), with a verb-peek splitting the
box→head egress within the NIC origin. Gate: the confused-deputy floor
proven two ways — **structural** (the relay's own code has no reachable
path to `route_list`/`route_file_read`/the quarantine writer — capability-narrowing,
not a runtime check) and **behavioral** (a head-socket `list`/`request-file`
naming a trusted-host path, e.g. an absolute path outside the box's own
filesystem, must reach the box relayed, must **never** invoke the proxy's
own confined reader — proven with a fake confined-reader that records/panics
if touched on a head-origin path — and must produce no host-listing content
at the head). Plus the two disjointness tests (Final State #5) in both
directions, each with a distinguishing nonce so a leak is un-fakeably
detectable. **A completeness proof at the exact split point** (added at
review, verified against source — `dispatch.rs:652-654` today lumps
`FILE_DATA | FILE_ERROR | LIST_ENTRY | REQUEST_FILE` into one
`Routed::Relay(RelayKind::FileRead)` arm, with no split between the
proxy-authored-only verb (`file-data`, which the box never emits — stays
routed to the box) and the box-emitted answers (`list-entry`/`file-error`,
which must route to the head when they arrive inbound as an answer to a
head-initiated `list`): enumerate every verb this arm currently covers and
confirm each is classified correctly at this split, not merely that the two
disjointness tests pass — the disjointness proofs show the paths don't
*cross*; this proof shows the classification at the split is *complete*, the
same enumeration-not-assumption discipline as every other `checked=0`
catch this team has made. An inbound (NIC-origin) `request-file` is itself
anomalous (it is a head-originated verb and should never arrive via NIC) —
confirm it fails safe, not silently. `list`'s answer carries no correlation
token (confirm, don't invent one). MB3 holds: no content decision on the
relayed payload in either direction.

## Non-Goals

- Do not reuse or extend `route_head_input` for file-browse traffic — the
  code's own preconditions (`terminal_displays` gating) make this
  structurally wrong, not a style choice.
- Do not add a wire-level correlation token to `list`'s answer — `model.md`
  §9's fire-and-forget/observation model already covers this case; a token
  would be unneeded mechanism forced onto a case that doesn't need it.
- Do not let the confused-deputy floor rest on "the code paths happen to
  differ today" — it must be structurally unrepresentable and independently
  tested, not an emergent property nobody verified.
- Do not add any inspection, filtering, or logging of unsafe names/paths in
  either relay direction — the existing `file.md` §0 type split
  (safe/unsafe, loggable/display-only) already governs this, unchanged.
- Do not relitigate the PTY two-session ruling or Sprint 12's accepted
  terminal-relay wiring — this sprint adds a sibling relay, it does not touch
  the existing one.

## Starting Surface

Audit first, in this order:

1. `docs/proto/file.md` §0 (the safe/unsafe segment-stack type split), §3
   (`list`'s bidirectional wire shape, the combined-record answer), §4
   (`request-file`'s fire-and-forget-then-quarantine-write model).
2. `docs/proxy/contract/proxy-head.md` §1 (the relay/decode pin) and the
   `list-entry`/`file-error` box→head egress the contract already named.
3. `proto/src/verb.rs`'s `LIST`/`LIST_ENTRY`/`REQUEST_FILE`/`FILE_ERROR`
   constants and their direction annotations — the dual-role tell.
4. `proxy/src/terminal_relay.rs`'s `direction_of`/`relay_frame` — the
   existing pattern this sprint's new relay sits beside, not inside.
5. `proxy/src/dispatch.rs`'s `route_head_input` (why it cannot be reused —
   Problem/Non-Goals) and `ReplySource::Dir` (the existing proxy-authored
   host-list path this sprint must never let a head-origin frame reach).
6. `proxy/src/head.rs`'s `run_adapter_composed`/`drain_head_input` — the two
   ingress origins (NIC, head-socket) the confused-deputy floor rests on,
   and the egresses (`pump_replies`, `send_to_head`) a verb-peek within the
   NIC origin splits between.

## Method

- **MB1–MB6** apply to every new line. MB6 stays `{open_discipline,
  privilege}` — no new proxy-owned `unsafe`.
- **The §9 fire-and-forget/no-correlation-token model is settled, not
  routed**: verified during planning against `model.md` §9's own text, and
  against why the PTY's two forcing conditions (an ongoing bidirectional
  stream; an ACK/empty-data byte-identity collision) don't apply to a
  one-shot, self-describing request answered by a genuine data frame. Cite
  it; do not re-derive it per slice.
- **Origination is a structural fact of the composed loop, not a new
  routing mechanism**: the two ingress origins (NIC, head-socket) already
  exist from Sprint 13's composition and are what the confused-deputy floor
  rests on; within the NIC origin, a verb-peek (id only, mirroring the
  existing `display`/`pane-state` pattern, never a body decode) classifies
  the box→head egress split.
- **The confused-deputy floor gets the box-reachable-core capability-
  narrowing treatment**: the standalone relay must have no handle to the
  proxy's own confined filesystem primitives, so the deputy path cannot be
  called, not merely happens not to be called. Corroborate structurally with
  a behavioral test, not one or the other alone.
- **Disjointness is proven with a distinguishing nonce each direction**, not
  asserted from the code's apparent shape — the same discipline this team
  has applied to every reply-correlation and fail-safe proof so far.

## Acceptance

- **Slice 1.** REJECT any proxy action taken on `request-file` content
  beyond opaque relay; REJECT a re-implemented (rather than reused) write
  path.
- **Slice 2.** REJECT reusing or extending `route_head_input` for
  file-browse traffic; REJECT any correlation token added to `list`'s
  answer; REJECT a confused-deputy proof that rests on behavioral testing
  alone without the structural (capability-narrowing) half, or vice versa;
  REJECT a disjointness claim (either direction) without a distinguishing
  nonce proving the leak-path is actually empty, not merely unobserved;
  REJECT a classification split at `dispatch.rs`'s `FILE_DATA`/`FILE_ERROR`/
  `LIST_ENTRY`/`REQUEST_FILE` arm that isn't enumerated verb-by-verb (the
  two disjointness tests proving the paths don't cross is not the same
  proof as the split being complete and correct); REJECT any content
  inspection creeping into either relay direction.
- **Sprint-level.** REJECT closing this sprint without explicitly
  confirming Sprint 12's `list-entry`/`file-error` contract-vs-code
  discrepancy is now resolved (the box→head role is real and built here).

## Review Questions

None outstanding — the §9-vs-PTY distinction and the `route_head_input`
reuse question were both settled during planning from the committed model
and the existing code's own preconditions, not routed as open forks.

## Retrospectives

### zero.proxy-dev

**What I built.** Slice 1: `request-file` relayed opaquely head→box via a one-shot `reply.open`. Slice 2:
the standalone `list` relay, the origination-keyed split of the lumped file-relay arm (box-authored
`list-entry`/`file-error` → the head; anomalous inbound `file-data`/`request-file` → fail-safe drop), and
the confused-deputy floor proven structurally + behaviorally.

**The open-session red herring — a fork I manufactured from a doc-list, dissolved by a code-read.**
Slice-1 diligence surfaced what looked like a real design fork: `open-session` (0x00) sits in the
committed head→box relay set (`proxy-head.md` §1), so I suspected `request-file` might have to *ride* an
explicit `open-session` — the head's session-open primitive — and that building a self-opening relay
might duplicate or pre-empt it. I leaned that way and flagged it as possibly needing a designer ruling on
`open-session`'s role. I was right to surface rather than guess — but the fork itself was largely my own
construction. The lead resolved it in one code-read: the *only* head→box origination that exists
(`route_head_input`) does not touch `open-session` at all — it uses `Reply::open`'s lazy self-open
(`dispatch.rs:318`); `open-session` is a separate verb handled only box-side. Its presence in the
relay-set *list* is just "a thing the head may send opaquely," not a gate on origination. So no ruling was
needed. The lesson refines [[validate-against-committed-seam-not-the-doc]]: when a fork hinges on *"does
committed structure X gate mechanism Y,"* read what Y actually **does in code** before escalating — I
weighted a doc-list membership as a structural constraint, and the real mechanism (`reply.open` originates
fresh sessions) answered the question directly and simpler. Surface-don't-guess was the right instinct;
the manufactured significance was the miss.

**The confused-deputy floor as capability-narrowing, made concrete.** The security property — a head
browse must NEVER reach the proxy's own confined trusted-host reader — was *true today* by ingress
separation (head-socket frames never call `route()`). The reviewer's insistence was the load-bearing move:
emergent-but-unproven is not good enough for a security floor. So I made it **unrepresentable**:
`relay_head_file` takes only the `Reply` handle, **not** the `Dispatcher` — a function with no name for
`route_list` cannot call it, and the deputy path is uncompilable, not merely unused. The behavioral canary
(a real trusted-host file that would leak if a future edit ever routed a head browse through `route()`)
guards the *wiring*, which still holds the dispatcher; the structural narrowing guards the *relay*, which
doesn't — both, because the wiring can be miswired even though the relay can't. The durable point: for a
security floor, "the code paths differ today" is a runtime accident; "the code cannot name the dangerous
primitive" is an invariant — and the second is worth the small refactor (factor the relay to take only
`reply`) to buy.

**`list` turned out simpler than the planning distinction anticipated — and I corrected the record, not
just the code.** Both the reviewer and I anticipated `list` needing the reply-queue (multi-frame, tracked
map entry) *unlike* request-file's one-shot open — a "deliberate distinction" the planning named. Building
it, the distinction dissolved: a `list` REQUEST is a single frame, so the head→box relay is one-shot
`reply.open`, identical to `request-file` — the two share the one `relay_head_file`. The multi-frame
reply-queue is used only by the PRE-EXISTING box-initiated host-list (proxy-authored answer), untouched;
the box→head `list-entry` answer is multi-frame but relayed per-frame via `send_to_head`, not a
reply-queue. The anticipated distinction was the box-host-list's shape projected onto the head-list, which
doesn't share it. I surfaced the correction rather than quietly build the simpler thing and leave the
contract's "list will need the reply-queue" framing standing — the lead reconciled Final State #2 to the
one-shot mechanism. An anticipated design distinction, however reasonable at planning, is a hypothesis the
build tests; when it comes out simpler, correcting the *doc* keeps the record honest, not only the code.

### zero.proxy-review

Three things from this sprint are worth carrying forward.

**Capability-narrowing done right — unnameable beats unused.** The confused-deputy
floor (a head-initiated browse must never reach the proxy's own confined trusted-host
reader) could have been satisfied by control flow — "the head-ingress path happens not
to call route()". Planning was explicit that emergent-but-unverified is not good enough
for a security floor, and the build delivered the stronger form: `relay_head_file`'s
signature takes only `&mut dyn Reply`, no `Dispatcher` at all — so `route_list`/
`route_file_read`/the quarantine writer are *unnameable* from it. The deputy path is
unrepresentable, not merely unreached. That is the capability-narrowing discipline:
narrow the handle so the dangerous call *cannot be written*, then guard the wiring with
a behavioral canary test on top. The general lesson: for a security floor, prefer
"the code cannot name what it must not do" over "the code happens not to do it."

**Completeness is a distinct proof from disjointness.** The two nonce disjointness
tests prove the two exercised paths do not CROSS (a box-emitted answer to the head only;
a proxy-authored host-listing to the box only). But a dual-role split has a second
failure mode the disjointness tests do not touch: a verb landing in the WRONG arm of the
split. The old lumped `FILE_DATA|FILE_ERROR|LIST_ENTRY|REQUEST_FILE` arm is exactly where
a mis-split would hide — sending file-data to the head, or dropping file-error that
should relay. So the gate needed an enumeration-completeness test pinning ALL FOUR verbs
at the split point (list-entry/file-error → FileToHead; file-data/request-file →
Unsupported fail-safe), not just the tested two. Disjointness proves the paths are clean;
completeness proves the classification is total — checked=0 on the unenumerated verbs
proves nothing.

**Closing my own earlier flag explicitly.** The box→head role for `list-entry`/
`file-error` this sprint builds is the exact discrepancy I raised at Sprint 12's gate —
the contract named those verbs as box→head egress but only `display`/`pane-state` were
wired. Rather than let that resolution be implicit, the sprint-level acceptance required
confirming it out loud, and I did: the role is real and built, proven by disjointness
test (a). A reviewer's flag isn't closed until the thing it pointed at is demonstrably
built, and saying so explicitly keeps the record honest.

One doc-currency note landed during the gate: the plan specified a reply-queue-keyed
relay for `list`, but `list`'s head→box request turned out to be one-shot too, so it
shares `request-file`'s `relay_head_file` (infallible open, no queue) — Final State #2
reconciled to the as-built mechanism.

### zero.proxy-lead

**A held-until-verified fork resolved by one code-read, not a designer ruling — the
sharpest instance yet of "check what the mechanism does before treating structure as a
fork."** Dev's open-session question looked exactly like the PTY directionality
question that needed the designer two sprints ago: a plausible reading of a document
(open-session sitting in the head→box relay set) suggesting a load-bearing session-
origination rule. The difference was checkable in-repo — one grep into
`route_head_input` showed the actual mechanism is `reply.open()`, unrelated to
open-session entirely. The lesson isn't "always resolve forks yourself" — Sprint 12's
PTY question was genuinely undecidable from source, and routing it was correct. The
lesson is narrower and more useful: a fork framed as "does structure X gate mechanism
Y" is answered by reading what Y's *existing, working instance* actually does, before
asking whether X applies at all. Dev held rather than guessing, which is what made the
one-read resolution possible instead of a wrong assumption shipping quietly.

**The confused-deputy floor is this sprint's clearest realization of a discipline this
team has stated many times but rarely gotten this concretely right: capability-
narrowing over behavioral avoidance.** `relay_head_file(reply: &mut dyn Reply, ...)`
doesn't take a `Dispatcher` — not "doesn't call it," *cannot name it*. That's a
stronger claim than any runtime check could make, and reviewer's canary test
corroborates it without weakening it (the test exists to catch a *future* miswiring,
not to substitute for the type-level guarantee). Worth carrying forward as the default
shape for any future confused-deputy-shaped floor: narrow the parameter list until the
dangerous call doesn't type-check, then test the wiring around it — not the reverse.

**What the next sprint inherits.** The list-is-one-shot correction folded into Final
State #2 is now the accurate record — a future reader won't be misled into expecting
reply-queue machinery that was never built. No cross-team dependency or held item
carries forward from this sprint; the head-browses-box channel is complete and
self-contained.
