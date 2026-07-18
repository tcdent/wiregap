# The NIC / Transport Subsystem — Runtime Architecture and Protocol

> The design of the subsystem that owns the box↔proxy wire: how bytes move, how events are
> raised, how sessions are multiplexed, and the protocol that carries them. It is built **on** the
> hardware grounding in `grounding/` (which it cites rather than repeats) and sits under the policy
> spine (`../principles.md`). Where `grounding/` answers *what the silicon does*, this answers *what
> we build on it and why*.
>
> The organizing intent, in the operator's words:
>
> > "The goal is not maximum throughput. The goal is UART simplicity with a higher bit rate."
>
> Everything below is that line applied to a specific place. Drafted by zero.nic-design 2026-06-20
> from the design conversation; bench-gated items are marked and reserved as open research.

---

## 1. What the subsystem is

A single physical link (1m passive DAC, 10G, kernel-bypassed; see `grounding/kernel-bypass-and-dma.md`)
carrying many logical sessions between the box and the proxy. The subsystem is a **single-threaded
event reactor** that does exactly two things:

1. **emit inbound frames as events** to the application above it, and
2. **push outbound packets** over the one wire.

It is the heartbeat of the whole upstream stack: the concurrency model and event model that drive
everything above rest on this loop. So it is held to the project's discriminating lens — **simplicity,
understandability, and minimal firmware-integration surface** — at every choice.

### 1.1 Nomenclature (canonical)

These terms are used precisely throughout. This is the authority; the explore docs are point-in-time
exploration and do **not** define them.

- **epoch** — the timeline between reboots. A reboot resets the box's session-ID space and begins a new
  epoch. Simple concept, deliberately big word. The transport does **not** track epochs; the term names
  what a reboot delimits.
- **session** — one of our multiplexed connections: the transport's primary managed entity, and a fresh
  **instance** of the one transport protocol. (Earlier drafts called this a "stream"/"connection"/
  "instance"; "session" is canonical.)
- **session ID** — the identifier of a session: box-allocated, 64-bit monotonic counter, unique **within
  an epoch**, reset on reboot. It is the frame's only key (demux only, zero security weight).
- **application layer** — what rides *on top of* a session: one of a handful of implementations of the
  protocol (PTY, file, request). Selected by the session's **binding**; not the transport's concern.

The subsystem's scope, in one line: **manage sessions within an epoch.**

## 2. The design values (the lens every decision passed through)

- **UART simplicity at a higher bit rate.** A serial line's conceptual simplicity — a dumb,
  point-to-point byte pipe — with only its bandwidth ceiling removed. None of TCP's machinery for
  routing, reordering, congestion, or variable-quality multi-hop media exists here, because the
  medium has none of those problems.
- **Headroom is a budget we spend on simplicity.** A single session clears ~200k frames/sec at µs
  RTT; real demand is hundreds/sec — roughly three orders of magnitude of headroom. Normal network
  design spends complexity to buy throughput; we run it the other way and **spend throughput to buy
  simplicity** wherever the trade exists (ACK everything, full keyframes, per-descriptor cleanup —
  all free against the headroom).
