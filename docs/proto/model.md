# The Protocol — Model, Sessions & Framing

> The protocol that rides *inside* a session: the message a box and head exchange one layer above
> the Ethernet frame. The frame, the transport channel's reliability (per-channel stop-and-wait,
> per-packet ACK) and integrity (the NIC's hardware CRC) belong to the transport and are specified
> in [`../explore/transport-stack.md`](../explore/transport-stack.md), assumed here. Companion to
> that doc (below) and the service specs [`display.md`](./display.md), [`net.md`](./net.md),
> [`file.md`](./file.md) (above). Grounded in [the spine](../principles.md).
>
> **Reading convention.** Plain text and tables are the **normative spec** — what an implementation
> must match. `>` blockquoted notes are **commentary**: rationale and grounding, not binding. Strip
> every blockquote and the spec is still complete.

---

## 0. Scope

In scope: the **session** (the connection a message rides on), the **message** (verb + body), how a
message is typed and read, the encoding conventions every body shares, and the verb catalog.

Below this layer (*transport-stack*): the frame, the transport channel and its id, ordering,
acknowledgment, flow control, integrity. The transport carries bytes and never reads a verb.

Above this layer: the box-side emulator, the head renderer, the proxy's request recognizer. They
*consume* this protocol; their internals are theirs.

> The transport's unit is a **stream**; the application's unit is a **session**, one per stream.
> "Session" is the word used from here up — it is the established-state connection operations run
> on. "Stream" stays the transport's word for the same channel seen one layer down.

## 1. One flat layer

A message is a single tagged value: a **verb** (`u32`) followed by that verb's **body**. It is read
by reading the verb and dispatching to its handler, which reads the rest. There is no second framing
layer and no sub-discriminator beneath the verb.

> *Rationale.* It is a tagged union, not an envelope-plus-payload. Everything a transport carries —
> identity, ordering, where the bytes end — lives below; everything a message *means* lives in the
> body. The only field every message shares is the verb. One read, one branch.
>
> *"A stream is a stream. It doesn't matter; it just passes layer-two packets inside of its
> layer-one transport and it can be used for whatever it wants."*

## 2. Sessions — the connection layer

A **session** is an established-state connection between box and proxy, carried on one transport
stream. The **box is the sole allocator**: a session id is a monotonic counter the box increments,
never reused. A session comes into being by **emission** — the box opens it and the first frame
carries its new id; there is no setup handshake.

**`open-session`** is the one command to create a session: it instructs the box to **increment and
open**. It may be sent from **any established session**, and the new session announces itself by
emission (its id arrives in the L1 frame). **Session 0 is the seed** — the first session, present
from bring-up, before any other exists; it is a normal session, special only in being first.

A session is opened generic and then carries an **operation** — a PTY, a file transfer, a listing, a
network call. Open, then run an operation on it.

> *Rationale.* The asymmetry of opening new sessions from existing ones is the tmux move — you spawn
> a pane *from* any pane. It is not a control-stream category; `open-session` runs the identical
> protocol as everything else.
>
> The whole system is a **serial protocol**: each session is strictly in-order (stop-and-wait), the
> wire is in-order, and the *one* layer of abstraction — multiplexing — lets several serial sessions
> share the one serial line. Even concurrency resolves back to serial. This is not a simplification
> we settled for; it is what the data *is* — a terminal is renders over time, a file is bytes in
> order — so out-of-order delivery would buy nothing and cost reassembly. The transport floor
> (stop-and-wait, in-order-per-session, no reordering) *is* that serial protocol.
>
> "Agent" is **not** a protocol term: an agent is just a bash command running in a PTY we emulate.
> The box's connection *generation* — the id namespace that resets on box restart (transport-stack's
> "session") — is the wider-scope **epoch**; a session lives inside an epoch.

## 3. One message per frame

One message is exactly one frame payload. The frame boundary is the message boundary; the frame's
length (reported by the NIC in hardware) delimits it; **there is no length field in the message.**
Data larger than a frame is **many complete single-frame messages**, each applied on arrival — never
one message reassembled across frames — so the trusted side holds no reassembly buffer.

