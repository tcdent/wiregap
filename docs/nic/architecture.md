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

The subsystem's trusted base has exactly three kinds of member, and — once this posture is fully
realized — **none is trusted by hope**:

- **small-and-ours** — the driver and protocol, verified by *reading* (`grounding/driver-minimality.md`);
- **structural hardware** — the IOMMU, the platform-containment floor the whole system already stands
  on (the same hardware trust that contains the box);
- **untrusted-but-contained** — the **NIC and its firmware**, whose *trustworthiness is immaterial*.

The third is the posture this section commits, and it eliminates the subsystem's last **"hope-and-pray"**
member. We do **not** trust the X520 firmware to be sound — it is bulky vendor code we cannot read. We
**contain** it. A maximally hostile NIC's only in-scope options are to *escalate* (bounded by the IOMMU
+ the descriptor-validating driver) or to *corrupt data* (caught by hardware CRC + retransmit);
everything else it can do is DoS / link-down, which is out of scope. So a fully-malicious firmware's
worst in-scope effect is *nothing*. We depend on the NIC to **function** (liveness), never to **behave**
(security) — exactly the box's status (assertions 4–5: trustworthiness made immaterial by removing every
place it could matter). This is the box's containment logic applied to a device *on* the trusted host —
the VFIO threat model (secure userspace drivers for assigned, untrusted devices), not a novel
construction — and it adds no new platform trust: a rogue NIC bounded by the IOMMU is the same
hardware-containment that bounds the box.

This posture is *more* compliant with the spine, not in tension with it. Assertion 16 says **reject
code you cannot read** — and the diligence-trust we had been extending to the un-readable firmware was a
quiet *exception* to 16, not a thing 16 endorsed. Containing the firmware instead of trusting it
**removes the exception**: we no longer trust code we cannot read; we contain it. Nothing in the spine
needs to change to accommodate this — it closes a gap the spine already wanted closed.

**Why this is necessary to the spec, and the honest gap.** It converts the firmware from the *weakest*
trust we had — diligence on un-reviewable code, an *un-answerable* question — into a *verifiable
structural property*: is the containment airtight? That replacement is what eliminates hope-and-pray. We
no longer place load-bearing trust in firmware soundness; we place it in containment, and containment is
checkable. The containment is **not yet fully grounded** — it requires (A) a driver provably robust to
any descriptor/buffer content (vector 2), and (B) the IOMMU airtight against bypass (interrupt
remapping, ATS-off, no peer-to-peer) — tracked in `concerns-to-watch.md`, detailed in
`grounding/kernel-bypass-and-dma.md §2`. But the difference from the posture it replaces is decisive:
the remaining work is **verification, not hope.** A grounded "the containment holds" *deletes* the soft
spot; an un-grounded "the firmware is sound" never could. The posture is committed now; closing (A)+(B)
is what makes it fact.

## 3. The runtime architecture — a flat, single-threaded reactor

One thread owns the wire, both rings, all session state, and one `epoll`. The loop:

```
poll the RX ring (busy-poll drain)         → for each frame: demux by session ID → its session reacts inline
drain pending output to the TX ring        → the reactor is the sole TX-ring writer
when idle: epoll_wait(eventfd, deadline)   → two wake reasons: a frame arrived, or a retransmit deadline passed
```

- **One `epoll` watches everything**: the NIC's idle-wakeup eventfd, the earliest retransmit
  deadline, and the application's own source fds (PTYs, etc.). Every readiness is an event.
- **The packet is the event.** A frame arrives, it is keyed by the session ID in its header, it
  bubbles straight to that session, which reacts. Nothing stages it, pulls it later, or
  paces it. There are **no intermediary buffers** — the decoupled "rings → staging → worker-pool →
  backpressure-pacing" pattern of conventional stacks (thousands of lines) does not exist here.
- **Busy-poll is the drain, not the architecture.** The system is event-driven, not polling-driven;
  the busy-poll only empties the ring while frames are present, then the loop blocks on the interrupt.