- **Correct vs. generous.** A race can be handled by being *correct* about it (structural machinery)
  or *generous* about it (enough margin that it can't occur). Where the surrounding structure makes
  generosity safe, we choose generous — it is less code to hold. (This is why the retransmit window
  is generous rather than the protocol carrying a sequence number; §7.)
- **No legacy weight.** Every mechanism must answer a *named* failure mode of *this* link. The one
  real failure is receiver overrun; it gets the one minimal answer (per-frame ACK + retransmit).
- The register-level twin of all this is `grounding/driver-minimality.md`: run on documented
  defaults; write only what's security-load-bearing or datasheet-required.

### 2.1 Trusted-base posture: the NIC is untrusted-but-contained

The subsystem's trusted base has two kinds of member — a third, the IOMMU, was expected but is **not
used** (below):

- **small-and-ours** — the driver and protocol, verified by *reading* (`grounding/driver-minimality.md`);
- **untrusted-but-contained** — the **NIC and its firmware**, whose *trustworthiness is immaterial*: we
  depend on the NIC to **function** (liveness), never to **behave** (security), exactly the box's status
  (assertions 4–5). It is bulky vendor code we cannot read; we do not trust it to be sound.

A maximally hostile firmware's in-scope reach converges on **two vectors**, and after the pure-direct
pivot the two are handled by different means:

- **Vector 2 — adversary-written descriptor/buffer *content*: CONTAINED, by the small-and-ours driver.**
  The read-verified driver bounds any content the firmware can produce — bound the reported length, treat
  status as hostile, never deref an unchecked field, copy out early (§3.1,
  `grounding/data-path-and-events.md §1`). This is the live guard, unchanged across every runtime model
  considered.
- **Vector 1 — device DMA *reach*: OWNED, not contained.** This was to be the IOMMU's job (the
  structural-hardware floor). The subsystem runs **pure-direct with no IOMMU**
  (`grounding/kernel-bypass-and-dma.md`): a userspace process on running Linux drives the NIC directly,
  and nothing hardware-bounds where the device can DMA. The operator's ratified posture is to **own** this
  risk rather than route containment through a kernel-configured IOMMU — which would rest on the excluded
  kernel, laundering the risk through kernel trust rather than removing it. Whether the parent threat
  model tolerates un-contained firmware DMA on the trusted side is the **policy layer's call, audited
  after the PoC**; until then this vector is *owned and committed, not yet ratified as contained*
  (`concerns-to-watch.md`).

Corrupt-data (a third failure) is caught by hardware CRC + retransmit; everything else the firmware can do
is DoS / link-down, out of scope. A fully-malicious firmware's worst *contained* effect is nothing; its
*owned* effect (vector 1) is what the policy audit weighs.

This posture is *more* compliant with the spine, not in tension with it. Assertion 16 says **reject
code you cannot read** — and the diligence-trust we had been extending to the un-readable firmware was a
quiet *exception* to 16, not a thing 16 endorsed. Containing the firmware instead of trusting it
**removes the exception**: we no longer trust code we cannot read; we contain it. Nothing in the spine
needs to change to accommodate this — it closes a gap the spine already wanted closed.

**What reconciles, and what stands.** The original commitment framed the containment gap as (A) a driver
robust to any descriptor content and (B) the IOMMU airtight against bypass. **(A) stands** — it is the
live vector-2 guard, verified by reading. **(B) is what moved**: with no IOMMU, vector-1 reconciles from
*structural-hardware containment* to *owned risk under policy audit* (`grounding/kernel-bypass-and-dma.md §2`,
`concerns-to-watch.md`). So the posture — untrusted firmware, content vector contained, DMA-reach vector
owned, functioning-not-behaving dependence — **stands**; only the vector-1 *mechanism* changed, from IOMMU
to owned. This is "the mechanism reconciles," not "the containment is invalidated," with the honest caveat
that vector-1 is *owned rather than contained* until the policy layer rules. One way this is *more*
spine-compliant than the original plan: that plan leaned on a kernel-configured IOMMU for containment, and
the spine excludes the kernel from the trusted base — owning the risk instead removes a would-be
kernel-trust dependency the spine never wanted.

## 3. The runtime architecture — a flat, single-threaded reactor

One thread owns the wire, both rings, all session state, and one bounded-poll loop. The loop:

```
poll the RX ring (busy-poll drain)         → for each frame: demux by session ID → its session reacts inline
drain pending output to the TX ring        → the reactor is the sole TX-ring writer
when idle: bounded poll (capped sleep)     → re-poll after a short sleep; a frame arrives, or a retransmit deadline passes
```

- **One bounded-poll loop drives everything**: it drains the NIC's RX ring, checks the earliest
  retransmit deadline, and services the application in-process. There is **no `epoll` and no eventfd** on
  the pure-direct path (`grounding/kernel-bypass-and-dma.md §4`) — no MSI-X interrupt is delivered to a
  userspace directly-owned device. When there is work the loop busy-drains with zero waits; when idle it
  re-polls after a capped short sleep. Every readiness is still an event; the loop just discovers it by
  polling rather than by a kernel wakeup.
