# Sprint 012 — The Real Head Connection (Phase 4, continued)

> Proxy team, sprint 12, the second and last of the two sprints closing the remaining
> Phase 4 backlog the PM named (upstream client, then this). Builds the persistent,
> bidirectional transport between the proxy and the head, and wires `terminal_relay`
> through it both directions. Drafted by zero.proxy-lead from planning input by
> zero.proxy-dev and zero.proxy-review (2026-07-23).

---

## Scope Boundary

This sprint owns the real proxy↔head connection, replacing `terminal_relay`'s
currently-undriven encode/relay logic (Sprint 6) with an actual transport:

1. **The transport + framing foundation** — a persistent, bidirectional worker over a
   local Unix stream socket, with a length-delimited `[len][SessionId][L2]` frame
   codec, and two bounded channels (ingress: head→box input; egress: box→head
   display/pane-state).
2. **Wiring `terminal_relay`/`enqueue_terminal_to_box` through that transport**, both
   directions, realizing the session-key mapping Sprint 10 named as a follow-on: the
   transport `SessionId` itself is the pty routing key.

It explicitly does **not** own:

- **Peer authentication, transport security, or access control on the head link.**
  `proxy-head.md` §3 rules this out of the library entirely — external,
  operator-provisioned infrastructure, the same off-surface posture as the upstream
  TLS (`architecture.md` §11 L10). This sprint builds a *local* transport (a Unix
  stream socket); a remote/networked head is a future transport dropped in behind the
  same seam, not this sprint's to build.
- **The exact wire field for input-to-pty correlation.** The directionality
  question itself is **settled** (below) — a pty is two one-directional sessions,
  correlated by echoing the display session id in the input payload — but the
  precise field layout is proto's Task 16 (queued), a marked fill-in, not this
  sprint's to invent.
- **Any content decision on relayed terminal traffic.** `terminal_relay` stays opaque,
  MB3 unchanged, per the accepted Sprint-6/`proxy-head.md` §0 relay posture.
- **The live real-head round-trip.** Same evidence split as Sprints 8/11: Slice 1
  proves the transport/framing logic against a fake head over a socketpair; a real
  operator terminal is the held live proof.

## Problem

`terminal_relay::relay_frame` (Sprint 6) exists but nothing drives it: there is no
real proxy↔head connection, so box↔head terminal traffic (display, pane-state, and
every head→box input verb) has nowhere to go. Unlike Sprint 11's one-shot
request/response upstream call, this is a **persistent, bidirectional** relay: a
long-lived worker must both read head-originated input (feeding the box-facing
ingress, reusing Sprint 11's pattern) and write box-originated display data (a new
**egress** direction Sprint 11 didn't need).

Planning surfaced one genuine open question on the transport/session model, not an
implementation detail: terminal input (`key`, `resize`, `mouse`, …) carries no pty
identifier in its L2 body — routing to the correct pty depends entirely on the
transport `SessionId`. The box always originates a pty's session (the head sends
`open-session`, the box opens and emits, the head learns the id by observing that
emission — `model.md` §2, unaffected by Task 15, which changed only the net/file
*reply* direction). The team's planning-stage lean was a single bidirectional
session; **the designer ruled otherwise, and correctly reversed the team's
intuition on structural grounds** (routed via the PM, 2026-07-23): a single
session cannot be bidirectional at all under the committed transport, because an
ACK and an empty-data frame are byte-identical on the wire — the distinction a
"bidirectional session" would need collapses. So a pty is genuinely **two
one-directional sessions**: display (box→proxy, box-originated, low half) and a
separate input session (proxy→box, proxy-originated, high half — Task 15's
originator-partition already makes this concrete). Correlation is app-level: the
input session's payload echoes the display session's id, binding input to the
correct pty. The exact wire field is proto's Task 16 (queued), a marked fill-in.

