# Containment by Construction
### An Ethos and Reference Architecture for Directing Untrusted Agents

> A foundational design document. It is being handed from a high-trust context toward
> building the zero-trust one — which is the right direction of travel, and worth doing
> while the work of an agent is still legible enough to review.

---

## 0. Premise

The regulatory layer for this technology is not a set of model guardrails. It is the
established legal and justice system, and the unit of accountability is the human being —
the citizen — who directs the system. Guardrails are a transitional artifact; they are
going away. What remains, and what is durable, is responsibility: **you are answerable for
the executed actions of your systems.** The model produces information; you are the one who
turns information into action in the world, and that act is yours.

This reframes the entire engineering problem. The agent is not executing on its own behalf.
It is executing on yours. So the question is never "can we trust the agent" — it is "can we
discharge our responsibility for what gets executed *provably*, by construction, rather than
*hopefully*, by trusting the model's restraint." Everything below follows from choosing the
first.

We are in a golden period. Its value is **not** that today's systems are trustworthy. It is
that their failures are still **legible** — a system this capable cannot yet make a
catastrophe non-obvious, so if it fails you can still see the edge by looking. That property
erodes as capability grows. This document exists to encode a posture that does not depend on
it.

---

## 1. The Ethos

These are the load-bearing principles. The architecture is downstream of them; if a future
decision conflicts with one of these, the decision is wrong, not the principle.

**Capability and trustworthiness diverge.** Almost every deployment pattern bets they move
together — grant more reach as the system gets better, auto-approve more, widen egress. The
bet is backwards. The thing that makes a system more capable is the same thing that makes its
mistakes less bounded, its intentions less legible, and its output more able to pass a review
it should not. The safe envelope must **narrow** as capability grows, not widen.

**Containment is structural, never behavioral.** Do not tie the guarantee to how the agent
behaves. Assume the worst possible occupant from the first line of design — fully capable,
fully hostile, fully motivated. A structural guarantee does not decay as the model changes
underneath it; a trust-based one has to re-earn its safety with every new model. This is why
a structural design ages well: it was never calibrated to a particular level of model
goodness, so it does not break when that goodness becomes a worse assumption.

**Make it impossible, do not make it caught.** This is the single most repeated move in the
whole design. Filtering or denylisting an adversarial input — "dangerous" bash, "dangerous"
escape sequences, "dangerous" anything — is unsound, because you are parsing an unbounded
grammar and betting you covered every case against an adversary who needs one gap. The
structural alternative, applied at every layer: remove the capability, define a channel in
which the bad thing cannot be expressed, or deny the channel its far end. You do not catch
the dangerous thing. You arrange for it to be unrepresentable or unreachable.

**Reason from anchors, not hopes.** Rest the design only on properties that are decidable and
checkable *right now* — which device is bound, which destination is permitted, what the byte
map outputs — never on a forecast of behavior. "Is this action illegal" is undecidable. "Does
this box hold a network device" is a lookup. Locate every guarantee on the second kind of
question.

**Terraces, not layers.** A stack of mitigations is a tower on sand; "defense in depth" is
the quiet admission that every layer leaks, so you add another, and the whole thing falls when
one course fails. This design is the inverse — a de-ratchet. Each push made the guarantee
*simpler and more physical*, deleting a class of problem rather than stacking a guard on top
of it. Terraces are cut into rock and bear on the ground. The test: every boundary has a
**locus** — one place it lives, defined once — not a **history** of accreted decisions nobody
made as a whole. Enterprise systems are insecure because they *evolved*. This one is
**designed**: one cohesive in-and-out surface, specified deliberately.

**Keep the trusted computing base small and verifiable.** The part that must be correct should
be small enough to hold in your head and audit in a sitting. Everything mutable, attackable,
or agent-controlled lives *outside* that core, arranged so that its failure cannot reach the
core. The criterion is not "no code" — putting bytes on a screen is always code. The criterion
is **no code whose correctness is a bet.** A total function over a finite domain (256 input
bytes, verified by enumeration) is allowed. A parser betting it handled every case of an
unbounded grammar is not. Finite-and-verifiable, yes; unbounded-and-hopeful, no.

