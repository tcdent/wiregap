# Filesystem Write-Path — Primitives Spec

### The open-discipline floor and the quarantine inertness mechanism

> **Spec, not exploration** — settles the primitives `filesystem.md` §1–§2 leave as
> sketches (architecture ledger L6, L7). Sprint `docs/teams/proxy/002-*.md`, Slice 1;
> status: **awaiting SA1–SA7 gate; judgment calls routed, not yet ratified — not frozen.**
>
> **Bench pin (not a deploy-host pin):** all primitive behavior verified by probe on
> `elsenor`, Debian 12, kernel `6.12.74+deb12-amd64`, 2026-07-18 (§7). No proxy deployment
> host is designated; re-verification on the real deploy host is a standing dependency.
> Kernel gates: `openat2(2)` ≥ 5.6; `nosymfollow` ≥ 5.10 — both cleared with margin.
> **Currency:** derived from the working-tree `filesystem.md` (modified, uncommitted).

---

## 0. The requirements (verbatim anchors — SA1)

From `filesystem.md` §1: the write must stay **"structurally rooted in the real quarantine
subtree no matter what the path contains, so that traversal is *unsayable* (the leaf rule)
and symlink-escape is *unreachable* (the open discipline)"**, with leaf creation
**no-overwrite-by-default**. From §2: **"nothing on the trusted side executes in — or
auto-processes — quarantine"** — no indexer, loader, or watcher; deposit-only ground until
a human promotes.

The leaf rule itself (`[A-Za-z0-9._-]`, ≥1 alphanumeric, element-count bound,
≥1 element) is Sprint-1-accepted ground (`architecture.md` §3 tier 1) and is **not
reopened here**; this spec is the second floor beneath it and the ground it lands on.

**Correction this spec supersedes (routed, not edited):** `filesystem.md` §1's sketch —
"refuse to follow a symlink at any component (`O_NOFOLLOW`)" — overcredits the primitive.
Per `open(2)` on the bench host, verbatim: *"If the trailing component (i.e., basename) of
pathname is a symbolic link, then the open fails, with the error ELOOP. Symbolic links in
earlier components of the pathname will still be followed."* The sketch as written does
not meet its own any-component requirement; `RESOLVE_NO_SYMLINKS` (§1 below) is what does.
The `filesystem.md` wording fix routes to its owner via the PM (sprint contract RQ2).

## 1. The open discipline (normative)

### 1.0 Startup, once, before any box frame is served — all fail-closed

1. **Pin the root:** `root_fd = open(quarantine_root, O_PATH|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC)`.
   Failure → the proxy does not serve filesystem writes. The root path string is never
   re-resolved afterward; a root change is a restart (interaction with L12 stated, not
   settled). An `O_PATH` descriptor is a valid `dirfd` for the `*at` calls below.
2. **Probe the primitive:** one benign `openat2` call (`"."` under `root_fd`, the §1.2
   resolve mask). `ENOSYS` → refuse writes. One reviewed path; **no fallback walk** (J1).
3. **Assert the mount floor** (§2.2).
4. Set `umask(077)` (provisional convention, marked).

### 1.1 The resolve mask (every `openat2` call in this spec)

```
RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV
```

Host-verbatim contracts (§6 for the full table): `RESOLVE_BENEATH` — *"Do not permit the
path resolution to succeed if any component of the resolution is not a descendant of the
directory indicated by dirfd"* (absolute paths rejected); `RESOLVE_NO_SYMLINKS` —
*"Disallow resolution of symbolic links during path resolution"* (any component; implies
`RESOLVE_NO_MAGICLINKS`, kept explicit anyway); `RESOLVE_NO_XDEV` — *"Disallow traversal
of mount points during path resolution (including all bind mounts)."* The kernel enforces
the mask **during** resolution — there is no validate-then-open window (§3, row 5).

### 1.2 The write, as a total function

