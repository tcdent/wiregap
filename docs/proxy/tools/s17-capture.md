# `s17-capture.sh` — the Sprint 17 live end-to-end capture runner

Staged runner for the proxy's live end-to-end capture: a real `serve` on real bench hardware, a real
scripted head over the rendezvous socket, a real scripted box driving a round trip, and a real TLS
connection to the real `api.anthropic.com`.

Companion to `docs/teams/proxy/017-live-end-to-end-capture.md`, which is the sprint contract this
script implements. Read that for the tell design; read this for how to run it without breaking
anything.

---

## Status: **run and PASSED against the Sprint-18 pipe, 2026-07-25**

Ran 2026-07-25 09:37:31 PDT on elsenor. All four tells satisfied, bench restored, Slice 3 accepted by
review. Artifacts from that run are not in the repo (they land outside it by design).

Sprint 18 had replaced the upstream client with a verbatim TLS pipe, changing what reaches the box —
`net-response` now carries the **entire** upstream response, not the body alone:

| | before Sprint 18 | now (confirmed live) |
|---|---|---|
| invalid credential | proxy-invented `net-error` | the upstream's own verbatim **401 response** |
| response bytes | body only | status line + headers + body |
| connections | client-managed | **one TCP connection per call** (`Connection: close`) |

The tells were re-derived for that shape before the run rather than after it, and the live response
confirmed the redesign was right:

- **The `request-id` header was present** (`req_011CdP6wybnrmm3yDQ5pibFb`), so the response side
  carries genuine per-call uniqueness — a stronger tell than anything in the original design. The
  narrowed FS#7 framing was pre-written for its absence and did not need to be used.
- **The real response came back `Transfer-Encoding: chunked`, and the raw chunk framing survived into
  the relayed bytes.** Any general-purpose HTTP client would have de-chunked it. That is the
  verbatim-pipe property demonstrated by the real upstream picking an encoding nobody anticipated —
  better evidence than the synthetic arms, because nobody chose it.

**Before re-running, re-read the tells against whatever the pipe does *now*.** That is the standing
lesson from this sprint, not a one-off: the capture was blocked for a whole sprint because a change to
the upstream client silently invalidated response-side tells designed against the old one.

## Safety

This script **changes system state**. It is not the read-only shape of
`docs/nic/tools/live-card-probe.sh`. It runs privileged, starts a proxy that unbinds two NICs from
`ixgbe` to drive them from userspace, writes into `/run/zerotrust-proxy`, and opens a real TLS
connection to the public internet.

**The load-bearing invariant — exactly two bench cards are ever touched:**

```
0000:41:00.0   proxy
0000:63:00.0   box
```

**`0000:61:00.0` is the production NIC and must never be touched.** It is *also* an `0x10fb`, so a
device-id scan would match it and take the host off the network. Every gate — in this script and in
the binaries it runs — is an **exact-BDF allowlist** for that reason. If you edit anything here, do
not introduce enumeration.

Other properties worth knowing before you run it:

- **All preconditions are checked before anything is unbound.** A failed precondition exits having
  touched nothing.
- **`set -e` is deliberately absent.** A mid-run failure must still reach the teardown; exiting early
  would leave the bench with unbound cards.
- **The teardown rebinds `ixgbe` on both bench cards even when a tell fails.** If the script is
  killed before teardown, rebind manually:
  ```
  for b in 0000:41:00.0 0000:63:00.0; do echo -n $b | sudo tee /sys/bus/pci/drivers/ixgbe/bind; done
  ```
- **Operator paste-and-confirm is required before any hardware touch.** That is a process
  requirement on the run, not something the script enforces.

## No billed call

The script uses a **deliberately invalid** credential
(`sk-ant-INVALID-deliberately-bogus-for-capture-DO-NOT-USE`) so the real endpoint rejects it with a
401 (operator ruling, 2026-07-24). The rejection *is* the tell: a genuine TLS connection and HTTP
round trip to the real host, with nothing metered.

The value is not a secret — it needs no keyfile, no `0600`, no `sudo cat` — and is deliberately
unlike a real key so it cannot be mistaken for one. It must be **non-empty**, or the authority ladder
closes at `Closed(Credential)` and the upstream call never happens at all, which would make the
capture vacuous rather than failed.

**A real credential must never be substituted here** without a fresh operator ruling. The
no-billed-call constraint is theirs, not a convenience.

## Prerequisites

Both feature-gated binary sets must be built first — they are off the default build by design, so the
production/TCB build never links the bench surface:

```
cargo build --offline --features serve         --bin serve
cargo build --offline --features bench-capture --bin s17_box
cargo build --offline --features bench-capture --bin s17_head
```

**Run these as three separate invocations. Do not combine them.** Cargo unifies features across a
single invocation, so building all three at once compiles the shared libraries with
`serve,bench-capture` together — and `bench-capture` enables `directpath/bench`. The resulting `serve`
links the bench internals *into the production binary*, which is exactly what the feature gate exists
to prevent. Measured, not theoretical: the combined build produced `serve = 3d0cbaab…` against the
separate builds' `38e14a57…`, the binary the accepted capture actually ran.

