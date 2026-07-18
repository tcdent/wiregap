# The Proxy — App-Composition Architecture

> **Slice-2 complete (2D coherence pass done, 2026-07-18); awaiting the full G1–G6 gate**
> of sprint `docs/teams/proxy/001-app-composition-architecture.md`. All sections present:
> the taxonomy, the placement tables (accepted at the Slice-1 gate), the per-subsystem
> placement prose, the grounded cross-cutting sections (state, observability,
> configuration), the open cross-cutting sections (lifecycle, process shape), and the
> consolidated open-questions ledger. Commit dependency: the designer's `terminal.md` §5
> per-vector fix (currently working-tree only) lands before or with this document.

---

## 0. What this document is

The single view that holds the entire proxy at once. The proxy's design lives in four
subsystem explorations — [`terminal.md`](./terminal.md), [`cloud/approach.md`](./cloud/approach.md),
[`cloud/anthropic-api.md`](./cloud/anthropic-api.md), [`filesystem.md`](./filesystem.md) — grounded
in the spine ([`principles.md`](../principles.md)). Each carries its own components and its own
guarantee; none holds the composition. This document is that composition: every component, at
scrutiny-unit granularity, placed against the four-tier scrutiny taxonomy (§2), plus the
cross-cutting topics no subsystem doc owns.

It **composes and places; it settles nothing the sources left open.** Every open design
decision it touches is carried as open, attributed to its owner. Where a source and a
committed artifact could diverge, the committed reality is cited and the gap flagged — never
corrected in place. Companion committed architecture: [`nic/architecture.md`](../nic/architecture.md)
(the NIC/transport subsystem's own composition, which this document cites and does not
re-specify).

## 1. The shape of the proxy

The proxy is the entire trusted computing base, and its guarantee is reviewability: small
enough for the operator to hold in a single sitting (`principles.md` §15). It is one custom
application on a stock Linux host, the only listener on the interface — not a thin appliance
(`containment-by-construction.md` §3.3). It fronts three channels between the box and
everything else: cloud egress, filesystem transfer, and the terminal.

One rule organizes all three (`console-and-protocol-design.md` §1.1): **bytes landing on
trusted ground are the dangerous direction; bytes landing on the box are free.** Applied per
subsystem, the whole proxy has exactly two gated directions — and a third channel with no
gate at all, its guarantee held by design properties instead:

- **Cloud** — egress is gated (the route recognizers, §3 tier 1); the response is a free
  opaque relay onto untrusted ground (`cloud/approach.md` §5).
- **Filesystem** — the write onto trusted ground is gated (the leaf-validator and its two
  structural floors, §3 tier 1); the read into the box is addressed, not gated
  (`filesystem.md` §0, §3).
- **Terminal** — no proxy-side gate at all. The guarantee is designed, not checked: the wire
  cannot represent harm, and the head never interprets (`terminal.md` §2). The proxy's
  terminal code is a memory-safe relay.

Everything else in the proxy is memory-safe plumbing over box bytes (tier 2) or feature
logic off the security surface entirely (tier 3). That asymmetry — a tiny invariant core, a
free ingress direction, and bulk that is allowed to be buggy — is the posture working as
designed, not an imbalance to fix (`containment-by-construction.md` §3.3).

## 2. The scrutiny taxonomy

Stated once, for the whole proxy; origin: `terminal.md` §5. The organizing rule: **the
trusted base carries no hope-and-pray member** — nothing is trusted merely because it was
examined and judged probably benign. Every component sorts by *what it can do with box
bytes* and *how its safety is assured* — never by where its code runs:

1. **Security-invariant core** — box-reachable code, or a design property, that decides
   *what crosses* or *how it is represented*. Total functions reviewed by enumeration, held
   to the reviewability budget (`principles.md` §15–§16); the highest scrutiny.
2. **Box-reachable supporting code (small-and-ours)** — parses box bytes but does not gate.
   Scrutinized for memory safety (no privilege escalation), reviewed by reading — not for
   security logic.
3. **Off-surface feature logic** — handles only already-safe bytes. A bug fails the
   *feature*, never containment.
4. **Untrusted-contained** — components whose trustworthiness is made immaterial: depended
   on to *function*, never to *behave* (`principles.md` §4–§5, applied to devices as well as
   the box). Scrutiny goes to the containment — and, where a vector is not contained, to an
   honest account of what is *owned* instead.

The taxonomy has a single sorting axis (design ruling, 2026-07-18). Components whose
adversary is not the box — upstream-facing code, operator-input-only code — are not given
new tiers; each row's placement note names its **adversary class** (box / upstream / device
/ none) as an attribute, and the two edges that attribute exposes are carried in §4–§5
(the upstream TLS stack — since ruled by policy, §11 L10; the config loader — still open).

## 3. The placement — the whole proxy against the taxonomy

### Tier 1 — Security-invariant core

*"Box-reachable code (or a design property) that decides what crosses or how it is represented"* (`terminal.md` §5.1).