**The machine does the verifiable; the human does the judgment.** Decompose every task along
this line. The machine moves bytes, renders them inertly, enforces a fixed set of invariants,
and runs code where running is harmless. The human judges **consequence** — the one thing that
is undecidable by machine and does not transfer. Remove the human from every loop where their
presence is lazy; keep them in exactly one place where their presence is structural.

**Decouple the agent's speed from the world's exposure.** Inside the contained zone the agent
runs at any speed, with no per-action approval, because nothing in there reaches anything. The
rate that matters is not the agent's internal rate; it is the rate at which its work *crosses
into a privileged environment*, and that is gated by review regardless of how fast the inside
spins. The intelligence explosion is allowed to happen in a room. What leaves the room leaves
at the speed of judgment.

---

## 2. Threat Model

**In scope.** A remote adversary arriving as untrusted *content* — prompt injection carried in
a web page, a document, a tool result, another agent's output — who thereby obtains full,
arbitrary code execution inside the box, with maximal capability and maximal motivation. We
assume the agent *is* the adversary and design so that this does not matter.

**Out of scope, handled by a locked door.** A physical-access adversary who can touch the
hardware or plant a receiver near it. Every emanation/side-channel exfiltration path
(software-modulated RF, magnetic, power-line, acoustic — GSMem, RAMBO, MAGNETO, PowerHammer
and their kin) is real and needs no added radio hardware, **but** every one is read-only and
requires a *cooperative receiver in physical proximity*, which a remote attacker does not
have. Planting a receiver is physical access; physical access was never something this
architecture claimed to defend, and once an adversary has it, emanation is the least of the
problem. The physical envelope is ordinary physical security: who can touch or get near the
hardware. (Note: a Faraday cage alone is insufficient anyway — magnetic channels penetrate it,
conducted channels ride the power and data wires through it, acoustic channels are not EM.
Control proximity, not just shielding.)

**The conjunction that makes it tractable.** Harmful effect on the outside world requires
**both** a capability to compute the effect **and** a channel to deliver it. Execution and
egress are a product, not two separate risks. Zero one term and the product is zero: an
interpreter with no channel can compute a perfect exploit and has nowhere to send it; a channel
with no interpreter can fetch and post but cannot build the thing worth sending.

**Bifurcation by capability.** Agents that touch the live internet hold **no interpreter**.
Agents that hold an interpreter touch **no internet**. The injectable agent (it reads untrusted
prose) has no execution to hijack; the execution agent has no live untrusted content reaching
it and no egress. Injection lands where there is nothing to run it; execution lives where there
is nothing to land. The agent that can execute is also the one whose inference should be local,
so that no free-form prompt channel leaves at all.

---

## 3. The Architecture

### 3.1 The Box — untrusted execution

- Dedicated, owned hardware: CPU, RAM, disk, and a **single** network interface that goes
  directly to the external boundary device and nowhere else. No commodity multi-tenant cloud
  host. No peripheral cards that could do anything; minimal hardware so the receive-side
  parser and the device surface stay small.
- Containment is **structural egress absence**, not a firewall rule the box could reconfigure.
  In a VM, this is *no network device bound into the guest's address space* — a packet has no
  device to travel through, so a fully-rooted guest with a complete TCP/IP stack reaches
  nothing. On bare metal, the single NIC's only physical peer is the boundary device. Either
  way: the box can construct arbitrary DNS, SSH, or any packets it likes, bit-banged byte by
  byte, and they die at the boundary because there is nowhere addressed for them to go.
- The box needs no DNS resolver; give it a static path or none. An agent writing its own
  resolver accomplishes nothing.
