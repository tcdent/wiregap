# The File Service — Wire Protocol

> The box↔proxy and head↔box file operations — `file-write`, `file-read`, `list`, and `request-file`.
> All address by the one segment-stack format of [`model.md`](./model.md) §6. The proxy's
> **quarantine**, the box's filesystem, and the trusted-host **readable root** are the grounds.
>
> **Reading convention.** Plain text and tables are normative; `>` blockquotes are commentary.

---

## 0. Addressing — one segment stack, safe or unsafe

Every name and path is a **segment stack**: `0x1F`-delimited segments, **scanned** — every byte
verified against the segment's alphabet, or the message is rejected — and joined by the receiver under
its own root, so `/` and `..` are unsayable and **traversal is structural, not caught.** The scan
**verifies; it never transforms.** A segment is one of two kinds:

- **safe** — verified against the restricted alphabet `[A-Za-z0-9._-]`, ≥1 alphanumeric. **Loggable**:
  these bytes cannot be a terminal escape even in a log piped straight to a terminal. For names the
  box **chooses** — a quarantine destination.
- **unsafe** — verified only against the permissive structural rule (any byte but `/`, `0x1F`, NUL;
  never `"."`/`".."`). Traversal-safe, but **not** verified safe to log — rendered inertly by the
  head, never written to a log. For **real filenames** — sources, listings, entry names.

> *Rationale.* The split is a **type**: `SafeSegment` vs `UnsafeSegment`, and the logger accepts only
> `SafeSegment`, so a real name *cannot* reach a log line — a compile error, not a discipline. Both
> are the same scan and `0x1F` structure; they differ only in the alphabet the scan enforces and in
> whether the result is verified safe to log. A real filename holds spaces and Unicode, so it can't
> meet the restricted alphabet — but it is still traversal-safe, and safe to *display* (the head
> interprets no control), just never *logged*.

## 1. `file-write` — box → quarantine

The box pushes bytes onto trusted ground. The session is occupied for write, declaring a **destination**
— a **safe** segment stack under the quarantine root (the box chooses a conforming name; it is logged).
What follows is `file-chunk` messages: **verbatim bytes, appended in order** at the running position.
Completion is the transport session ending — the box ceasing to emit on it; there is no protocol close
signal.

> *Rationale.* Files are serial, so the write cursor is implicit — no offset, no field to validate, no
> confinement argument. The proxy moves bytes to the cursor; the only structured input is the safe
> destination it acts on. (Parallel / offset writes are retired, re-admittable by deliberate revision.)

## 2. `file-read` — box ← trusted host

The box names a **source** — an **unsafe** segment stack under the trusted-host readable root (a real
filename: traversal-safe, not logged). The proxy streams the file's bytes back as `file-data`, a
**verbatim** relay, opening with `O_NOFOLLOW` symlink discipline.

> *Rationale.* The response lands on the box (untrusted), so the proxy neither reads nor neutralizes
> it; disclosure isn't defended. The source is a real name, hence unsafe — confined but never logged.

## 3. `list` — bidirectional

One format, **both directions**: `list` (request a directory) → `list-entry` (the entries). The
direction is just who opened the session — box→proxy lists the **host**, head→box lists the **box**.
Enumeration uses native `read_dir`, **never `ls`**.

- **request** — `list`, an **unsafe** segment stack (the directory; a real path).
- **response** — a run of entry records, each `[fixed metadata][unsafe name]`:

| field | bytes | kind |
|---|---|---|
| `size` | 8 | `u64` |
| `mtime_secs` | 8 | `u64` |
| `mtime_nanos` | 4 | `u32` |
| `type_flags` | 4 | `u32` (low byte: `0` file / `1` dir / `2` symlink) |
| `name` | var | **unsafe** segment, `0x1F`-terminated |

Symlinks are reported, not followed. Records run to the frame end.

> *Rationale.* Directory and names are real, hence unsafe — display-only, never logged. It is the
> protocol's one repeated combined record (fixed metadata + a scanned name), legal under model.md
> law 10.

## 4. `request-file` — head → box (triggers a write)

The operator's extraction trigger. It names a **source** — an **unsafe** segment stack on the box (a
real filename; the box reads its own filesystem). The box reads it and opens a `file-write` session
whose quarantine **destination** is a **safe** segment stack (§1).

> *Rationale.* Real source named here (unsafe), safe destination declared there; the bytes leave the
> box only as a verbatim read.

## 5. `file-error`

Any operation that does not succeed — missing path, permission, full disk — is one `file-error`, the
shared error body (`model.md` §8), flowing from whoever serves the operation.

## 6. Decided / Open

**Decided.** One segment-stack format, two kinds (safe = verified-loggable / unsafe = real names,
display-only) as a type split with the logger accepting only safe; `file-write` destination safe +
verbatim appended chunks (no offset, files serial); `file-read` / `request-file` sources unsafe;
bidirectional `list` over an unsafe directory + `[metadata][unsafe name]` entries; native `read_dir`;
the shared `file-error`.

**Open.** The `type_flags` bits beyond the three file types; whether `list` carries a leading entry
count or runs to the frame end (currently rest-of-frame).

---

*One stack addresses everything: a safe alphabet where a name is chosen and logged, an unsafe one
where it is real and only shown. Bytes flow verbatim in and out, directories come back as fixed
metadata plus a scanned name, and a real filename can be displayed but never detonate a log.*