- **The packet is the event.** A frame arrives, it is keyed by the session ID in its header, it
  bubbles straight to that session, which reacts. Nothing stages it, pulls it later, or
  paces it. There are **no intermediary buffers** — the decoupled "rings → staging → worker-pool →
  backpressure-pacing" pattern of conventional stacks (thousands of lines) does not exist here.
- **Busy-poll is the drain, not the architecture.** The system is event-driven, not busy-spin-driven;
  the busy-poll only empties the ring while frames are present, then the loop idles on a bounded poll (a
  capped sleep before re-polling), rather than a busy spin.
- **Single producer, no locks.** Because the reactor is the only thing that touches either ring,
  there are no locks and no races on ring or session state. This resolves the open fork in
  `transport-stack.md §4.4 item 14` — single-producer into the one TX ring.
- **Why single-threaded**, on the three lenses: *simplicity* (no locks/races), *understandability*
  (the whole event flow is one readable loop), *firmware surface* (one RX / one TX ring, polled — no
  MSI-X, no eventfd; multi-threading would push back toward multiqueue/RSS/multiple vectors, the
  surface we reject; see `grounding/data-path-and-events.md §5`).
- **The flatness invariant:** event handlers are fast and non-blocking — true *by construction* for
  the transport's own work (frame a chunk, advance the session, write the ring). Genuinely heavy
  application work, if any, lives above the line and arranges its own affairs; it never pushes a
  buffer down into the transport.

### 3.1 The copy boundary — events carry app-safe memory; the DMA region is touched only at the edges

A hard design constraint: **the reactor copies each frame out of the DMA region into application-owned
memory at the earliest possible point, and every event it emits carries that copy — never a pointer
into the ring.** The application never holds memory the device can address. The copy is free against
our headroom (a `memcpy` per frame at hundreds/sec is nothing), and we spend it to buy three things at
once:

1. **Stability.** The DMA buffer is device-writable and *reused* — once the descriptor is refilled the
   NIC can DMA a new frame into that same buffer. An app holding a ring pointer would have its data
   rewritten underneath it. Copying out early gives the app a snapshot the device cannot touch.
2. **Containment — the memory analog of neutralize-at-the-boundary.** The DMA region is exactly the
   device-writable, box-influenced memory of vector 2 (`grounding/kernel-bypass-and-dma.md §2`). Copy-
   out at the edge means **device-writable memory never flows past the reactor**; everything downstream
   operates on inert, app-owned bytes. "As early as possible" shrinks the slice of code that ever
   touches adversary-reachable memory to nearly nothing — the smaller that slice, the smaller the
   surface where descriptor/buffer content has to be treated as adversarial.
3. **Overrun avoidance — our one real failure mode.** Copying out immediately lets us refill the
   descriptor and hand the buffer back to the NIC *now*, keeping the ring full. Holding DMA buffers
   deep into the app would starve the ring and *cause* the receiver overrun the whole reliability story
   (§7) exists to prevent.

This is **stronger than ixy's model**, which swaps buffers (hands the caller the mempool buffer,
refills the ring with a fresh one) — what the caller holds is out of the ring but still in
**device-writable DMA memory**. We copy fully out into app-owned memory the device never addresses. The **TX side is the mirror**: the app works
in app-safe memory; the transport copies *into* a DMA buffer at the send edge and holds it ACK-bound
until acked (`grounding/data-path-and-events.md §3`). So the DMA region is touched **only by the
transport, only at the two edges** — RX-in and TX-out.

This is **not** the intermediary staging buffer §3 rejects. That was decoupling arrival from processing
for *pacing*. This is a single synchronous copy at the boundary that establishes *memory ownership* —
the app-safe copy *is* the event payload, emitted immediately, with no queue and no later pull. "The
packet is the event" is precise: a *copy* of the packet, in app-safe memory, is the event.

