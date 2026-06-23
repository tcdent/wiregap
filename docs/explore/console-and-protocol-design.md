# The Console, Protocol & Multiplexer

### Design Decisions for the Human-Facing Layer of *Containment by Construction*

> This document records the decisions made for the interface and transport that sit
> **on top of** the containment architecture — the thing a human looks at and types into
> while directing contained agents, and the channel that carries an agent's work back off
> the box. It is the companion to *Containment by Construction*: that document defines the
> boundary; this one defines what crosses it toward the human, in what shape, and which
> pieces carry risk. The containment guarantees themselves (structural egress absence, the
> small trusted base, the single human-in-the-loop at promotion) are assumed, not re-argued.
>
> **The primary goals are no egress and no privilege escalation. Render fidelity is not a
> goal.** The renderer is built to be robust wherever robustness is free, and never at the
> expense of the two goals that are load-bearing. Fidelity is a courtesy the design extends,
> not a promise it makes — a stale pane is a cosmetic state, never a failure.
>
> **The whole design sorts on one question: does a byte land on trusted ground, or not?**
> Bytes written onto a trusted host (the proxy or the user's machine) are the highest-risk
> surface in the system and must be made incapable of execution by construction. Bytes flowing
> into the zero-trust box are taken un-seriously by design — the box is expected to acquire
> things that may harm its own health, and its health is not a security concern. This is the
> mirror of containment: the parent document makes the box unable to act; this layer makes
> trusted ground unable to be acted upon.

---

## 0. Scope

Four things and their seams:

1. The **console** — a fresh, minimal macOS application replacing the terminal emulator and
   the multiplexer (tmux) at once, built for the actual workflow: project-oriented,
   multi-agent, some agents foregrounded and others receded.
2. The **transport and its services** — a greenfield substrate that moves bytes, with two
   services layered on it: a live pane view, and file extraction off the box. This replaces
   the legacy PTY stream with a format that carries terminal *display state* and *file bytes*
   and is structurally incapable of carrying terminal *capability*.
3. The **box-side capture** — running real PTYs on the box and resolving their output into the
   protocol before anything leaves.
4. **Code extraction** — the box writing artifacts onto trusted ground, into a quarantine where
   separate downstream tooling handles promotion. (Its inverse, getting bytes *onto* the box, is
   deliberately *not* a protocol feature — the box pulls from whitelisted egress; see §4.4.)

---

## 1. Posture

Two ideas shape every decision below. The first sorts the entire system; the second keeps its
trusted core small. Everything else in this document is one of these two applied to a specific
place.

### 1.1 Byte-creation on trusted ground is the highest-risk surface we expose

The organizing axis of the whole design is a single question asked of every byte that moves:
**does it land on trusted ground, or not?** A trusted host is the proxy or the user's own
machine. Bytes arriving on either are the most dangerous thing in the system, and the bar for
permitting such a write is **structural impossibility of execution — never diligence, never
"unlikely."** Bytes flowing the other way, into the zero-trust box, are taken **un-seriously by
design**: the box is *expected* to acquire things that may be harmful to its own health, and harm
to the box's health is not a security concern, because the box can do nothing with anything it
holds.

These are not two points on a spectrum of caution. They are **categorically opposite postures**,
and the line between them is *where the bytes land* — not what the bytes are, and not where they
came from:

- **Bytes onto a trusted host (proxy or user machine):** maximal seriousness. The receiving side
  must interpret nothing the box controls as an instruction; the dangerous capability must be made
  *unrepresentable*, not filtered. This is the single reason the renderer is a renderer and not an
  emulator, the reason neutralization happens upstream of the user's workstation, and the reason
  extraction deposits into an inert folder while interpreting only a destination that cannot
  express anything but a leaf. Each is the same posture applied at a different trusted host — not
  separate cleverness, one idea applied repeatedly.
- **Bytes into the zero-trust box:** deliberate indifference. The box may read from any whitelisted
  source using any existing tool, may catch a virus, may have its own core freely edited by its own
  agents. None of it matters, because the box is contained and its internal state was never
  load-bearing.

This is the mirror of the parent document's containment claim, stated for the interface layer:
*Containment by Construction* makes the box unable to act; this layer makes trusted ground unable
to be acted upon. Together they bound the system from both ends — nothing the box does reaches out,
and nothing reaching a trusted host can execute. **Writing into trusted ground is the dangerous
direction; it is where the box has any ability at all to put bytes onto a system where execution
would matter, so it is the direction that gets hardened.** Reading into the box is the safe
direction and needs no protection, because interpretation then happens on untrusted ground where
execution is already assumed and harmless.

The single surface this binary does not fully reach is the **egress allowlist** (§3) — the one
place the box gets to *address* a destination off itself. It is not "writing to our trusted host,"
but it is the box influencing what happens beyond the box, which is why it is the one surface that
needs recurring human curation rather than a structural guarantee. It is not an exception to the
posture; it is the residual case the clean binary leaves over.

### 1.2 The frozen interpretation surface

**What is frozen is the proxy's interpretation surface — not the format's expressiveness.** The
distinction is the whole reconciliation between "specify once and stop" and "leave room to grow,"
which only look contradictory until you ask *where* a given change lands.

The proxy interprets a small, fixed set: the framing envelope it reads, the destination it
confines, the byte-map it applies, the egress match it enforces. **That set is frozen and stays
small** — it is the must-be-correct core, and the danger to a contained system is its drift.
Growing what the proxy must parse or enforce is real surface creep: expensive, dangerous, and only
ever done as a conscious, reviewed spec revision. This is the accretion that makes evolved
enterprise systems insecure, and the freeze exists to refuse it.

But most of what the format *carries*, the proxy never interprets — it **relays opaquely.** Style
descriptors, glyph payloads: bytes the proxy passes through without reading. And content in that
pass-through zone is **free to evolve with no proxy change at all**, because evolving it is a
*client render* change, on the side that is allowed to change freely precisely because it is
outside the security path. A new styling attribute added in some future spec is one additional
branch in the head's draw routine and **zero change to the trusted core.** The server does not
need a code change to accommodate it; it was already relaying those bytes blind.

So the rule that tells future-you which spec changes are cheap and which are not: **a change in
the pass-through zone (render features) is cheap and head-only; a change that grows what the proxy
parses or enforces is expensive and deliberate.** Styling lives entirely in the pass-through zone
— the most freely-evolvable part of the whole system, the opposite end of the spectrum from the
egress allowlist. That is why leaving styling deliberately open is safe where leaving the proxy's
surface open would not be: they are different layers, and only one of them is load-bearing.

This is also why two things below are true at once. Where a choice is made for *simplicity*
(keyframe-only display, no held state), simplicity and the posture point the same way — less for
the proxy to interpret. And where a choice is made for *completeness* (the full styling set, with
reserve), completeness costs the posture nothing — the proxy was never going to read those bytes.
The freeze and the generosity sit in different layers and do not fight.

---

## 2. Subsystems — the discrete components

Eight components plus the protocol that joins them. Risk classification is in §3; the base/service
layering of the transport is in §4.

**The Box.** The untrusted execution environment. Owned hardware, one network interface, full
bash, free internal tooling. **The system enforces nothing about the box's internals** — it
installs what it wants, organizes its own filesystem, sets its own conventions. Agents on the same
box can discover one another; preventing that was rejected as a goal. Co-visibility is an
*operations* posture, not a security one. The box may hold credentials, because every dangerous use
of one is structurally unreachable.

**The Box-Side Capture & Render Client.** Runs on the box. Allocates the real PTYs agents attach
to, runs a **headless terminal emulator** that resolves the PTY's escape grammar into a grid of
cells, and serializes that grid into the protocol. This lets native terminal programs keep behaving
natively while no escape sequence ever leaves the box. It lives in the untrusted zone, so it is
allowed to be loose — old C is acceptable. The one open research question lives here (see §5).

**The Box→Proxy Link.** A dedicated 1m DAC cable from the box's one NIC (Intel X520-DA2) to the
proxy — the box has exactly one physical peer, never a switch. **Kernel-bypassed and driven from a
minimal userspace driver; TCP and the kernel network stack are rejected** (rationale in §4.1). Raw,
no encryption, no security layer: the adversary *is* the box, and eavesdropping a private cable is
physical access, out of scope.

**The Proxy — Egress Guard.** Enforces the one outbound invariant: requests go only to a
**human-curated allowlist** of exact host+path destinations, with **verb gating** and **no
redirect-following**. Its concern is the box reaching an endpoint that escalates into code
execution. The credential lives on the box, not here — so this is a *destination enforcer*, not a
credential broker, which keeps it small.

**The Proxy — Stream Relay.** Hosts the base transport and its services (§4). It accepts the box's
streams off the single link, dispatches each to the right service, applies the **inert byte-map** to
display content, writes extracted files into the quarantine, serves read-requested files back, and
relays the safe stream to the head. **It holds no display state — it is a pure relay** (§4.2). The
neutralization and the file-path confinement are the load-bearing parts; the stream bookkeeping and
routing are feature logic allowed to be buggy.

**The Proxy→Head Transport.** Carries already-inert frames to the head. **Transport-agnostic by
construction** — a LAN head connects directly. It is agnostic *so that* a remote head can tunnel
over whatever secure transport it already has (SSH-as-tunnel is the conceptual example), but that
tunneling is an **affordance the design preserves, not a component it builds** (see §5).

**The Head — macOS Console & Multiplexer.** The GUI. A **pure renderer**: a font, a grid, a cursor
it alone advances, and zero escape interpretation — it paints cells. It owns the window chrome and
all layout. The multiplexer lives entirely here: it routes streams to panes and holds the layout
model. Any cleverness about *when* to paint or when to request a redraw lives here too, touching
nothing trusted. It may *also* speak ordinary SSH-with-a-real-emulator to arbitrary servers as a
**completely separate code path**, sharing the window chrome and nothing else with the box-render
path.

**The Quarantine.** A folder on the trusted side where extracted files land, byte-identical. This is
where the transport's responsibility ends. Whatever reads the folder, shows a human the diff, and
crosses an artifact into a privileged environment is **separate downstream tooling** — the promotion
step belongs to the parent architecture's human-in-the-loop and is out of scope here (noted in §3).

**The Protocol.** The base transport and its three services (pane, file-write, file-read), specified
in §4.

---

## 3. Risk classification

Risk is deliberately concentrated in one place and made absent everywhere else.

### High risk — the egress allowlist

**The single high-risk surface in the system, and the only place a structural guarantee is replaced
by recurring human judgment.** Everywhere else the box's capability is irrelevant because it cannot
act; the egress allowlist is the one path where box output reaches the outside world, so capability
matters again. The liability is **arbitrary code execution through an approved endpoint**.
Mitigations: exact host+path matching, verb gating to read-shaped surfaces, no redirect-following (a
redirect is the box reaching an unreviewed destination behind an approved entry), a hard cap on list
size. The residual risk is judgment-based and *per-entry* — each destination rests on a human having
decided it cannot escalate, and no structure catches that if it is wrong. This warrants the most
attention and the most conservatism, and the curation criterion belongs written beside the list.
(Data *exfiltration* is explicitly **not** a concern this design defends against — the scoped
liability is execution.)

### Trusted but verifiable — the small core that must be correct

Load-bearing for security, but each one's correctness is a **lookup or an enumeration, never a bet**.
They must stay small enough to read in a sitting.

- The **inert byte-map** on display content: a total function over the 256 input bytes, verified by
  enumeration. Printable ASCII passes; everything else renders as a visible literal. A novel input is
  handled identically to a known one — there is no "anticipate" step to fail open.
- The **file-destination confinement**: the same total-function shape applied to the file service's
  path (§4.3). Each path element is validated as a well-formed leaf; the proxy supplies every
  separator. Traversal is not *caught*, it is *unsayable*.
- The **egress destination-match**: exact host+path comparison, static and compiled-in, never
  runtime-configurable by anything the box can influence.
- The **base transport's frame reader and stream-hello parser** (the hello that opens a stream and
  declares its service): bounded, dumb. They parse *our* trivial framing, never the legacy escape grammar.