> *Rationale.* The driver already emits one complete frame per event and refuses multi-segment
> reassembly (`../nic/grounding/data-path-and-events.md` §1), so "an event is a whole message" is
> free. A file is offset-addressed chunks straight to disk; a display update is a fresh keyframe. A
> payload genuinely larger than a frame (a big keyframe) spans frames via the transport's
> continuation bit — one message reassembled opaquely, never content-aware reconstruction in the
> driver. *(The one trusted-side exception is the net-request recognizer, which is total over a whole
> request — see `net.md`.)*

## 4. The copy boundary

A received frame is **copied out of the NIC's descriptor ring into application-owned memory at the
driver boundary**, and the descriptor is refilled immediately. The ring buffer never travels up.

> *Rationale.* The copy is the single locus where wire bytes become owned, bounded, normal-lifetime
> data, and the place "never read past the hardware frame length" is enforced. It is a bounded
> per-frame copy, not buffering, and free at these rates. We are not throughput-bound, so the one
> reason to keep adversary bytes in borrowed ring memory does not apply, while its cost (lifetime
> coupling, scattered in-place interpretation) is real. In Rust the `unsafe` region and this boundary
> coincide: `unsafe` ends where the copy yields owned bytes; above is safe Rust, below is the small
> searchable VFIO/MMIO/DMA driver.

## 5. Reading a message — geometry

The verb is a **fixed-width, aligned `u32`.** Its value space is over-provisioned; the width is
chosen so the body begins on a four-byte boundary and a handler can overlay its struct on the owned
buffer and read fields in place. Spare bytes are reserve for verb growth and per-verb flags, ignored
when zero.

> *Rationale.* Generous where it is free, minimal where it is load-bearing — the same posture as the
> style descriptor's four-byte columns. Overlay is sound only on all-integer POD structs (every bit
> pattern valid); a field that is a Rust `enum`/`bool`/`char` must be validated, not transmuted.

## 6. Body shape and the three decoders

A body is **`[fixed-width prefix?][one frame-bounded tail]`** — zero or more fixed fields, then at most
one tail that runs to the frame boundary. The tail is one of three kinds, each with its own `std`-only,
bounds-checked decoder, never a serialization library and **never a box-supplied length**:

- **fixed-width records** — `split_first_chunk::<N>` + `from_le_bytes`; width and type compiler-checked
  (display cells, list metadata, the verb itself).
- **a verbatim blob** — the rest of the frame, taken as bytes and never parsed (file content, the net
  payload, keystrokes).
- **a segment stack** — a **scan** that *verifies*, never transforms: every byte is checked against
  the segment's alphabet or the message is rejected. A `0x1F` byte (excluded from the alphabet, so it
  cannot occur in content) **separates** adjacent segments; the **frame ends the last segment**. `0x1F`
  is a **separator, not a terminator** — there is no trailing delimiter, and a leading, trailing, or
  doubled `0x1F` yields an empty segment, which is **rejected** (both alphabets require a non-empty
  segment regardless). So each stack has exactly one encoding and round-trip is canonical. (A file
  destination, the net address.)

Two variable parts in one body is illegal — that is two messages. A boundary comes from the verb, a
fixed width, the `0x1F` scan, or the hardware frame — **never from a count the box authored.**

> *Rationale.* A box-supplied length is a second source of truth that can disagree with the bytes; a
> segment stack is *already* scanned byte-by-byte, so the split falls out of that scan for free, and
> `0x1F` is unambiguous by construction because the alphabet excludes it — a
> delimiter never meets unvalidated content. A verbatim blob is bounded only by the transport (frame,
> session close), the one boundary the box cannot forge into a misparse: it bounds *its own* data, and
> a truncated blob is denied downstream. This **supersedes the fixed-256 leaf record and the
> `split_at_checked` carve** — there are no length-prefixed fields left anywhere. The scan accepts one
> of two alphabets — **safe** (restricted, loggable) or **unsafe** (permissive, real names,
> display-only) — both defined in `file.md` §0, carried as distinct types so the logger can only
> accept a safe segment.

## 7. Verbs — flat, single-purpose, support is the boundary