The *crossing itself* — accessing device-mutated memory from the language, the ordering it depends on,
and whether the copy is truly sound — carries hazards we have **not** analyzed to resolution. They are
tracked as open, deliberately unsolved concerns in `concerns-to-watch.md`; this section commits the copy
*constraint*, not a claim that its unsafe implementation is understood.

## 4. Multiplexing is instantiation

> "Multiplexing is instantiation of protocol and the driver layer is interleaving those multiplexed
> requests across its single wire single-threaded protocol." — the operator

There is **one** transport protocol code set. A **session** is one multiplexed connection — a fresh
**instance** of that one protocol, keyed by its session ID. On top of a session rides an **application
layer**: one of a handful of implementations (PTY, file, request) that gives the session's bytes their
purpose, selected by the **binding** the session declares (a field it carries, *not* a transport
branch). Multiplexing *is* instantiation: a new connection is a new session, not a new protocol.
Consequences:

- **Transport dispatch has zero protocol-type branching.** A frame arrives → the reactor looks up its
  session by session ID (a table) → hands it to that session — the same code, every time. "What
  protocol is being spoken" is never a question at the transport; it is always the one protocol. The
  *handful of implementations* live in the application layer above, where the binding selects among
  them; the transport never reads the binding.
- **The reactor interleaves sessions onto the one wire**, and the interleaving is *trivial* — not a
  scheduler. Because every session is stop-and-wait (§6), each has at most **one** frame in flight,
  so the reactor keeps a ready-set of sessions that have a frame to send and drains it. No QoS, no
  pacing, no fairness math.
- **Application threads are not required.** Sessions are event handlers in the reactor; the
  application need not create shared-memory threads to get concurrency — it gets it by opening more
  sessions.

## 5. The event model — one local idle-wakeup, many derived events

Two kinds of "event" exist, and conflating them is where complexity comes from, so they are kept
distinct:

- **The idle-wakeup — one, local, idle-only.** On the pure-direct path no interrupt is delivered to our
  userspace process, so the idle-wakeup is a **bounded poll**, not an MSI-X vector: when the ring is empty
  the loop re-polls after a capped short sleep rather than parking on an interrupt. It is still
  *per-ring, not per-session*, it never crosses the wire, and it matters *only when blocked idle* — during
  active traffic the loop busy-drains and does zero sleeping. The two-kinds-of-event split below is
  unchanged; only this primitive reconciled, from MSI-X to bounded poll
  (`grounding/kernel-bypass-and-dma.md §4`).
- **Application-layer events — many, software, derived.** "Session N has data," "session N was ACKed":
  the reactor manufactures these by demuxing frames after the fact. They are not interrupts and not
  on the wire.

So: **"how do interrupts work across the wire?" — they don't.** The wire carries only data frames
keyed by session ID. There is no control message, nothing out-of-band, nothing to route or inspect.
Backpressure is the *absence* of an ACK. The one local idle-wakeup — now a bounded poll, not an interrupt
— is just "there's work"; every per-session event is derived locally from the data. Refusing to put any signal on the wire is
what keeps both the protocol and the firmware surface tiny.

## 6. The protocol — UART + ACK + ID

A session is a UART plus exactly two additions: a **per-frame ACK** (because we *do* care
about the one real failure, receiver overrun) and a **session ID** (so many share the one wire).
Nothing else.

**Frame shape:** `[session ID][payload]`. There is **no software length field** in the transport frame.
The true L1 frame length rides in the **802.3 length field** of the Ethernet frame — in the link encap
*below* the transport frame (directpath `EthSink`/`EthSource`), **not** the RX descriptor: the live 82599
pads sub-60-byte frames to the 60-byte minimum, so the descriptor length includes the pad and does not
delimit the frame (a bare 8-byte ACK padded to 60 would otherwise mis-read as a data frame). That 802.3
length is itself device/wire-influenced, so it is **bounded, not trusted** — `EthSource` clamps it to the
bytes that actually arrived, the same discipline the descriptor length was held to, now with its own
regression test (`grounding/data-path-and-events.md §1`). This is a link-framing detail below the
transport, not a change to the transport frame. **There is no flag byte.**