Input: the validated element array `E = [e1 … en]` (n ≥ 1, from the Sprint-1
leaf-validator) and the transfer's chunk stream. Output: **`Committed | Denied(reason)`**
— every path through the algorithm ends in exactly one of the two; any syscall outcome
not explicitly admitted below denies. No retry-with-modification exists.

1. `cur ← root_fd`.
2. For each intermediate `e1 … e(n-1)`:
   a. `mkdirat(cur, ei, 0700)` — admit success or `EEXIST`; any other errno → Denied.
   b. `cur ← openat2(cur, ei, flags=O_RDONLY|O_DIRECTORY|O_CLOEXEC, resolve=§1.1 mask)`
      — failure → Denied. (`EEXIST`-tolerance in (a) is safe **because** (b) re-checks
      under the mask: a symlink there → `ELOOP`; a regular file → `ENOTDIR`; probes
      P2/P9.)
3. Leaf: `fd ← openat2(cur, en, flags=O_WRONLY|O_CREAT|O_EXCL|O_CLOEXEC, mode=0600,
   resolve=§1.1 mask)` — failure → Denied; `EEXIST` is the no-overwrite collision
   surfacing by design (probes P6/P7/P8).
4. **One fd per transfer:** the fd is held for the transfer's lifetime; chunks land via
   `pwrite(2)` at their offsets; close on completion or the file layer's own timeout
   (layering per `nic/architecture.md` §8 — the file layer owns fd/offsets/timeout). The
   leaf is **never reopened mid-transfer** — this is what makes `O_EXCL` compatible with
   `filesystem.md` §4's write-at-offset reassembly, and it forecloses the
   same-transfer-rewrite case mechanically (J-coll).
5. Every `Denied` is surfaced legibly. Leaf elements are safe segments (loggable by
   construction, `docs/proto/file.md`), so the denial may name the path with no
   neutralization pass.

Side effects on failure are inert and stay: intermediate directories already created, and
partial deposits on timeout, persist as bytes on deposit-only ground (§4, R-F).

A structural strength worth stating: the write path's only create-primitives are
`mkdirat` (a directory) and `openat2(O_CREAT|O_EXCL)` (a regular file, `0600`) — no
`symlinkat`, `linkat`, or `mknodat` exists anywhere on the path, so the box **cannot
create** a symlink, hardlink, device node, or setuid file in the quarantine at all. The
§3 closure rows that deny symlink/hardlink/devnode attempts therefore defend against
*environmentally planted* artifacts only; a *box-planted* one is unconstructible.

## 2. Quarantine inertness (normative)

### 2.1 The mount floor (kernel-enforced)

The quarantine root is a **dedicated filesystem** mounted only there, options:

```
noexec,nosuid,nodev,nosymfollow
```