| Component | Subsystem | Tier | Capability over box bytes | Grounding | Placement note |
|---|---|---|---|---|---|
| Route matcher (register enforcement) | cloud | 1 | Reads the box-supplied address, matches it against the operator's route register; no match → whole request denied; never widened at runtime (no redirects, no box-influenced destination) | `cloud/approach.md` §2, §6, §8; `principles.md` §14, §17 | — |
| Per-route request recognizers (inert-route class) | cloud | 1 | Recognizes a request against the route's rule (e.g. "an allowlisted GET"), exact match; forwards verbatim only what is fully recognized, denies the rest | `cloud/approach.md` §2–§3, §8 | — |
| Anthropic request recognizer | cloud | 1 | Full recursive recognizer over the box-authored body; admits only inert client-side shapes (custom tools untyped; `type`-tagged only on exact vetted client-side identity); everything else fails closed | `cloud/anthropic-api.md` §6, §8; `cloud/approach.md` §3 | SOURCE-OPEN ×2 (`anthropic-api.md` §8): placement (in-transport module vs isolated service) and the exact recognized shape — both owned by the subsystem design effort, carried open here. |
| Safe address representation | cloud | 1 | Defines the only representation box-originated bytes are ever kept in (matched, logged); proxy supplies the structure (`://`, `.`, `/`), a malicious character is unrepresentable | `cloud/approach.md` §7 | Boundary case, defended: tier 1's own text covers what "decides … *how it is represented*" (`terminal.md` §5.1). This unit does not merely parse box bytes memory-safely (tier 2); it defines the representable alphabet for the only box bytes the proxy keeps. Adversary class: box. |
| FS write-path leaf-validator | filesystem | 1 | Total function over leaf characters (`[A-Za-z0-9._-]`, ≥1 alphanumeric → traversal unsayable); includes the element-count bound and ≥1-element rule (frame well-formedness); proxy owns every separator | `filesystem.md` §1, §4; `terminal.md` §5.1 | — |
| FS write-path open-discipline floor | filesystem | 1 (design property) | Confines where the gated write can *land* regardless of path content — symlink escape unreachable, not caught | `filesystem.md` §1 | Boundary case, defended: the second floor of the write confinement — it decides what crosses at the filesystem level, and tier 1 admits design properties explicitly. SOURCE-OPEN: requirement firm, primitives (`openat`, `O_NOFOLLOW`, `O_CREAT`+`O_EXCL`) pending design review. Adversary class: box (symlink-shaped filesystem state). |
| FS quarantine-inertness invariant | filesystem | 1 (design property) | Nothing on the trusted side executes in — or reads-and-interprets — quarantine: no indexer, loader, or watcher; deposit-only ground until a human promotes | `filesystem.md` §2 | Boundary case, defended: same shape as the head's no-interpreter invariant (an explicit tier-1 member): a maintained design property that landed box bytes are never interpreted on trusted ground. SOURCE-OPEN: enforcement mechanism (`noexec,nosuid,nodev` mount, no auto-indexing) pending design review. Adversary class: box (deposited content). |
| Wire-protocol inexpressiveness | terminal / wire | 1 (design property) | The wire carries display state and a deliberate control vocabulary and nothing else; escape/in-band command/reflected query are unrepresentable | `terminal.md` §0, §2, §5.1 | Owned by the protocol effort, not this subsystem. Committed artifact: the `proto` crate (structure-only validation; L2-only per proto sprint 010) — cited as reality, not re-specified here. |
| Head no-interpreter invariant ("renders, never interprets") | terminal | 1 (design property) | The head paints glyphs and emulates nothing; a future feature interpreting control on the wire would breach it | `terminal.md` §2–§3, §5.1 | Maintained design invariant of the head, reviewed as a property — not a runtime check, and not a proxy check. |

### Tier 2 — Box-reachable supporting code (small-and-ours)

*"Parses box bytes but does not gate … scrutinized for memory safety, reviewed by reading — not for security logic"* (`terminal.md` §5.2).

| Component | Subsystem | Tier | Capability over box bytes | Grounding | Placement note |
|---|---|---|---|---|---|
| Proxy terminal relay | terminal | 2 | Moves display frames box→head and input head→box; performs no check (the protocol already cannot express harm); must not be exploitable by box bytes | `terminal.md` §3, §5.2 | Per-session demux/keying inside the relay carries zero security weight — isolation between agents is physical separation, out of scope (`containment-by-construction.md` §3.3). |
| Wire-protocol parser (proxy side) | terminal / wire | 2 | Consumes box-emitted frames; structure-only validation (verb/shape/alphabet), no reserved-value policy | `terminal.md` §5.2 | Committed reality: the `proto` crate — the L2 message codec, one crate with feature-gated services (`terminal` display cell-stream, `file`, `net`, `control`); `no_std` + `forbid(unsafe_code)`, so the memory-safety bar is met by construction in safe Rust. The L1 session envelope is the nic-owned `transport` crate, a distinct layer (§4, "The wire, layered"). |
| Head frame parser | terminal | 2 | Consumes box bytes on the operator's machine → memory-safe, built to the bar | `terminal.md` §3, §5 (closing) | Location does not lower the bar: box-reachable code on the operator's own machine still sorts by capability, not location. |
| FS read service (addressing + streamer) | filesystem | 2 | Parses box-supplied leaf addressing, opens one file, streams its bytes; no HTTP, no routing; interpreter kept off trusted ground | `filesystem.md` §3 | Explicit source label: memory-safe but **not** security-invariant core; a logic bug fails *safe* (box reads some other readable file — harmless). Adversary class: box. |
| NIC driver — descriptor-validating / parsing code | nic | 2 | Reads device-written descriptor/buffer content as if from an adversary: bound the reported length, treat status as hostile, never deref an unchecked field, copy out of DMA at the earliest point | `terminal.md` §5.2; `nic/architecture.md` §2.1 (vector 2), §3.1 | Adversary class: device (firmware-authored ring content) enclosing box-authored frame bytes. Committed `driver` crate. The pure-direct control path (fail-closed device-identity gate before any state-changing op) is part of this small-and-ours surface (`nic/architecture.md` §2.1; `directpath` crate). |
| NIC transport protocol + reactor | nic | 2 | Session/ACK/retransmit protocol and the single-threaded event loop; parses wire bytes downstream of the driver's copy boundary | `nic/architecture.md` §2.1 ("the driver and protocol" — small-and-ours), §3, §6–§7 | Committed `transport` + `reactor` crates (safe Rust, `no_std` logic). The `transport` crate is the L1 layer under the `proto` L2 codec (§4, "The wire, layered"). |