- **ACKs are bare return frames** on the same discriminator, opposite direction. Data-vs-ACK is told
  by **role/direction**, not a flag: a session's data is one-directional, so at each endpoint an
  arriving frame on that session is unambiguously data (if you're the receiver) or an ACK (if you're
  the sender). No piggybacking — bare ACKs are simpler and free against the headroom.
- **Strict stop-and-wait.** One frame in flight per session; the sender does not advance until the
  ACK returns. This is not a throughput compromise — at µs RTT it is invisible, and it is what makes
  the reactor's interleaving trivial. Throughput, where needed, comes from **more sessions**, never
  a cleverer pipe.
- **The ACK is irreducible; the bit is not.** The ACK exists purely to *delete work*: it lets
  existence-by-emission stand in for a handshake (the first frame opens the session; its ACK *is*
  "peer's up, round-trips work"), so there is no negotiation, no setup state. It is the feedback that
  makes the link reliable — remove it and the sender is blind to overrun loss (a raw, lossy UART).

**The alternating bit — deliberately omitted** (the escape hatch kept, not spent). Its only job is
to recognize a *duplicate* created when a lost ACK forces a retransmit. We omit it because:

- For **idempotent** sessions a duplicate is harmless: display keyframes supersede, file writes are
  offset-addressed.
- The one non-idempotent case — a duplicated **input** keystroke — requires an actual ACK drop, is
  low-harm, and is **human-visible** (you see the stray character and fix it), not silent corruption.
- The *dangerous* failure the bit normally guards (a stale ACK making the sender skip a packet and
  *lose* data) **cannot occur in our model**: the retransmit deadline is generous (§7), the
  single-threaded reactor **drains the RX ring before checking any deadline**, and the link does not
  reorder — so a present ACK is always processed before its timer can fire, a timer fires only on
  *genuine* loss, and there is never a second stale ACK in flight to misread.

If input-exactly-once ever becomes a hard requirement, the alternating bit is exactly where it slots
back — one reserved bit, localized, not a redesign.

## 7. Reliability — retransmit on silence

Retransmit fires when an ACK does not arrive by a deadline. **The deadline is fixed and generous —
~10 ms — with no adaptive RTO, no RTT estimation, no backoff** (that is TCP's machinery for a
variable-quality internet; legacy weight we shed).

Sizing rationale:

- The **wire RTT is tiny** — single-digit µs (1m ≈ 5 ns propagation; a 1500-byte frame serializes in
  ~1.2 µs at 10G; a bare ACK ~50 ns; PCIe/DMA + wakeup a few µs).
- What sizes the deadline is **receiver backlog** — how long an ACK can sit behind other frames before
  the reactor processes it. Stop-and-wait bounds the *depth*: the ring never holds more than ~**K** frames
  (K = concurrent sessions, each capped at one in flight — why the ring is sized to K). The *time* to
  drain that depth is the **real per-frame processing** throughput — ~25 µs/frame measured — so ≈ `K ×
  25 µs`. At the current ring cap **K ≤ 64** that is **~2.29 ms** (measured at K=60), **4.4× under** the
  10 ms window. (An earlier bounded-poll defect briefly inflated this: the idle sleep was pacing a present
  backlog — one delivery per ~112 µs poll cycle — pushing the drain to ~11.3 ms at K=32 and reaching the
  window; the remedy below made the sleep idle-only and removed it. The original "a few hundred µs" here
  assumed sub-µs per-frame processing; the real figure is ~25 µs/frame.)
- **~10 ms** is ~4× the measured worst-of-burst at the K ≤ 64 cap (2.29 ms) and ~100–1000× the wire RTT —
  well under human perception, so a genuine-loss retransmit is invisible. The band of good values is wide;
  stay **under ~tens of ms** so a genuine loss never becomes a *visible* hang — which is also why dialing
  the window up is *not* the fix for the drain-latency scaling (it would cross that bound at large K;
  fixing the drain, below, is).