### Risk-irrelevant — compromise assumed, or content already inert

**No risk that matters**, for one of two reasons: the component lives in the untrusted zone where
total compromise is assumed and contained, or it only ever handles content already made inert
upstream.

- **The Box**, **the Capture & Render Client**, **the Box→Proxy Link** — the untrusted zone. A
  hostile box can only emit malformed frames (garbage in their own pane, or a rejected transfer) and
  reaches nothing.
- **The base transport's byte movement** — moving bytes interprets nothing; the box link goes nowhere
  regardless of how many streams ride it.
- **The quarantine write** — writing verbatim bytes to a file interprets nothing as code.
- **The Proxy→Head Transport** and **the Head's box-render path** — downstream of neutralization;
  they handle only already-inert frames and are outside the trusted base.

Agent keying is **routing, not isolation** — zero security weight; a misroute is cosmetic. Any
workload that genuinely must not be seen by another is separated **physically** (a separate box on a
separate link), never by in-environment isolation.

### Conventional risk, isolated and opt-in — the head's SSH path

The console's optional ability to SSH into ordinary servers carries **normal terminal risk** (a real
emulator over a legacy PTY), but it is opt-in, talks only to servers the user chose, and is **walled
off from the box-render path** — two doors, no hallway. It changes nothing about containment as long
as that wall holds.

