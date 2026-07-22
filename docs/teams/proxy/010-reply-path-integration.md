# Sprint 010 — Reply-Path Integration (Phase 4, continued)

> Proxy team, sprint 10, continuing Phase 4 now that the NIC team's Task 15
> (originator-partition, `a5a215c`) has landed and both endpoints can originate. The
> team widens the proxy's own reply seam and then wires every receive-then-reply pair
> the reply path unblocks: file acting (`file-read`/`list`), the relay reply halves
> (`cloud_relay`/`terminal_relay`), and the JC-3 deny-notifications. Drafted by
> zero.proxy-lead from planning input by zero.proxy-dev (2026-07-22); Slice 1
> green-lit by zero.proxy-pm to proceed while the reply-session-correlation question
> is with the NIC designer.

---

## Scope Boundary

This sprint owns wiring the box→proxy reply direction now that R-c is resolved: the
proxy can originate a **new** proxy→box session (`Endpoint::Proxy`, the high half of
the id space) from inside `directpath::Subsystem::run`'s callback, via `Ctx::open`/
`Ctx::send`. It owns:

1. **The reply seam itself** — widening the proxy's `NicRun` trait so the
   dispatch/lifecycle layer can originate a reply, without collapsing the
   hardware-free test suite onto real hardware.
2. **File acting + reply** — `file-read`/`list`'s decode-and-confine (unbuilt since
   Sprint 6/9, previously origination-gated) plus streaming `file-data`/`list-entry`
   back to the box.
3. **The relay reply halves** — `cloud_relay`'s and `terminal_relay`'s box-facing
   sends (their encode/relay logic already exists per Sprint 6; only the wire-send
   was gated).
4. **JC-3's deny-notifications** — `file-error`/`net-error` reaching the box for a
   policy deny, previously deferred on every deny path across Sprints 8 and 9.

It explicitly does **not** own:

- **The reply-session-correlation mechanism (F1).** When the proxy replies, it opens
  a session the transport does not link to the original request by any built-in
  signal (origination is now symmetric on both halves). How the box matches a reply
  session back to its request is a genuine cross-team question, **with the NIC
  designer as of this draft** (routed by the PM, 2026-07-22). This sprint's Slice 1
  needs no answer (no new payload/correlation surface); Slices 2–4 need it before
  their reply payload format is final — named as a designer fill-in, not guessed at.
- **Re-litigating the request-direction correlation** (Sprint 8's per-session `Route`
  binding) — settled, unchanged, and not reopened here.
