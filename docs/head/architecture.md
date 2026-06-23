# The Head — Client Architecture

> The operator's native client: the application that renders contained sessions to the operator
> and carries their input back. Where [`../proxy/terminal.md`](../proxy/terminal.md) describes the
> terminal subsystem from the proxy's side — the wire protocol's inexpressiveness and the proxy's
> role as a relay — this specifies the **head application's own construction**: its platform, its
> render stack, the language and process split between protocol and presentation, and the seam
> between them. Companion to [`../proxy/terminal.md`](../proxy/terminal.md),
> [`../proto/display.md`](../proto/display.md), and [`../proto/model.md`](../proto/model.md);
> grounded in [the spine](../principles.md).
>
> **Reading convention.** Plain text and tables are the normative spec; `>` blockquotes are
> commentary — rationale, and the conversation that produced it. Strip the blockquotes and the
> spec is complete.

---

## 0. The head's one job

The head **displays; it does not interpret.** It renders a bounded display vocabulary and paints
glyphs; it emulates nothing and executes nothing on the protocol's say-so. This is the maintained
design invariant from [`../proxy/terminal.md`](../proxy/terminal.md) §2 — *"renders, never
interprets"* — viewed from inside the client.

The safety this rests on is **not** a data-cleaning step. There is no point where dangerous bytes
become safe bytes; the format cannot represent danger in the first place (`display.md` §4). What
the head does is **bounded interpretation**: a total function over a known vocabulary that handles
exactly what it recognizes and drops everything else.

> *"The data is just a byte stream that is bounded and interpreted. … It's not trying to be a
> gracefully degrading parser that makes concessions. It's trying to be a binary filter of either
> 'I know what this is' or 'I don't know what this is.'"*
>
> This is the same total recognize-or-reject shape as the proxy's net recognizer and the file
> leaf-validator (`model.md` §7, *"support is the boundary, not origin"*). It is a property of how
> the interpretation is written — total, bounded, default-drop — and is therefore **language- and
> side-agnostic**: it holds wherever the interpreter runs.

## 1. Platform — native macOS, single target

The head is a **native macOS application, single-target.** It is not a cross-platform application
that happens to run on macOS.

> *Rationale — surface minimization.* The governing rule is that a dependency which brings a
> *larger capability surface than the subset we use* is itself a liability.
>
> *"Involving that whole stack of a vulnerability surface means that our render, even if we only
> support a subset, is informing a render engine that contains a larger set."*
>
> This rejects Electron (a full browser engine and HTML/CSS/JS interpreter) and any legacy
> terminal library (an escape-sequence emulator) as a *class*: both put box-derived content within
> reach of an interpreter whose reachable set dwarfs ours. The precise test for any dependency is
> not its code size but whether **box-derived content can reach an interpretive surface inside
> it.**

Single-target is a deliberate removal of complexity, not an accident of focus. Cross-platform
abstractions (a portable GPU layer, a portable windowing layer) exist to paper over multiple
targets; committing to one target removes that entire class of conditional compilation and
lowest-common-denominator abstraction. The boundaries the head keeps are clean *module* boundaries
— justified by **reviewability**, so the security-relevant interpreter is readable in isolation —
not portability seams.

Portability is consciously traded for stability and minimal surface. A future port is a per-OS
native backend, not a portable abstraction retrofitted underneath.

## 2. The render layer

**A terminal grid is tile-compositing, not text rendering.** The protocol delivers *resolved
cells* — the box-side emulator has already resolved width, wrap, scroll, and grapheme composition
(`display.md` §0) — so the renderer's input is a grid of `(codepoint, fg, bg, attrs)` at fixed
positions, never a byte stream. There is **no parser in the render path**: control is unrepresentable
in the renderer because the renderer's vocabulary is geometry and a texture.

The stack:

| Layer | Choice | Note |
|---|---|---|
| Compositor | **Metal**, native | A glyph-atlas + colored-quad tile compositor. Chosen over a portable GPU abstraction for API stability: Metal is a decade-stable native API; a portable layer tracks an evolving spec and breaks on a quarterly cadence. |
| Rasterization | **Core Text**, native | Best-in-class hinting/AA at small sizes; the OS's irreplaceable craft. |
| Fonts | a **closed, curated set we ship** | Not OS font fallback. |

The closed font set is load-bearing twice over. It **bounds the glyph surface** — the box selects
*which glyph* by codepoint index, never *the glyph data* — so the classic "hostile font file →
rasterizer exploit" primitive is absent, and there is no open-ended fallback cascade for box
codepoints to steer. It also gives **render determinism**: the operator sees the same thing on
every machine.

### Emoji as glyphs

Emoji are rendered as **our own monochrome outline glyphs** (e.g. Noto Emoji), never color raster.