### Tier 3 — Off-surface feature logic

*"Handles only already-safe bytes … a bug fails the feature, never containment"* (`terminal.md` §5.3).

| Component | Subsystem | Tier | Capability over box bytes | Grounding | Placement note |
|---|---|---|---|---|---|
| Cloud originator | cloud | 3 | Encapsulates an already-admitted request verbatim inside the proxy's own authenticated TLS; owns transport, credential, and the upstream call; zero signing code | `terminal.md` §5.3; `cloud/approach.md` §1; `containment-by-construction.md` §3.2–§3.3 | Adversary class: none on input (receives only what the tier-1 gate admitted). Credential attachment is operational (`containment-by-construction.md` §3.3). |
| Upstream TLS stack (library dependency) | cloud | 3 | None — parses upstream-authored bytes on trusted ground, both directions | `containment-by-construction.md` §3.3 (cite-through) | Adversary class: **upstream** — off the box-byte axis entirely; sharpest instance of the adversary-class attribute (design ruling 2026-07-18: attribute note, no new tier). RESOLVED (zero.policy, 2026-07-18): outside the box-reachable surface and outside the §15/§16 budget — box bytes reach TLS only as opaque plaintext to encrypt; its parser sees only the cloud's records (§4, cloud walk; §11, L10). |
| Cloud response relay | cloud | 3 | Relays the upstream response opaquely to the box; never inspects, neutralizes, modifies, or sanitizes | `cloud/approach.md` §5 | Adversary class: upstream (framing); box-influenced content rides opaque inside and is never parsed. Safe direction — the payload lands on untrusted ground, so a relay bug fails the feature; hardening this direction would re-import the confidentiality worldview the spine discards. |
| Quarantine writer | filesystem | 3 | Writes verbatim bytes at the gated destination (at an offset for chunked transfers); interprets nothing | `terminal.md` §5.3; `filesystem.md` §1, §4 | Adversary class: box — but the bytes are already-safe when they arrive: the destination is confined by the two tier-1 floors, and writing interprets nothing. Explicit tier-3 source label (`terminal.md` §5.3). |
| Head renderer / pane multiplexer | terminal | 3 | Renders the grid, multiplexes panes, captures input — the tmux replacement | `terminal.md` §3, §5.3 | A bug is a misread or a crash, never an interpretation (`terminal.md` §3). |
| Input path (head → box) | terminal | 3 | Operator keystrokes and resize flow onto untrusted ground, well-formed but unsanitized — the box may be sent anything | `terminal.md` §4 | Adversary class: none (operator-authored). Off the box-reachable surface entirely; no reflection — channels split and one-directional, no query frame. |
| Audit log writer | cloud (cross-cutting) | 3 | Writes the audit trail: "request to [safe address] — allowed or denied, and why"; draws only from the safe representation, so no box payload can reach a log line | `cloud/approach.md` §7 | Adversary class: none — the dangerous content is structurally absent from the representation logs are drawn from. The log *viewing* path is a cross-cutting mechanics question (§11, L13), not settled here. |
| Config / route-register loader | cross-cutting | 3 (provisional) | None — loads operator-authored configuration: the route register (`approach.md` §6), quarantine root and readable root (`filesystem.md` §4); the box can author no rule, and config is never sourced from quarantine (`filesystem.md` §2) | `cloud/approach.md` §6; `filesystem.md` §2, §4; `principles.md` §14 | **Open placement, provisionally tier 3 by elimination** (adversary class: none → not box-reachable tiers 1–2; not untrusted-contained). Tier 3's severity clause is *not* satisfiable from source: a register-load defect alters the tier-1 gate's input, so "fails the feature, never containment" does not hold. Cannot be fully defended from source → carried as an open question per the sprint method, not coin-flipped. |

### Tier 4 — Untrusted-contained

*"Components whose trustworthiness is made immaterial: we depend on them to function, never to behave"* (`terminal.md` §5.4).