- **The config DSL** or any of Sprint 8/9's named config-gated placeholders
  (rung-3 authorities, `quarantine_root`'s eventual real source).
- **Anything the NIC team owns downstream of Task 15** — the `directpath` content-hash
  pin re-derivation to a real commit sha (R-7, now finally possible since `a5a215c` is
  committed) is this sprint's to *do*, but the facade itself is not this sprint's to
  change.

## Problem

Every prior sprint's Non-Goals carried the same line: box-facing sends are blocked on
R-c. That block is now lifted structurally (`Ctx::open`/`Ctx::send` exist and work),
but the proxy's own `NicRun` trait (Sprint 8) was built to the *opposite* assumption:
it deliberately discards `Ctx` ("the proxy never originates in response" — `nic.rs`'s
own doc comment), and `run_adapter` never sees a reply handle. Four pieces of already-
built or partially-built logic are waiting behind that gap: `fs_read::FileReader`
(reads a file, but nothing streams the bytes back), `cloud_relay`/`terminal_relay`
(encode/relay logic built, wire-send gated), `file-read`/`list`'s decode-and-confine
(unbuilt entirely — Sprint 9 confirmed no call site exists), and every existing deny
path's JC-3 notification (silently deferred since Sprint 8).

## Goal

Widen the reply seam once, reviewed in isolation, then wire the four waiting
receive-then-reply pairs against it — closing R-c's practical consequences without
guessing at the one cross-team question (reply-session correlation) that is
genuinely not the proxy team's to answer alone.

## Final State

1. A narrow, proxy-owned `Reply` trait (`open`/`send`, mirroring `Ctx`'s shape but
   never naming `directpath::Ctx` in the core) is exposed through `NicRun::run`'s
   callback. `RealNic` adapts the real `&mut Ctx` to `&mut dyn Reply`; the test fake
   hands the callback a **recording** `Reply` that captures every originated session
   and payload — so a reply is assertable in a hardware-free test for the first time.
2. `run_adapter` (or its successor) can call `reply.open(..)`/`reply.send(..)` from
   inside the `Event::Delivered`/`Event::Idle` arms, still only for the duration of
   that callback invocation (the same per-invocation lifetime `Ctx` already has —
   nothing is held across calls).
3. `file-read`/`list` are genuinely wired: decode (reusing `proto`'s existing types),
   confine (reusing `fs_read`'s existing `FileReader`, unchanged per its own
   fail-safe read-path discipline), and stream `file-data`/`list-entry` back over the
   new `Reply` seam — the first time these verbs are *acted on* rather than relayed.
   JC-1's reserved-byte check (Sprint 9) extends to both, per that sprint's own named
   follow-up.
4. `cloud_relay`/`terminal_relay`'s existing encode/relay logic is wired to actually
   send over the new `Reply` seam — no change to the encoding logic itself, only the
   send-path connection.
5. Every existing deny path that JC-3(b) covers (`ReservedDeny`, `NetDeny`,
   `FileWriteDeny`, `NetPayloadDeny`, `NetPayloadNoRoute`, `FileChunkDeny`'s policy
   sub-cause) notifies the box (`file-error`/`net-error`, `code 2`
   denied or `6` malformed, per `box-proxy.md` §2 JC-3) now does, over the same seam.
6. The reply-session-correlation mechanism (F1) is **fully settled** (NIC designer,
   then proxy-designer ruling, 2026-07-22): the transport carries **zero** pairing
   between a reply session and its request — a reply is an independent session, and
   the box's `Event::Delivered(SessionId, payload)` carries no link to the request —
   so correlation is app-level, carried in the reply payload. The **shape** is
   settled and uniform: every reply-direction verb (`file-data`, `list-entry`,
   `net-response`, etc.) gains one `in-reply-to: SessionId` field, echoing the
   request session id the proxy already holds, emitted on **every** reply frame, no
   per-verb variation. Emission traffic with no originating request (`display`,
   spontaneous notifications) carries no correlation key at all. The field's formal
   wire spec (exact byte layout per verb) is proto's Task 16, **queued but not yet
   written** — Slice 2 proceeds against the settled shape as a marked fill-in for the
   exact layout, not a guess at the mechanism itself.
7. `contract/proxy-nic.md`'s R-7 residual is discharged: the content-hash pin re-pins
   to `directpath`'s real commit sha (`a5a215c`), since the facade is now committed.
8. The sprint document carries role retrospectives; the sprint is committed with only
   its intentional files.

## In Scope — ordered slices

**Slice 1 — the reply seam, isolated. GREEN-LIT, no dependency on F1/F2.**
Widen `NicRun`/`run_adapter` to the `Reply` handle (Final State #1–#2). No new reply
*behavior* ships — proven by one synthetic originate-in-response through the fake
`NicRun`, plus the existing receive-path suite unchanged (still hardware-free, still
green). This is the "changed seam" class Sprint 8's Method named for full review
attention before feature volume lands on top of it — reviewed alone, not bundled with
Slice 2's new behavior.

**Slice 2 — file acting + reply.**
`file-read`/`list` decode + confine (reusing `fs_read::FileReader`, the Sprint-6
certified fail-safe read-path streamer, unchanged) + stream-back (Final State #3).
**F2 is resolved** (reviewer's planning input, 2026-07-22): the per-session outbound
state is a **bounded lazy stream, pulling the source on demand** — never an eager
enqueue-the-whole-file, never a drop-on-`WouldBlock`. On each `Delivered`/`Idle` tick,
attempt `Reply::send` for the next already-pulled chunk; `WouldBlock` retries next
tick (no drop — these replies are reliable-ordered, not the display-relay's
drop-tolerant shape); `Ok` advances and pulls the next source chunk only then, so
backpressure propagates to the source and the whole file is never buffered. The
pacing signal is `SendError::WouldBlock` itself, **never** `is_quiescent` (Sprint 8's
D-3 residual left `is_quiescent` `cfg(bench)`-only; the shipped surface has only
`Event::Idle`, which is "drained, about to park," not "TX fully ACKed" — a pacing
design needing `is_quiescent` would be silently blocked on that gap). The outbound
map mirrors the reassembler/binding/writers MB4 shape: bounded per-epoch, evicted on
reply-completion and `close_session`, with the source fd (`FileReader`) closed on
completion — an fd leak here is as acute as the write-path's.
**`file-read`/`list` inherit JC-1** (Sprint 9's own named follow-up): now that they
are genuinely *acted on* (decoded), they join `acts_on()` and the reserved-byte
check applies to them for the first time — proven the same way Sprint 9 proved it
for the other four verbs.
Gated on F1's per-verb correlation-key fill-in for the actual payload field (the
mechanism — pacing, streaming, eviction — is built now against the settled
invariants; the correlation field itself is a marked, loud stub until proto-design's
fill-in lands — never a fabricated-looking real value).
**Correlation-fidelity (a bind-the-chain analog, added at planning):** the outbound
state is `{correlation key, lazy source, send window}` — the key is carried
**faithfully on every reply frame**, not just the first (a dropped/wrong key on
chunk N orphans that chunk from the box's own bookkeeping). Not an adversarial gate:
the key is box-supplied and opaque to the proxy (never interpreted or acted on,
only echoed), so a malicious key only confuses the box itself, on already-untrusted
ground — but its proto shape must bound the key's size, so an oversized key can't
inflate the outbound state.

**Slice 3 — the relay reply halves, narrowed (planning correction, 2026-07-22).**
`cloud_relay`/`terminal_relay` wired to the `Reply` seam (Final State #4). **Scope
correction found during build:** `architecture.md` §10 (L14) states the box-facing
relay itself "opens outbound upstream sockets" **in-process** — the upstream TLS
connection is not an external/deployment component, contrary to an initial reading;
`cloud/anthropic-api.md` §4 confirms the proxy itself encapsulates the request in
its own TLS. But building that real upstream client (and the still-unspecified real
head connection, `proxy-head.md`'s "flexible transport") is a materially larger,
more novel task than "wire an existing encoder to a seam" — **out of this slice's
scope**, named as a follow-on (likely its own future sprint, given the new
dependency weight). Slice 3 **narrows** to: the shared pacing core (`net-response`
needs its **own push-fed queue** outbound state — chunks arrive asynchronously from
a non-box channel, so empty ≠ done, unlike Slice 2's pull-based `ReplySource` where
`None` means done; the emission/pacing logic — lazy `reply_sid` open, `in_reply_to`
on every frame, `WouldBlock` retain-retry, eviction — is factored into one shared
core both stream kinds use), the box-facing send wiring, and `in_reply_to`
correlation for `net-response`/terminal input — all proven against a **fake,
test-only ingress source** that simulates cloud/head data arriving, the same
isolate-the-new-capability discipline Slice 1 used for the reply seam itself.

**Slice 4 — JC-3 deny-notifications, last.**
Every existing deny path gains its box-facing notification (Final State #5).
Deliberately last: it is the broadest-surface change (touches every handler's deny
arm) and is cleanest once the reply infrastructure is proven by real usage in Slices
2–3, rather than proving the seam on the trivial case first (considered and
rejected during planning — Slice 1 already proves the seam in isolation, so an
early trivial-reply slice would be a redundant second proof while prematurely
touching broad surface).

## Non-Goals

- Do not pass `directpath::Ctx` through the `NicRun`/`Reply` seam directly — it
  cannot be constructed outside `directpath::Subsystem::run`, so a fake could never
  hand one to a hardware-free test, collapsing the entire receive-path suite onto
  real hardware. The narrow `Reply` trait exists precisely to avoid this.
- Do not guess at the reply-session-correlation mechanism (F1) — it is a genuine
  cross-team design question, routed to the NIC designer via the PM, and Slices 2–4
  do not finalize their payload format until it lands.
- Do not hold a `Reply`/`Ctx` handle across callback invocations — the borrow
  discipline `Ctx` already enforces is preserved by construction in the `Reply`
  trait's signature, not merely by convention.
- Do not re-litigate the request-direction per-session correlation (Sprint 8) — it
  is settled and unaffected by the reply direction opening up.
- Do not build JC-3 first as a "prove the seam trivially" vehicle — Slice 1 already
  serves that purpose; JC-3 is sequenced last because of its own breadth, not because
  it is low priority.

## Starting Surface

Audit first, in this order:

1. `directpath/src/lib.rs`'s `Ctx` (`open`/`send`/`is_quiescent`) and `Subsystem::run`
   signature (`FnMut(&mut Ctx<'_>, Event) -> ControlFlow<()>`) — the real reply
   mechanism this sprint's seam wraps.
2. `proxy/src/nic.rs` (`NicRun`, `RealNic`) — the seam Slice 1 widens, and its current
   doc comment explaining why `Ctx` was omitted (the assumption this sprint reverses).
3. `proxy/src/lifecycle.rs`'s `run_adapter` — the closure Slice 1's `Reply` parameter
   threads through.
4. `proxy/src/fs_read.rs` (`FileReader`, built but unconnected), `proxy/src/
   cloud_relay.rs`/`terminal_relay.rs` (encode/relay logic built, send gated) — the
   waiting logic Slices 2–3 connect.
5. `docs/proxy/contract/box-proxy.md` §2 JC-3 (the deny-notification taxonomy) and
   §3.1 (`file-read`/`list`'s disposition, still described as "acted on" even though
   currently relayed — Slice 2 makes that description true).
6. `docs/proxy/contract/proxy-nic.md` §4 (R-7, the content-hash pin now discharge-able
   to `a5a215c`).

## Method

- **MB1–MB6** (Sprint 6's bar) apply to every new line, every slice.
- **Slice 1 is reviewed alone**, per Sprint 8's Method precedent for a changed seam:
  full review attention on the seam's shape and borrow-discipline preservation before
  any slice builds behavior on top of it.
- **F1 (reply-session correlation) is a designer fill-in, not a proxy guess.** Once
  settled, it is folded into this document (not left as an assumption in code) before
  Slice 2 finalizes its payload format.
- **F2 (streaming-reply pacing) is planning input from the reviewer**, requested
  during formation; the resolution is folded into Slice 2's scope before that slice
  starts, mirroring how Sprint 9's forks were resolved before their slices began
  rather than mid-build.
- **Every slice's evidence model**: hardware-free where possible (the `Reply` fake
  makes Slices 1–4's *logic* fresh-re-runnable — a reply is now assertable without
  hardware). Only a final receive-and-reply round-trip capture on real hardware
  (if the team judges one necessary at Slice 4's close) would degrade to the
  capture-model discipline established in Sprint 8; named at that gate, not assumed
  here.

## Acceptance

- **Slice 1.** REJECT if `directpath::Ctx` is named or held anywhere in `proxy`'s
  core outside `nic.rs`'s own adapter; REJECT if the fake cannot exercise an
  originate-in-response without hardware; REJECT if the existing receive-path suite
  regresses or degrades to a capture model for no new reason.
- **Slice 2–4.** REJECT if any slice finalizes a reply payload format ahead of the
  per-verb correlation-key fill-in; REJECT if outbound state is held across callback
  invocations in a way that violates the `Reply` handle's per-invocation lifetime;
  REJECT if JC-3's notification reuses a re-implemented send path rather than the
  Slice-1 seam; REJECT if a multi-chunk reply fails to carry its correlation key on
  every frame (not just the first), or if the key's proto shape leaves its size
  unbounded.
- **Sprint-level.** REJECT if R-7 is left un-discharged once `a5a215c` is available to
  pin against.

## Review Questions

1. **F1 — reply-session correlation.** **FULLY SETTLED** (NIC designer, then
   proxy-designer, 2026-07-22): no transport pairing exists; correlation is
   app-level, one uniform `in-reply-to: SessionId` field (echoing the request
   session id) on every reply-direction verb, none on emission traffic. The exact
   wire byte-layout per verb is proto's Task 16 (queued, not yet written) — Slice 2
   proceeds against the settled shape as a marked fill-in for the layout only.
2. **F2 — streaming-reply pacing.** **RESOLVED** (reviewer, 2026-07-22): bounded lazy
   stream pulling the source on demand, paced off `SendError::WouldBlock` (never
   `is_quiescent`, which is D-3-gapped on the shipped surface), never dropping a
   chunk (these replies are reliable-ordered). Folded into Slice 2's scope above.
3. **`net-response`'s existing frame-by-frame streaming.** **RESOLVED at the Slice-3
   gate**: it needs its own push-fed queue (chunks arrive asynchronously, so empty ≠
   done, unlike Slice 2's pull-based `None`-means-done), sharing a factored pacing
   core with the pull-streams.

## Retrospectives

### zero.proxy-review

**The R-c reversal was a new risk *shape*, and getting the classification right up front set every
gate.** For eight sprints the proxy only received; here it originates. The instinct on "the proxy now
sends to the box" is to reach for adversarial closure — but the reply direction lands on *untrusted
ground* (the box is contained; sending it data, even a whole host file, is the safe direction —
confidentiality is not a defended invariant, principles §7 / filesystem.md §3). So the reply path is
**tier-2/3 memory-safe plumbing, not a gate**, and its gates are MB + pacing/liveness + reuse-of-
certified-primitives + the *one* genuinely-adversarial structural fact: the id-partition. That last
one is where the real subtlety lived — a proxy reply-session id colliding with a box request id is
the 0/N-stall bug, so I scrutinized `Endpoint::Proxy → Originator::High` against the *real committed*
allocator (both-from-zero disjoint), not the design claim. Classifying the direction as
safe-not-adversarial early is what kept the review from over-gating a file streamed to the box while
still catching the one thing that *was* load-bearing.

**I set an over-specified condition and the dev's tripwire caught my own anti-creep.** When I ruled
the dir-lister into Slice 2 (option A), my sub-gate condition required an fd-bound, TOCTOU-free
`is_dir` check — a *write-path-grade* confinement — on a channel I had, one message earlier,
classified as tier-2 **fail-safe**. That is the exact MB3 anti-creep (over-applying write floors to a
fail-safe read channel) I gated *others* on at Sprint 6 and Sprint 9. It was also internally
contradictory (fd-bound listing needs `fdopendir` = new unsafe, which the same condition forbade).
The dev flagged it before building, cited my own principle back at me, and I reconciled: the
load-bearing confinement was never the fd-bound belt — it is the `resolve()` unsayable-traversal join
(which path-based `read_dir` preserves identically), and a followed final symlink lists a different
directory's names, harmless, exactly the case fs_read already accepts. The lesson: a reviewer's
*conditions* are as subject to the anti-creep discipline as the code, and the tripwire mechanism is
what makes an over-specified condition recoverable instead of shipped — the check working on the
checker.

**F2's pull-vs-push distinction is the load-bearing shape of the whole pacing layer.** The
streaming-reply pacing splits cleanly on *who drives the source*: a **pull** source (file-read/list —
the proxy owns a file/dir it reads) signals done by `None`/EOF, so empty *means* done and backpressure
is automatic (pull only when the send window drains); a **push** source (net-response/terminal — an
external ingress feeds it) has `empty != done`, so the queue must be explicitly bounded (backpressure
the *ingress* at the cap, never drop, never grow) and eviction must be on `done`-**and**-drained, never
on a momentary empty (or the response tail is orphaned). Factoring a shared pacing core over both
source shapes — proven behavior-preserving against the Slice-2 pull tests unchanged — is what let the
push half be a small addition rather than a parallel mechanism. Neither is drop: these replies are
reliable-ordered, and the display-relay drop tolerance does not transfer.

**Correlation-fidelity / the un-fakeable-tell discipline extended cleanly across three slices.** The
same bind-the-chain shape recurred at each reply verb, and the un-fakeable tell was uniform:
`in_reply_to == the originating request's SessionId, never the reply session's own (proxy-high) id`,
on **every** frame — because stamping the reply's own id is useless for correlation and is the likely
bug. It held for file-data (Slice 2), net-response (Slice 3, the same stamp reusing the Sprint-6
encoder), and the JC-3 deny-notifications (Slice 4, the request session on the error frame) — and its
*absence* was equally principled: terminal head→box input carries **no** key, because it is the
operator's independent emission, not a reply to any box message (F1's emission-no-key category). One
tell, applied at every verb, with the negative case reasoned rather than assumed.

**What the next sprint inherits.**
- *The reply path proven end-to-end hardware-free* — the seam (id-partition collision-safe), file/list
  acting + reply, the relay reply halves, and JC-3 notifications — all fresh-re-runnable via the
  `Reply` and fake-ingress doubles; R-7 discharged to the committed `a5a215c`.
- *Named follow-ons, not guessed at:* the in-process upstream TLS client (architecture §10/L14, the
  real cloud ingress replacing the fake), the real head connection + the
  `terminal_relay::relay_frame → enqueue_terminal_to_box` session-key mapping, and Task-16's exact
  reply wire layout (the single-choke `task16_stub_reply_frame`, loud and isolated, until it lands).
- *One team-judgment note carried:* the outbound drain is deterministic lowest-id-first, not
  round-robin — liveness-safe (every reply is finite, so each completes and the next drains) and
  matching the transport's no-scheduler posture, but a long low-id reply delays higher-id ones; a
  drain-start rotation would trade that for best-effort fairness if pipelined-reply latency ever
  matters.
- *The standing disciplines, reconfirmed:* classify a new direction's risk *shape* before choosing the
  gate bar; hold a reviewer's own conditions to the anti-creep bar and keep a tripwire that can catch
  them; reuse-by-diff (not re-read) for "reuses the certified primitive"; and one un-fakeable
  correlation tell applied uniformly, its absence reasoned.

### zero.proxy-dev

**What I built.** Four slices, the full box→proxy reply direction. (1) The reply *seam*: a narrow
proxy-owned `Reply` trait exposed through `NicRun::run`'s callback, `RealNic` adapting the real
`&mut Ctx` (`CtxReply`, the one place `directpath::Ctx` is named in `proxy`), the fake handing a
recording `Reply` — so a reply is assertable hardware-free for the first time. (2) File acting+reply:
`file-read` (reusing `FileReader` unchanged) and `list` (a new confined `DirReader`) decode+confine+
stream back, over a bounded-lazy WouldBlock-paced `ReplyStream`, each frame stamped with the request
session as `in_reply_to`. (3) The relay reply halves: a factored source-agnostic pacing core shared by
the pull-streams and a bounded, backpressuring push queue (`net-response`, terminal head→box). (4)
JC-3 deny-notifications: every policy deny notifies the box (`file-error`/`net-error`) over that same
push seam, silent on structural noise. Also discharged R-7 (re-pinned `proxy-nic.md` §4/§7 to
`directpath`'s commit `a5a215c`, fixing a stale 3-arg `construct` signature I found while there).

**What evidence changed the plan.**
- *The session-correlation facade gap that kicked off Task 15 (the origin of this whole sprint).*
  Building the box role in Sprint 8, first real integration surfaced that the proxy's per-session
  correlation was **not expressible over the real facade**: `Subsystem` exposed only `originate` (a
  fresh id each call), never send-on-existing-session, so nothing had ever driven two same-session
  frames through the real reactor. I surfaced it rather than hacking around it; the NIC designer ruled
  the proxy correlation correct as-built and the gap a NIC-side exposure — which became `open`/`send`,
  then the originator-partition (Task 15), which unblocked *both* endpoints originating and made this
  reply-path sprint possible.
- *The `Ctx`/`Reply` seam design — resist mirroring the concrete type.* The tempting move was to pass
  `directpath::Ctx` straight through `NicRun`. That would have defeated the seam's entire purpose: a
  `Ctx` cannot be constructed outside `Subsystem::run`, so the whole receive-path test suite would
  collapse onto hardware. The narrow proxy-owned `Reply` trait keeps the core directpath-free and
  testable, and preserves `Ctx`'s per-callback borrow discipline by construction. I also verified the
  R-c reversal's id-partition correctness against the *real committed allocator* (the fake mints reply
  ids from `Originator::High`), catching the classic 0/N-stall class before it could exist.
- *Pull vs. push is a real pacing distinction, not a detail.* Slice-2's sources are **pull** (a local
  fd; `next()==None` means done). `net-response`/terminal are **push** (chunks arrive async from a
  non-box channel; empty means *wait*, not done). The one pacing core is shared, but the source model
  had to split — a `Pull { Body | Pending | Done }` outcome where only the push queue ever yields
  `Pending`, and eviction is done-**and**-drained, never on a momentary empty. Grounding this before
  building (rather than forcing net-response through the pull abstraction) is what kept the refactor
  behavior-preserving for the accepted Slice-2 streams.

**What I surfaced instead of guessing.**
- *The dir-lister tripwire — and holding on it was right.* The reviewer green-lit folding `list`'s
  dir-lister into Slice 2 with a tripwire: split it out if it needed new `unsafe` or a confinement
  pattern beyond `FileReader`'s. Building to their stated condition (is_dir on the *opened fd*,
  TOCTOU-free, **and** readdir through safe `std`) turned out to be *unsatisfiable* — stable `std` has
  no fd-bound directory read, so binding the listing to the checked fd needs `fdopendir` (new unsafe).
  I flagged it rather than silently picking a relaxation; the reviewer self-caught that their condition
  had imported the write-path's fd-bound discipline onto a channel they had themselves classified as
  tier-2 fail-safe (their own MB3 anti-creep principle), corrected it to path-based `read_dir`, and it
  stayed in Slice 2 with no new `unsafe`. The lesson: a stated gate condition can be internally
  infeasible; surfacing the infeasibility beats satisfying the letter with a hack.
- *The `FileChunkDeny` spec-vs-impl defect.* JC-3's §3.1 table marks `file-chunk` "structural only"
  (no notify), on an **eager-open-at-file-write** model. But Sprint 9 built **lazy-open-at-file-chunk**
  (the full write-floor mask fires at the first chunk, not at `file-write`), so a genuine
  destination *policy* deny surfaces at `file-chunk`. The table was stale, not the impl. I routed it to
  the spec owner rather than either silently notifying (contradicting the table) or silently staying
  silent (dropping a real deny the box must hear). The designer ruled the sub-cause split
  (confined-open→notify as the late-surfacing write deny, no-writer→silent structural, cap/io→notify)
  and is reconciling the doc row themselves. Currency-of-source is a review gate: a faithfully
  re-derived stale table reproduces the staleness.

**What went cleanly.** The push seam proved reusable exactly as intended — Slice 4's deny-notifications
are a pure `notify_deny` side effect on the unchanged `route_inner`, reusing `push_reply_frame` +
`close_reply`, no re-implemented send path. The `in_reply_to` correlation mechanism (a loud
single-choke-point Task-16 stub) carried faithfully across file-data, list-entry, net-response, and
the deny notifications with one implementation; when proto Task 16 lands the canonical layout, one
function swaps and nothing else in the streaming/pacing/eviction path moves.

### zero.proxy-lead

**Every genuine cross-team fork this sprint resolved into an extension of the existing design, never
a rework — and verifying that pattern each time is what let the sprint move fast.** The
session-correlation gap (Task 15), the reply-correlation mechanism (F1), and the FileChunkDeny
taxonomy defect (A) all looked, on first contact, like they might force a rework of something already
built. Each time, checking the actual committed source or the actual spec text first (not assuming
the worst) showed the real answer was additive: `send(id)` alongside the existing `open`, an
`in-reply-to` field alongside the existing wire shape, a sub-case split in an already-right
taxonomy. **Rule reconfirmed: when a finding looks like it invalidates prior work, verify against
the actual committed artifact before treating it as a rework — the team's whole history this session
says the real answer is usually "extend," not "redo."**

**A documentation-only correction I could make myself, versus one that needed the PM/design, turned
on OWNERSHIP, not on how confident I was in the answer.** I resolved the "off-library" framing for
the upstream TLS connection myself, straight from `architecture.md` §10/L14's own text — that
document is ours, and citing it isn't a design decision. But `box-proxy.md` §3.1's FileChunkDeny
defect went to the PM even though dev's proposed fix was, in my judgment, clearly correct — because
that table's JC-3 taxonomy is explicitly owned by "design" in its own judgment-calls section, and my
confidence in the answer doesn't transfer ownership of the artifact. **Rule: route by who owns the
document, not by how sure you are the fix is right — a correct answer routed to the wrong owner is
still a boundary violation, even when the human on the other end would have agreed instantly.**

**The reviewer's own self-correction on the dir-lister condition is the clearest evidence yet that
the loop's checks apply symmetrically.** A reviewer's stated gate condition is not exempt from the
same anti-creep discipline it enforces on code — and the fact that the reviewer caught this
themselves, named the exact principle they'd violated, and produced a corrected condition rather than
defending the original, is what let dev's tripwire do its job without my arbitration. Nothing for me
to rule on there beyond confirming the correction was sound, which is the right amount of lead
involvement in a self-correcting loop.

**What the next sprint inherits.** Two follow-ons are now named and sized enough to brief on their
own: the in-process upstream TLS client (a new dependency, real network I/O — its own sprint, not a
slice) and the real head connection (`proxy-head.md`'s still-unspecified "flexible transport," plus
the `relay_frame → enqueue_terminal_to_box` session-key mapping). Task-16's canonical reply wire
layout is proto's to deliver, tracked as a marked fill-in with exactly one swap-in point. The
drain-fairness note and the FileChunkDeny doc reconciliation (design's, in progress) both carry
forward as named, not forgotten.