> *"If we replace emoji with our own mapping to glyphs, which are just standard font renderables,
> then we can sidestep all of the complexity of ever incorporating pixel-based rendering from
> proprietary file formats into our glyph grid."*
>
> Color-emoji formats each bolt a *second decoder* onto the outline path — `sbix`/`CBDT` carry
> embedded PNG (the FreeType CVE-2020-15999 path), `OT-SVG` an embedded SVG document. Outline emoji
> keep the **entire grid on the single bezier-fill decoder** already run for every letter. Zero new
> parsers, ever — image data made *unrepresentable* in the glyph pipeline, not filtered out.

A **default render color** for emoji is applied as a render-layer `codepoint → color` lookup,
attached to the glyph at atlas-population time — **below the serialization layer, never editing the
cell stream.** The stream remains a faithful record of what the box emitted; color is purely how
the head chooses to paint a glyph it already holds. The same table recovers the few emoji that
carry meaning *in* color (status dots, warning signs) by mapping those codepoints to their
canonical colors; the rest default to the cell foreground.

## 3. Input — capture and the event model

The head captures the operator's input and emits **structured input events**. It is a thin, mode-free
input surface: it does not encode terminal byte sequences, does not track terminal modes, and knows
nothing about the application receiving the input.

### Capture

Input is captured through **native AppKit** — an `NSView` implementing `NSTextInputClient` — not a
portable windowing library.

> A portable library on macOS is *implemented on* this same machinery; it has no capability AppKit
> lacks. `NSTextInputClient` **is** the native path for keyboard layouts, dead keys, and IME
> composition — the layer where we most want native fidelity, and the reason to go direct rather than
> wrap.

**IME composition stays head-side until commit.** Marked-text / preedit is a render-side overlay the
head draws at the cursor; only the *committed* text becomes an input event. The box and the
application never see composition — only clean committed input.

### The event model — symmetric with output

The protocol carries **clean structured input events**, exactly as it carries clean resolved cells.
Legacy PTY byte-encoding — escape sequences, control codes, and the terminal modes that select among
them — is a **contained box-side adapter**, the mirror image of the output emulator
([`../proxy/terminal.md`](../proxy/terminal.md) §1). Output converts legacy app bytes *up* into our
cell format; input converts our structured events *down* into legacy PTY bytes. In both directions
the wire is greenfield, and legacy lives only in an adapter at the box edge.

> *Rationale — design for the native case; contain the legacy.*
>
> *"The information of the architecture that we want is, in most cases, informed by that kind of
> minimal elegant solution to what we actually need and does not get bogged down by legacy PTY."*
>
> The extrapolation that fixes the design: the box-side backend is, eventually, a first-class terminal
> in its own right — native applications emitting our cell format directly and consuming structured
> input events directly. A native app needs no escape sequences; it needs the *event*. Designing input
> for that case now means the native future requires **no new architecture** — it is simply an
> application that speaks the protocol directly — while the legacy adapter keeps real apps working
> today.

The canonical event the head emits:

```
enum InputEvent {
    Key      { key: KeySym, mods: Modifiers, text: Option<String>, kind: Press | Repeat | Release },
    Mouse    { button: Button, cell: (row, col), mods: Modifiers, kind: Down | Up | Move | Scroll },
    Paste    { text: String },     // a first-class event, not a "bracketed paste" mode
    Focus    { gained: bool },     // a first-class event, not a focus-reporting mode
    Resize   { rows: u16, cols: u16 },
    Viewport { offset: i32 },
}
```

> The information content is the **Kitty keyboard protocol's** model — keysym, modifiers, event type,
> text — carried as a struct instead of escaped into `\e[…u`. The richness legacy terminals needed an
> extension to recover, a clean protocol has for free.
>
> **Modes dissolve.** Everything legacy handles through modes — DECCKM cursor-key encoding, bracketed
> paste, focus reporting, mouse reporting, key disambiguation — exists only because legacy smushes rich
> input through a narrow, lossy byte channel. In the structured model there is nothing to
> disambiguate; the event *is* the meaning. Mode-dependence does not vanish, it **relocates entirely
> into the legacy adapter** — the one component that needs the app's modes (learned from the app's
> output) to down-convert. The protocol and the head are mode-free.

Where the complexity lands:

| Component | Role |
|---|---|
| **Head** | OS event (`NSEvent` / `NSTextInputClient`) → `InputEvent`. Thin, mode-free, no encoding, no escape tables. IME preedit is a render overlay; only commits emit events. |
| **Box legacy adapter** | Tracks the app's terminal modes and converts `InputEvent`s *down* into legacy PTY bytes. All the legacy mess, contained, untrusted. |
| **Box native app** *(future)* | Consumes `InputEvent`s directly — no adapter, no modes, no loss. |

One split worth stating: **mouse divides in two.** Local selection/copy is head-side — it operates on
the grid the renderer already holds, with no box involvement. Mouse *reporting* to an app is a `Mouse`
event the box adapter encodes, with the same mode-dependence as keys.

