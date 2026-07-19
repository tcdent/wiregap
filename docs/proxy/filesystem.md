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

**The leaf rule confines what the box can *say*; a second floor must confine where the bytes can
*land*.** Textual validation is necessary but not sufficient — a symlink anywhere in the path
could redirect the actual write out of the quarantine without the box ever saying `..`. So the
write needs a filesystem-level floor as well: the open must resolve *strictly beneath* the
quarantine root, so that no symlink or `..` at **any** component — not merely the last — can escape
it, and no mount boundary is crossed. This is where the naïve primitive fails: `O_NOFOLLOW` refuses
a symlink only at the *final* component (`open(2)`), leaving an intermediate-component symlink free
to redirect the write out. The primitive that actually closes the gap is **`openat2` with
`RESOLVE_BENEATH`** (resolution confined beneath the root descriptor), narrowed further by
`RESOLVE_NO_XDEV` (no mount crossing) and no-symlink resolution; the leaf is created no-overwrite
(`O_CREAT | O_EXCL`), so a name collision fails closed rather than silently overwriting. With that
floor, traversal is *unsayable* (the leaf rule) **and** symlink-escape is *unreachable* (the
resolution discipline) — the two floors a confined write needs. (Primitives settled in
filesystem-review; ledger L6.)

This is the array-of-leaves move: the filesystem analog of the cloud route's exact-match
whitelist. Same posture, different surface.

---

## 2. The quarantine — inert by construction

Confining the destination is only half; *where* the bytes land must also be inert. The invariant
is firm: **nothing on the trusted side executes in — or auto-processes — quarantine.** Reads of it
are immaterial (bytes are bytes, and the proxy's own configuration and core live elsewhere, so it
never sources its instructions *from* the quarantine); a `.bashrc`, a systemd unit, a config
deposited there are inert if nothing runs them. What the invariant rules out is not just the proxy
executing there, but *anything* on the trusted side that would read-and-interpret box content —
**no indexer, loader, or watcher** (the same detonation risk as rendering a box-content log in a
real terminal). Deposit-only ground until a human promotes from it.

*How* that inertness is enforced is a design-review question, not settled here. The intended
approach — to confirm, not take as final — is to mount the quarantine `noexec,nosuid,nodev` so
"nothing executes here" is a kernel-enforced floor rather than the proxy's restraint, and to point
no auto-indexing service at the region. The *requirement* (inert) is the assertion; the *mount
flags* are the likely way to meet it.

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

*(File chunking and reassembly for large transfers are the substrate/protocol's concern; from this
subsystem's view it appends verbatim bytes in order at the running cursor — the wire carries no
offset (`proto/file.md` §1).)*

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

The filesystem subsystem is **two structural floors and a character class**:

1. The write is confined to the quarantine on two floors — *textually*, by the leaf-validator
   (`[A-Za-z0-9._-]`, at least one alphanumeric → traversal unsayable), and at the *filesystem
   level*, by an open discipline that keeps a symlink from redirecting the write out (§1;
   primitives pending design review).
2. The quarantine is **inert by construction** — nothing on the trusted side executes in or
   auto-processes it (§2; enforcement mechanism pending design review).

The read path is the safe direction: addressed, not gated, and interpreter-free. Everything
else — quarantine structure, readable scope — is operational.
