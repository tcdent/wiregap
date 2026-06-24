# The Protocol Library — Implementation Overview

> A general map for building the protocol as **one standalone Rust crate** — the single source of
> truth for the box↔proxy↔head wire. The proxy, the head, and the box each link it and use the slice
> they need; the crate proves its own correctness as a closed loop, independent of any consumer.
>
> This is a **planning overview, not a spec and not a detailed design.** The normative wire contract
> lives in [`model.md`](./model.md), [`display.md`](./display.md), [`file.md`](./file.md), and
> [`net.md`](./net.md). This document frames *how* that contract becomes a library and *which* parts
> each consumer needs — enough for the team to plan sprints. Where a real decision is required, it is
> called out in §8 (Open decisions) rather than guessed.

---

## 0. What the library is, and is not

**It is** the wire grammar in Rust: turn owned bytes into typed messages, and typed messages into
bytes — losslessly, totally, and the same way for everyone. One crate, consumed in parts.

**It is not** the transport beneath, nor the policy above. Two boundaries fix the scope:

- **Below — the copy boundary** ([`model.md`](./model.md) §4). The library begins where owned bytes
  begin: its input is an owned frame buffer (`&[u8]` / `&mut [u8]`), never the NIC ring. The driver,
  DMA, IOMMU, stop-and-wait, ACKs, and session multiplexing live *below* it and are not in the crate.
  All `unsafe` lives in that driver, not here — **the library is safe Rust end to end.**
- **Above — effect and policy.** The library hands a consumer a *parsed message*; deciding and acting
  on it is the consumer's. It does **not** match routes, recognize HTTP, run a PTY, touch a
  filesystem, render a cell, or read a clipboard. It carries the bytes that those actions read; it
  performs none of them (laws 5–7).

> *Why a single crate.* The protocol is only sound if every party encodes and decodes it identically.
> One crate makes that structural: a verb's layout exists in exactly one place, so the proxy's decoder
> and the box's encoder cannot drift. The crate is the contract; the conformance vectors (§6) are its
> signature.

## 1. Layering (bottom to top)

| layer | responsibility | in the crate? |
|---|---|---|
| transport | delivery, ACK, ordering, session multiplexing, the ring | no (below) |
| copy boundary | owned frame bytes appear | the crate's input edge |
| frame envelope | session id + flag byte (continuation) + the L2 message | byte layout: see §8 (Open) |
| L2 message | `[verb u32][body]` — one flat dispatch (law 1) | **yes** |
| body decoders | the three: fixed records, verbatim tail, segment-stack scan (§6 of model.md) | **yes** |
| typed messages | the verb catalog as Rust types | **yes** |
| service modules | terminal, file, net, errors | **yes** |
| self-validation | round-trip, fuzz, conformance vectors | **yes** (§6) |

## 2. The codec core — the heart

Three decoders, and only three (law 10, [`model.md`](./model.md) §6), are the entire parsing
vocabulary; every message is built from them:

- **fixed-width records** — `split_first_chunk::<N>` + `from_le_bytes`, read in place.
- **verbatim tail** — the rest of the frame, opaque, copied as-is.
- **segment-stack scan** — `0x1F`-delimited, every byte verified against an alphabet or rejected; the
  scan **verifies, never transforms**.

The core enforces the invariants structurally, not by convention:

- **Body shape** is `[fixed prefix?][one frame-bounded tail]` (law 10) — two variable parts is two
  messages, made unrepresentable by the types.
- **Boundaries** come only from the verb, a fixed width, the `0x1F` scan, or the frame length — never
  from a length or delimiter inside box content (law 11).
- **Totality** is the security property: parsing every possible input yields either a typed message or
  a typed rejection — **never a panic, never a read past the frame.** This is what the fuzz loop (§6)
  exists to prove, because the box is hostile by assumption.

- **Little-endian** throughout; **`std`-only, no serialization dependency** (hand-written codec).

## 3. The segment stack and the safe/unsafe split

One addressing format, shared by file and net ([`file.md`](./file.md) §0). Its correctness rests on a
**type split**: `SafeSegment` (restricted alphabet, loggable) vs `UnsafeSegment` (real names, display-
only). The library makes the guarantee a compile error, not a discipline — **the logger accepts only
`SafeSegment`**, so a real filename or domain physically cannot reach a log line. Traversal is
structural (no `/`, no `..`); the receiver joins under its own root.

## 4. The verb catalog and service modules

The crate is organized by the verb blocks of [`model.md`](./model.md) §8. Each module owns its
messages' types and their encode/decode; nothing else defines a body.

| module | messages | notes |
|---|---|---|
| **control** | `open-session` | the session-allocation seed (law 8) |
| **terminal / display** | the `u32` cell-op stream | [`display.md`](./display.md); decode-heavy for the head |
| **terminal / input** | `key` `mouse` `paste` `focus`, `resize`, `viewport` | the actuation header + `KeySym`/`Modifiers`/`Button` enums ([`model.md`](./model.md) §8 Input events) |
| **terminal / pane-state** | `pane-state` | box→head scrollbar extent + input-interest |
| **file** | `file-write`/`chunk`/`read`/`data`, `list`/`list-entry`, `request-file`, `file-error` | segment stacks (§3); `list-entry` repeated records |
| **net** | `net-address`/`payload`/`response`/`error` | address stack (§3); payload reassembly (§5) |
| **errors** | the shared error body | one grammar, used by `file-error` and `net-error` |