### Containment

Input is **head → box, onto untrusted ground** — the safe direction
([`../proxy/terminal.md`](../proxy/terminal.md) §4). The head *generates* input; box bytes never drive
it, so input handling is off the box-reachable surface entirely. This is a pure-ergonomics decision
with **zero containment weight** — which is also why the legacy adapter's complexity is free: it runs
on the untrusted box, where elaboration cannot cost containment.

### Consequence for the protocol

`model.md`'s `key | bytes` verb is the legacy artifact showing through the wire. In the greenfield
model the input verb carries a **structured `InputEvent`**, and byte-encoding moves into the box-side
adapter beside the output emulator — making the box a clean converter in both directions and the head
a thin, mode-free surface onto it. This section specifies what the head *produces*; the `KeySym` /
`Button` / `Modifiers` enumerations and the verb encoding are the protocol effort's to define.

## 4. The security model — nested boundaries

The head's safety is a stack of boundaries, each eliminating a whole class of attack *before* the
next is reached. The proxy is the head's only network peer, and it terminates the adversarial link
and re-originates a trusted one.

| # | Boundary | Mechanism | Class eliminated |
|---|---|---|---|
| 1 | **Network** | Proxy terminate-and-re-originate; the box has no route to the head | Stack/scan/service attacks — the head is not network-exposed to the box |
| 2 | **Channel** | Proxy relays only the bounded protocol | Arbitrary bytes/packets — the box's sole influence is relayed payload content |
| 3 | **Representation** | The spec's inexpressiveness (assertion 12) | Injection — no control, no font data, no image blob is *representable* |
| 4 | **Interpretation** | Total recognize-or-drop (§0) | Parser exploits — there is no graceful-degradation soft middle |

What remains at the bottom is only: *is our interpreter of a bounded payload, from a trusted peer,
memory-safe?* — small, ours, reviewable.

> *"The proxy has made us safe from adversarial network attacks. Basically everything outside of
> the protocol and our interpretation of the protocol is the remaining unsafe portion, which we
> can't avoid because we need to communicate on some level."*

**Font injection is impossible**, not merely caught: the spec has no verb that carries font data,
so a font cannot cross — the same unrepresentability as control.

> *"Any of the traditional attack surfaces we were worried about, for example font injection, are
> just not possible."*
>
> The narrow residual — a box-chosen codepoint indexing *our* font into Core Text — is the leftover
> *after* injection is removed, not a counterexample to it: no hostile-font primitive, just a
> table-index influence into vetted outlines (§2).

**For display, the proxy is a relay, not a gate** (`display.md` §4). It passes display content
through untouched, so that content is box-authored even though the peer is trusted, and the head is
its *sole* interpreter. The head's interpreter therefore earns the memory-safety bar on its own
merits — robust against a box that may author a *malformed* frame, not merely an inert one.

## 5. The codec — assembly is not disassembly

Assembly and disassembly are distinct actions on distinct machines, and **no shared codec executes
on both.**

> *"The assembly of the protocol happens on the box … The disassembly only ever happens on the head
> UI. The actual shared surface would be sharing of protocol constants but not sharing of assembly
> versus disassembly."*
>
> The box runs an *encoder*; the head runs a *decoder*; these are different functions even in the
> same language. The head's display decoder is **net-new and head-only** — the proxy relays display
> without decoding it, the box encodes it. The only artifact genuinely shared between components is
> the **format definition** (op numbers, bit layouts, verb ids, the `0x1F` delimiter), which is
> cross-language-trivial and backed by conformance vectors.

The protocol codec is a **Rust crate co-locating encode and decode**, so it verifies itself:

- a **round-trip property** — `decode(encode(x)) == x` — proving encoder and decoder agree (the
  drift defense, made executable);
- a **fuzz harness over the decoder** — proving robustness. By construction it cannot find an
  *exploit* (safe Rust over a total recognizer yields a panic at worst, never UB), but it can find
  a *panic* on malformed input — a robustness bug worth fixing, since a box should not be able to
  crash the operator's UI with a malformed frame.

Placing the verified decoder in Rust means a seam between it and the native presentation layer.

## 6. The seam — IPC, not FFI

The head is **two processes**: a Rust codec and a Swift presentation layer, coupled by **IPC**
(macOS XPC). It is *not* an in-process FFI link. (The proxy↔head boundary is, separately, always a
network boundary; IPC is only this intra-head seam.)

> *"The value of IPC is the decoupling and the removal of a third (unsafe) language for coupling."*

Three properties justify it:

1. **Decoupling.** Two independent binaries, each built and tested in its own toolchain, linked
   only by a wire contract — no staticlib linking, no dual-language build.
