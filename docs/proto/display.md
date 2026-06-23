# Terminal Display — The Cell Stream

> The box→head display payload: the stream of cells and directives that paints a terminal. The
> box-side emulator resolves the grid (width, wrap, scroll, grapheme composition); this conveys the
> resolved result, which the head applies to a held grid and paints. Companion to [`model.md`](./model.md);
> supersedes the 16-byte style descriptor, keyframe-header, and display byte-map of
> [`../explore/console-and-protocol-design.md`](../explore/console-and-protocol-design.md) §4.2.
>
> **Reading convention.** Plain text and tables are the normative spec; `>` blockquotes are commentary.
> Strip the blockquotes and the spec is complete.

---

## 0. The model — one cell stream, one apply loop

A display payload is a flat sequence of **`u32` cells**. There is **no keyframe/delta distinction on
the wire**: a "keyframe" addresses every cell, a "delta" addresses some; both are the same cell stream,
and the head applies the ops in order onto a held grid — one loop, no frame-type branch. The head's
grid and render state **persist across messages**.

> *Rationale.* If a keyframe is just a delta that addresses every cell, the receiver has one operation
> — apply positioned cells — and never branches on frame type. A freshly-attached consumer holds
> nothing until the box sends a full repaint (the serial-console behavior of `model.md` §9). Wrap,
> scroll, width, and grapheme resolution are the box emulator's job; the head obeys.

## 1. The cell — a `u32`, op in the top byte

Each cell is a `u32`: the **top byte is the op**, the **low 24 bits the payload**.

```rust
let op      = cell >> 24;            // 0x00–0xFF
let payload = cell & 0x00FF_FFFF;
match op {
    0x00 => draw(payload),
    0x01 => goto(payload),
    0x02 => set_fg(payload),
    0x03 => set_bg(payload),
    0x04 => set_underline(payload),
    0x05 => set_attrs(payload),
    0x06 => set_dims(payload),
    0x07 => set_cursor_pos(payload),
    0x08 => set_cursor_style(payload),
    _    => {}                       // 0x09–0xFF reserved → ignored
}
```

> *Rationale.* One shift decodes the op. The `_` arm makes the dispatch total — an unknown op is
> ignored, the forward-compat path (a future op skipped by an older head, like a reserved bit).

## 2. The ops

Positions are **row in the high 12 bits, col in the low 12 bits** (`goto`, `set_dims`, `set_cursor_pos`),
0–4095 each. Colors are **24-bit RGB** (`0xRRGGBB`).

### `0x00` draw

| bits | field |
|---|---|
| 0–20 | Unicode scalar value |
| 21–23 | column width: `0` zero-width, `1` normal, `2` wide |

Draws the scalar at the write-position in the current style, then advances by the width: width 1 a
normal cell, width 2 a wide cell (two columns), width 0 a combining mark that composes onto the current
cell without advancing. An out-of-range scalar (> `0x10FFFF`) or surrogate draws the replacement glyph.

> *Rationale.* The width is supplied by the box, which already resolved it, so the head never consults
> a Unicode width table; a multi-codepoint grapheme is a base draw plus zero-width draws.

### `0x01` goto
Payload = `row(12) | col(12)`. Sets the write-position; an out-of-grid position is clamped.

### `0x02`–`0x04` set-fg / set-bg / set-underline-color
Payload = `0xRRGGBB`. Sets that color in the current style until changed.

### `0x05` set-attributes
Payload is the **whole** attribute state (replace, not toggle):

| bit | attribute |
|---|---|
| 0 | bold |
| 1 | dim |
| 2 | italic |
| 3 | inverse |
| 4 | strikethrough |
| 5 | blink |
| 6 | conceal |
| 7–8 | underline style: `0` none, `1` single, `2` double, `3` curly |
| 9–23 | reserved (zero) |

### `0x06` set-dimensions
Payload = `rows(12) | cols(12)`. Sizes the grid (the head bounds its own allocation). Wrap is not
implied — the box resolved it; dimensions are for sizing and addressing.

### `0x07` set-cursor-position
Payload = `row(12) | col(12)`. The **text cursor**, distinct from the write-position `goto` moves.

### `0x08` set-cursor-style
| bits | field |
|---|---|
| 0–1 | shape: `0` hidden, `1` block, `2` underline, `3` bar |
| 2 | blink |
| 3–23 | reserved |

### `0x09`–`0xFF` reserved
Ignored by the head — where later ops land (a style reset, scrollback hints), each a new op value,
never a sub-field of an existing one.

## 3. Render state

The head holds, per pane: the **grid** (each cell its content + style snapshot), the **write-position**
(advanced by `draw`, set by `goto`), the **current style** (set by the format ops), the **cursor**, and
the **dimensions**. `draw` captures the current style into the cell. Defaults before any op: position
`(0,0)`, terminal-default colors, no attributes, cursor hidden.

## 4. The proxy — relay; neutralization is by construction

Display content **is neutralized** — adversary content cannot be interpreted as control — but **by
construction, not a step the proxy runs.** The format cannot represent control: a `draw` is a codepoint
the head paints, never a byte sequence anyone parses; every other op sets inert state. So the proxy
**relays** the display payload; it neither filters nor walks it.

> *Rationale.* This is neutralization in the spine's preferred form — *make it impossible, do not make
> it caught*: control is unrepresentable, not scrubbed. It supersedes the display byte-map (the filter
> a raw-byte display needed); the filter is gone, not the neutralization. A control codepoint (`U+001B`)
> is just a glyph the head renders or replaces — no parser to drive. So the proxy's operations reduce
> to recognize / confine-write / relay, neutralization being a property of the format.
>
> **Misrepresentation is out of scope.** Deceptive scalars (bidi overrides, homoglyphs, zero-width) are
> not a concern: terminal output is never executed and no load-bearing decision rests on the pane — the
> one consequential judgment is promotion, which reviews an extracted *file*, not the live display. A
> pane that misrenders is cosmetic.

## 5. Deliberately absent

- **Embedded protocols** — Sixel/Kitty images, OSC-8 hyperlinks, OSC-52 clipboard, notifications.
  Excluded permanently as a class: capabilities, not ways to draw a cell.
- **Compression / RLE** — none. A large repaint that exceeds a frame spans packets via the continuation
  bit (one message reassembled, not a delta).
- **Size caps** — none of consequence; 12-bit coordinates reach 4096×4096, far beyond any real terminal.

## 6. Decided / Open

**Decided.** The `u32` cell, op-in-top-byte; the unified stream (no keyframe/delta distinction, one
apply loop); the op set; width carried in `draw`; box-resolved wrap/scroll/composition; the proxy
relays with no neutralization step. Supersedes the descriptor, keyframe-header, and byte-map of console
§4.2.

**Open.** The **attribute bit** and **cursor shape** assignments above (proposed); whether **default
colors** are themable; a possible **style-reset** op.

---

*A cell is a `u32`: a glyph with a width, or a move, a color, a size, a cursor — the op in the top byte,
the meaning in the low three. The box resolves the terminal; the head applies a flat stream onto a held
grid and paints, the same loop whether it redraws everything or one character; and the proxy relays the
stream untouched, because nothing in it can represent control.*