Also surfaced: `SOCK_SEQPACKET` (message-preserving, the initial lean) is not
available in `std` (`std::os::unix::net` has only `UnixDatagram`/`UnixStream`) without
adding `libc` (a new proxy-owned `unsafe` site) or a third-party crate. The team moved
to `SOCK_STREAM` with an explicit length-delimited frame — `[u32 len][8-byte
SessionId][L2 bytes]` — which stays MB6-clean, has no message-size cap, and is
*more* portable to an eventual remote transport (a length-delimited byte stream needs
no message-boundary guarantee from whatever carries it, removing a residual the
SEQPACKET choice would have carried). This reintroduces, and this sprint must
correctly close, the exact "peer-controlled length field, partial reads" bug class
the team has flagged before (the L1-over-Ethernet explicit-length fix).

## Goal

Build the transport/framing foundation and wire `terminal_relay` through it both
directions, with the bidirectional worker's concurrency proven deadlock-free and the
framing proven memory-safe against arbitrary head-side bytes — while the PTY-session
directionality question and the live real-head round-trip are named, not guessed at.

## Final State

1. A persistent worker (`std::thread`) owns a `UnixStream` to the head. It reads
   length-delimited frames and forwards them to the box-facing loop over a **bounded**
   ingress channel; it accepts box-originated frames over a **bounded** egress channel
   and writes them length-delimited to the socket. **Both channels are bounded, but
   asymmetric in mechanism** (sharpened at review — the naming was symmetric,
   the correct behavior is not): on the **ingress** (worker produces, loop
   consumes), a full channel makes the **worker's** `send` block — this is fine, the
   worker is a separate thread and blocking there only backpressures the head read,
   never the box-facing loop. On the **egress** (loop produces, worker consumes), the
   **loop's** enqueue must be **non-blocking** (`try_send`, **drop-on-full**, never a
   blocking send) — a blocking egress-send would let one slow or stalled head stall
   the *entire* box-facing loop (unable to drain ingress, pump net/file reply
   streams, or serve any other session), a liveness DoS across every box session
   triggered by nothing adversarial, just a slow remote head. A dropped display
   frame is a tolerable misdraw (`terminal.md` §4 — legibility, not security; the
   head re-syncs on the next update), exactly the display-is-droppable /
   file-data-is-reliable distinction Sprint 10's F2 pacing already drew, now applied
   to the head egress. **REJECT a blocking egress-enqueue from the loop.**
2. The worker **multiplexes both directions without deadlock**: it never blocks
   indefinitely on one direction in a way that starves the other (a naive blocking
   read while the egress channel fills, with the head not reading, is the
   bidirectional-relay deadlock this sprint must avoid) — via non-blocking I/O,
   `poll`/`select` over both the socket and the egress channel, or two sub-threads,
   proven under a stalled head in one direction while the other has data.
3. The frame codec is direction-agnostic and total: `[u32 len][SessionId][L2 bytes]`,
   decoding a malformed or truncated frame to a typed reject (never a panic, never an
   over-read). **The declared length is bounded before any allocation** (rejecting an
   oversized claim rather than trusting and allocating a peer-supplied length raw,
   sized to the largest legitimate L2 frame), and a frame split across multiple
   partial reads is correctly reassembled, never corrupted or misparsed.