2. **Fault isolation.** A codec panic dies in its own process; the presentation layer shows
   "reconnecting" and respawns it, rather than the operator's whole window going down.
3. **No third, unsafe language.** An FFI seam's substrate is **C**, and it *requires* `unsafe` on
   both sides — raw pointers across the boundary, `unsafe` Rust to deref them, Swift's `Unsafe*`
   pointer APIs to read them. IPC couples through a **byte stream in each language's own
   memory-safe idiom**: no C, no raw pointers crossing a language line, no required `unsafe` at the
   seam. The only `unsafe` that remains is the unavoidable, platform-blessed kind — Swift talking
   to Metal/AppKit/Core Text — not bespoke FFI to our own code.

> The deepest reason it fits: the intra-head seam becomes **the same shape as the rest of the
> system** — memory-safe peers talking a bounded byte protocol, never coupling through shared
> unsafe memory. The head is not an exception to the architecture; it is the architecture recursed
> one level in: `box ↔ proxy ↔ head`, and now `codec ↔ renderer`.

### The handoff

What crosses the seam is the **post-parse render-input** — the decomposition the renderer needs
anyway, not a redundant second wire format.

> *"Perhaps it gets us post-parse in a way that we already need to decompose it for render anyway.
> Structs at the handoff or something."*
>
> Because both binaries ship as a unit, this is an **internal struct contract** versioned by
> recompiling both sides — no public-protocol compatibility burden.

State placement follows from the IPC choice:

- The **Rust codec** owns parse, the apply-loop, and the authoritative per-pane grid (not scrollback —
  that lives box-side and is reached via the `viewport` verb), and pushes **resolved-cell** structs
  `{pane, [Cell{codepoint, fg, bg, attrs, pos}]}` (full grid or dirty region) on change.
- The **Swift renderer** is near-stateless: it holds no protocol state and no apply-loop, only the
  *current* displayed frame (for compositing and selection/copy). A presentation-layer restart is
  trivially recoverable — the codec re-pushes the current grids, with no round-trip to the box.
- Input crosses back as scalar structs (`key` / `resize` / `viewport`), which the codec encodes
  into head→box verbs.

## 7. Scrutiny placement

Mapping the head's parts onto the taxonomy of [`../proxy/terminal.md`](../proxy/terminal.md) §5:

| Component | Tier | Assurance |
|---|---|---|
| Rust codec (decode/encode) | box-reachable bounded interpreter | Memory-safe, total recognize-or-drop; round-trip + fuzz verified |
| The seam | byte protocol between safe peers | No required `unsafe`; an internal struct contract |
| Swift renderer | off-surface feature logic | A bug fails the *feature*, never containment |
| Swift ↔ Cocoa/Metal/Core Text | unavoidable native interop | The platform-blessed path, not bespoke FFI |

The shape worth noticing: the head's box-reachable surface is one small, fuzz-verified Rust
interpreter behind the proxy's network boundary; everything that touches the bulky native APIs is
off-surface, and the two halves couple without a line of bespoke `unsafe`.

## 8. Decided / Open

**Decided.** Native macOS, single-target. A **Swift presentation layer** (Metal tile compositor;
Core Text rasterization over a closed, shipped font set; outline glyph-emoji with a render-layer
color lookup; AppKit `NSTextInputClient` capture emitting **structured input events**, with legacy
byte-encoding contained in a box-side adapter) and a **Rust protocol codec** (co-located
encode+decode, round-trip + fuzz verified), as **two processes coupled by IPC (XPC)** carrying
post-parse resolved-cell structs. The codec owns parse + apply-loop + grid; the renderer is
near-stateless. Cross-platform is sacrificed for stability and minimal surface.

**Open.**

- The `InputEvent` model is specified from the head's side (§3); its `KeySym` / `Button` /
  `Modifiers` enumerations and the input verb encoding are the protocol effort's to incorporate,
  superseding `model.md`'s `key | bytes`.
- The exact handoff struct shapes, dirty-region strategy, and restart-resync — to firm up against
  the implementation rather than ahead of it.
- Single-threaded decode (on the runloop) vs a worker, decided by responsiveness under heavy pane
  output; viable single-threaded given the system is not throughput-bound.
- The emoji color-table scope (which codepoints carry canonical color).
- Native Core Text rasterization is the choice; a pure-Rust rasterizer remains the noted
  counterweight (it sidesteps the TrueType hinting bytecode VM entirely), not reopened unless text
  crispness or surface concerns demand it.

---

*The head displays and never interprets: a bounded recognizer of resolved cells, behind a proxy
that gives it no adversarial network and a protocol that cannot represent control, a font, or an
image. It is two memory-safe peers — a Rust codec that parses and holds the grid, a native Swift
layer that paints it — coupled by a byte stream and not a single line of bespoke unsafe glue, the
whole system's shape recursed one level into the operator's own machine.*