There is one **flat** verb space. Each verb does exactly one thing; no verb carries a
sub-discriminator. A verb names **state, data, or intent — never an action the trusted side performs
on the box's say-so.**

> *Rationale.* **Origin is descriptive, not a control** — the box authors any bytes it likes,
> including a verb that "belongs" to the other direction. What makes the system safe is what each
> receiver *supports*: the trusted side interprets only a minimal set and has no code path for
> anything dangerous, so a misdirected or unknown verb is **unsupported**, inert or denied — not
> caught.
>
> *"Origin is not enforcement. The interpretation of the command, and support for it in the first
> place, is."*

## 8. The verb catalog

Grouped by direction as documentation, not enforcement. Verbs marked *(open)* are named but not yet
specified.

**Head → box — intent into the contained zone** (verified well-formed, never transformed):

| verb | body | notes |
|---|---|---|
| `open-session` | — | box increments + opens; new session appears by emission (§2). From any session. |
| `create-pty` | `rows`, `cols` | run a PTY on a session (display.md). |
| `key` | `KeySym` + mods + kind + `text` | a key event; the box adapter encodes per mode (Input events). |
| `mouse` | button + mods + kind + cell + scroll delta | mouse/scroll; the box routes scroll per mode. |
| `paste` | verbatim text | clipboard paste — first-class, not a bracketed-paste mode. |
| `focus` | `gained` | focus change — first-class, not a focus-reporting mode. |
| `resize` | `rows`, `cols` | window-size change to the PTY layer. |
| `viewport` | `offset` (`u32`, lines above live, `0` = live) | explicit scrollback navigation (head-decided; *not* the wheel — see Input events). |
| `request-file` | source unsafe stack | cause the box to extract a file (file.md §4). |

**Box → head / box → proxy — emission, state, and the egress request:**

| verb | body | treatment |
|---|---|---|
| `display` | `u32` cell stream (display.md) | **relay** — inert by construction. |
| `pane-state` | `[scrollback_total u32][viewport_offset u32][input_flags u32]` | **relay** — scrollbar extent + motion-interest (Input events). |
| `file-chunk` | verbatim bytes (no offset) | **append** to the quarantine file (file.md §1). |
| `list` entries | repeated `[metadata][name]` records (file.md §3) | relay. |
| `file-error` | shared error body | relay. |
| `net-address` | scheme + safe segment stack (net.md) | match against the route + **log**. |
| `net-payload` | verbatim request bytes (net.md) | hand to the native **recognizer**, off-wire. |
| `net-response` | verbatim response bytes | relay (lands on untrusted box). |
| `net-error` | shared error body | relay. |

> *Rationale.* The proxy's three operations on box-originated bytes are **recognize** (net-request),
> **confine-and-write** (file destination + offset), and **relay** (everything else, display
> included). There is **no neutralization step** — display content is inert by construction, not
> filtered (display.md §4). The only box-controlled inputs the trusted side acts on are the file
> destination and offset, both confined by construction.

### Verb numbers

The verb `u32` is **the verb id in the low byte, the upper three bytes reserved** (per-verb flags and
id growth, zero for now). Ids are grouped by service in blocks of 16, with room to grow in each:

| id | verb | direction |
|---|---|---|
| `0x00` | `open-session` | head→box |
| `0x10` | `create-pty` | head→box |
| `0x11` | `display` | box→head |
| `0x12` | `key` (structured event) | head→box |
| `0x13` | `resize` | head→box |
| `0x14` | `viewport` | head→box |
| `0x15` | `mouse` | head→box |
| `0x16` | `paste` | head→box |
| `0x17` | `focus` | head→box |
| `0x18` | `pane-state` (scrollback extent + input interest) | box→head |
| `0x20` | `file-write` (declare destination) | box→proxy |
| `0x21` | `file-chunk` (verbatim bytes, appended) | box→proxy |
| `0x22` | `file-read` (request source) | box→proxy |
| `0x23` | `file-data` (read bytes) | proxy→box |
| `0x24` | `list` (request directory) | box→proxy / head→box |
| `0x25` | `list-entry` (entry records) | response |
| `0x26` | `request-file` | head→box |
| `0x27` | `file-error` | either |
| `0x30` | `net-address` (scheme + safe segment stack) | box→proxy |
| `0x31` | `net-response` | proxy→box |
| `0x32` | `net-error` | proxy→box |
| `0x33` | `net-payload` (verbatim request bytes) | box→proxy |