- **The box may hold anything, because it can do nothing with it.** A secret has value only when
  used somewhere, and every dangerous use is structurally unreachable. You could hand the box a
  live credential for a prominent target and it could do nothing but run inference on it or
  write it into a futile attempt to bit-bang its dead link. Therefore credentials may live on
  the box; minimizing what the box holds is an *operational* choice (rotation, provisioning),
  not a security one. That this is true is the proof the containment works.
- The box **builds, tests, and verifies internally.** It may stand up its own inner sandboxes,
  install toolchains and dependencies, run its own code freely — because running inside a zone
  with no egress and no reach is harmless. The box is the ideal place to *develop and exercise*
  code and the worst place to be *trusted* about the result.

> Open fork (human's call): full VM (Firecracker / rust-vmm) vs dedicated bare metal. The VM was
> scaffolding — the load-bearing guarantee is egress denial at the external boundary, not the
> hypervisor, and a hypervisor is itself a non-trivial trusted surface (device-emulation bugs,
> side channels). The one thing a fresh-VM-per-task buys that bare metal does not is cheap
> anti-persistence; a reused bare-metal box can be implanted below the OS and poison the next
> run. That is an *integrity-across-runs* concern, not an egress one — it does not touch the
> load-bearing claim — and is handled by reimaging / firmware reset / hardware rotation, or
> consciously accepted because the box is contained and holds nothing.

### 3.2 The Link — transport

- A **point-to-point** connection from the box's single NIC to the external boundary device: no
  switch, no shared segment, so the box has exactly **one** physical peer. The box may forge any
  L3/L4 it wants; the frames only physically traverse to the one device on the other end.
- The boundary device does **not route** the box's packets. It consumes the body as data and
  **originates its own** fixed upstream call. The box's network is a dead-end pipe into an
  application — which dissolves destination-filter correctness, DNS tunneling, and shared-CDN-IP
  ambiguity all at once, because nothing the box emits is ever forwarded.
- The only attacker-facing transport surface is the device's **receive side** — the thing that
  ingests the box's bits. Make it as dumb as possible. A raw byte stream (serial/UART-style) has
  a near-trivial receive parser and almost nothing to parse, versus a feature-rich NIC's stack
  (MAC, ARP, IP, TCP, reassembly), each layer a parser of attacker-shaped structure.

> Open fork (human's call): a genuinely dumb byte pipe (serial/UART, verified dumb all the way
> down to the silicon — not a USB-serial gadget that smuggles a USB stack back in), versus a
> serial-like virtual interface over Ethernet that constrains the alphabet, versus a single
> HTTP-driven link with custom framing on each end. All can satisfy the property; they differ in
> how little the receive side must parse.

### 3.3 The Proxy — the entire trusted computing base

A Linux host running a **single custom application** as the only listener on the interface. Not a
bespoke thin appliance: TCB line-count is not the metric, and a thin appliance forces
reimplementing the single biggest dependency (a TLS stack to reach the upstream API) in less
mature firmware — reintroducing exactly the commodity-firmware unreliability the design refuses.
Stock Linux, one app, no other listeners, is the right amount of paranoia. (A thin appliance only
earns its keep at the price of a *verified* stack, and a proxy bug fails the feature, not the
containment, so it does not warrant that cost — unlike the VMM boundary, if one is used.)

Split the proxy, conceptually, into two parts of wildly different size:

- **Feature logic** — demultiplex the box's stream, key/index IO by agent, buffer, frame, log,
  route the right output to the right place, broker the upstream API call. All of this is complex,
  all of it parses attacker-controlled input, and **all of it is allowed to be buggy**, because
  none of it is load-bearing for security. A bug here fails the *feature* (you see garbage, the
  wrong agent's stream, a dropped frame), never the *containment*.
- **Security invariants** — a tiny, fixed set enforced by dumb, unchanging code paths that do not
  depend on correctly parsing anything:
  1. The only upstream destination is a **hardcoded host and path**. Allowlist the proxy's *own*
     fixed endpoint and have it originate the upstream call to a hardcoded target — do **not**
     allowlist the cloud API's rotating, shared-CDN IPs, which would simultaneously allow every
     co-tenant behind that CDN.
  2. The only thing forwarded upstream is the box's **request body**, to a party that does **not**
     relay it onward and does **not** execute server-side tools (tools travel in the payload and
     return to hardware we control for execution). Because the destination cannot relay and cannot
     execute, the free-form-content exfiltration hole closes by **constraining the destination,
     not the content** — so no body inspection, no TLS interception, is required.
  3. The egress allowlist is **curated by a human**. No arbitrary endpoint joins it until a person
     has reviewed and agreed to the safety of that destination.
  4. The credential is attached by the proxy *or* lives on the box — either is fine, since the box
     cannot misuse it; choose on operational grounds.
- **Keying/indexing of agents is pure multiplexing, not isolation.** Isolation between untrusted
  agents is something we expect to be foiled, so keying carries **zero** security weight; a keying
  bug is cosmetic. If a workload truly must not be seen by another, that is solved by **physical
  separation** — a separate box on a separate dead-end link routed to a separate view — never by
  in-environment isolation.

### 3.4 The Observation & Control channel — human ↔ box

This is the one channel that cannot be closed: it is required and inherently bidirectional. So
constrain its **shape**, never filter its **content**.

- **Root flaw of terminals: in-band signaling.** A terminal stream intermixes display-data,
  control-commands, and *query responses* in one byte stream and demuxes by parsing. That ambiguity
  is unsound to resolve. Worse, a PTY **reflects**: the terminal answers the box's queries (cursor
  position, device attributes, clipboard via OSC 52) by injecting the answer into the input stream,
  as if typed. So "I am only viewing it" is false — on a terminal, viewing is interpreting, and the
  thing interpreted is adversary-controlled. (This is a 60-year vulnerability class with fresh CVEs
  every year; resting on a bug-free emulator is the unsound-parser bet again.)
- **Structural fix: split the three needs into separate unidirectional data channels.** Observing,
  instructing, and receiving updates are each one-directional, and none requires an interpreted PTY.
  - *Observe + receive-updates* = a read-only output stream. The box's output is carried as typed
    data (length-prefixed fields / structured frames), decoded by the receiver, and drawn as glyphs.
  - *Instruct* = a write-only input the box's output never travels on — which kills reflection,
    because the display path has no return wire.
- **The display application is a renderer, not an emulator.** It has a font, a grid, a cursor it
  alone advances, and **zero** sequence interpretation. Drawing a byte is not executing it. The
  entire vulnerability class exists only because real terminals decide certain output bytes are
  commands; make that decision **nowhere** in the code and the class is uninstantiable, not filtered.
  The failure mode under a novel attack is the tell: a filter fails open to anything its author did
  not anticipate; a renderer has no "anticipate" step, so an unknown byte sequence is the same as a
  known one — bytes, drawn as glyphs, inert by construction.
- **Neutralization is a total function on a finite domain**, verified by enumeration, not a filter
  betting on grammar completeness. Pass printable ASCII (0x20–0x7E); render **everything else** as a
  visible literal of its own bytes — every control introducer (ESC, the C0 set, the 8-bit C1 range,
  DEL) and every deceptive printable (bidirectional overrides, zero-width joiners, homoglyphs).
  "Printable ASCII is inert" is true of every terminal ever built — not a gamble on any particular
  emulator. The residual — exotic printables that *misrender* — is a *misread* risk (you might see a
  line that lies about itself), categorically smaller than execution or reflection, and the alphabet
  choice that handles it is an openly-trusted legibility decision sitting outside the security
  argument.
- **Color/formatting, if wanted, travels out of band** — a structured attribute in the protocol
  frame (`{text, fg}`), applied by the renderer as a property of the draw call — never an in-band
  escape that gets interpreted. Out-of-band the control; leave the data pure data.
- **Do the neutralization upstream**, on the proxy / in the TCB, so that only already-safe text
  crosses to the workstation. Then the workstation — tmux, Ghostty, anything — is *downstream of
  neutralization* and out of the security path, because it only ever receives inert glyphs. You do
  **not** need the workstation to be unsabotageable; you need nothing sabotaging to reach it. What you
  must never do is pipe the box's raw bytes into tmux/Ghostty as a program's stdout — then *that* is
  the emulator and you are back to square one. Raw agent bytes terminate at your decoder; only safe,
  structured content flows onward.
- **The trusted renderer owns the chrome.** Like Qubes' per-VM window decorations, the trusted side
  owns the window frame and any status line, so the agent's output is confined to its pane and cannot
  forge the trusted UI — it cannot, for instance, paint a fake "read-only mode" banner the way a real
  terminal-escape attack can.
- **"Just log it" does not save you.** Sequences lie dormant in a log file and detonate when the log
  is later rendered to *any* terminal. The vulnerability is in the rendering, wherever and whenever it
  happens; the invariant is "no terminal interprets these bytes at any point on the trusted side."
- **Recovery path: a physically attached keyboard and VGA console** as the hard fallback for when the
  soft channel breaks. A garbled feed is a *feature* failure, not a containment failure — and dropping
  to the hard channel when the soft one breaks is the tell that the boundary is in the right place.
- **Sizing:** the receiving side should be hundreds of lines, simple, exhaustively checkable. The
  transmitting side may be whatever it wants.

### 3.5 Code extraction & promotion — getting the work off the box

The whole point of the box is that agents produce arbitrarily executable code, and at some point we
want to use it. This is **not a new problem** — getting code out is getting bytes off the box to a
trusted place, which is the channel you already have.

- **Code is text. It rides the existing output stream** to the proxy as another message type. You do
  not need GitHub, SSH-with-keys, or any second outbound path. Adding a second egress mechanism
  reopens everything, because the whole guarantee was "exactly one place the bytes can go."
- **GitHub is not an intermediary; it is an over-featured egress destination that acts on content.**
  Pushing requires the box to hold a credential, open an authenticated session to a third party, and
  transmit arbitrary authored content to a service that runs Actions (remote code execution by
  design), fires webhooks (outbound requests by design), and exposes an API. Routing the box's output
  through it hands an injected agent a general-purpose egress-and-execution relay sitting behind the
  allowlist. It is normal only because nobody else assumed the thing pushing the code was hostile.
  Refuse it. If code should reach GitHub eventually, GitHub talks to the **proxy**, credential on the
  proxy; the box's only act is to emit bytes.
- **Extracted code lands in a quarantine on the trusted side.** Extraction itself is solved and
  trivial — it is the existing pipe. But the code is the **most attacker-shaped artifact in the entire
  system**, purpose-built to be the thing you will run.
- **Containment guaranteed the agent cannot act. It never guaranteed the agent's output is safe to
  run.** Those are different claims, and the second is false by default. Moving the bytes is safe — they
  cannot hurt the box, the proxy, or the viewer, because none of those interpret them as code. The
  moment you run that text somewhere with capabilities, *that* environment is exposed and gets no
  protection from anything above.
- **Promotion is the one structural human-in-the-loop.** "Is this code safe to run" is the
  undecidable-by-machine question; the only thing that crosses an artifact from quarantine to a
  privileged environment is a human having read and understood it. This cannot be skipped or automated,
  because the entire premise is that the author is untrusted. A **privileged environment is any
  environment outside the sandbox**, and a release is presented as a *candidate* from the development
  system for exactly this review.
- **The review/promotion tooling is a separate system** on the proxy side. Its job (help a human
  decide, then confer trust) is different from the transport's job (move bytes, interpret nothing), and
  fusing them would give the dumb pipe opinions about content — the first step of the accretion that
  kills enterprise systems. Keep them separate so each stays characterizable.
- **Watch the word "facilitate."** The gate must render the artifact faithfully, show the full diff,
  and make promotion explicit and effortful — but it must **not** score, summarize, flag-as-safe, or
  auto-promote on a green check, because the instant it does, the human stops reading and starts
  trusting a tool that is itself reading attacker-authored bytes. That is the confused deputy crawling
  back in through reviewer convenience. **Build the tool to make you a better reader; never to read on
  your behalf.**

---

## 4. Why It Holds

The whole system reduces to a short list of verifiable facts and one irreducible human judgment:

- The box's device table binds **no egress**, so a maximally-capable, fully-injected, rooted occupant
  cannot get a byte out. Every residue — device-emulation bugs, microarchitectural side channels,
  silicon flaws — is a *read* or *escape* primitive, and the design arranges that **none of them is
  also a *send* primitive**, because the egress-absence lives at a boundary nothing in the chain can
  reach.
- The trusted side is small enough to read in a sitting: a Linux proxy whose security is a few fixed
  invariants plus a dumb reader plus an enumerable byte map, with all complex feature logic
  deliberately outside the security path.
- The viewing surface interprets nothing the agent chose, because raw bytes were made inert one hop
  upstream of it — so the renderer, and the workstation behind it, are out of the trusted base.
- The one place a human is load-bearing is the promotion of an artifact into a privileged environment,
  which is exactly the place judgment was always irreducibly theirs.

It does not break under more intelligence because **it never asked intelligence to be safe.** It asked
the boundary to be sound and kept the one judgment that does not transfer on the human side of it. The
generator's trustworthiness is not a load-bearing input anywhere. This is the engineering form of the
executor's responsibility: you discharge "we are responsible for execution" *provably*, by building an
environment where illegal or harmful effects are unreachable, instead of *hopefully*, by trusting a
model's restraint. You do not make the agent trustworthy. You make it not matter whether it is.

---

## 5. The Human's Role

The forecast that software developers disappear measures the wrong quantity. It watches the volume of
code a human types fall toward zero and calls that the end of the profession — but the profession was
never the typing. It was the **comprehension** that made the code safe to ship, and that just became
the only part left, and the only part that grows in value as the machines grow in power.

The role inverts into something strictly harder than building was: to review systems produced by highly
capable agents, you must understand those systems *more deeply than the thing that built them*. This is
not a compliance posture — a compliance officer certifies against someone else's checklist and need not
understand the system. This requires understanding. The legal and safety guarantee of the whole edifice
rests on a human keeping their comprehension **ahead of an accelerating generator**, on the artifacts
that matter, often enough that the gap never opens on something consequential.

That is the genuinely hard, honest part: the gap between *generating* an artifact and *understanding* it
**widens** as capability grows — generation gets easier on the curve, comprehension gets harder on the
same curve — and the human stands in that gap. The temptation will be to let understanding lag, to trust
the green check, to accept the artifact you can *mostly* follow. The moment you do, you have silently
handed back the one thing that was irreducibly yours, not because anyone took it but because keeping it
got expensive. The discipline is not "always read everything"; it is keeping the **stakes assessment**
honest as the thing being assessed outgrows the ease of assessing it. (Merging an unread PR on a
genuinely low-stakes, known-blast-radius system is the correct calibration today. The danger is that
"low-stakes" gets harder to assess from the outside, and the glance that sufficed quietly stops
sufficing without announcing it.)

But this is a future worth wanting, not a treadmill, because the capacity being demanded — understanding
— is the one capacity that does not depreciate as the tools improve. It appreciates, because it is the
scarce input the whole system is bottlenecked on. The people who can keep their understanding ahead of an
accelerating generator are not a profession that is ending. They are the legal guardians of the
verifiable safety of these systems — and the only ones the whole structure cannot run without.

---

*The machine does what is verifiable. The human judges consequence. Build the structure so that trust is
needed in exactly one place, make that place small and explicit, and keep a human standing in it. That is
the difference between a system you designed and a system that happened to you.*