### Out of scope here, but load-bearing — promotion

The crossing of an extracted artifact from quarantine into a privileged environment is the **one
structural human-in-the-loop** and is high-stakes. It belongs to the parent architecture and to
**separate downstream tooling**, not to this document. The only constraint this document asserts: the
transport deposits bytes in the quarantine and stops; the gate that helps a human decide must be a
separate system that **makes you a better reader, never reads on your behalf** — no scoring, no
flag-as-safe, no auto-promote.

---

## 4. The Transport and Its Services

Everything crossing between box and head rides **one base transport**, and every distinct capability
— a live pane, a file pulled off the box — is a **service layered on top of it**. A dumb common
substrate moves bytes and knows nothing about what they mean; above it sit the service
implementations the application supplies, one per thing actually needed. The base is shared and found
once; the services differ and are added on top. In code this is a pane handler and two file handlers
(write and read) dispatched by the hello that opens each stream — **three services, not a plugin
framework**; the layering is a way of thinking, not a registry to build.

**A note on "layers," since the parent document argues against them.** *Terraces, not layers* there
is a **security** principle: do not stack leaky mitigations and hope the pile holds; let each
boundary bear on the ground at its own locus. The layering here is **functional composition**, not
security stacking. The safety guarantees do not ride these layers — neutralization, path
confinement, and egress-destination each live at their own locus and hold regardless of how many
service layers sit above the transport. Building services on a base transport does **not**
reintroduce the tower-on-sand the parent rejects, because nothing load-bearing for safety depends on
the stack staying upright. The base stays dumb precisely so the complex, service-specific logic lives
above it and the security loci stay characterizable alone.

### 4.1 The base transport — the common substrate

A greenfield binary substrate. JSON was rejected — re-encoding cell-and-style data or raw file bytes
as quoted objects is pure overhead, and a JSON parser is a parser the proxy does not want.