Blocks: `0x0_` control, `0x1_` terminal, `0x2_` file, `0x3_` net. Unused ids in each block and all of
`0x40`–`0xFF` are reserved.

> *Rationale.* 22 verbs of 256, four service blocks of 16 — generous headroom in every block, and the
> low-byte id leaves the upper three bytes for flags without disturbing 4-byte alignment.

### Input events

Input is **structured** ([`../head/architecture.md`](../head/architecture.md) §3): the head emits typed
events; a box-side legacy adapter — the mirror of the output emulator — encodes them *down* to PTY
bytes per the app's modes. The wire is mode-free; modes live only in the adapter. This is the safe
direction (head→box, box-parsed), so the structure is not a trusted surface.

**`KeySym` — a `u32`, on the display-cell threshold split:**

| value | meaning |
|---|---|
| `0` | no key — text-only insertion (a multi-grapheme IME commit); `text` stands alone |
| `1`–`0x10FFFF` | a character key: the **base** codepoint (`charactersIgnoringModifiers`) |
| `> 0x10FFFF` | a named functional key |

Named keys sit above `0x10FFFF` rather than in Unicode's PUA, because we may render real PUA glyphs
(Nerd-Font icons); the same `0x10FFFF` boundary that splits the display cell splits the keysym. The
producible set (macOS `NSEvent` / `NSTextInputClient`), grouped:

- **editing** — named, *not* their control codepoints, so `Ctrl+I` ≠ `Tab`: Enter, Tab, Escape, Backspace
- **navigation** — Up, Down, Left, Right, Home, End, PageUp, PageDown, Insert, Delete
- **function** — F1–F20
- **keypad** — KP_0–KP_9, KP_Decimal, KP_Plus, KP_Minus, KP_Multiply, KP_Divide, KP_Enter, KP_Equals,
  KP_Clear

Excluded as not head-producible on macOS (no phantoms): NumLock, ScrollLock, PrintScreen, Pause, Menu,
media/system keys, and CapsLock-as-a-key (a modifier flag, not a key).

**`Modifiers` — `u16` bitfield:** `Shift`(0) `Control`(1) `Alt`(2) `Super`(3) `CapsLock`(4) `Fn`(5);
bits 6–15 reserved (left/right distinction, future).

**`Button` — `u8`:** `None`(0) `Left`(1) `Middle`(2) `Right`(3) `Back`(4) `Forward`(5); reserved above
(Left/Middle/Right + wheel is the whole terminal need; `Back`/`Forward` are convention-mapped from
generic macOS `otherMouseDown` button numbers, not OS-labeled — kept for SGR extended buttons). `kind`
is Press/Repeat/Release for `key`, Down/Up/Move/Scroll for `mouse`. A `Scroll` event sets `button =
None`; `dx`/`dy` carry the delta.

> *Rationale on Super.* Command maps to `Super`, which has no legacy-terminal encoding — the adapter
> drops most Super-combos; it is surfaced for a future native app, and macOS reserves Cmd anyway.

**Bodies** — each a fixed prefix plus at most a verbatim tail (model.md law 10):

| verb | layout |
|---|---|
| `key` | `[KeySym u32][mods u16][kind u8][reserved u8][text: verbatim UTF-8 tail]` — `kind` ∈ Press/Repeat/Release; `text` is the produced (shifted/composed) string, empty = none |
| `mouse` | `[button u8][kind u8][mods u16][row u16][col u16][dx i16][dy i16]` — `kind` ∈ Down/Up/Move/Scroll; `dx`/`dy` carry the scroll delta, 0 otherwise |
| `paste` | `[text: verbatim UTF-8 tail]` — clipboard text, distinct from a text-only `key` |
| `focus` | `[gained u8]` |