`noexec`/`nosuid`/`nodev` per the `filesystem.md` §2 sketch. **`nosymfollow` is an
addition** (J2): it blocks symlink-following on the mount for **every** process — covering
trusted-side readers (the promotion human's tools) that the write-side §1.1 mask cannot
reach (kernel ≥ 5.10; `MOUNT_ATTR_NOSYMFOLLOW` present in the bench headers, §7).
Dedicated-fs is load-bearing beyond the flags: it makes a hardlink to an outside inode
impossible (hardlinks cannot cross filesystems) and makes any mount appearing inside the
subtree an ops violation `RESOLVE_NO_XDEV` refuses to traverse anyway. Provisioning
detail (partition/LV/image) is operational; the fs is provisioned case-sensitive (no
casefold — §4, R-G).

### 2.2 The startup assertion (new convention — J3, routed)

At startup the proxy checks its own floor: `statvfs(quarantine_root)` must show
`ST_NOEXEC|ST_NOSUID|ST_NODEV` in `f_flag`, else the proxy refuses to serve filesystem
writes. Mechanism verified observable on the bench (§7: `f_flag` mirrors real mount
options exactly). `nosymfollow` is **not** `statvfs`-visible; its verification method is a
routed sub-fork (J3b — recommended default: a single-line `/proc/self/mountinfo` check).
Until ruled, the assertion covers the three visible flags and the gap is a named residual
(R-C).

### 2.3 The no-auto-processor invariant (operational — honestly not kernel-enforced)

`noexec` blocks `execve` of files on the mount. It does **not** block an interpreter
invocation — `bash /quarantine/x` reads bytes as a program regardless of the flag. The
invariant "nothing trusted-side reads-and-interprets quarantine content" is therefore
closed by **host configuration, not the mount**, as an enumerable checklist:

1. `updatedb`/`plocate` `PRUNEPATHS` includes the quarantine root.
2. No systemd `.path`/`.timer`/`.automount` unit targets the subtree.
3. No trusted-side `inotify`/`fanotify` consumer watches it.
4. No indexer, thumbnailer, AV/content scanner, or backup agent that parses file
   *content* touches it (a content scanner is an interpreter; `filesystem.md` §2 reads
   remain immaterial — it is read-and-*interpret* that is barred).
5. No shell profile, cron entry, or unit file references quarantine paths.
6. The promotion tooling is the sole sanctioned reader — human-mediated, out of scope
   (`filesystem.md` §4).

The split, stated honestly: **flags = kernel-enforced floor, checked at startup;
checklist = operational invariant, documented and curated, not enforceable by the proxy.**

## 3. Adversarial closure (SA3)

Attempt → denying floor → observed evidence (probe IDs from §7; all PASS on the bench):

| # | Adversary's attempt | Denied by | Evidence |
|---|---|---|---|
| 1 | `..` / `.` as an element | Unsayable at the leaf-validator (≥1-alnum) — never reaches a syscall | Sprint-1 ground; belt: P4 (`EXDEV`) |
| 2 | `/`, absolute path | Unrepresentable in the leaf alphabet; belt: `RESOLVE_BENEATH` | P5 (`EXDEV`) |
| 3 | Symlink as final component | `RESOLVE_NO_SYMLINKS`; at leaf-create, `O_EXCL` refuses the existing name | P3 (`ELOOP`), P6 (`EEXIST`) |
| 4 | Symlink as intermediate component — the sketch's gap | `RESOLVE_NO_SYMLINKS` (this is exactly what `O_NOFOLLOW` alone missed) | P2 (`ELOOP`) |
| 5 | TOCTOU: swap a dir for a symlink after validation, before open | No window exists — the mask is enforced *during* kernel resolution; the algorithm has no separate validate step | `openat2(2)` contract, §6 |
| 6 | Magic link (`/proc/self/fd/…`) | `/` unsayable; belt: `RESOLVE_NO_MAGICLINKS` (also implied by `NO_SYMLINKS`) | P10 (`ELOOP`) |
| 7 | Mount/bind-mount crossing inside the subtree | `RESOLVE_NO_XDEV` — "including all bind mounts"; dedicated-fs makes an inside mount an ops violation besides | §6 contract; P4/P5 errno class |
| 8 | Hardlink escape to an outside inode | Impossible cross-filesystem; dedicated fs (§2.1) makes every hardlink target in-quarantine | Reasoning, §2.1 |
| 9 | Overwrite an existing deposit (or squat its name) | `O_CREAT\|O_EXCL` — collision fails and surfaces | P7 (`EEXIST`), P8 (fresh create OK) |
| 10 | Regular file planted as an intermediate "directory" | `O_DIRECTORY` in the descent open | P9 (`ENOTDIR`) |
| 11 | Deposited `.bashrc` / unit file / binary becomes execution | Inertness: `noexec` (direct exec), `nosuid`, `nodev`; checklist items 2/5 (nothing loads/references it); box cannot create device nodes or set modes (only §1.2 creates: regular, 0600) | §2; R-A names the interpreter residual |
| 12 | Planted symlink detonates a trusted-side *reader* (promotion tools) | `nosymfollow` mount-wide | §2.1 (J2) |
| 13 | Depth/count/size exhaustion | Leaf-validator bounds (count/length) + bounded chunking; worst case is a denied write or `ENOSPC` — a feature failure, never containment | Sprint-1 ground; §1.2 totality |

## 4. Residuals — named and bounded (SA4)

- **R-A — interpreter invocation.** `noexec` cannot stop `interpreter /quarantine/x`.
  Bound: checklist §2.3 (no trusted-side process invokes interpreters on quarantine
  content) + promotion discipline (the sole sanctioned read path is human-mediated).
  Worst case requires a trusted-side actor actively violating the stated invariant.
- **R-B — the no-auto-processor invariant is operational.** The proxy cannot enforce host
  configuration; it checks the flag floor (§2.2) and documents the rest. Bound: the
  checklist is enumerable and short; drift is auditable by reading it against the host.
- **R-C — `nosymfollow` not covered by the startup assertion** until J3b is ruled. Bound:
  the write path never follows symlinks regardless (§1.1); the gap concerns reader-side
  belt only, and the mount option is still set — only its *assertion* is pending.
- **R-D — both floors rest on kernel enforcement** (`RESOLVE_*`, mount flags). This
  intensifies ledger **L11** (kernel excluded from the reviewable core yet the enforcement
  substrate here); stated, tied to L11's existing design+policy routing — no new row.
- **R-E — bench pin, not deploy pin.** All evidence is from `elsenor` 6.12.74;
  re-verification on a designated deploy host is a standing dependency (banner).
- **R-F — inert litter.** Failed/timed-out transfers leave empty intermediate dirs and
  partial deposits. Bound: bytes on deposit-only, inert ground; cleanup is operational.
- **R-G — casefold.** If the dedicated fs were casefold-enabled, distinct-case names
  collide → `EEXIST` → a legible deny, never an overwrite (fail-closed either way).
  Provisioning pins case-sensitive (§2.1); belt: the failure mode is still a deny.
- **R-H — the root pin's own path.** §1.0 step 1 opens `quarantine_root` with
  `O_NOFOLLOW`, which covers **only the final component** (§0's own correction): symlinks
  in the root path's intermediate components are followed at pin time. This safety is
  **operator-config trust, not the mechanism** — the root path is operator-authored
  (adversary class: none; the box cannot influence it, L12 interaction) — and is stated
  here so no reader credits the root pin with any-component protection. Hardening
  (resolving the root via `openat2`+`RESOLVE_NO_SYMLINKS` from a pinned ancestor) would
  constrain legitimate operator paths for zero containment gain; not taken by default —
  the choice itself is routed as **J4** (§5).

## 5. Judgment calls — surfaced and routed, not settled (SA7)

Per the sprint contract (RQ4), each with a recommended default; **none is settled here**:

- **J1** `openat2`-only, no fallback walk (rec: yes — one reviewed path, `principles.md` §15).
- **J2** `RESOLVE_NO_XDEV` + `nosymfollow` inclusion (rec: yes — cheap, strictly narrowing).
- **J3** the startup-assertion convention itself (rec: yes — ratify as standing convention).
  **J3b** `nosymfollow` verification method (rec: single-line `mountinfo` check).
- **J-coll** collision policy: `O_EXCL` always fails a collision (rec: yes; §1.2 step 4
  forecloses the only legitimate same-name rewrite case mechanically).
- **J4** root-path hardening (`openat2`+`RESOLVE_NO_SYMLINKS` from a pinned ancestor) vs
  name-as-operator-trust (R-H). Rec: don't harden — it constrains legitimate operator
  symlinked root paths for zero box-surface containment gain; a one-line change if design
  prefers hardening at ratification.
- **J-kernel** whether tier-1 floors may rest on kernel enforcement at all — L11's class;
  routed with L11 (rec: acceptable, the posture the kernel already carries; policy's call).
- **Provisional scalars (marked, non-blocking):** `umask 077`; dirs `0700`; files `0600`;
  leaf-validator count/length bounds inherited as-is from Sprint-1 ground.

## 6. Primitive-semantics grounding (SA5)

Every load-bearing claim, its contract source (host man-pages, quoted verbatim where
load-bearing), and its bench evidence:

| Claim | Contract (host man-page, 2026-07-18) | Bench evidence (§7) |
|---|---|---|
| `O_NOFOLLOW` is final-component-only | `open(2)`: "Symbolic links in earlier components of the pathname will still be followed" | — (the finding itself) |
| `RESOLVE_NO_SYMLINKS` refuses a symlink at **any** component | `openat2(2)`: "Disallow resolution of symbolic links during path resolution … implies RESOLVE_NO_MAGICLINKS" | P2, P3, P10 (`ELOOP`) |
| `RESOLVE_BENEATH` confines to the `dirfd` subtree, rejects absolute | `openat2(2)`: "not a descendant of the directory indicated by dirfd … absolute values of pathname … rejected" | P4, P5 (`EXDEV`) |
| `RESOLVE_NO_XDEV` refuses mount crossings incl. bind mounts | `openat2(2)`: "Disallow traversal of mount points … (including all bind mounts)" | contract; errno class as P4/P5 |
| `O_CREAT\|O_EXCL` fails on an existing name, incl. a symlink | `open(2)` `O_EXCL` semantics | P6, P7 (`EEXIST`); P8 (fresh OK, mode `0600`) |
| `O_PATH` fd is a valid `dirfd` for `*at` calls | `open(2)` `O_PATH` | P1–P10 all run against an `O_PATH` base |
| `statvfs` `f_flag` exposes `ST_NOEXEC/ST_NOSUID/ST_NODEV` | `statvfs(3)` | §7: flags mirror `findmnt` exactly |
| `nosymfollow` exists as a mount option/attr | kernel ≥ 5.10; `MOUNT_ATTR_NOSYMFOLLOW` in `/usr/include/linux/mount.h` on the bench | header grep, §7 |

## 7. Bench verification appendix

Bench: `elsenor`, Debian 12, kernel `6.12.74+deb12-amd64`, 2026-07-18. Probe: a ~90-line C
program — sprint record `docs/teams/proxy/002-openat2-probe.c`, sha256
`286fcb42239fbc08f2e8096c01589a4fa79396eda1002352b36b0b3916868aa1`, build
`gcc -Wall -O0`, result **10/10 PASS** — that pins an
`O_PATH|O_DIRECTORY|O_NOFOLLOW` base descriptor over fixtures (a real dir, an
absolute-target symlink, a relative in-tree symlink, a `/proc/self/fd` symlink, an
existing file) and issues one `openat2(2)` per adversarial row under the §1.1 mask.
Observed, all PASS:

```
P1  real directory descent           -> opened
P2  symlink intermediate component   -> denied ELOOP   (the O_NOFOLLOW gap, closed)
P3  symlink final component          -> denied ELOOP
P4  ".." escape                      -> denied EXDEV
P5  absolute path                    -> denied EXDEV
P6  O_CREAT|O_EXCL on symlink name   -> denied EEXIST
P7  O_CREAT|O_EXCL on existing file  -> denied EEXIST
P8  O_CREAT|O_EXCL fresh leaf        -> opened, mode 0600
P9  file as intermediate component   -> denied ENOTDIR
P10 magic-link via symlink           -> denied ELOOP
```

`statvfs` assertion mechanism: `f_flag` on a live `nosuid,nodev` mount reads
`ST_NOSUID=1, ST_NODEV=1, ST_NOEXEC=0`, byte-for-byte consistent with `findmnt` for the
same mount. `MOUNT_ATTR_NOSYMFOLLOW` (`0x00200000`) and `MS_NOSYMFOLLOW` (`256`) present
in the bench kernel headers. A designated deploy host re-runs this appendix (R-E).