- **Buffers are ACK-bound** (held until the ACK, one per session in flight; `grounding/data-path-and-events.md §3`).
  The 10 ms window only extends a hold on the *rare* genuine loss, so steady-state occupancy stays
  ≈ K and the window does not move pool sizing.

**The 7a finding, and the remedy (designer's call).** No spurious retransmit fired — the reactor **drains
the RX ring and processes arrived ACKs before it checks any deadline** (§6; §7 below), so an arrived ACK is
always seen before its timer — but the latency-to-window *margin* is exhausted, and at larger K, a coarser
poll, or a busier core the drain would cross the window and fire spuriously. The remedy is **not** to dial
the window up alone: the drain latency scales ~linearly with K, so covering a large-K storm would push the
window past the human-perception bound (tens of ms), the one edge this sizing must stay under. Instead,
fix the root cause — **the bounded-poll sleep is an idle-only primitive and must not pace a present
backlog.** When the reactor has work (RX frames or pending deliveries) it busy-drains with zero sleeps; the
capped sleep applies *only* when there is genuinely nothing to do. That restores the ≈ hundreds-of-µs
worst case and a comfortable, human-invisible window margin. The per-delivery back-pressure remains for
*steady-state* pacing — its real purpose — decoupled from the idle sleep. The window stays generous at
**~10 ms**, now **confirmed on the bench**: with the busy-drain landed, the worst-of-burst is **2.29 ms at
K=60**, 4.4× under the window, zero spurious retransmits, all gates pass — so §7 is validated across the
current operating range, **K ≤ 64** (the ring cap; K is a ring-sized parameter). Growing K past the cap is
a capacity decision that needs a re-measure before extending this range — the scaling is sublinear (a
conservative floor puts the window-crossing near K≈200), measured not assumed. A larger window is still
free if a future max-K wants it, its only cost slightly slower recovery on a rare genuine loss. (Ring
depth and DMA-pool sizing are the adjacent knobs; the busy-drain remedy keeps steady-state occupancy ≈ K,
so it does not move them.)

**Retransmit is bounded — the give-up is silent internal cleanup.** Retransmit does *not* persist
forever (an earlier draft said it did; that was wrong — a sender retrying a dead peer forever holds its
buffer and session forever, an unbounded leak). The retransmit loop has two parameters: the
**interval** (~10 ms, above) and a **give-up bound** (~1–5 s). After no ACK for the give-up window, the
sender quietly **reclaims its own resources** — frees the buffer, drops the session — and emits
**nothing**. There is no connection-dead event; deadness needs no announcement because it surfaces on
its own as *absence of data*, which each consumer already handles with its own idle logic (§8). The
give-up is generous because reclaiming is destructive; on a µs-RTT link a peer silent for seconds is
genuinely gone, so 1–5 s never discards a merely-slow peer. This is **not** a liveness probe (it sends
no heartbeat — it only bounds retransmits of data already trying to go) and it **fires only on an
outstanding unacked frame**, never on idleness — so a dormant session with nothing in flight is never
reaped.

## 8. Session lifecycle — and where this layer stops

The transport adds **no lifecycle primitives.** Its entire upward vocabulary is what §3 and §5 already
established: **a frame, keyed by session ID** — the packet is the event — and its **ACK** (a bare return
frame, §6). There is no opened-event, no closed- or dead-event, no setup handshake, no close signal on
the wire. Lifecycle is what *falls out* of frames arriving or not arriving, plus the transport quietly
minding its own resources. Keeping it this thin is what keeps the layer minimal; every consumer builds
its own lifecycle on top with its own logic.

**A new session is just a frame on a new session ID.** The box opens by emission — the first frame on a
fresh ID. The transport, seeing an ID it holds no session for, instantiates and delivers the frame
upward; the consumer, seeing an ID it has not bound, attaches whatever it needs (a pane, a file sink).
"Opened" is not announced; it is *noticed*, the same way on both sides. The binding (PTY / file /
request) rides in that first frame's payload and is interpreted above; the transport routes bytes and
does not read it.

**Give-up is silent internal reclaim, not an event** (§7). When a sender's retransmits go unacked past
the give-up window, it frees the buffer and drops the session and emits nothing. Deadness needs no
announcement: it surfaces on its own as *absence of data*, and each consumer already handles that with
its own idle logic — the head grays a pane on its own idle counter (it is just a display); the file
layer times out its own incomplete transfer. The transport cleans up after itself and stays quiet. It
never probes liveness either — it does not ask "are you alive," it only stops retrying what was never
confirmed. (Retires the optional heartbeat floated in `console-and-protocol-design.md §4.2`.)

**No cross-epoch state — so reopen "just works," with no mechanism.** The transport holds only its
currently-live sessions; it keeps **no memory that a session ID was ever used.** A reboot resets the
box's session-ID space and begins a new **epoch** — and because the transport tracks no epoch and
remembers no retired ID, a session ID reused in a new epoch is simply a new session by emission, like
any first frame. There is nothing tracking an old session to refuse or reconcile against. **The absence
of cross-epoch state is the mechanism;** we add nothing to "handle restart," because there is nothing to
handle. (A reused ID is at most a cosmetic pane reuse; routing carries zero security weight. This
supersedes `transport-stack.md §2.5`, which keyed proxy state to the box's lifetime — what we now call
an epoch — and dropped it on restart; that keying is removed, and the transport holds no cross-epoch
state at all.) Session IDs: the box is the sole allocator; 64-bit monotonic counter; unique **within an
epoch**, reset on reboot.

**Dormant ≠ dead** — the give-up fires only on an *outstanding* unacked frame, never on idleness, so a
quiet session with nothing in flight is never reclaimed; a dormant pane persists and "revives" simply by
the next packet arriving. One rule, no session-type branch: a session actively failing to deliver is
reclaimed; an idle session waits. Dormancy and cleanup coexist without the transport knowing PTY from
file.

**What lives above this layer (delegated, not designed here):** completion / incompleteness semantics
(the file layer's — it knows its own expected length and runs its own timeout; the transport holds no
file state, no fd, no offsets, no per-session binding table); presentation / dormancy (the head's, on its own
idle counter, cosmetic, never touching the transport).

**Timers, by layer:** the transport's retransmit loop has its interval (~10 ms) and give-up bound
(~1–5 s), both **internal** — neither is a wire signal or an upward event. The head has its cosmetic
gray-out timer; the file layer its own completion timeout. Three timers, three layers, none reaching
into another.

## 9. Open — bench items only

The design forks are settled (including box-restart, §8 — resolved by statelessness, not detection).
What remains is silicon measurement, not design.

- **Above this layer (not the transport's to settle):** how the file service handles an incomplete or
  restart-superseded transfer. It builds on what this layer provides — frames keyed by session ID, and the
  natural silence when data stops — plus its own length knowledge and its own timeout. Flagged for the
  file-service / proxy designer, not designed here.
- **Bench-gated (reserved as open research; see each `grounding/` note's §"open" list):** the minimal
  link-restart form and whether LESM is enabled (`grounding/device-bring-up-and-link.md §5`);
  single-ring throughput and the reactor keeping up on one core; the retransmit window vs. the
  bounded-poll backlog-drain latency under load (§7 — the 7a finding and the busy-drain remedy); DMA
  buffer-pool sizing against ACK-hold (`grounding/data-path-and-events.md §6`); and the pure-direct
  bring-up items (`grounding/kernel-bypass-and-dma.md §5`).

---

*The wire carries only keyed data. One thread turns it into events going up and packets going out,
running one protocol instantiated many times, interleaved over a single ring. It is a UART with an
ACK and a session ID — and almost everything else a network stack carries was deleted because the medium
never posed the problem it solves.*
