# The Net Service — Wire Protocol

> The box↔proxy wire contract for an outbound network call. The proxy's native handling — recognizing
> the payload, building the authenticated TLS HTTP request, calling the upstream — is off-wire
> ([`../proxy/cloud/approach.md`](../proxy/cloud/approach.md)). Companion to [`model.md`](./model.md).
>
> **Reading convention.** Plain text and tables are normative; `>` blockquotes are commentary.

---

## 0. Shape

A network call is a set of **unidirectional messages** — the box emits the request, the proxy emits the
response; the protocol neither pairs nor correlates them (that is the transport's, `model.md` §9). Its
messages:

- `net-address` (box→proxy) — the request's address.
- `net-payload` (box→proxy) — the request's verbatim body; one or more chunk messages, each with a
  fixed-width continuation-flag header, **reassembled by the proxy** before recognition (§2).
- `net-response` (proxy→box) — the upstream response, verbatim; streamed frame-by-frame, **not**
  reassembled by the proxy (§3).
- `net-error` (proxy→box) — any non-response outcome.

Address and payload are **separate messages** because they are delimited differently — the address is
a scanned stack, the payload a verbatim blob (`model.md` law 11).

## 1. `net-address` (box → proxy)

A fixed `scheme` prefix, then a **safe segment stack** (`model.md` §6, file.md §0) — the box supplies
only the components, the proxy supplies `://` and `/`:

| field | kind |
|---|---|
| `scheme` | `u32` enum (`0` https; reserved) |
| address | **safe** segment stack — first segment the domain (dots within it), the rest the path |

The proxy matches the assembled address against the route register and **logs** it.

> *Rationale.* The same scanned `0x1F` stack as a file path, on the safe (restricted, loggable)
> alphabet — so a separator or control byte is unrepresentable and the logged address is safe by
> construction. The box constructs a conforming URL; a non-conforming one is denied. Decomposition
> isn't for traversal (an exact-matched URL escapes nothing) — it's for the log guarantee.

## 2. `net-payload` (box → proxy)

The request's **verbatim** body — query, headers, body. It is **not** assumed to fit one frame: a real
request (the Anthropic worst case is a multi-kilobyte POST) routinely exceeds the wire frame, so the body
is sent as **one or more `net-payload` chunk messages**, each carrying a fixed-width **continuation
flag** as its header prefix, and the **proxy reassembles the whole request before the recognizer reads
it.** The body shape (`model.md` §6's `[fixed-width prefix][one frame-bounded tail]`):

| field | kind |
|---|---|
| continuation flag | one byte — `0x00` = more chunks follow, **any nonzero** value = this is the final chunk |
| content | this chunk's **verbatim** content tail (the flag prefix stripped before reassembly) |

The proxy decodes each chunk, strips the 1-byte flag, and **concatenates the content** into the request
buffer; the flag being nonzero ends the request. An empty body (no flag byte) is a typed reject
(`ShortBody`), never a panic. Reassembly is **mandatory, not an optimization** — recognition is *total
over a complete request* (`../proxy/cloud/approach.md` §3); a partial request cannot be recognized, only
denied. The reassembly buffer is **bounded** — a request whose chunks never carry a final flag, or that
exceeds the cap, is denied; it **fails closed.** Never represented, never logged; the reassembled bytes
are read off-wire by native code, which recognizes the HTTP request and forwards it verbatim or denies.

> *Rationale.* This is the **one trusted-side reassembly** in the protocol. Everywhere else large data is
> many independent single-frame messages (a file is appended chunks, a repaint a fresh keyframe), so the
> trusted side holds no buffer. The recognizer cannot be: total over the request, it must hold the whole
> request to rule on it. The continuation flag is a **structural, fixed-width header field**, never a
> value searched for *inside* the content (law 11) — so a box-authored content byte can never be mistaken
> for the boundary, and adversarial content never steers the reassembly. (An earlier framing spoke of a
> "transport continuation bit"; no such transport field exists — the boundary lives in each message's own
> header, confirmed at the Sprint-6 reassembly finding.) The content concatenates **opaquely**; on the
> wire it is opaque, and the dangerous bytes never enter the representation a log draws from, so the log
> holds only the safe address.

## 3. `net-response` (proxy → box)

Upstream response bytes, **verbatim**, streamed frame-by-frame; the stream ends when the proxy stops
emitting (a transport event, not a protocol close). Relayed untouched — it lands on the untrusted box.

> *Rationale — the asymmetry.* The proxy does **not** reassemble the response. Unlike the egress
> payload, nothing inspects it (§2 reassembles only because the recognizer is total), so each frame is
> relayed as it arrives and the **box** reassembles, never the proxy. The sensitive direction buffers;
> the free direction streams.

## 4. `net-error` (proxy → box)

Any outcome that is not an upstream response — refusal by the recognizer, unreachable, dropped,
timeout, malformed — is one `net-error`, the shared error body (`model.md` §8).

## 5. Off the wire (native, not specified here)

In `proxy/cloud`: the route register and match, the request recognizer, the TLS encapsulation,
credential and redirect policy. The wire carries a safe address (logged), a verbatim payload, and a
verbatim response — nothing more.

---

*A network call is a few unidirectional messages: a safe, logged address; a verbatim payload the proxy recognizes
off-wire and forwards or denies; a verbatim response relayed to the box. The address is the one
box-authored thing the proxy keeps; the rest passes through and is gone.*