**The transport rejects TCP and the kernel network stack — the largest by-assumption surface in the
trusted base.** Everything else in this design is small enough to read in a sitting or static and
vetted; the kernel's TCP/IP stack is neither. It is enormous, written in C, parses attacker-shaped
input (the box's packets), and sits *underneath* the entire trusted base, reached by the adversary
before any of our careful code runs — so a stack vulnerability is a hole beneath everything, that no
byte-map or leaf-confinement can guard because they execute too late. "Battle-tested" is not the same
as "small and verifiable," which is the bar the rest of the design holds. So the network stack is the
one component present by *assumption of soundness* rather than by *smallness*, and this transport
exists to delete it from the path.

**Kernel-bypass on a dedicated Intel X520-DA2, driven from userspace.** The NIC is unbound from the
kernel driver (via VFIO) and its registers and DMA are mapped into our process; from there **no
kernel code is ever in the data path** — packets DMA straight into our buffers and the kernel's
network stack touches no byte the box sends. We drive the card with a **minimal userspace poll-mode
driver built from the 82599 datasheet primitives — learning from the `ixgbe` driver's behavior but
using none of its source** — lighting up the fewest registers that bring up a single bidirectional
link (one RX ring, one TX ring; no multiqueue, RSS, offloads, or filtering). Every card feature we do
not touch is configuration space that cannot carry a bug in *our* code. The card stays mostly dark.

**The link is high-bandwidth, not serial-constrained — this is the opposite of the UART fork.** A 1m
DAC cable at 10 Gb, point-to-point, kernel-bypassed, gives **single-digit-microsecond round-trip
latency** — and that low latency is *because* the kernel stack is gone, not despite the simple
protocol. UART was rejected earlier for a bandwidth ceiling that made live interaction and file
transfer mutually exclusive; this is the inverse — a fast link whose only remaining latency is the
tiny RTT. The throughput worry that haunts simple protocols is retired on arithmetic (below), not
hand-waving.

**No routing, no addressing — the cable is the addressing.** One link, two endpoints, never a switch.
Everything TCP carries to survive a hostile shared medium is absent here: no addresses (one peer), no
reordering (a point-to-point link delivers in order), no congestion control (one sender, one receiver,
a dedicated link — there is no congestion to control), no path-MTU or window-scaling (known link).
Trimming TCP would smuggle its hostile-medium worldview in; instead the protocol is **derived from
this link's actual failure modes**, which is a far shorter list. The frame carries only a **stream
discriminator** (which pane / which transfer — pure demux, zero security weight, a misroute is
cosmetic) and its payload. TCP and UDP are consulted for inspiration on *framing*, never for
mechanism.

**The one real failure mode is receiver overrun, addressed by reliability derived for it.** On a clean
point-to-point link the sole loss source is the receiver draining its descriptor ring slower than the
card DMAs into it — a rate-matching problem, not a network one. (Bit errors are caught by the X520's
hardware CRC and the frame dropped, so corruption needs no software check; a CRC-dropped frame simply
looks like a lost one.) The reliability mechanism is therefore **per-stream stop-and-wait with a
per-packet ACK** — and this is sufficient, not a compromise, for three reasons that fit together:

- **In-order delivery is a requirement, not a limitation.** A terminal's state is serial — each
  keyframe supersedes the last — so a frame arriving out of order would be meaningless. Stop-and-wait
  provides per-stream ordering for free: frame N+1 cannot leave before N is ACKed, so it cannot arrive
  first. The protocol's simplicity and the display's needs are the same shape.
- **The throughput is enormous, by arithmetic.** At microsecond RTT, one-frame-per-RTT is roughly
  **~200,000 frames per second on a single stream** — orders of magnitude past anything a terminal
  produces (bounded by human-perceptible rates and, for redraw storms, by box-side emulator speed, not
  the link). The stop-and-wait throughput collapse is a millisecond-RTT phenomenon; at these latencies
  it does not bind.
- **Aggregate throughput scales by instance, not protocol complexity.** Where the pipe must actually
  be filled (a large file), the work parallelizes: a file is causally splittable (byte ranges have no
  dependency), so it ships as K independent streams reassembled by offset, each a dumb stop-and-wait
  stream, the trusted side still doing only inert offset-writes. This is **one validated codepath run
  in parallel with a little orchestration**, never a second protocol — so concurrency adds throughput
  without adding attack surface. Causally-serial streams (a pane) cannot be split this way, but they
  never approach the single-stream ceiling, so they do not need to be.

**Protocol bringup dissolves into the ACK; device bringup is separate and below it.** Because every
packet is ACKed, there is no handshake to design — the first packet's ACK *is* "the peer is up and
round-trips work," and the no-ACK-yet case is just the resend rule applied to packet one. This is
distinct from **device bringup** — the register sequence that brings the PHY, the link, and the DMA
rings up — which lives *below* the protocol (you cannot send a packet until the wire is electrically
live) and is the small-but-exacting, datasheet-driven part. No packet-level mechanism replaces it.

**The honest trusted-base accounting for the transport**, with no overclaiming:

- **Our minimal protocol** — small, ours, audited; the thing that parses the box's framing.
- **Our minimal poll-mode driver** — built from the datasheet, lighting up one RX/TX ring; the part
  most deserving scrutiny, since it interprets descriptor rings the firmware populates from the wire.
- **The X520 firmware** — trusted by *deliberate, diligence-backed choice*: the reference part for
  userspace networking, a complete public datasheet (so the device is driven from documentation, not a
  blob), and a long field history. Recorded as a specific, re-examinable trust decision, not an
  assertion.
- **Boot-time kernel PCI enumeration + IOMMU** — one-time, non-attacker-facing (the box does not
  control the PCI bus), and *not in the data path*. The IOMMU is a **confinement benefit, not a
  surface**: it bounds the NIC's DMA to our buffers, so "we gave a device direct memory access" does
  not mean "the device can scribble all of RAM." We keep it deliberately.

The kernel TCP/IP stack — the prior largest by-assumption surface — is **retired from the trusted
base entirely.** What replaces it is each either small-and-ours, static-and-vetted, or
boot-time-and-non-adversarial; none of it is a per-packet bet on a large parser, which was the whole
problem. *(The exact minimal bringup sequence, the precise primitive set, the ring-level rate-matching
detail, and how small the protocol can ultimately be are the subject of active hands-on hardware
research; this section commits the posture, not the final register-level specifics.)*

### 4.2 Service: the pane session — a held view, an unheld stream

A pane is a **held** thing only on the box: the head attaches to a live PTY, and the session persists
there with continuous grid state. But **the proxy holds no state about it** — the pane is, to the
proxy and to any consumer, simply a **stream you receive or do not.** There is no registration of
consumers, no per-consumer tracking, no cached "latest keyframe." There is nothing to enumerate
because there is nothing held — and a thing that cannot be registered cannot have its registration go
wrong. Multiple consumers are inherently free: a stream fanning out to N listeners needs no
coordination, because it is the same bytes reaching everyone.

The behavior this produces is a **serial-console vibe**, stripped of the legacy cruft. Attach
mid-stream and you see whatever arrives next; if it looks stale, you wait for the next keyframe or you
ask for a redraw — the console equivalent of pressing Enter to repaint a stale terminal. A blank
moment on attach is not a cost paid for statelessness; **it is simply what a stream is.** Any
heuristic about when to repaint or request a redraw lives entirely on the head and is free to be as
clever or as dumb as wanted, because it touches nothing trusted and nothing stateful.

Display flows **one direction** (box → head); input flows on a **separate channel** back (§4.2.1).
The channels are split for **separation of concerns and orderliness — not for security.** (Reflection,
the classic terminal vulnerability, is already impossible here because the head is a renderer that
generates no responses to box queries and the protocol has no query frame for the box to send; the
channel split is not what carries that weight, and the document does not pretend it does.)

**The grid is resolved on the box.** Character width — wide CJK/emoji, zero-width combining marks — is
computed by the emulator that owns the grid; the head paints what it is told.

**The style descriptor — fixed-width, 16 bytes, four columns.** Styling has **no attack surface**
(the proxy relays it opaquely, §1), so the goal here is the inverse of everywhere else: not minimize
but **be a complete, modern representation of the rendering features the world actually uses, with
deliberate room for the ones we did not foresee.** Sixteen bytes laid out as four equal four-byte
columns — three color columns and one attribute column — which is both comfortable reserve and clean
struct geometry (everything on four-byte boundaries):

- **Foreground**, **background**, and **underline color**, each a four-byte column: 3 bytes of RGB
  plus **one reserved byte** per color (room for a future per-color flag, alpha, or a palette
  escape hatch — reserved-zero for now). Underline color as its own channel gives colored error
  squiggles.
- **Attributes**, a four-byte column: the real-world set spoken for in the low bits — bold, dim
  (distinct from bold), italic, inverse, strikethrough, blink, conceal, and underline as an enum
  (none/single/double/curly) — with the **remaining bits explicitly reserved-zero** as growth space.

**Every field is always present** — fixed-width, no variable-length cell, no "present only when
curly" special case. **Reserved bits and bytes are zero and ignored by the renderer**, so a newer
descriptor with an attribute an older head does not know degrades gracefully — the head simply does
not paint it. The descriptor **cannot carry an escape**, because it is integers, not a string.

This is where §1 pays off concretely: a styling attribute added in a future spec is **one more
branch in the head's draw routine and zero proxy change** — it populates reserved space the format
already carried opaquely. That is not surface creep, because the proxy never interpreted these bytes;
the freeze is on what the proxy reads, and styling was always in the pass-through zone. So sixteen
bytes with generous reserve is not over-provisioning — it is the cheapest insurance against the one
mistake styling can make, which is leaving something out. We are not bandwidth-constrained, and the
cost is bytes per *run*, not per cell.

**Cells: the atom is the cell-run** — position, style descriptor, run-length, then UTF-8 payload. A
display frame body is a packed list of these. Run-length encoding does real work: terminal screens are
mostly repeated cells in one style, collapsing a screen to a handful of runs before any transport
compression.

**Display is keyframe-only, and v1 has effectively one display frame type: the keyframe** — a full
grid snapshot carrying grid contents, cursor (position/shape/visibility), and dimensions (implicit in
the grid's shape). Cursor and resize are therefore *not* separate frame types: a cursor move or a
resize is communicated by sending a fresh keyframe. This is chosen for **simplicity, which serves the
posture**: at kilobytes-per-frame and human interaction rates, a full keyframe per change is cheap, so
any partial-update mechanism would be premature, and — per §1 — keeping it out of the frozen surface
means no future code handling cases the original never contemplated. (An optional **heartbeat**
keepalive carries no display state and so does not break keyframe-only; it is largely subsumed by the
transport's per-packet ACK, which already signals liveness, and is needed only if a quieter
dead-peer signal is wanted on an idle stream.)

**Hard law: every display frame answers "what does the screen show," never "what should the client
do."** No frame ever carries an action — no run-this, no clipboard-write, no open-URL, no OSC-52
smuggled back as a structured field. The instant a frame can make the viewer act, in-band signaling is
back.

**No embedded protocols.** UTF-8 gives braille (U+2800–U+28FF), box-drawing, powerline glyphs, and
emoji for free — they are just codepoints. Image protocols (Sixel, Kitty, iTerm) are **excluded** —
not for minimalism but because they are parsers and embedded-capability channels, the same in-band
door closed everywhere else. The renderer may, by a deliberate spec revision, gain new ways to draw a
cell — never a new thing the stream can ask the client to do.

**Emission is change-driven.** An idle agent sends nothing. **60fps is a latency ceiling, not a frame
rate** — when the screen changes, the frame is on the wire fast enough that nothing feels slower than
a local terminal; zero traffic at idle. A periodic keyframe **may** also be emitted (and a client may
request one), which bounds how long a freshly-attached or confused consumer stays stale — but this is
a **render-convenience default, not a contractual guarantee.** Nothing about the security goals
degrades if a pane sits stale, so the trusted path owes no cadence promise; the cadence is a soft knob,
not a load-bearing rule.

#### 4.2.1 The input channel (head → box)

The clean mirror of display, **minus neutralization** — the threat direction is reversed. Input
carries *your* keystrokes into the contained zone, the safe direction; you may send the box anything,
because the box is the contained thing. There is no adversary on this end, so no byte-map: input must
only be **well-formed**, not **sanitized**. Two message types: **text** (bytes routed to the PTY's
stdin, flowing to the program as if typed) and **resize** (rows and cols routed to the PTY *layer* — a
window-size change, not stdin — after which the program re-queries and the box emits a fresh keyframe
at the new size). **Resize travels as its own message type, never as an escape inside the text
stream** — the input-side echo of *out-of-band the control, keep data pure data*. The likely
candidate for a future spec revision is structured **signal injection** (a clean "SIGINT this PTY" so
Ctrl-C is reliable) — outside the v1 surface, admitted only deliberately if at all.

### 4.3 Service: file transfer — the write path into trusted ground

This is the service that earns the most scrutiny, because it is **the box writing bytes onto a
trusted host** — the highest-risk direction in the system (§1.1). Its justification is not "a safe
way to get data off the box" (the box can already leak via egress; data-egress is out of scope) and
not "the plumbing of promotion" (that is a *consequence*). The justification is structural: **any
channel that lands box-controlled bytes on trusted ground must make execution on that ground
impossible, and no existing transfer tool clears that bar, because existing tools earn their value
by *interpreting* what they are told.** So we built the one channel dumb enough to be safe writing
into trust: it deposits bytes and interprets nothing.

**Why not rsync, scp, an upload server?** Because each is *a program on the trusted side that takes
box-supplied instructions and acts on them.* rsync can run a command on the remote (`--rsync-path`),
runs server-side logic, rides a remote shell; scp rides ssh; an upload endpoint runs a parser. Every
one of them is, by construction, a remote-execution surface *pointed at the trusted host* and handed
to the adversary. The attack surface is not incidental to the convenience — **the convenience is the
attack surface**, because the way these tools are convenient is by being instructable, and
instructable-from-the-box is exactly what cannot be allowed when the bytes land on trusted ground.
The bespoke channel exists precisely because nothing off the shelf is non-interpreting enough: a
thing dumb enough to be safe here is a thing that does not exist as a product, since products earn
their keep by being smart. This is the same move as the renderer-not-emulator on the display path —
never let the trusted side interpret adversary-controlled input as instructions. Extraction is that
principle applied to the write direction.

A file transfer is **transient**: a read of the box's filesystem at a point in time, serialized onto
the wire and then over. It holds nothing, has no grid, no lifecycle. It is **unattributed** — a file
is a filesystem snapshot, not an agent's output, and which agent triggered the read is incidental and
not carried.

**It is a service on the existing transport, not a new channel.** Code is text and assets are bytes;
they ride the same base transport into the quarantine. A second outbound mechanism (git push, scp, an
authenticated session to a third party) would both reopen execution-on-trusted-ground *and* add a
second egress path the box could author — so none is added. If code should reach somewhere external
eventually, that somewhere talks to the *proxy*; the box's only act is to emit bytes.

**A transfer is its own stream, which makes it trivially simple.** The stream opens declaring the
service and the **destination**; the rest is the file's **verbatim bytes**. The proxy confines the
destination, opens the file under the quarantine root, and writes bytes as they arrive. A small file
is one stream; a large file is split into byte ranges shipped as **K parallel streams reassembled by
offset** (§4.1) — the box choosing the chunking, the trusted side doing only inert offset-writes, so
parallelism adds throughput without adding trusted-side complexity. Per-stream ordering and integrity
come from the transport (per-packet ACK; the X520's hardware CRC drops corrupt frames), so there is no
per-chunk framing or correlation logic in the service itself. This is *easier* than the display path:
no emulator, no grid, no width math. "Open file, write bytes at offset."

**The bytes are verbatim and safe; the destination is the one piece of box-controlled input the
trusted side acts on — and it is where the real security argument lives.** Writing bytes to a file
interprets nothing, so the payload is sound by construction. But "the proxy acts on the destination"
is the sentence that should make us nervous, because acting on adversary input is the whole danger.
The question that decides whether this channel is sound or merely rsync-in-disguise is: *can the box,
via the destination, make the trusted side do anything other than place bytes in the quarantine?* The
answer must be structurally no — and the array-of-leaves construction below is what makes it no. Had
we let the destination be a path string, extraction *would* be rsync-in-disguise: a box-authored
instruction the trusted side acts on with a grammar rich enough to escape. The path handling is not a
tidiness detail; it is the entire reason the write-into-trust channel is sound.

**The destination is impossible-to-traverse by construction, not validated-then-rejected.** A path is
dangerous because it is a *traversal language* — separators and parent-references are how you climb. So
the destination is **not a path string**; it is an **array of leaf names**, and the proxy supplies
every separator and the quarantine root. Each element is a well-formed leaf by a total-function
alphabet (alphanumerics plus dot/dash/underscore, required to contain at least one alphanumeric, so a
dots-only element like `..` is **not a valid leaf at all**). A dot *inside* a leaf is fine
(`button.ts`); the extension is not special, just bytes in the name. Because no element can *be* a
climb and the box never supplies a separator, **traversal is unsayable, not caught** — no "is this
traversal?" branch anywhere, only "is this a well-formed leaf," which is a lookup. Subdirectories are
fully supported: depth is array length, a count the proxy joins, never a path the box authored. The
asymmetry is the whole property: **the bytes are written verbatim and never touched** (they cannot
hurt a folder), **the path elements are always validated** (they are the only thing that could escape
one). The proxy also owns a sane **element-count bound** and a **require-at-least-one** —
well-formedness of the frame, the same category as the length field having to be sane, not a security
branch reasoning about intent.

**The quarantine must be inert ground.** A non-interpreting deposit is only safe if *where* it lands
is also non-interpreting. If the box could write into a directory the trusted side later executes,
sources, or auto-loads — anything read as code or config — then "just depositing bytes" silently
becomes "writing the trusted host's behavior," and the deposit *is* the breach, with no interpretation
required. So the quarantine is ground nothing on the trusted side ever runs, sources, or loads; the
bytes sit there inertly until a **human** promotes them. The write path is non-interpreting *and* it
lands in ground that is itself non-interpreting — both halves are required, and neither alone is
sufficient.

**Moving the bytes is safe; running them is not — and those are different claims.** Promotion —
crossing the artifact into somewhere with capabilities — is the separate, structural human-in-the-loop,
and it is **out of scope here** (§3). The transport's job ends when the bytes are inert in the
quarantine; that folder is the entire interface between this document and promotion.

### 4.4 Reads onto the box — two cases, one principle

Getting bytes *onto* the box is the safe direction (§1.1), but it splits by *where the bytes are read
from*, and the split is the same trusted-ground question that runs through everything.

**Case 1 — reading from the external world (the internet): not a protocol feature.** The box reads
web content, package indices, a Git repo, by **fetching from a whitelisted endpoint over its existing
egress.** This earns no place in the spec, because the interpretation happens on **untrusted ground** —
a fully-featured client (curl, a Git client) is pointed at the box itself, where execution is already
assumed and harmless. The box may fetch and run whatever is there; a hostile box will fetch the latest
regardless of intent; the box may catch a virus in the process. **None of this is a security event** —
a virus on the box is a virus in a room with no doors. The only controls are the ones egress already
requires, applied identically no matter what is fetched: **effective whitelisting, and a hosting
destination that cannot escalate to execution** (the query-parameter example shows a `GET` verb is not
proof of read-only; curation is per-endpoint and human). This folded into the one high-risk surface
the system already curates; it added nothing new.

**Case 2 — reading documents from the trusted host: a dumb read service on the transport.** When the
operator wants to make material on the *trusted host* readable to the box, the naïve answer is to run
a web server (nginx) serving those files — but that puts a **full request-interpreter on trusted
ground**, an arbitrary-code-execution surface pointed at the host whose integrity is the whole game.
That is the liability the design refuses everywhere else, so it is refused here too. Instead the
transport grows a **read service**: the read-mirror of extraction. The box opens a read stream naming a
**destination** (the same array-of-leaves addressing); the proxy opens that one file and streams its
bytes to the box. It interprets nothing — no HTTP, no headers, no routing, no config, no modules, no
CVEs — it is "open one file, stream bytes," as dumb as the write channel with the direction flipped.
This is the symmetric law in action: **trusted ground never runs a rich interpreter; the box does.**

**The read service's path-confinement is addressing, not a security boundary — and this is the
direction-flip that matters.** On the *write* path, leaf-confinement was load-bearing: traversal could
escape the quarantine and a stray write could corrupt trusted state. On the *read* path it is merely
how the box names which file it wants, because **reading is harmless regardless of what is read.** We
are not hiding information from the box — there is no secret the box benefits from, since secrets only
matter when *used* and use is exactly what the box structurally cannot do. So the read service could in
principle expose the entire readable filesystem without a security problem; the addressing scheme is
for sanity, not safety. The disclosure boundary that would matter for a normal system is a non-concern
here, which is why the read channel is even simpler than the write channel it mirrors.

Three consequences worth stating plainly:

- **Box-core updates are a read, not a new mechanism.** When a human-authored fix must reach a
  now-adversarial box, the human places it where the box can read it — a vetted Git repo (Case 1) or a
  file on the trusted host (Case 2) — and the box pulls it. The crucial property either way: the fix
  reaches the box by the box **reading from a source**, never by anything **pushing into** the box —
  which preserves the directionally-clean link and keeps the human edit on trusted ground until the box
  reads it. The box editing its own core is likewise fine (untrusted ground); both are non-events.
  (Scope an external box-core source tightly, since the box runs whatever is there — acceptable because
  the box runs untrusted code anyway, but do not overload that egress entry with anything where
  source-compromise would matter for another reason.)
- **Proxy-core changes flow through extraction + promotion, like any artifact.** The box may *propose* a
  new proxy version as an extracted artifact a human reviews and promotes — it proposes, it never writes.
  This is possible but deliberately infrequent and unofficial, because the proxy is meant to stay frozen.
  The box can never write the proxy's code, config, or allowlist by any path.
- **Bootstrap is an out-of-band operator step.** The first box core and the proxy core are both laid down
  by the operator directly, at provisioning time, before the zone is adversarial and before the channel
  exists. Neither core is installed through the channel it implements — the box core because the transport
  cannot run before the thing that speaks it exists, the proxy core because it is the TCB and the box must
  never touch it. After bring-up, the box core may be freely re-edited or re-read; the proxy core only
  ever changes by deliberate operator action on trusted ground.

---

## 5. Cross-cutting design laws

The invariants that keep the whole thing coherent. A future change that violates one is wrong even if
it looks locally reasonable.

1. **Byte-creation on trusted ground is the highest-risk surface; sort everything by it.** A write
   onto a trusted host (proxy or user machine) must make execution *structurally impossible*, never
   merely unlikely. A write into the zero-trust box is a non-event — its health is not a security
   concern. The line is *where bytes land*, not what they are. Writing into trust is the direction that
   gets hardened; reading into the box never does.
2. **Never let the trusted side interpret box-controlled input as an instruction.** The renderer
   draws, it does not emulate. The write path deposits, it does not execute. The only box-controlled
   input the trusted side acts on (a file destination) is constrained so it cannot express anything but
   an inert placement. Off-the-shelf transfer tools are rejected here because they interpret — that is
   what makes them convenient and what makes them unsafe writing into trust.
3. **The proxy's interpretation surface is frozen.** What the proxy parses and enforces — the
   envelope, the destination confinement, the byte-map, the egress match — is specified once and
   stays small. Growing it is a deliberate spec revision, never routine creep. The fixedness of
   *that* set is the posture.
4. **What the proxy relays opaquely is free to evolve client-side.** Pass-through content (style
   descriptors, glyph payloads) can grow in a future spec with zero proxy change, because the proxy
   never read it — evolving it is one branch in the head's renderer. Cheap, head-only growth lives
   in the pass-through zone; expensive, deliberate growth touches the interpretation surface. Know
   which layer a change lands in before making it.
5. **Display state only, never capability.** No display frame may make the client act.
6. **Out-of-band the control; keep data pure data.** Color is a struct field, not an escape. Resize is
   a typed message, not an escape. The file destination is an array of leaves, not a path string.
   In-band signaling is the original sin in every direction.
7. **The thing that moves bytes never holds the secret; the thing that holds the secret never
   interprets content.** The egress guard and the stream relay stay separate even on one host. The base
   transport moves bytes and knows no service; each service interprets only its own.
8. **The base is dumb; services layer on top.** Find the commonality once (the transport), add a
   per-service implementation above it. Three services (pane, file-write, file-read), not a framework.
   The base never grows opinions about what it carries.
9. **Make it impossible, not caught.** Neutralization is a total function over a finite domain; path
   traversal is unsayable, not filtered. Never a denylist betting on grammar completeness.
10. **Hold no state you do not need.** The proxy is a pure relay; a pane is a stream you receive or do
    not. Nothing registered, nothing enumerated, nothing to go wrong.
11. **Robustness only where it is free.** Build the renderer to be as robust as it can be at no cost to
    the trusted path — but never purchase render fidelity with state in the trusted path or a guarantee
    the box must keep. No egress and no escalation are the goals; fidelity is a courtesy.
12. **Two doors, no hallway.** The optional SSH-emulator path and the box-render path share chrome and
    nothing else.
13. **Multiplexing is not isolation.** Stream identity is routing with zero security weight; true
    separation is physical.
14. **Keep the kernel as a guard, never as an attacker-facing surface.** The kernel TCP/IP stack is a
    large, attacker-facing parser beneath the whole trusted base — remove it (kernel-bypass, a minimal
    userspace driver, a protocol derived from the link's real failure modes). The IOMMU is the opposite:
    a confinement the kernel provides that protects the trusted base — keep it. "Battle-tested" is not
    "small and verifiable"; trust the kernel where it bounds the adversary, not where it parses the
    adversary's bytes.
15. **Derive from the medium, do not trim a general protocol.** Every mechanism must answer a named
    failure mode of *this* link. A 1m point-to-point cable has no routing, no reordering, no congestion —
    so those mechanisms do not exist here. The one real failure (receiver overrun) gets the one minimal
    answer (per-stream stop-and-wait + per-packet ACK); throughput scales by running the dumb codepath in
    parallel, never by making it clever.

---

## 6. Decided / Excluded / Open

### Decided

The **trust-direction posture**: byte-creation on trusted ground (proxy or user machine) is the
highest-risk surface and must be made execution-impossible by construction; bytes into the zero-trust
box are non-events. This is the axis the whole system sorts on, and it is what justifies the
ingress/egress asymmetry as requirement-driven rather than theater. The frozen-interpretation-surface
posture (the proxy reads a small fixed set; pass-through content evolves client-side); risk
concentrated in the egress allowlist. The base-transport-plus-services architecture (three services —
pane, file-write, file-read — not a framework). **TCP and the kernel network stack rejected** as the
largest by-assumption surface in the trusted base: kernel-bypass on a dedicated **Intel X520-DA2** over
a 1m DAC, driven by a minimal userspace poll-mode driver built from the datasheet (learning from
`ixgbe` behavior, using none of its source), lighting up one RX/TX ring with the card otherwise dark. A
**bare protocol derived from the link's one real failure mode** (receiver overrun) — no routing, no
addressing, no reordering, no congestion control, because a point-to-point link has none of those
problems; **per-stream stop-and-wait with per-packet ACK**, sufficient by arithmetic (~200k frames/sec
per stream at microsecond RTT) with aggregate throughput scaling by **parallel instance**, not protocol
complexity; per-stream in-order delivery as a *requirement* (terminal state is serial); integrity
inherited from the NIC's hardware CRC; protocol bringup dissolved into the ACK, device-init separate and
below it; the kernel retired from the data path (boot-time PCI/IOMMU only, IOMMU kept as DMA
confinement). The frame carries a stream discriminator (demux, zero security weight) and payload. A
**16-byte, four-column style descriptor** (fg, bg, underline-color, attributes — the
complete modern styling set with reserved bytes and bits as deliberate growth space). Run-length-encoded
UTF-8 cell-runs with box-side width resolution. **Keyframe-only display** — one display frame type, with
cursor and dimensions folded into the keyframe; change-driven emission with periodic keyframes as a soft
convenience, not a guarantee. A **stateless proxy** that holds no display state and relays a stream to
whoever is attached; serial-console behavior on attach. The asymmetric input channel (text + resize,
unsanitized, separate from display for orderliness, not security). **File transfer as the write path
into trusted ground** — justified because writing onto a trusted host demands a non-interpreting channel
and no off-the-shelf tool qualifies (rsync/scp/upload-servers rejected because they interpret): the
transfer is its own stream, verbatim bytes, array-of-leaves destination with proxy-owned joins and
traversal unsayable by construction, landing in an **inert quarantine** nothing on the trusted side
executes. **Reads onto the box split by source**: reading from the internet uses whitelisted egress (not
a protocol feature — interpretation on untrusted ground); reading documents from the trusted host is a
**dumb read service** on the transport (the read-mirror of extraction — no interpreter on trusted
ground, path as addressing not a security boundary, because hiding bytes from the box is pointless).
Box-core fixes reach an adversarial box by the box *reading* from a source, never a push; proxy-core
changes go through extraction+promotion (propose, never write); bootstrap lays both cores down out of
band before the channel exists. The transport-agnostic socket to the head; the head as a pure renderer
owning layout, chrome, and all repaint heuristics; the optional SSH-to-servers path walled off.
Credential on the box; real TLS only on the cloud egress hop.

**SSH-as-transport for a remote head** is a preserved **affordance, not a component**: the proxy→head
transport is agnostic so that SSH (or any tunnel) can carry it, but the design builds and blesses no
tunneling stack of its own.

### Excluded from the frozen surface (re-admission is a deliberate spec revision)

- **Render deltas (intermediary partial-update frames).** Excluded for simplicity, which serves the
  posture: full keyframes are cheap, and keeping partial updates out of the surface means no future code
  handling redraw cases the original never contemplated. If ever re-admitted, note that within a stream
  per-stream stop-and-wait already delivers frames in order (so a delta needs no added sequencing) and a
  keyframe re-bases the grid; the case that would justify them — continuous full-screen redraw — is
  self-identifying under change-driven emission.
- **Separate cursor and resize display frames** — folded into the keyframe; a re-admission only if
  deltas are.
- **Structured signal injection on the input channel** (reliable Ctrl-C) — a plausible future revision,
  outside v1.
- **Embedded protocols** (Sixel/Kitty images, OSC-8 hyperlinks, OSC-52 clipboard, notifications) —
  excluded permanently as a class, because they are parsers and capability channels, not ways of
  drawing a cell.

### Shelved experiment

- **NeoVim-class rendering as a target.** "Someday interesting," not a requirement. Because the box
  render is *headless* (no glass, no vsync, no scanout), the emulator may collapse intermediate redraws
  and emit only settled grids — a thing a real terminal structurally cannot do. That is a genuine
  mechanism for scrolling *smoother than a local PTY*; the floor is "no worse than NeoVim today," and the
  architecture has no way to make it slower. A downside-free experiment, pursued after the pipeline is
  proven on ordinary streams.

### Open — research still required

**How to drive the virtual PTY on the box.** The box-side client must allocate real PTYs and run a
headless emulator that resolves the escape grammar into a grid the protocol can serialize. Two paths:
**adopt an existing VT-emulator core** as the resolution engine, or **greenfield** one that emits our
cell-runs directly. The choice is **unconstrained by security** — this component is in the untrusted
zone, so its correctness is never load-bearing for containment and it may be as loose as convenient. It
is constrained instead by **correctness** (faithful resolution of real terminal output) and, *only if
the NeoVim experiment is later pursued*, by **throughput** — the headless emulator's speed is the one
place box-side performance becomes user-visible, gating how fast keyframes can be produced under a
redraw storm. Loose on security there; not necessarily loose on speed if the smoothness win is wanted.

*(Candidate emulator cores exist as starting points for that evaluation; they are prompts for the
research, not a settled choice.)*

**How small the driver and protocol can actually be on the X520.** §4.1 commits the *posture* —
kernel-bypass, bare protocol derived from the link's failure modes, one RX/TX ring, card mostly dark —
but the register-level specifics are active hands-on hardware research. The open items: the **minimal
device-bringup sequence** (the exact datasheet register order to bring up PHY, link, and DMA rings —
small but exacting, the part where "minimal" most fights "correct"); the **precise primitive set** (the
fewest card features that yield a working bidirectional link, and specifically whether to stay
single-ring with a frame-level stream discriminator or use hardware multiqueue — the former keeps the
register surface smallest, the latter would let hardware separate streams, and which is the smaller
*total* surface is exactly what the deep dive decides); the **ring-level rate-matching detail** (whether
a polling receiver can ever be outrun at these rates, and if so whether per-packet ACK or a credit hint
is the cleaner overrun prevention); and **how few lines the protocol can ultimately be**. The goal of
that dive is to minimize the one trusted-base surface that is neither tiny-and-ours nor static-firmware:
our own driver-plus-protocol code, built from the datasheet rather than from the kernel driver's source.

---

*The machine moves bytes and renders them inert; the human judges the one thing that does not transfer.
The console is built so that the only judgment it demands of you sits at the egress allowlist — small,
explicit, and yours — and so that an agent's work crosses into the world only through a quarantine a
human has read. Everything else is a frozen surface, a dumb relay, and a stream you pick up or you do
not.*