| Component | Subsystem | Tier | Capability over box bytes | Grounding | Placement note |
|---|---|---|---|---|---|
| The box | — | 4 | Authors every byte the proxy gates; internal freedom total; nothing it does inside reaches outside | `terminal.md` §5.4; `principles.md` §4–§6, §18–§19; `containment-by-construction.md` §3.1 | Contained by structural egress-absence; exactly one physical peer on a dead-end link (`containment-by-construction.md` §3.2). |
| Box-side PTY emulator | terminal | 4 | Runs legacy apps against an emulated PTY; resolves their output; emits the wire data structure | `terminal.md` §1, §3 | Scrutiny **none** per source: a compromise is a non-event — it can only emit (mis)representations of a screen, which cannot act. No containment of its own; subsumed within the box's boundary. |
| X520 NIC + firmware | nic | 4 (per-vector: contained + owned) | Bulky vendor code we cannot read, driven directly from userspace; presents descriptor/buffer content to the driver and DMAs into host memory | `terminal.md` §5.4 (current, 2026-07-18); `nic/architecture.md` §2.1 | **Both labels, per vector — deliberately not collapsed** (a single containment label would recreate the trusted-by-diligence rung). **Content vector: CONTAINED** — by the tier-2 descriptor-validating driver (bound length, status hostile, copy out early), hardware CRC + retransmit for corruption, the fail-closed device-identity gate, and the physical point-to-point link as the one remaining structural-hardware floor. **DMA-reach vector: OWNED, not contained** — pure-direct, **no IOMMU**; nothing hardware-bounds where the device can DMA; the ratified posture owns the residual rather than routing containment through the kernel trust the spine excludes. SOURCE-OPEN: whether the threat model tolerates un-contained firmware DMA on the trusted side is the policy layer's call, audited after the PoC (`nic/architecture.md` §2.1; `nic/concerns-to-watch.md`). |

### Unplaced — surfaced, not settled

| Component | Subsystem | Tier | Capability over box bytes | Grounding | Placement note |
|---|---|---|---|---|---|
| Linux kernel (proxy host) | cross-cutting | — (open) | None directly; underlies every non-NIC syscall the proxy makes | `principles.md` §16; `filesystem.md` §2; `containment-by-construction.md` §3.3 | No tier's definition fits from source: excluded from the reviewable trusted path (`principles.md` §16, and the NIC link bypasses it entirely), yet the intended enforcement substrate for the quarantine-inertness floor ("kernel-enforced floor", `filesystem.md` §2) and the stock host under the one-app proxy (`containment-by-construction.md` §3.3). Not authored box-byte code, not feature logic whose bugs are containment-neutral, not structurally contained, not in the reviewable core. Placement is design/policy's call — surfaced here, not settled. |
| Promotion tooling | — | — (boundary) | Reads the quarantine, shows a human the diff, crosses artifacts into privileged ground | `filesystem.md` §4 | Out of scope for the proxy per source and sprint contract — noted as the downstream structural human-in-the-loop boundary, not placed. |

## 4. The subsystems against the tiers

### Cloud

The cloud subsystem is the uniform three-step shape for every upstream destination: **match**
the route against the operator's register, **recognize** the request against that route's
rule, **forward** it verbatim inside the proxy's own TLS — or **deny** it
(`cloud/approach.md` §8). All three deciding steps are tier 1, and deliberately so: because
the bytes ride through unchanged, there is no emit-side backstop — a recognizer that is too
permissive *forwards* what it missed, so the recognizer **is** the containment
(`cloud/approach.md` §3). What varies per route is only the recognizer's weight: near
nothing for an inert allowlisted GET, a full recursive body recognizer for the Anthropic
route, the worst case the whole approach is built against (`cloud/anthropic-api.md` §0).
Both decisions that instance leaves open — where the Anthropic recognizer runs, and its
exact recognized shape — belong to the subsystem design effort (`cloud/anthropic-api.md` §8)
and stay open here.

Downstream of the gate, everything is off the security surface. The originator encapsulates
an already-admitted request in TLS the proxy owns; the response relays opaquely onto
untrusted ground, where interpretation is assumed and harmless (`cloud/approach.md` §5).
Neither performs a check, and that is the design: hardening the response direction would
grow trusted surface to defend a direction that needs no defense.

**A request is a trusted address and an untrusted payload** (`cloud/approach.md` §7) — a
representation principle that runs through the whole subsystem and deserves stating
outright. The address (scheme, domain, path) is the only box-originated data the proxy ever
*keeps*: it goes into a purpose-built safe representation (tier 1 — it decides how box bytes
are represented) in which a malicious character is unrepresentable, and that representation
is what gets matched against the route and written to the log. The payload (query, headers,
body) is never represented, never logged, never rendered: the proxy may inspect it in-memory
to make the allow/deny call, then reconstructs it into the outbound request, and that is the
payload's entire life on trusted ground. The audit consequence is structural: **the only
box-originated content that ever reaches a log is safe by construction, so a log cannot
detonate when rendered — there are no dangerous box bytes in it to detonate.** This is not a
gate and not a component; it is a classification the recognizer, the originator, and the
audit log writer each obey, which is why it appears distributed across their rows rather
than as a row of its own.