- **Single producer, no locks.** Because the reactor is the only thing that touches either ring,
  there are no locks and no races on ring or session state. This resolves the open fork in
  `transport-stack.md §4.4 item 14` — single-producer into the one TX ring.
- **Why single-threaded**, on the three lenses: *simplicity* (no locks/races), *understandability*
  (the whole event flow is one readable loop), *firmware surface* (one MSI-X vector, one RX/one TX
  ring, one eventfd — multi-threading would push back toward multiqueue/RSS/multiple vectors, the
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
refills the ring with a fresh one) — what the caller holds is out of the ring but still in the IOMMU-
mapped region. We copy fully out into non-mapped memory. The **TX side is the mirror**: the app works
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

## 5. The interrupt / event model

Two things are called "interrupt"; conflating them is where complexity comes from, so they are kept
distinct:

- **The hardware interrupt — one, local, idle-wakeup only.** A single MSI-X vector meaning "the ring
  went non-empty." It is *per-ring, not per-session*; it never crosses the wire (a local NIC→CPU
  signal); and it is taken *only when blocked idle* — during active traffic the loop busy-polls and
  takes zero interrupts. This is the entire firmware surface for events. (Grounded:
  `grounding/data-path-and-events.md §4`.)
- **Application-layer events — many, software, derived.** "Session N has data," "session N was ACKed":
  the reactor manufactures these by demuxing frames after the fact. They are not interrupts and not
  on the wire.

So: **"how do interrupts work across the wire?" — they don't.** The wire carries only data frames
keyed by session ID. There is no control message, nothing out-of-band, nothing to route or inspect.
Backpressure is the *absence* of an ACK. The one hardware interrupt is a local "wake, there's work";
every per-session event is derived locally from the data. Refusing to put any signal on the wire is
what keeps both the protocol and the firmware surface tiny.

## 6. The protocol — UART + ACK + ID

A session is a UART plus exactly two additions: a **per-frame ACK** (because we *do* care
about the one real failure, receiver overrun) and a **session ID** (so many share the one wire).
Nothing else.

**Frame shape:** `[session ID][payload]`. There is **no software length field** — the length
is read from the RX descriptor rather than carried in our framing. But that descriptor field is
firmware-written from box-influenced wire bytes, so it is **bounded by the driver, not trusted**
(`grounding/data-path-and-events.md §1` — the descriptor is adversary-influenced input). **There is no
flag byte.**

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
- What sizes the deadline is **receiver backlog** — how long an ACK can sit behind other frames
  before the reactor reaches it. Stop-and-wait bounds this: the receiver's ring never holds more than
  ~**K** frames (K = concurrent sessions, since each is capped at one in flight — which is why the
  ring is sized to K). So worst-case ACK latency ≈ `K × per-frame-processing + wire RTT` ≈ a few
  hundred µs even for hundreds of sessions.
- **~10 ms** is ~100–1000× the wire RTT, several times over the worst-case backlog, and still well
  under the threshold of human perception — so a genuine-loss retransmit is invisible. The band of
  good values is wide (hundreds of µs to tens of ms); stay **under ~tens of ms** so a genuine loss
  never becomes a *visible* hang.
- **Buffers are ACK-bound** (held until the ACK, one per session in flight; `grounding/data-path-and-events.md §3`).
  The 10 ms window only extends a hold on the *rare* genuine loss, so steady-state occupancy stays
  ≈ K and the window does not move pool sizing.

Bench-gated (one knob): confirm 10 ms stays comfortably above the worst observed ACK latency under a
deliberate redraw storm; the only cost of a larger window is slightly slower recovery from a rare
genuine loss, so it can be dialed up freely if a measurement surprises us.

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
  single-ring throughput and the reactor keeping up on one core; the 10 ms window under load; DMA
  buffer-pool sizing against ACK-hold (`grounding/data-path-and-events.md §6`); the IOMMU group
  composition (`grounding/kernel-bypass-and-dma.md §5`).

---

*The wire carries only keyed data. One thread turns it into events going up and packets going out,
running one protocol instantiated many times, interleaved over a single ring. It is a UART with an
ACK and a session ID — and almost everything else a network stack carries was deleted because the medium
never posed the problem it solves.*
