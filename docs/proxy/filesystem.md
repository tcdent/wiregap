# The Filesystem Subsystem

### Moving files between the box and trusted-host storage

> An exploration document for the proxy's filesystem-transfer subsystem — the channel that
> moves files between the box and the trusted host. Companion to the cloud subsystem docs
> ([`cloud/approach.md`](./cloud/approach.md)) and the foundational *principles*; grounded in
> the spine (principally assertions 6, 7, 12, 15). Research and theory informing the design,
> not the specification.

---

## 0. The shape: the trust-direction binary, mirrored

The filesystem subsystem splits on the same axis as the cloud subsystem, just relabeled —
because both obey one rule (console §1.1): *bytes landing on trusted ground are the dangerous
direction; bytes landing on the box are free.*

- **Write path (box → trusted host)** — the box deposits files onto trusted ground. The
  dangerous direction; **gated**.
- **Read path (trusted host → box)** — the box pulls files into itself. The safe direction;
  **not gated, only addressed**.

One subsystem, one real gate (write), and a read path that is structurally free. The cloud
subsystem's egress/ingress asymmetry in different clothes.

---

## 1. The write gate — confine the destination, never the bytes

The bytes are written **verbatim** and never inspected: writing bytes interprets nothing as
code, so the payload is safe by construction. The only box-controlled input the proxy acts on
is the **destination**, and that is the whole security argument.

The destination is **not a path string** — it is an **array of leaves**. The proxy supplies
every separator and the quarantine root; the box supplies only leaf names. Each leaf is
validated by a total function over its characters — the byte-map's sibling:

> A leaf is valid iff every character is in `[A-Za-z0-9._-]` **and** it contains at least one
> alphanumeric.

That at-least-one-alphanumeric requirement is the entire traversal defense: `..` and `.` are
not valid leaves, so "parent" and "current" cannot be *said*; and because the proxy owns the
separators, the box never gets a `/`. Traversal is **unsayable, not caught**. (The proxy also
owns a sane element-count bound and a require-at-least-one-element rule — frame well-formedness,
not intent-reasoning.)

This is the array-of-leaves move: the filesystem analog of the cloud route's exact-match
whitelist. Same posture, different surface.

---

## 2. The quarantine — one invariant

Confining the destination is only half; *where* the bytes land must also be safe. That reduces
to a single invariant:

> **The proxy never executes in quarantine.**

Whether the proxy can *read* from quarantine is immaterial — bytes are bytes. A `.bashrc`, a
systemd unit, a config file deposited there are all inert if nothing runs them. The box can
write nothing outside quarantine (the leaf-validator confines it), and nothing inside
quarantine executes. Those two facts are the complete write-path security story.

---

## 3. The read path — free, and interpreter-free

Reading is harmless regardless of what is read: the box may acquire any information (principles
§7), and a secret matters only when *used*, which the box structurally cannot do. So the
leaf-addressing on the read path is **addressing, not a security boundary** — the
direction-flip from the write path. A logic bug there fails *safe* (the box reads some other
file — harmless); it is memory-safe (it parses box input) but it is **not** part of the
security-invariant core.

Like the cloud subsystem's response relay, the read path exists to keep an **interpreter off
trusted ground**. The naïve way to make trusted-host files readable to the box is a web server
(nginx) — a full request-interpreter, an RCE surface, on the host whose integrity is the whole
game. Instead the read service is the dumb mirror of the write channel: *open one file, stream
its bytes.* No HTTP, no routing, no parser. (Same reason the write path refuses rsync / scp /
upload servers: each is a program on the trusted side that acts on box-supplied instructions;
the bespoke channel exists precisely because nothing off-the-shelf is non-interpreting enough.)

---

## 4. Surface, seam, and state

- **Box-reachable security core contributed by this subsystem: one gate** — the write-path
  leaf-validator. The read addressing is off the security surface (memory-safe, fails safe).
  Minimal, and a mirror of the cloud subsystem (one gate, the rest free).
- **The quarantine is the interface to promotion, and promotion is out of scope.** The
  subsystem deposits inert bytes and *stops*. Whatever reads the quarantine, shows a human the
  diff, and crosses an artifact into a privileged environment is separate downstream tooling —
  the structural human-in-the-loop. This subsystem owns the quarantine as a *write target*; it
  does not promote.
- **State is minimal:** the quarantine root and the readable root (config). Transfers are
  transient and unattributed — a filesystem snapshot at a point in time, not held.

*(File chunking and offset reassembly for large transfers are the substrate/protocol's concern;
from this subsystem's view it writes verbatim bytes at an offset.)*

---

## 5. Operational, not security

Two knobs carry no security weight:

- **Quarantine structure** — flat, or scoped per box / session / project. The leaf-array
  already lets the box build subdirectories; whether the *root* is partitioned is an
  operational and provenance choice. The validator confines either way.
- **Readable-root scope** — what the read service exposes. Reading is harmless, so this is
  sanity, not safety; a narrow, operator-curated readable root keeps the box's context and the
  read surface legible.

---

## Summary

The filesystem subsystem is **two facts and a character class**:

1. The write gate — the leaf-validator (`[A-Za-z0-9._-]`, at least one alphanumeric) — confines
   every box write to the quarantine root; traversal is unsayable.
2. The proxy never executes in quarantine.

The read path is the safe direction: addressed, not gated, and interpreter-free. Everything
else — quarantine structure, readable scope — is operational.