> *The grammar-reuse question (terminal layer).* `key` and `mouse` share an actuation header, and
> `create-pty`/`resize` share a dimensions body. Whether the crate expresses these as shared grammars
> referenced by multiple verbs (the error-body pattern, generalized) or as independent layouts is a
> structuring choice for the team — see §8.

## 5. Reassembly and framing

The default is **one message per frame** (law 2): large data is many complete single-frame messages
(a file is appended chunks; a repaint is a fresh keyframe). There are exactly **two exceptions** where
one logical message spans frames via the continuation bit, reassembled opaquely:

1. a display repaint larger than a frame ([`display.md`](./display.md)) — reassembled by the **head**;
2. a `net-payload` larger than a frame ([`net.md`](./net.md) §2) — reassembled by the **proxy**, the
   one trusted-side reassembly buffer, because the recognizer is total over a whole request.

The library defines the reassembly *rule* (accumulate while the continuation bit is set; finalize on
the clear bit; never read length from content). Whether it also ships a **bounded reassembler**, and
where the cap and fail-closed policy live, is a layering decision — see §8.

## 6. The self-validation loop — the crate proves itself

The crate is a **closed loop**: it can exercise the full encode → wire → decode path with no consumer
present, and assert the protocol is sound. This is a first-class deliverable, not an afterthought.

- **Round-trip property** — for every message type, `decode(encode(m)) == m`, and a decoded message
  re-encodes to the identical bytes. Over the whole catalog.
- **Adversarial fuzz** — arbitrary and malformed byte streams (the box is hostile) yield only
  `Ok(msg)` or `Err(reject)`: never a panic, never an over-read, never an allocation the input
  controls. This is the totality guarantee (§2) under test.
- **Conformance vectors** — golden byte sequences (the worked dumps in [`model.md`](./model.md) §8)
  checked both directions. These double as the cross-consumer contract: proxy, head, and box all
  validate against the same vectors, so "they speak the same protocol" is a checked fact.
- **The full-loop harness** — a test that builds every message, serializes, reassembles where
  multi-frame, decodes, and asserts equality — the protocol validating itself, end to end.

## 7. Consumer surfaces — who links what

Each consumer compiles only its slice (feature-gated — granularity is a §8 decision), but the
validation loop always exercises the whole. Illustrative, for the team to refine against the actual
binaries:

| consumer | decodes | encodes | relays (envelope only) |
|---|---|---|---|
| **proxy** | `net-address` (match+log), `net-payload` (reassemble→recognize), file addressing (confine) | `net-response`/`error`, `file-data`/`error`, `list-entry` | display, input, between box and head |
| **head** | display, `pane-state`, `file-data`, `list-entry`, errors | `key`/`mouse`/`paste`/`focus`, `resize`, `viewport`, `request-file`, `list`, `open-session` | — |
| **box** | input, `create-pty`, `resize`, `viewport`, `request-file`, `list`, `file-data`, `net-response` | display, `pane-state`, `file-*`, `net-address`/`payload` | — |

> The **box is the widest consumer** (it speaks nearly the whole protocol); the **proxy parses the
> envelope of everything to route, but deep-parses only what it acts on** (net + file addressing) and
> relays the rest inert; the **head is terminal-only** — it never sees file or net internals.

## 8. Open decisions — for the team and escalation

These are genuine forks the implementation must settle. The team's PM escalates the ones reserved for
the operator; the rest are the team's to choose and are flagged here so they are chosen, not defaulted.

- **Envelope ownership** — does the crate own the L1 frame header (session id + flag byte) or only the
  L2 `[verb][body]`, taking session/flag as context from the transport? (Layering; affects where
  reassembly and multiplexing sit.)
- **Reassembler residence & cap** — does the crate ship the bounded continuation-bit reassembler, or
  only the rule, leaving the buffer and cap to each consumer? Where is the fail-closed cap configured,
  and to what value? (`net.md` §2 mandates *bounded*; the number is unset.)
- **Message representation** — one crate-wide verb enum, per-service enums, or a trait per message;
  in-place field overlay vs owned structs at the decode edge.
- **Grammar reuse** — express shared bodies (actuation header, dimensions, error) as named reusable
  grammars, or keep per-verb layouts that merely coincide (§4).
- **Feature-gating granularity** — per-service cargo features, per-direction, or all-in.
- **`no_std`** — is any consumer constrained enough to need it, or is `std` universal?
- **Named-key integer table** — the `> 0x10FFFF` functional keysym values are deliberately *not* yet
  pinned (full draft first, per the design conversation); the crate cannot finalize the input codec
  until that table is assigned.

---

*One crate is the protocol: the three decoders, the safe/unsafe segment types, the verb catalog by
service, and a loop that proves its own round-trip and rejects the adversarial. Everyone links the
same source of truth and validates against the same vectors, so the wire cannot drift between the box,
the proxy, and the head.*