The provisioned filesystem roots must exist, with the quarantine writable by the drop uid (65534):

```
/run/zerotrust-proxy/quarantine     # writable by uid 65534
/srv/zerotrust-proxy/readable
```

Provisioning those is the operator's step, not the script's.

## Running

```
sudo -v && docs/proxy/tools/s17-capture.sh
```

**Start order is load-bearing** and the script enforces it: `serve` blocks on `accept()` *before* it
constructs the NIC, so the head must connect before the box starts, or the capture simply hangs.

## Artifacts

Written **outside the repo** — they are per-run evidence, not source. Default
`${TMPDIR:-/tmp}/s17-capture/capture-<timestamp>/`; override with `S17_CAPTURE_OUT`.

| file | what it holds |
|---|---|
| `prerun-state.txt` | root ownership/modes, mount options, post-run quarantine listing |
| `binshas-before.txt` / `binshas-after.txt` | binary hashes, to show nothing changed underneath the run |
| `serve-proc-exe-sha.txt` | **the running-bytes tell** — pid, cmdline, and `sha256` of `/proc/<pid>/exe` |
| `serve.log`, `head.log`, `box.log` | the three processes' output |
| `response-body.bin` | the bytes the box received |
| `box-exit.txt` | `s17_box`'s exit status — recorded, not gated (the tells fail closed without it) |
| `credential-echo.txt` | PASS/FAIL of the credential-echo gate |
| `quarantine-write.txt` | tell #4 — the confined write, with owner and mode |

## Two design points that are not obvious from the code

**The running-bytes tell hashes `/proc/<pid>/exe`, not a path.** A path can be replaced between build
and run; the kernel's handle on the executing image cannot. It is read as root while `serve` is
blocked on `accept()` — before `bring_up` — which is a stable, race-free window. The script also
resolves the pid's `exe` link before trusting it, because `pgrep -f` would match the `sudo` wrapper
too and hashing sudo's binary would be a silently wrong hash.

**The credential-echo gate lives in the script, not in `s17_box`.** The box must never hold the
credential — that is the containment model's premise, and handing it the value to self-check would
invert it. The script already knows the value; the box only records the bytes it received. The gate
also checks a distinctive 16-character tail, to catch a truncated echo that a full-value match would
miss.

## Provenance

Written during Sprint 17 Slice 3 and iterated against review (the hang bug, the `cmdline` match, the
keyfile permission inconsistency, and the verify-before-kill race were each review findings). It
lived in an ephemeral session scratchpad until Sprint 18 closed, at which point the loss risk was
flagged and the PM directed it into the repo.

Two changes to executable lines were forced by leaving the scratchpad: the repo path is now resolved
from the script's own location rather than one hardcoded home directory, and artifacts now land
outside the repo instead of beside the script.

A `shellcheck` 0.9.0 pass and a review read for exit-status masking then produced six more, all
hardening rather than behaviour changes. `shellcheck` is clean at `--severity=style`:

| change | why |
|---|---|
| `mkdir -p "$OUT"` now exits on failure | unchecked, every later `tee` failed silently and the run narrated success with an empty artifact directory |
| the `sha256sum … \| tee binshas-before.txt` pipeline now exits on failure | it is a **tell**, not narration — a silently-empty file makes the bin-sha claim vacuous while looking present |
| the precondition block uses explicit `if/fi` | `A && echo \|\| { fail=1; }` also takes the else-arm when the echo fails; these are the gates protecting the production NIC |
| `sudo tr … < /proc/PID/cmdline` → `sudo cat … \| tr` | **a real bug**: the redirect is opened by the invoking user, so the `sudo` applied only to `tr`, which needs no privilege. Works today only because `/proc/PID/cmdline` is world-readable; fails under a `hidepid=` mount while looking correct |
| `s17_box`'s exit status captured, recorded, and warned on | was computed and printed but unreferenced; still not a gate, deliberately — see the in-file note on why every downstream tell fails closed without it |
| three `sudo cmd > "$OUT/…"` redirects annotated `# shellcheck disable=SC2024` | the invoking-user redirect is *intended*: `$OUT` is user-owned, and a later non-`sudo` grep of `head.log` depends on it. The one artifact that genuinely needs a root write — `response-body.bin` — is written by the privileged binary opening its own path |
| the teardown reports the ACTUAL driver state, not the bind write's exit status | the write returns EBUSY when a card is already bound — the normal case when a binary restores its own card — so the old form printed "check manually" on clean runs, training the operator to skim past the one message that matters |
| the pid-verification failure path now reaps the backgrounded job | it exited without reaping, leaving a root-owned `serve` blocked on `accept()` forever. A process leak, not a bench-safety gap — no card is unbound at that point |

Every other executable line is byte-identical to the version that produced the Sprint 17 capture
attempts.