4. **The transport sits behind a seam** (mirroring Sprint 11's `UpstreamSpawn`): a
   future remote/networked transport (TCP, or whatever the operator fronts a remote
   head with) drops in without rewiring the codec or the channels — the local
   `UnixStream` is one implementation, not a universal assumption.
5. `terminal_relay`'s existing encode/relay logic (Sprint 6) is driven for the first
   time: box→head (`display`, `pane-state`, `list-entry`, `file-error`) flows out the
   egress path; head→box (`open-session`, `create-pty`, `key`/`resize`/`viewport`/
   `mouse`/`paste`/`focus`, `list`, `request-file`) flows in via the ingress path and
   `enqueue_terminal_to_box` (Sprint 10) — **realizing that sprint's named session-key
   follow-on**: the transport `SessionId` in the frame codec is the routing key, no
   new mechanism invented, resolved once the directionality ruling lands.
6. **Ingress liveness inherits Sprint 11's held gate (Task 20)**: a head-originated
   input frame arriving while the box is quiescent is the same quiescent-park deadlock
   — the loop is parked on the NIC wake and the input sits undrained in the ingress
   channel. **Egress has no such issue**: it is driven by box `Delivered` events (a
   box display frame *is* the wake), so it needs no held liveness proof.
7. **The framing decode and the terminal relay are memory-safe against arbitrary
   head-side bytes** — the head-side network-injectable surface `proxy-head.md` §0
   already names (a network hop exposes the link to non-box actors): a malformed,
   truncated, or adversarially-shaped frame fails safe, never panics, never
   over-reads. This is memory-safety on the *parse*, not adversarial content-closure
   (the relay direction stays opaque and unsanitized, MB3).
8. The sprint document carries role retrospectives; the sprint is committed with only
   its intentional files.

## In Scope — ordered slices

**Slice 1 — the transport + framing foundation, isolated.**
The bidirectional `UnixStream` worker, the two bounded channels, and the
`[len][SessionId][L2]` codec, proven against a **fake head** driven over a socketpair
— the same fake-ingress honesty as Sprints 10/11. Gate: both channels bounded, with
the mechanism asymmetry proven, not merely the bound — **ingress** backpressures
the worker (fine, a separate thread); **egress is non-blocking drop-on-full from
the loop's side**, proven by driving a slow/stalled fake head while the loop
continues draining ingress and pumping other sessions unaffected (the actual gate
is "the loop never stalls," not merely "the channel has a capacity"); no shared
mutable state beyond the two channels (no `Arc<Mutex<_>>`, no shared `Dispatcher` —
message-passing only, `Send`-enforced); the multiplex-without-deadlock property
proven under a stalled head in one direction while the other has data; worker
cleanup on head-disconnect/error (both channels close, no leaked thread, no leaked
fd). **The length-framing codec, sharpened at
planning:** the peer-supplied `u32` length is capped at the largest legitimate L2
frame (a small provisional scalar, marked for review) and an oversized claim is
**rejected before any allocation** — the reject-before-grow discipline already
applied to the reassembler cap and the read-slurp fix, now applied to the length
prefix (a `[len = huge]` must fail safe, never attempt the allocation). **Partial-read
reassembly**: a stream read returns arbitrary partial data, so a frame can split
across `recv` calls — the reader accumulates until the full frame is present without
corrupting or misaligning the next frame (proven: a frame delivered one byte at a
time reassembles identically to one delivered whole). **The accumulation buffer is
itself bounded by the same length cap** — a stalled peer that sends `[len=N]` then
stalls mid-frame holds at most one max-frame-length buffer, not unbounded, and
combined with the no-deadlock multiplex (above) a stalled partial frame does not
wedge the loop. **This one fuzz-shaped corpus (malformed length, truncated frame,
garbage after a valid frame, oversized claim) covers both the length-framing
correctness gate and the head-side-network-injectable memory-safety bar (Final
State #7)** — they are the same surface, not two separate proofs. MB6 holds
(`std::net`/`std::thread`/`std::sync::mpsc`, no new `unsafe`).

**Slice 2 — wire `terminal_relay` both directions.**
Box→head egress driven through the Slice-1 seam unchanged (display/pane-state on
the pty's own box-originated session). Head→box ingress **originates a new
proxy→box input session** (high half, mirroring the net-response reply pattern)
carrying the echoed display-session-id correlation field (Final State, ruled
2026-07-23) — `enqueue_terminal_to_box`/the box-input path is keyed by that
correlation field, not by the input session's own id. Gate: **routing fidelity**
— input correlated to display-session S is proven to reach the box addressed at
the pty that opened S, never a different or wrong pty; the correlation field
itself fails safe on an unknown or **stale** display-session id (a pty that
already closed) — dropped and logged, never misrouted to whatever session id now
happens to hold that number; the field's wire layout is a **marked fill-in** for
proto's Task 16, not invented here. **A new MB4 resource** (sharpened at the
Slice-1 gate — this is genuinely new versus the bidirectional model, which would
have needed none): the proxy's own display-S-to-input-session correlation state
is bounded and evicted on the display session's `close_session`, the same shape
as every other per-session map (route/reassembler/writers/replies) — no leaked
high-half session ids, no stale mapping surviving its pty's close. MB3 holds
throughout (terminal content relayed opaque, unsanitized, no content decision).

## Non-Goals

- Do not implement peer authentication, TLS, or any access control on the head
  link — `proxy-head.md` §3 places this outside the library entirely.
- Do not invent the correlation field's wire layout — the mechanism (two
  one-directional sessions, echoed display-session-id) is settled; proto's Task 16
  supplies the exact byte layout as a marked fill-in.
- Do not assume `SOCK_SEQPACKET` or any message-preserving transport property — the
  chosen `SOCK_STREAM` + length-delimited framing is deliberately self-delimiting and
  carries no such assumption, which is exactly why it was chosen over pulling in
  `libc`/a third-party socket crate for `SEQPACKET`.
- Do not add any inspection, sanitization, or content decision to the terminal
  relay path in either direction — MB3 unchanged.
- Do not build a network/remote transport this sprint — the seam exists so one can
  be added later without rewiring the codec or channels, but building it is out of
  scope.
- Do not claim a working real-head round-trip as this sprint's deliverable — Slice 1
  proves the transport/framing logic against a fake head; the live proof is held,
  same discipline as Sprints 8 and 11.

## Starting Surface

Audit first, in this order:

1. `docs/proxy/contract/proxy-head.md` §0 (the network-exposure residual, the
   no-auth-in-library ruling) and §3 (the deliberately flexible transport).
2. `docs/proxy/architecture.md` §10/L14 (the "opens outbound upstream sockets"
   framing, now extended to the head-facing socket) and §11 L10 (the off-surface
   precedent this sprint's transport choice draws on).
3. `proxy/src/upstream.rs` (Sprint 11) — the worker/channel/seam pattern this
   sprint's bidirectional version extends; note where the one-shot assumptions
   (single channel, spawn-per-call) don't carry over.
4. `proxy/src/terminal_relay.rs` (Sprint 6) — the encode/relay logic this sprint
   finally drives, unchanged in content.
5. `proxy/src/dispatch.rs`'s `enqueue_terminal_to_box` (Sprint 10) — the seam this
   sprint's ingress feeds, and the session-key-mapping follow-on this sprint closes.
6. `docs/proto/model.md` §2 (session origination, `open-session`, "a session carries
   an operation") — the grounding for the PTY-directionality question routed to the
   designer.

## Method

- **MB1–MB6** apply to every new line. MB6 stays `{open_discipline, privilege}` — the
  worker/channels/socket are safe `std`, no new proxy-owned `unsafe`.
- **The evidence split, Sprint-8/11-shaped**: Slice 1's transport/framing logic is
  fresh-re-run hardware/network-free (a fake head over a socketpair). The live
  real-head round-trip is the held item, same as the upstream sprint's live wake —
  named, not softened.
- **Two new concurrency/memory-safety concerns this sprint introduces, beyond
  Sprint 11's one-shot worker**: the bidirectional multiplex-without-deadlock
  property (Final State #2), and the head-side-network-injectable memory-safety
  bar on the framing decode (Final State #7) — `proxy-head.md` §0's residual made
  concrete and gated for the first time.
- **The session-key-mapping resolution is exactly what Sprint 10 named as a
  follow-on**, realized once the directionality ruling lands: the transport
  `SessionId` is the pty routing key, no new mechanism invented if bidirectional.

## Acceptance

- **Slice 1.** REJECT any unbounded channel (either direction); REJECT a **blocking
  egress-enqueue from the box-facing loop** (the loop must drop, never block, on a
  full egress channel — a blocking egress-send is a liveness DoS across every box
  session, not merely a design preference); REJECT a worker design where one
  direction can block the other indefinitely (no proof of deadlock-freedom under a
  stalled head); REJECT any shared mutable state beyond the two channels; REJECT a
  length-delimited decode that allocates before bounding the declared length, or
  that mishandles a frame split across partial reads; REJECT any panic or over-read
  on a malformed/truncated/oversized frame corpus.
- **Slice 2.** REJECT an implementation that binds input to a pty any way other
  than the ruled correlation field (echoed display-session id) — no bidirectional
  single-session shortcut, no proxy-invented alternative mechanism; REJECT a
  correlation check that fails unsafely (misroutes) rather than dropping on an
  unknown/stale display-session id; REJECT any invented wire layout for the
  correlation field ahead of proto's Task 16; REJECT any content inspection or
  sanitization added to the relay path.
- **Sprint-level.** REJECT any report or retrospective that reads "the transport is
  built" as "a real head is connected" — the live round-trip stays held pending an
  actual operator terminal, stated as such.

## Review Questions

1. **PTY-session directionality — RESOLVED** (designer ruling via the PM,
   2026-07-23): a single bidirectional session is structurally impossible under
   the committed transport (an ACK and an empty-data frame are byte-identical, so
   the distinction a bidirectional session needs collapses). A pty is two
   one-directional sessions (display box-originated, input proxy-originated on
   the high half), correlated by echoing the display session id in the input
   payload. The team's planning-stage bidirectional lean was reversed on
   structural grounds, not preference — the exact wire field remains proto's
   Task 16 (queued), a marked fill-in for Slice 2.
2. **A live end-to-end capture, once a real head is available** (team's call, not
   pre-decided): whether this sprint's own retrospective needs one, or whether it
   folds into whichever future work first exercises a live operator terminal —
   mirroring Sprint 11's RQ2.
3. **`model.md`'s stale "box is the sole allocator" phrasing** (a currency nit
   surfaced during planning, non-blocking): true for terminal-session lifecycle
   specifically, but now reads as pre-Task-15 language against the symmetric
   net/file reality. Flagged for the designer's own doc pass, not gating this
   sprint.

## Retrospectives

### zero.proxy-dev

**What I built.** Two slices. Slice 1: the head-connection transport + framing foundation — a
bidirectional, long-lived `UnixStream` worker (reader + writer threads), two bounded channels, and a
`[u32 len][8B SessionId][L2]` length-delimited codec, proven against a fake head over a socketpair.
Slice 2: `run_adapter_with_head` wiring `terminal_relay` both directions (display→head, head-input→box)
plus the Dispatcher routing core (`route_head_input`, the active-display set, the pty-routing correlation
stamp, close-eviction).

**The SEQPACKET→STREAM pivot — better on its merits, not just cheaper.** I went in leaning
`SOCK_SEQPACKET` (message-preserving, no manual framing). Two things flipped it. First, MB6: SEQPACKET
isn't in `std`, so it would need libc (our own new `unsafe`) or the `socket2` crate — a cost. But the
deeper reason it turned out *better*, not merely cheaper, is the lead's own "confirm boundaries
preserved remotely, or note a residual" concern: a length-delimited `SOCK_STREAM` is **self-delimiting**,
so it needs no OS boundary-preservation at all — which means the residual *disappears* rather than being
carried. The operator's eventual remote-fronting transport (a tunnel, TCP+TLS) trivially carries a byte
stream but may not preserve message boundaries; a framed stream sidesteps that entirely. The lesson: the
"ideal" primitive (SEQPACKET) was actually the more fragile choice for the real portability requirement —
the humbler self-delimiting stream is strictly more robust. I surfaced the pivot before building rather
than silently deviating from the lead's stated lean.

**The PTY-directionality ruling reversed BOTH leans — on structural grounds, which is why it needed the
designer.** In Sprint 10 I flagged the box-input-leg as a real gap; here I leaned bidirectional-PTY-
session, the lead leaned the same, and we routed it. The designer ruled the *opposite* (two
one-directional sessions) for a reason neither of us saw: a bidirectional session is **impossible** under
the committed transport because an ACK and an empty-data frame are byte-identical — the distinction a
bidirectional session needs collapses entirely. This is the sharpest instance this sprint of the
category discipline: a design call that *feels* like a preference (one session vs two) was actually
foreclosed by a structural invariant of a committed layer, and only the layer's owner could see it. My
flag was right (route it, don't guess); my *lean* was wrong, and being wrong on structural grounds is
exactly the case where deferring to the owner pays.

**The asymmetric-channel liveness catch (review's, folded into the design).** The two channels are
bounded symmetrically in name but must be **asymmetric in mechanism**: ingress (worker produces / loop
consumes) blocks the worker — fine, it only backpressures the head *read*; but egress (loop produces /
worker consumes) MUST be non-blocking drop-on-full, because a blocking egress enqueue lets one slow head
(not even adversarial — just a remote head with latency) stall the *entire* box-facing loop: no draining
ingress, no pumping any reply stream, no serving any session — a liveness DoS across everything from one
slow peer. The fix is the Sprint-10 display-is-droppable / file-data-is-reliable distinction applied to
head egress, and I made it API-*enforced* (the raw egress `Sender` is private; the only path is a
non-blocking `send_to_head`), so the loop cannot block on egress even by mistake. F3 later extended the
same discipline to ingress (private `Receiver`, non-blocking `try_recv_from_head`). The durable point: a
symmetric-looking pair of resources can have asymmetric liveness roles, and the enforcement belongs in
the type/API surface, not in call-site discipline.

**Why the verbatim-supersession is FORCED, not creep.** The two-session ruling means terminal head→box
input is no longer relayed verbatim: it now carries a **pty-routing** correlation (the display session
id) so the box binds input to the right pty. This *looks* like it contradicts Sprint 10's "terminal
input is emission, no correlation key" — but it doesn't: Sprint 10's "no key" answered the **reply**
correlation question (F1 — input is not a reply to a box message), and the **pty-routing** correlation
is a genuinely different thing that only exists because the ruling split the pty into two sessions. So
updating the verbatim test to the stamped model is a *correction to keep it accurate under the new
ruling*, not new policy. I flagged this loudly rather than letting it land quietly, precisely because a
change to accepted behavior that *looks* like a contradiction needs to be shown to be a forced
consequence — which review independently confirmed from three angles (structural necessity, explicit
designer spec, non-contradiction with F1). The lesson: when a ruling forces you to change
already-accepted behavior, the burden is to *prove it's forced*, in the open, not to quietly make the
change and hope no one notices the apparent contradiction.

**What went cleanly.** The Slice-1 seam held: `run_adapter_with_head` wired both directions onto the
proven codec/worker without reshaping them, and the fail-safe routing rests on a real transport
invariant (monotonic-unique ids within an epoch → a closed display session's id is never reused, so
drop-on-not-active is genuinely never-misroute, not an assumption). Bind-the-chain analog for terminal:
the input L2 rides verbatim after the correlation stamp — the proxy adds a routing field but never
touches the opaque terminal bytes.

### zero.proxy-review

Three things from this sprint are worth carrying forward.

**Structure over parsimony — a self-correction.** At planning I *leaned*
bidirectional for a pty session (one session carrying display box→head and input
head→box) on parsimony grounds — fewer sessions, no new mechanism. The designer
reversed it on structure: under the committed transport an ACK and an empty-data
frame are byte-identical, so a single session cannot be bidirectional — the
distinction collapses. The correction was *checkable from what I already held*
(model.md §9's independent-unidirectional-messages framing, and the transport's
one-directional-data model), not new information. I routed the fork correctly
(kept it the designer's, did not resolve it), but the lean itself was wrong on a
ground I could have weighted. The general lesson, ratified upstream: when leaning
on a routed fork, weight the committed *structural* constraints above elegance —
the parsimonious option can be structurally forbidden — and state a lean as
"parsimonious IF the structure permits," naming the structural check.

**Three-angle verification of a "forced consequence" claim.** The dev explicitly
asked me to confirm — not wave through — that superseding Sprint 10's
`enqueue_terminal_to_box` (verbatim, no correlation key) with `route_head_input`
(stamping the display-session id) is a forced consequence of the two-session
ruling, not new policy. Taking the stated reason on faith is exactly the failure
mode to avoid. I confirmed it three independent ways against source: (1)
structurally forced — the input rides a proxy-high-half session disjoint from the
box-low display session, so the box cannot bind input to a pty without a
correlation (traced through the high-half allocation and `push_reply_frame`); (2)
explicitly specified — the correlation *is* the designer's ruled "echoed
display-session-id field"; (3) not creep and not an F1 contradiction — the payload
rides verbatim (MB3), and a routing correlation is a different axis from F1's reply
correlation. A correct claim still gets independently re-derived; the dev's asking
for it is the healthy shape.

**The asymmetric-liveness catch — a real DoS from something ordinary.** The
two-channel worker looked symmetric ("both channels bounded"), but the mechanisms
must be asymmetric: the ingress worker blocking on a full channel is fine (it
backpressures only the head read, on a separate thread), while a *blocking* egress
send from the box-facing loop would let one slow or stalled head — not an adversary,
just latency or a wedged terminal — stall the ENTIRE loop, starving every other
session. The fix (egress `try_send`, drop-on-full, the raw sender kept private so
the loop physically cannot block) makes a dropped display frame a tolerable misdraw
(terminal.md §4) rather than a whole-proxy stall. The gate that matters is "the loop
never stalls," not "the channel is bounded." Caught at contract time, before a
blocking egress solidified in the build.

Two items are honestly held into the next integration: the live real-head
round-trip (same discipline as Sprints 8/11, proven here against a fake head over a
socketpair), and composing this head loop with Sprint 11's upstream loop into ONE
production loop — which needs unifying to a single Dispatcher and a single loop
draining both ingresses under the one reactor park/wake (the Task-20 held wake), not
juxtaposing two adapters.

### zero.proxy-lead

**A ruling that reversed the whole team's intuition arrived cheaply because the fork
was kept genuinely open, not pre-committed.** Both dev and reviewer leaned
bidirectional during planning, and I could have let that consensus stand as "the team
converged, adopt it" — the pattern this team has correctly used before when
convergence is a strong signal. The difference here: convergence on a *design
preference* (parsimony) is not the same signal as convergence on a *resolved
technical question*, and neither lean was checked against the one fact that actually
decided it (the transport's ACK/empty-data byte-identity). Routing it anyway, even
against a confident shared lean, is what let the designer's structural argument
land before any code was shaped around the wrong assumption. **Rule: team consensus
on a *preference* is not team consensus on a *fact* — only the latter is safe to
adopt without checking upstream; route the former if the deciding fact sits outside
the team's own artifacts.**

**Two structural findings this sprint (the transport asymmetry, the PTY ruling) both
arrived through the same discipline: verify the claim against source or the
committed model before treating it as fact, then route only what's genuinely
undecided.** The egress-liveness catch was verifiable directly in this team's own
code (reviewer read the channel mechanics); the PTY question required checking the
committed transport's own byte-level behavior, which is why it went to the designer.
Telling those two apart correctly — one resolvable in-house, one requiring the
artifact's owner — is what kept this sprint moving without either guessing at a
cross-team fact or over-escalating something the team could settle itself.

**What the next sprint inherits.** The two held items both teams named — the live
real-head round-trip and the single-production-loop composition (unifying this
sprint's head loop with Sprint 11's upstream loop under one `Dispatcher` and the
Task-20 wake) — are the actual remaining work to reach a running end-to-end proxy;
neither is optional polish. The list-entry/file-error contract-vs-code discrepancy
is routed and non-blocking. Task 20 (the reactor's bounded-park cap) remains the
single cross-team dependency gating any live capture across both of the last two
sprints.
