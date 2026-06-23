# The Net Service — Wire Protocol

> The box↔proxy wire contract for an outbound network call. The proxy's native handling — recognizing
> the payload, building the authenticated TLS HTTP request, calling the upstream — is off-wire
> ([`../proxy/cloud/approach.md`](../proxy/cloud/approach.md)). Companion to [`model.md`](./model.md).
>
> **Reading convention.** Plain text and tables are normative; `>` blockquotes are commentary.

---

## 0. Shape

A network call is **one session**, bidirectional, correlated by the session id. Its messages:

- `net-address` (box→proxy) — the request's address.
- `net-payload` (box→proxy) — the request's verbatim body.
- `net-response` (proxy→box) — the upstream response, verbatim.
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

The request's **verbatim** body — query, headers, body — its own message, bounded by the frame and
session. Never represented, never logged; native code reconstructs the HTTP request and recognizes it.

> *Rationale.* The recognizer (the worst-case trusted surface) reads it off-wire and either forwards
> it verbatim or denies. On the wire it is opaque — the dangerous bytes never enter the representation
> a log draws from, so the log holds only the safe address.

## 3. `net-response` (proxy → box)

Upstream response bytes, **verbatim**, streamed, ended by the session close. Relayed untouched — it
lands on the untrusted box.

## 4. `net-error` (proxy → box)

Any outcome that is not an upstream response — refusal by the recognizer, unreachable, dropped,
timeout, malformed — is one `net-error`, the shared error body (`model.md` §8). Ends the session.

## 5. Off the wire (native, not specified here)

In `proxy/cloud`: the route register and match, the request recognizer, the TLS encapsulation,
credential and redirect policy. The wire carries a safe address (logged), a verbatim payload, and a
verbatim response — nothing more.

---

*A network call is one session: a safe, logged address; a verbatim payload the proxy recognizes
off-wire and forwards or denies; a verbatim response relayed to the box. The address is the one
box-authored thing the proxy keeps; the rest passes through and is gone.*