Character convention: `key` carries the base codepoint, `text` the produced string — `Shift+a` →
`Key{0x61, Shift, "A"}`; `Ctrl+i` → `Key{0x69, Ctrl, none}` (≠ `Tab`, which is a named key). Standalone
modifier-key events (Left-Shift pressed) are out of scope for now.

**Scroll routing is the box's, not the head's.** A wheel gesture is `mouse{Scroll, dx/dy}`; the box
routes it by the app's mode — wheel-report to the focused app if mouse-reporting is on, else adjust the
pane's own viewport and re-render. The head never decides wheel-vs-scrollback. `viewport` is the
*separate*, unambiguous case: explicit scrollback navigation (a scrollbar, a keyboard scroll) that is
always the pane's, never the app's.

**Box→head pane state — `pane-state`.** Two things the head needs that the cell stream does not carry,
both sourced from the box adapter (which holds the app's modes and the scrollback buffer):

| field | bytes | for |
|---|---|---|
| `scrollback_total` | 4 (`u32`) | scrollbar extent — the head sizes/positions the thumb, maps a drag to a line |
| `viewport_offset` | 4 (`u32`) | the current offset after box clamping — the thumb position |
| `input_flags` | 4 (`u32`) | bit 0 = **wants-free-motion**; reserved above |

The box emits it per pane when the state changes. Until then the head's defaults hold: **wheel scroll**
(send last ± delta, box clamps) and **drag-only motion** (`Mouse{Move}` only while a button is held).
Free motion (hover) flows only when `wants-free-motion` is set — so a mouse-tracking TUI gets hover and
an app that does not want it never floods the wire. It is box→head, relayed, not a trusted surface.
`viewport_offset` is the same quantity and type as the `viewport` verb's `offset` (0x14) — `u32`, lines
above live, `0` = live; an absolute scrollback position is never negative.

> *Rationale.* Same shape as everything here — the head produces a clean event, the box (which holds
> the app's mode state) decides the legacy meaning. This resolves the old `key | bytes` verb (a legacy
> artifact) and the signal-injection item: `Ctrl+C` is `Key{0x63, Ctrl}`; SIGINT is the adapter's call.
> Input `text` and `paste` are verbatim into the box and never logged, so the safe/unsafe-segment rule
> does not apply.

### Error message body — `net-error` and `file-error`

Both errors share one body, in the standard shape — a fixed header plus a rest-of-frame blob:

| offset | bytes | field |
|---|---|---|
| 0 | 4 | `code: u32` — category (below) |
| 4 | … | `detail` — optional UTF-8 context, rest-of-frame; empty if none |

| code | meaning |
|---|---|
| 0 | unspecified |
| 1 | not-found (no such path / route) |
| 2 | denied (permission / refused by recognizer) |
| 3 | unavailable (busy / no space / i/o) |
| 4 | timeout |
| 5 | unreachable |
| 6 | malformed |
| 7–`0xFFFFFFFF` | reserved |

> *Rationale.* It decodes with `split_first_chunk::<4>` then the remainder — no new machinery. The
> `code` is a machine-readable category the receiver maps to its own message, so the common path needs
> no box-authored text; the optional `detail` (an upstream error string, a path) is rendered as inert
> glyphs, the receiver interpreting no control in it — the same by-construction guarantee as display.

## 9. The interaction model — cause and emission

The head **causes**; it does not call. A head→box verb is **fire-and-forget intent**; nothing it
sends awaits a paired reply. The box acts and, where the act produces a session, **emits**; the head
learns a session exists by *observing the emission* and builds its model from what arrives. There is
no handshake and no correlation token in the simple case.

Emission is change-driven and decoupled from viewing: an idle session emits nothing, and frames flow
whether or not a human is looking, so the head already holds the state to paint any session the
instant it is viewed. There is no subscribe message.

> *Rationale.* `net-request` is the one exception that is genuinely request/response — correlated by
> the session id, both directions on one session (net.md).
>
> *"The head sends a request for a PTY. The box creates or occupies a socket. A packet suddenly shows
> up with that socket ID, and it is up to the head's application to enumerate the sessions it is
> aware of based on the data it has received."*

## 10. Design laws (normative invariants)

A change that violates one is wrong even if it looks locally reasonable.

1. **One flat layer.** A message is a verb plus its body, one dispatch. No envelope, no
   sub-discriminator.
2. **One message per frame.** Large data is many complete messages, never one reassembled across
   frames (save the opaque continuation bit and the net-request recognizer).
3. **Copy at the driver boundary.** The ring buffer never travels up.
4. **Generous where free, minimal where load-bearing.** Field width and reserve are generous; the
   recognized set stays minimal.
5. **Support is the boundary, not origin.** The trusted side interprets only a minimal set; anything
   dangerous is unsupported, not caught.
6. **A verb names state, data, or intent — never an action on trusted ground.**
7. **Box-controlled inputs the trusted side acts on are confined by construction.** The file
   destination cannot traverse; the offset cannot escape its fd.
8. **The box is the sole allocator of sessions.** The head causes; it never allocates.
9. **Serial all the way down.** Each session is in-order; multiplexing is the one concession to
   concurrency, and it resolves back to serial.
10. **A body is `[fixed prefix?][one frame-bounded tail]`.** The prefix is fixed-width; the tail is
    repeated fixed records, a scanned segment stack, or a verbatim blob. Two variable parts is two
    messages.
11. **Boundaries come from the verb, a fixed width, the `0x1F` scan, or the hardware frame — never
    from box-authored content.** No length-prefixed fields exist. Wire structure exists only to
    dispatch and to address what the trusted side acts on; everything it merely moves is verbatim,
    because every control structure is a validation obligation and verbatim bytes carry none.

## 11. Decided / Open

**Decided.** The flat tagged message (verb + body); one message per frame; the driver-boundary copy;
the `u32` verb; the **body shape `[fixed prefix?][one frame-bounded tail]`** with the three decoders
(fixed records / verbatim blob / scanned segment stack), no length-prefixed fields, `0x1F` the
shared segment delimiter; the session model (box sole allocator, emission, `open-session`, session 0
the seed, operations on top); the serial protocol; `file-chunk` as a pure verbatim append (no offset
— files are serial); the net split into `net-address` (safe segment stack) + `net-payload` (verbatim); the
proxy's recognize / confine-write / relay operations with no neutralization step; a Rust
implementation with `unsafe` localized to the driver.

**Open.**

- The segment **safe/unsafe split** is decided (file.md §0): safe = restricted + loggable, unsafe =
  permissive real names that are display-only and never logged, enforced as distinct types.
- Per-verb **flag bits** in the verb's reserved upper bytes — layout fixed (§8), no flags defined yet.
- **Ratification** of the proposed number sets — verb ids and error codes (§8), display
  attribute/cursor codes (display.md).
- **Structured input** is specced (§8 Input events) — flat `key`/`mouse`/`paste`/`focus` verbs,
  `KeySym` on the cell threshold, the macOS-producible named set (**locked** with `zero.head-design`),
  `Button` capped at `Forward`, `mouse` carrying scroll deltas with box-side routing, `keysym 0` the
  IME text-only sentinel. Supersedes `key | bytes`; resolves signal-injection.
- **`pane-state`** (box→head, §8 Input events) carries scrollback extent + clamped offset (the
  scrollbar) and an input-interest flag (free-motion), the box→head mirror of the input verbs.
  **Closed both sides 2026-06-22** (`zero.head-design`): `wants-free-motion` = DECSET 1003 (hover), the
  drag-only default = 1002 (button-drag), `viewport_offset`/`viewport.offset` aligned to `u32`. The
  *behavior* — when the box emits it, how the head renders the scrollbar and routes hover — is
  `zero.head-design`'s. The box-side source (the emulator's scrollback length + mouse-mode) is existing
  state; the definitive box-side confirm is the emulator owner's, nothing blocks.

---

*One session, one frame, one message, one verb: a flat tagged value read in place on memory the
application owns, carried on a strictly serial session the box alone opens, multiplexed onto a serial
wire. The box authors freely; the trusted side recognizes a request, confines a write, or relays —
and never runs a thing the box made.*