One honest tension was named at the gate and has since been ruled. The upstream TLS stack
is a **bulky library the operator cannot read, running on the trusted host** — on its face
the kind of code `principles.md` §16 excludes from the trusted path ("borrow vetted ideas;
exclude bulky code"). **Ruled (zero.policy, 2026-07-18, relayed via the PM): the TLS
library does not count against the §15/§16 reviewability budget.** The budget scopes to
the **box-reachable surface**, not to every byte in the trusted process — and the TLS
library sits outside that surface: box bytes reach it only as opaque plaintext to be
encrypted on the proxy→cloud hop, never as input to TLS's own parser, which only ever sees
the cloud's records. A TLS bug fails the upstream feature or leaks upstream content —
undefended by the spine (`principles.md` §7) — but never grants the box reachable
execution. Per the ruling's surface-and-budget grounding: **off-surface, out of budget** —
not a tier needing justification. (§11, L10, resolved; one re-confirmation note rides with
L11.)

### Filesystem

The filesystem subsystem is two structural floors and a character class (`filesystem.md`
Summary). The dangerous direction — box bytes landing on trusted ground — is confined
entirely at tier 1: textually by the leaf-validator (traversal unsayable: `..` cannot be
*said*), and at the filesystem level by the open-discipline floor (symlink escape
unreachable, primitives pending design review). Where the bytes land is then made inert by
the third tier-1 member, the quarantine-inertness invariant: nothing on the trusted side
executes in or auto-processes the quarantine, so a deposited `.bashrc` or systemd unit is
bytes, not behavior (mechanism pending design review). The writer itself is tier 3 — by the
time bytes reach it, both floors have already done the work, and writing verbatim interprets
nothing.

The read path is the direction-flip: leaf addressing there is addressing, not a security
boundary, and a logic bug fails safe — the box reads some other readable file, which is
harmless because the box may acquire any information (`principles.md` §7). It sits at
tier 2 for one reason only: it parses box input, so it owes memory safety. Its deeper
purpose is keeping an interpreter off trusted ground — one file opened, bytes streamed; no
HTTP server, no routing, nothing off-the-shelf that acts on box-supplied instructions
(`filesystem.md` §3).

### Terminal

The terminal subsystem is the proof that the taxonomy sorts by capability, not location —
and the one subsystem with **no proxy-side gate at all**. Its guarantee lives at two design
loci, both tier 1 and neither a runtime check: the wire protocol cannot represent control or
harm (the protocol effort's domain), and the head renders without ever interpreting
(`terminal.md` §2). Between those loci the proxy's own terminal code is tier-2 plumbing: a
relay that moves frames and performs no check, because there is no check to perform — the
harm is unrepresentable upstream of it. Legacy PTY jank never crosses the wire: the box-side
emulator converts a subset of PTY into the protocol, and what the protocol cannot represent
does not cross — not stripped, unrepresentable (`terminal.md` §1). The emulator itself needs
no scrutiny at all; a compromise there can only misdraw a screen, and sits inside the box's
containment anyway.

The head runs on the operator's own machine, and still splits across three tiers: its
no-interpreter invariant is tier 1, its frame parser — box-reachable code on trusted ground —
is tier 2 and built to the memory-safety bar, and its renderer/multiplexer is tier 3 feature
logic whose worst bug is a misread or a crash. The input direction is tier 3 by the
direction asymmetry: operator keystrokes flow onto untrusted ground unsanitized, and the
channels are split and one-directional with no query frame, so display content can never
loop back as if typed — the classic terminal reflection vulnerability designed out
(`terminal.md` §4).

### The wire, layered

Two committed crates carry the wire, at two different layers, and the distinction matters
when reading the tier-2 rows. The **`proto` crate is the L2 message codec** — the single
source of truth for the box↔proxy↔head codec, one crate with feature-gated services: the
terminal display cell-stream (`docs/proto/display.md`), file transfer, net egress, and
control. It validates structure only (verb, shape, alphabet) and is `no_std` +
`forbid(unsafe_code)` end to end. The **`transport` crate is the L1 session envelope** —
the nic subsystem's session protocol (session ID + per-frame ACK) that carries proto's L2
bytes opaquely and never inspects them. There is exactly one session-framing layer, and it
is L1: the boundary was reconciled at proto sprint 010, which excised proto's own L1-frame
`envelope.rs` (the forbidden second framing layer), leaving the transport's L1 frame as the
sole session framing and the `proto` crate a flat L2 message codec that owns no envelope
(`docs/proto/library.md` "Envelope ownership"; `docs/proto/model.md` law 1).
So "wire-protocol parser" (terminal / wire, tier 2) is the `proto` codec, and "NIC transport
protocol" (nic, tier 2) is the `transport` layer beneath it — two units, two crates, one
stack.

### NIC / transport

The NIC subsystem contributes the taxonomy's fourth tier in its sharpest form, and this
document cites its committed composition (`nic/architecture.md`) rather than re-deriving it.
Above the device, everything is small-and-ours tier 2: the descriptor-validating driver that
reads device-written content as if from an adversary and copies out of DMA at the earliest
point, and the session protocol + reactor that parse wire bytes downstream of that copy
boundary. Below them sits the X520 and its firmware — bulky vendor code, untrusted, its
hostile reach split **per vector** (`nic/architecture.md` §2.1): the *content* vector is
**contained** by the driver guard, hardware CRC + retransmit, the fail-closed
device-identity gate, and the physical point-to-point link — the one remaining
structural-hardware floor; the *DMA-reach* vector is **owned, not contained** — the
subsystem runs pure-direct with no IOMMU, so nothing hardware-bounds where the device can
DMA, and the ratified posture owns that residual rather than routing containment through
the kernel trust the spine excludes. Whether the threat model tolerates it is the policy
layer's call, audited after the PoC. The row carries both labels because collapsing them to
one would either overstate the containment (a trusted-by-diligence rung, the thing the
taxonomy exists to forbid) or erase the part that genuinely is contained.

## 5. Cross-cutting components

Three components belong to no single subsystem, and two of them are where the taxonomy's
single axis is honestly strained rather than silently stretched:

- **The audit log writer** (tier 3) is downstream of the §4 cloud classification: it draws
  only from the safe representation, so the dangerous content is structurally absent from
  everything it writes. How the operator *views* logs without creating an interpretation
  surface is an open mechanics question (§11, L13).
- **The config / route-register loader** (tier 3, provisional — open placement) touches no
  adversary bytes at all: it loads what the operator authored, the box can author no rule,
  and config is never sourced from the quarantine. But its correctness is
  containment-relevant — a register-load defect alters the tier-1 gate's input — so tier 3's
  "fails the feature, never containment" clause does not honestly hold. The placement is
  provisional by elimination and carried as an open question for design.
- **The Linux kernel under the proxy host** is unplaced, deliberately. The sources both
  exclude it from the reviewable trusted path (`principles.md` §16; the NIC link bypasses
  it entirely) and lean on it as the enforcement substrate for the quarantine floor
  ("kernel-enforced floor", `filesystem.md` §2). No tier's definition fits, and forcing one
  would launder the question. Its standing is design/policy's call, surfaced here.

## 6. State

The proxy's state discipline is derivable from the sources by composition — no subsystem
holds durable box-originated state, and the composition makes that a proxy-wide invariant
worth stating as such (derived here from the citations below, not new design):

> **The proxy's durable state is exactly the operator's authorization artifacts plus the
> audit trail. Box-originated data has no durable representation on trusted ground except
> the safe address in the log.**

The durable set, enumerated:

- **The route register** — the operator's standing-authorization register: destinations and
  their rules (`cloud/approach.md` §6; `principles.md` §14).
- **The filesystem roots** — the quarantine root and the readable root; explicitly "config"
  and explicitly the whole of the filesystem subsystem's state (`filesystem.md` §4).
- **The upstream credential** — held by the proxy (or on the box, an operational choice);
  revocable, and revocation is the whole answer to its theft (`cloud/approach.md` §1;
  `containment-by-construction.md` §3.3; `principles.md` §7).
- **The audit trail** — append-only output, drawn solely from the safe representation
  (`cloud/approach.md` §7; this document, §7).

Everything else is transient, bounded, and uninterpreted beyond its recognized structure:
in-flight request/response buffers (the payload's entire life on trusted ground is
in-memory inspection at most, then outbound reconstruction — `cloud/approach.md` §7);
file transfers ("transient and unattributed — a filesystem snapshot at a point in time,
not held" — `filesystem.md` §4); per-session protocol state in the transport layer
(session table, ACK/retransmit cursors — holding only currently-live sessions, with no
memory that a session ID was ever used: "the absence of cross-epoch state is the
mechanism" — `nic/architecture.md` §8, committed `transport` crate); and the DMA rings
below the copy boundary (`nic/architecture.md` §3.1).

Two structural absences complete the picture. The terminal path holds **no interpretation
state on the proxy**: the grid is resolved on the box (the emulator) and held at the head
(`terminal.md` §1, §3; `docs/proto/display.md`) — the relay in between keeps nothing but
the frame in flight, which is the console posture ("less for the proxy to interpret",
`console-and-protocol-design.md` §1.2) in state form. And there is **no box-attributed
history**: transfers are unattributed by source, and the log holds decisions about safe
addresses, not box content — so no accumulation of box bytes anywhere on trusted ground
that a later feature could be tempted to interpret.

## 7. Observability

The posture, grounded; the mechanics (where logs are written, how the operator views them,
metrics/health) are open and live in the §11 ledger.

**What is observed is the proxy's own decisions, never box content.** The audit trail is
*"request to [safe address] — allowed or denied, and why"* (`cloud/approach.md` §7). Its
input is structurally confined: the only box-originated content that can reach a log is
the address, which is safe by construction, because the payload never enters the
representation logs are drawn from (§4, cloud walk). No neutralization pass exists because
none is needed — the classification, not a filter, keeps logs inert.

**Refusal is the signal.** The proxy blocks in the open: a non-conforming request is denied
whole and the denial surfaced legibly, never silently fixed up (`cloud/approach.md` §3).
When a route's legitimate traffic drifts — a new tool version, a new path — the proxy fails
closed and the failure *names the unvetted thing*, which is precisely what the operator
reviews before deliberately widening the register (`cloud/approach.md` §6;
`cloud/anthropic-api.md` §6). A box reaching for forbidden capability is therefore visible
by design — the golden-period legibility the architecture is built to preserve
(`containment-by-construction.md` §0): the system's failures stay inspectable instead of
vanishing into silent conformance.

**What observability is not.** It is not payload capture and not data-movement monitoring:
the spine discards confidentiality as a defended harm (`principles.md` §7), so nothing
watches "what data left" — the log records effect-decisions. And it does not extend to
trusting what the box *displays*: the worst the box can do on the terminal path is show a
misleading screen, "a misread, openly accepted as legibility, not security"
(`terminal.md` §4). Observability of the box's behavior is the operator watching the
screen; observability of the *boundary* is the decision log. The two are different
instruments and only the second is load-bearing.

One firm constraint the mechanics must inherit (`filesystem.md` §2; `cloud/approach.md`
§7 states it as the reason the log classification exists): **box content is never rendered
on trusted ground** — a log viewer, indexer, or dashboard that interpreted box bytes would be the
detonation surface the representation discipline exists to make impossible. The open
questions — transport, storage, rotation, the safe viewing path, whether any of this
spends reviewability budget — are §11 ledger items, not settled here.

## 8. Configuration

What is configured is grounded; how configuration is loaded, validated, and reloaded is
open (§11 ledger). The proxy's configuration is, in substance, **the operator's authority
made data** (`principles.md` §14: standing approval by class):

- **The route register** — each route a destination plus the rules an emitted request must
  satisfy; the register is the curated set of standing authorizations, not a list of "safe"
  addresses (`cloud/approach.md` §6). Its first and most essential entry is the Anthropic
  route (`cloud/anthropic-api.md` §0). Route rules include configuration that drifts with
  upstream releases — the exact-string whitelist of vetted client-side tool identities —
  and every extension is an operator review after a legible fail-closed, the promotion
  pattern at the route-rule level (`cloud/anthropic-api.md` §6).
- **The filesystem roots** — the quarantine root (write target) and the readable root
  (read scope). The readable root's breadth is sanity, not safety — reading is harmless —
  and the quarantine's internal structure is an operational/provenance choice; the
  validator confines either way (`filesystem.md` §4–§5).
- **The upstream credential** — whose custody (proxy-attached vs on-box) is explicitly an
  operational decision, since the box cannot misuse what it cannot reach effect with
  (`containment-by-construction.md` §3.3; `cloud/anthropic-api.md` §8).

Three placement constraints on configuration are source-firm, whatever mechanism lands:

1. **The box authors none of it.** No destination the box can influence at runtime, no rule
   the box can author (`cloud/approach.md` §6) — configuration's adversary class is *none*,
   and keeping it so is a design constraint, not an accident.
2. **Configuration never lives in — and is never sourced from — the quarantine.** The
   proxy's own configuration and core live elsewhere; it never reads its instructions from
   ground the box can write (`filesystem.md` §2).
3. **The proxy never widens its own configuration.** Every widening is the operator's
   deliberate act after reviewing a legible refusal (`cloud/approach.md` §6). Fail-closed
   is the default state of every gap.

The unresolved half is real and named rather than absorbed: format, location on disk,
load/validation semantics, whether the config parser itself owes total-function discipline
(it is operator-authored input, so the taxonomy does not force it — but the config
loader's open placement in §5 shows the stakes), and how a register change takes effect
(restart vs reload). All of it is §11 ledger material with the config-loader row (§3
tier 3, provisional) as its placement anchor.

## 9. Lifecycle

No source specifies the proxy's lifecycle. This section separates what the sources *do*
constrain from what is genuinely open — and names the open part as questions (ledger
items L1–L3), never as invented answers.

Three constraints are derivable (each labeled derived, with its citation):

- **Fail-closed is the lifecycle default** *(derived from `cloud/approach.md` §2, §6 —
  every gap fails closed until the operator acts)*. A proxy that cannot load its register
  has no standing authorizations and serves nothing; startup with partial authority is not
  a state the posture admits.
- **The transport layer's lifecycle is already committed, and it is thin** — sessions open
  by emission, die by silence, and the transport holds no cross-epoch state, so a restart
  at that layer has "nothing to handle" (`nic/architecture.md` §8). The open questions
  live in the application layer above it, not on the wire.
- **State minimalism bounds the crash blast radius** (§6): durable state is operator
  artifacts plus the log; an interrupted transfer is a transient snapshot lost, not a
  corruption to reconcile.

The open questions:

- **L1 — Startup ordering and readiness.** What must hold before the proxy serves each
  channel (register loaded and validated, roots resolved, link up), and what "ready" looks
  like to the operator.
- **L2 — Shutdown and crash posture.** What happens to in-flight requests, transfers, and
  sessions on stop or crash, and what the operator sees afterward. The derivable
  constraints above bound the damage; they do not specify the behavior.
- **L3 — Upgrade and change discipline.** The proxy's guarantee is that a human understood
  it completely (`principles.md` §15). What re-triggers that sitting — a binary upgrade, a
  route-rule extension, a config change — is a policy-adjacent question the sources do not
  answer, and it shapes how change lands operationally.

## 10. Process shape

One baseline is source-anchored; three forks are open (ledger items L4, L14, L15).

The anchor: the proxy is **a single custom application on a stock Linux host, the only
listener on the interface** (`containment-by-construction.md` §3.3) — deliberately not a
thin appliance. Beneath it, the committed NIC runtime is a **flat, single-threaded
reactor** in userspace (`nic/architecture.md` §3). Those anchors bound the shape without
settling it:

- **L4 — Recognizer placement** *(source-open, `cloud/anthropic-api.md` §8)*. Whether the
  Anthropic-request recognizer runs as a module bound to the proxy's transport layer or as
  a separate isolated service the proxy delegates to. The source calls this "a real
  boundary decision" — the recognizer reads attacker-authored JSON — and it interacts with
  the reviewability budget (`principles.md` §15).
- **L14 — Process composition.** Whether the cloud, filesystem, and terminal services and
  the NIC reactor share the single process the CbC anchor describes, or split along
  subsystem seams. The single-threaded commitment is the transport subsystem's, not stated
  proxy-wide; nothing in the sources settles the composition.
- **L15 — Privilege shape.** The pure-direct bring-up performs privileged operations
  (sysfs unbind, bus-master enable, hugepage mapping — committed `directpath` crate;
  `nic/architecture.md` §2.1), while the steady-state relay has no evident need of them.
  Whether bring-up and steady-state split into separate privilege domains, or the process
  drops privileges after bring-up, is unstated in the sources.

## 11. Open questions — the ledger

Every decision this document touches that remains open, in one place, each routed to its
owner. Nothing in this table is settled by this document; resolved planning questions
(taxonomy axis, owned-residual representation, the `terminal.md` §5 exemplar fix) are not
listed. L10, ruled by policy after the acceptance gate, is kept in place and marked
resolved so the ledger numbering stays stable.

| # | Open question | Surfaces at | Origin | Routed to |
|---|---|---|---|---|
| L1 | Startup ordering and readiness | §9 | this document (composition) | design |
| L2 | Shutdown and crash posture | §9 | this document (composition) | design |
| L3 | Upgrade and change discipline — what re-triggers the reviewability sitting | §9; `principles.md` §15 | this document (composition) | design + policy |
| L4 | Anthropic recognizer placement (in-transport module vs isolated service) | §3 tier 1; §10 | `cloud/anthropic-api.md` §8 (source-open) | cloud subsystem design |
| L5 | Exact recognized request shape (fields and tool shapes admitted, all else denied) | §3 tier 1 | `cloud/anthropic-api.md` §8 (source-open) | builders at spec-freeze; frozen as trusted surface |
| L6 | FS open-discipline primitives (`openat`, `O_NOFOLLOW`, `O_CREAT`+`O_EXCL` direction) | §3 tier 1; §4 filesystem walk | `filesystem.md` §1 (source-open) | filesystem design review |
| L7 | Quarantine inertness mechanism (`noexec,nosuid,nodev` mount direction, no auto-indexing) | §3 tier 1; §4 filesystem walk | `filesystem.md` §2 (source-open) | filesystem design review |
| L8 | NIC DMA-reach residual — does the threat model tolerate un-contained firmware DMA on the trusted side | §3 tier 4; §4 NIC walk | `nic/architecture.md` §2.1 (source-open) | policy layer, audited after the PoC |
| L9 | Config / route-register loader tier placement (tier 3's severity clause does not hold) | §3 tier 3; §5 | this document (placement) | design |
| L10 | **RESOLVED (zero.policy, 2026-07-18, via the PM)** — the upstream TLS library does not count against the §15/§16 reviewability budget: the budget scopes to the box-reachable surface; box bytes reach TLS only as opaque plaintext to encrypt (its parser sees only the cloud's records); a TLS bug fails the feature or leaks undefended content (`principles.md` §7), never grants the box reachable execution. Off-surface, out of budget. | §3 tier 3; §4 cloud walk | this document (placement; adversary-class attribute) | resolved — policy ruling; see addendum below |
| L11 | Linux kernel standing — excluded from the reviewable path yet the quarantine floor's substrate | §3 unplaced; §5 | this document (placement) | design + policy |
| L12 | Configuration mechanics — format, location, load/validation, reload vs restart take-effect, whether the parser owes total-function discipline | §8 | this document (composition) | design |
| L13 | Observability mechanics — log transport, storage, rotation, the safe viewing path, metrics/health; inherits "box content is never rendered on trusted ground" | §7 | this document (composition) | design |
| L14 | Process composition — one process per the CbC anchor, or split along subsystem seams | §10 | this document (composition) | design |
| L15 | Privilege shape — bring-up vs steady-state privilege domains | §10 | this document (composition) | design + nic team |

Promotion tooling is deliberately absent from this ledger: it is out of scope for the
proxy (`filesystem.md` §4), a boundary rather than an open question.

**Addendum (L10/L11, non-blocking re-confirmation).** Per the PM's relay of the policy
memo (2026-07-18), the memo resolving L10 partly bundled its TLS reasoning with a premise
since corrected at the source (the pre-fix IOMMU framing of `terminal.md` §5). The TLS conclusion stands on its independent leg — the box
never controls TLS's parser input — so this is a re-confirmation item, not an expected
reversal. It rides with L11, which is already open with design + policy for the same
class of question; no new ledger row.
