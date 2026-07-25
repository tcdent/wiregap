#!/usr/bin/env bash
#
# s17-capture.sh — staged runner for the Sprint 17 live end-to-end capture.
#
# Companion notes: docs/proxy/tools/s17-capture.md (read that first).
#
#   usage: sudo -v && docs/proxy/tools/s17-capture.sh
#
# ------------------------------------------------------------------------------------------------
# RUN AND PASSED 2026-07-25 09:37:31 PDT on elsenor — all four tells, Slice 3 accepted by review.
# ------------------------------------------------------------------------------------------------
# (This banner previously read "DO NOT RUN — STALE AS OF SPRINT 18". That was correct until the tells
# were re-derived for Sprint 18's verbatim pipe and the capture was run and accepted. Corrected rather
# than left standing: a false "do not run" is read by the next operator before they touch hardware.)
#
# Sprint 18 replaced the upstream client with a verbatim TLS pipe, so `net-response` now carries the
# ENTIRE upstream response — status line, headers, body — where it previously carried the body alone.
# An invalid credential therefore arrives as the upstream's OWN 401 response, not as a proxy-invented
# net-error, and this run exercises the RELAY path. The live response confirmed the redesign: the
# `request-id` header was present (real per-call uniqueness), and it came back
# `Transfer-Encoding: chunked` with the raw chunk framing intact in the relayed bytes — the
# verbatim-pipe property demonstrated by the real upstream choosing an encoding nobody anticipated.
#
# BEFORE RE-RUNNING, RE-READ THE TELLS AGAINST WHAT THE PIPE DOES *NOW*. That is the standing lesson,
# not a one-off: this capture was blocked for an entire sprint because a change to the upstream client
# silently invalidated response-side tells designed against the old one. The preconditions, the
# verify-before-kill ordering, the teardown and the `ixgbe` rebind each took review iterations to get
# right — revalidate the tells, but do not delete and rewrite the machinery around them.
#
# ------------------------------------------------------------------------------------------------
# THIS SCRIPT CHANGES SYSTEM STATE. It is NOT the read-only shape of docs/nic/tools/live-card-probe.sh
# ------------------------------------------------------------------------------------------------
# It runs privileged, starts a proxy that UNBINDS two NICs from `ixgbe` to drive them from userspace,
# writes into /run/zerotrust-proxy, and opens a real TLS connection to the real api.anthropic.com.
#
# THE LOAD-BEARING SAFETY INVARIANT: exactly two bench cards are ever touched —
#
#     0000:41:00.0  (proxy)          0000:63:00.0  (box)
#
# The PRODUCTION NIC AT 0000:61:00.0 IS ALSO AN 0x10fb AND MUST NEVER BE TOUCHED. Every gate here and
# in the binaries is an EXACT-BDF allowlist for that reason — never a device-id scan, which would
# match the production card and take the host off the network. If you edit this script, do not
# introduce enumeration.
#
# Preconditions are ALL checked before anything is unbound. The teardown rebinds `ixgbe` on both
# bench cards; it runs even when a tell fails, so the bench is never left with unbound cards.
#
# `set -e` is deliberately NOT used: a mid-run failure must still reach the teardown.
#
# ------------------------------------------------------------------------------------------------
# NO BILLED CALL, NO REAL CREDENTIAL (operator ruling, 2026-07-24). A deliberately-invalid credential is
# used so the REAL Anthropic endpoint reliably rejects with 401. That rejection IS the tell: a genuine TLS
# connection and HTTP round trip to the real host, correctly translated into a box-facing net-error — with
# nothing metered. The bogus value is not a secret, so it needs no keyfile, no 0600, and no sudo cat; it
# is deliberately unlike a real key so it cannot be mistaken for one. It must be NON-EMPTY, or the ladder
# closes at Closed(Credential) and the upstream call never happens at all.
#
# Fails safe: every precondition is checked BEFORE anything is unbound or called.
set -uo pipefail
BOGUS_KEY="sk-ant-INVALID-deliberately-bogus-for-capture-DO-NOT-USE"

# Resolved from this script's own location (docs/proxy/tools/), never a hardcoded home directory, so
# the script works in any checkout. It was written against one absolute path while it lived in a
# scratchpad; that would have silently broken for anyone else the moment it entered the repo.
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)/proxy"
[ -f "$REPO/Cargo.toml" ] || { echo "cannot locate the proxy crate from $HERE — expected $REPO"; exit 1; }

# Artifacts land OUTSIDE the repo. They are per-run evidence (logs, hashes, a response body), not
# source, and writing them beside this script would deposit them into the docs tree. Override with
# S17_CAPTURE_OUT if you want them somewhere specific.
OUT="${S17_CAPTURE_OUT:-${TMPDIR:-/tmp}/s17-capture}/capture-$(date +%Y%m%d-%H%M%S)"
# Checked, and load-bearing twice over. Unchecked, a failed mkdir (unwritable $TMPDIR, or an
# S17_CAPTURE_OUT pointing somewhere this user cannot write) would let every later `tee "$OUT/..."`
# fail silently against a nonexistent directory — the run would narrate success on the terminal with
# NOTHING landing in the artifact directory. It is also the precondition that makes the deliberate
# `sudo cmd > "$OUT/..."` redirects below correct rather than merely lucky (see SC2024 note there):
# those redirects are opened by the INVOKING user, so $OUT must be invoking-user-writable, which is
# exactly what succeeding here establishes.
mkdir -p "$OUT" || { echo "cannot create artifact directory $OUT — nothing touched."; exit 1; }
say() { printf '\n=== %s ===\n' "$*"; }

say "PRECONDITIONS (nothing is touched until all pass)"
fail=0
# Explicit if/fi throughout this block, not `test && echo || { fail=1; }`. In that idiom the else-arm
# also runs when the test SUCCEEDS but the echo fails, which would set fail=1 spuriously. Here that
# happens to err safe — a spurious abort before anything is touched — but these are the gates standing
# between this script and the production NIC, and "wrong in the safe direction" is a poor thing for a
# reader to have to derive. They are written unambiguously instead.
for d in /run/zerotrust-proxy/quarantine /srv/zerotrust-proxy/readable; do
  if [ -d "$d" ]; then echo "  ok   $d"; else echo "  MISSING $d"; fail=1; fi
done
if sudo -u '#65534' test -w /run/zerotrust-proxy/quarantine; then
  echo "  ok   quarantine writable by uid 65534"
else
  echo "  NOT writable by 65534"; fail=1
fi
if [ -n "$BOGUS_KEY" ]; then
  echo "  ok   deliberately-invalid credential set (non-empty, so the ladder reaches Served)"
else
  echo "  bogus credential is empty — the upstream call would never be attempted"; fail=1
fi
# WELL-FORMEDNESS, and this is a real mitigation rather than tidiness (017 Final State #5).
#
# `write_request` (proxy/src/upstream.rs) performs ZERO validation on the credential before writing it
# into the header block. A CR or LF in the value would not merely malform one header — it would
# TERMINATE THE HEADER BLOCK EARLY, moving the body boundary and turning the rest of the request into
# something the upstream reads differently from what this capture claims was sent. Sprint 18 FS#5's
# byte-exact wire-request proof (full-buffer equality) is proven only for a WELL-FORMED credential, so
# the capture must not silently step outside the ground that proof covers.
#
# Until Sprint 18 that safety rested on discipline — "the value is pinned in this file, so of course it
# is clean". This checked precondition replaces the assumption with a gate, before anything is unbound.
# Visible ASCII only (0x21–0x7E): excludes CR, LF, tab, space, and every control byte. (NUL is not in
# scope here and cannot be — bash truncates a string at an embedded NUL, so one can never reach this
# variable in the first place. Stated rather than silently claimed as covered.)
#
# Implemented as a BYTE-COUNT comparison, not a regex, and that is not a style preference. The obvious
# form — `grep -qE '^[!-~]+$'` — is LINE-ORIENTED, so a value containing the single most dangerous
# character here, LF, splits into two lines and the pattern happily matches the first one. It ACCEPTS
# exactly the input this gate exists to reject. (Found by exercising the check against a CR value, an LF
# value, a space, a tab and an empty string before trusting it; only the LF case exposed it.)
# Comparing byte counts has no line semantics at all: every byte must survive `tr -cd '!-~'`, or one of
# them was not visible ASCII. `wc -c` is used rather than a command substitution of the string itself,
# because `$(...)` strips trailing newlines and would re-hide a credential ending in LF.
key_bytes=$(printf '%s' "$BOGUS_KEY" | wc -c)
key_visible=$(printf '%s' "$BOGUS_KEY" | LC_ALL=C tr -cd '!-~' | wc -c)
if [ "$key_bytes" -gt 0 ] && [ "$key_bytes" -eq "$key_visible" ]; then
  echo "  ok   credential is well-formed (visible ASCII only — no CR/LF/control byte to split headers)"
else
  echo "  CREDENTIAL MALFORMED — contains CR, LF, whitespace, or a control byte. A CR/LF would"
  echo "       terminate the header block early and move the body boundary; the wire request would"
  echo "       not be the one FS#5 proves. STOP."
  fail=1
fi
for b in 0000:41:00.0 0000:63:00.0; do
  drv=$(basename "$(readlink /sys/bus/pci/devices/$b/driver 2>/dev/null)" 2>/dev/null)
  echo "  bench $b driver=${drv:-<unbound>}"
done
prod=$(basename "$(readlink /sys/bus/pci/devices/0000:61:00.0/driver 2>/dev/null)" 2>/dev/null)
if [ "$prod" = ixgbe ]; then
  echo "  ok   production 61:00.0 still ixgbe (never in any allowlist)"
else
  echo "  PRODUCTION NIC NOT ixgbe — STOP"; fail=1
fi
[ "$fail" = 0 ] || { echo "PRECONDITIONS FAILED — nothing touched."; exit 1; }

# RE-RUN HYGIENE (found on the second attempt, and it is the write path behaving CORRECTLY): the confined
# create is O_CREAT|O_EXCL, so it deliberately never overwrites — a leftover destination from a prior
# attempt makes the chunk DENIED, failing tell #4 for a reason that has nothing to do with the run. Clear
# the destination first; the prior attempt's artifacts are already preserved in its own capture dir.
if sudo test -e /run/zerotrust-proxy/quarantine/s17-capture.bin; then
  echo "  clearing a prior attempt's quarantine destination (O_EXCL would otherwise deny the write)"
  sudo rm -f /run/zerotrust-proxy/quarantine/s17-capture.bin
fi

say "PRE-RUN STATE (capture evidence)"
# Narration: a failure here costs context in the write-up, not a tell, so it is not gated.
{ ls -ld /run/zerotrust-proxy /run/zerotrust-proxy/quarantine /srv/zerotrust-proxy/readable
  findmnt -no TARGET,OPTIONS --target /run/zerotrust-proxy/quarantine; } | tee "$OUT/prerun-state.txt"

# THIS ONE IS A TELL, NOT NARRATION — so its failure is fatal. binshas-before.txt is what binds every
# claim in the write-up to specific bytes; if `sha256sum` fails (a binary missing because a required
# --features build was skipped, say), an UNCHECKED pipeline leaves a silently-empty file sitting in the
# artifact directory. The bin-sha coherence tell then LOOKS present in the artifact listing while being
# entirely vacuous — a false green, and the exact shape this project treats as worse than a red.
# `pipefail` is set, so $? here genuinely reflects sha256sum rather than tee.
#
# Exiting is safe at this point specifically: no card has been unbound yet, so there is nothing to
# tear down. Past the `serve` start below, a bare exit would strand the bench with unbound cards.
if ! sha256sum "$REPO/target/debug/serve" "$REPO/target/debug/s17_box" "$REPO/target/debug/s17_head" \
     | tee "$OUT/binshas-before.txt"; then
  echo "FAILED to hash the binaries — the bin-sha tell would be vacuous. Nothing touched."
  echo "Build them first:  cargo build --features serve --bin serve"
  echo "                   cargo build --features bench-capture --bin s17_box --bin s17_head"
  exit 1
fi

say "1/3  serve  (blocks on accept; deliberately-INVALID credential via env — no billed call)"
export ANTHROPIC_API_KEY="$BOGUS_KEY"
# shellcheck disable=SC2024  # deliberate: see the mkdir note above.
# SC2024 warns that `sudo cmd > file` opens the redirect as the INVOKING user, not root. That is the
# intended behaviour here, not an oversight. $OUT is created by this script as the invoking user (and
# that mkdir is now checked), so the log lands user-owned in a user-owned directory — which is what the
# later non-sudo `grep ... "$OUT/head.log"` depends on. Routing it through `sudo tee` instead would add
# a pipe and a second sudo only to make the file root-owned, for no benefit. The one artifact that DOES
# need root to write is response-body.bin, and it is written by the privileged binary itself opening its
# own path — the pattern SC2024 actually recommends, already used where it is actually required.
sudo -E "$REPO/target/debug/serve" >"$OUT/serve.log" 2>&1 &
# The job THIS SCRIPT backgrounded — ours BY CONSTRUCTION, not by name-matching. Captured immediately,
# because the next `&` below would overwrite `$!`. It is the handle used to reap serve on the
# verification-failure path, where SERVE_PID is empty by definition and there is nothing else to trust.
# Note this is the SUDO wrapper's pid, not serve's; see the reap for why that distinction matters.
SUDO_JOB_PID=$!
# `pgrep -f` would ALSO match the `sudo -E .../serve` wrapper (its cmdline ends in "serve"), and hashing
# sudo's exe would be a silently WRONG hash. Match the exact process NAME instead (sudo's comm is "sudo"),
# then PROVE the pid is ours by resolving its exe link before trusting anything it reports.
SERVE_PID=""
for _ in $(seq 1 30); do
  cand=$(pgrep -x serve | head -1)
  if [ -n "$cand" ] && [ "$(sudo readlink "/proc/$cand/exe")" = "$REPO/target/debug/serve" ]; then
    SERVE_PID="$cand"; break
  fi
  sleep 1
done
if [ -z "$SERVE_PID" ]; then
  echo "serve did not start (or pid unverifiable) — see $OUT/serve.log"
  cat "$OUT/serve.log"
  # REAP WHAT WE STARTED. This branch is reached precisely when the exe-verification above could not
  # identify serve — which does NOT mean serve is not running. Without this, a root-owned serve is left
  # blocked on accept() forever, since the head never connects after we exit.
  #
  # The target is $! — the job this script backgrounded — and deliberately NOT the `$cand` that pgrep
  # matched. Killing the pgrep match would be wrong in exactly the case that put us on this branch: an
  # unrelated process merely NAMED "serve", which is the possibility the exe-verification exists to rule
  # out. Known-ours by construction beats known-ours by name.
  #
  # $! is the SUDO wrapper, not serve. sudo forwards SIGTERM to its child, so terminating it normally
  # takes serve with it — but the children are enumerated FIRST and swept afterwards, because a sudo
  # that dies without forwarding would reparent serve away and leave nothing to find.
  if sudo kill -0 "$SUDO_JOB_PID" 2>/dev/null; then
    mapfile -t kids < <(pgrep -P "$SUDO_JOB_PID" 2>/dev/null)
    sudo kill -TERM "$SUDO_JOB_PID" 2>/dev/null
    for _ in $(seq 1 5); do sudo kill -0 "$SUDO_JOB_PID" 2>/dev/null || break; sleep 1; done
    sudo kill -KILL "$SUDO_JOB_PID" 2>/dev/null
    for k in "${kids[@]}"; do [ -n "$k" ] && sudo kill -KILL "$k" 2>/dev/null; done
    echo "  reaped the backgrounded serve job (sudo pid $SUDO_JOB_PID) — no stray root process left"
  fi
  # A bare exit is correct HERE and nowhere past this point: serve blocks on accept() before it
  # constructs the NIC, so no card has been unbound yet and there is nothing to rebind.
  exit 1
fi
# THE RUNNING-BYTES TELL: hash the kernel's own handle on the executing image, not a path. Read as root,
# while serve is blocked on accept() (before bring_up) — a stable, no-race window.
{ echo "pid: $SERVE_PID"
  # `sudo cat ... | tr`, NOT `sudo tr ... < file`. The redirect form is opened by the invoking user —
  # sudo does not affect redirections — so the `sudo` would apply only to `tr`, which needs no
  # privilege whatsoever, while the privileged part (opening another user's /proc entry) ran
  # unprivileged. It happens to work because /proc/<pid>/cmdline is world-readable by default, but it
  # would fail under a `hidepid=` mount while LOOKING like it should have succeeded, and this block is
  # the running-bytes tell — a silently-empty line here is a missing tell, not a cosmetic gap. The two
  # lines below always got this right by passing the path TO the sudo'd command; this one did not.
  echo -n "cmdline: "; sudo cat "/proc/$SERVE_PID/cmdline" | tr '\0' ' '; echo
  echo -n "exe -> "; sudo readlink "/proc/$SERVE_PID/exe"
  sudo sha256sum "/proc/$SERVE_PID/exe"
} | tee "$OUT/serve-proc-exe-sha.txt"

say "2/3  s17_head  (must be root — socket is root-owned; must connect BEFORE the box)"
# shellcheck disable=SC2024  # invoking-user redirect into the user-owned $OUT is intended; see above.
sudo "$REPO/target/debug/s17_head" >"$OUT/head.log" 2>&1 &
for _ in $(seq 1 60); do grep -q "CONNECTED" "$OUT/head.log" 2>/dev/null && break; sleep 1; done
grep -q "CONNECTED" "$OUT/head.log" || { echo "head never connected — see $OUT/head.log"; }

say "3/3  s17_box  (drives the round trip — real TLS+HTTP to the real host, rejected 401, unbilled)"
RESP_BODY="$OUT/response-body.bin"
# shellcheck disable=SC2024  # invoking-user redirect into the user-owned $OUT is intended; see above.
sudo "$REPO/target/debug/s17_box" "$RESP_BODY" >"$OUT/box.log" 2>&1
BOX_EXIT=$?
echo "box exit: $BOX_EXIT" | tee "$OUT/box-exit.txt"
# NOT a gate, and deliberately so: the run must continue to the teardown regardless, or the bench is
# left with unbound cards. It is recorded rather than branched on because **every downstream tell is
# evidence-based and fails closed without it** — verified, not assumed:
#
#   * the credential-echo gate keys off `sudo test -s "$RESP_BODY"`; a box that died before writing
#     leaves no body, so the gate reports NO BODY, never PASS;
#   * tell #4 keys off a nonce grepped out of box.log; a box that died before emitting it leaves
#     CHUNK_NONCE empty, so the write-check is skipped and reports NOT SATISFIED.
#
# Neither can produce a false PASS from a crashed box. That property is why this is a warning rather
# than an exit — preserve it if you add a tell, or this line has to become a gate.
if [ "$BOX_EXIT" != 0 ]; then
  echo "  *** s17_box exited non-zero — the tells below are expected to fail closed; read box.log ***"
fi

say "CREDENTIAL-ECHO GATE — assert the upstream's reply carries no part of the credential"
# Verbatim relay of an ERROR body is not symmetric with a success body: a 4xx CAN echo request material,
# and in the worst case that could include the credential — which must never reach the box. Not true of
# Anthropic today, but that is a vendor behaviour, not a property the design guarantees. Asserting it
# converts an assumption into recorded evidence.
#
# The check lives HERE, not in s17_box, deliberately: the box must never hold the credential — that is the
# containment model's premise, and handing it to the box to self-check would invert it. The script already
# knows the value; the box only records the bytes it received.
if sudo test -s "$RESP_BODY"; then
  ECHO_OK=1
  sudo grep -qF -- "$BOGUS_KEY" "$RESP_BODY" 2>/dev/null && ECHO_OK=0
  # Also a distinctive tail, to catch a TRUNCATED echo that the full-value check would miss.
  KEY_TAIL="${BOGUS_KEY: -16}"
  sudo grep -qF -- "$KEY_TAIL" "$RESP_BODY" 2>/dev/null && ECHO_OK=0
  if [ "$ECHO_OK" = 1 ]; then
    echo "  PASS — no credential material in the upstream reply ($(sudo stat -c%s "$RESP_BODY") bytes checked)"
  else
    echo "  *** FAIL: CREDENTIAL MATERIAL FOUND IN THE UPSTREAM REPLY ***"
    echo "  The body is NOT printed here, deliberately. Investigate $RESP_BODY directly."
  fi
  echo "credential-echo: $([ "$ECHO_OK" = 1 ] && echo PASS || echo FAIL)" > "$OUT/credential-echo.txt"
else
  echo "  no response body recorded — nothing to assert over"
  echo "credential-echo: NO BODY" > "$OUT/credential-echo.txt"
fi

say "VERIFY THE WRITE **BEFORE** KILLING SERVE — this ordering is load-bearing"
# The box's final phase completes on the TRANSPORT ACK, which the proxy emits on frame RECEIPT — before
# its app-level dispatch performs the confined open and the append. Killing serve the moment the box exits
# can land inside that window, losing the write and failing tell #4 while looking like a proxy defect
# rather than a script race: a false negative that points debugging at exactly the wrong place. So: prove
# the write landed first, THEN tear down.
CHUNK_NONCE=$(grep 'nonce#2' "$OUT/box.log" 2>/dev/null | awk '{print $NF}' | head -1)
QFILE=/run/zerotrust-proxy/quarantine/s17-capture.bin
WRITE_OK=0
if [ -n "$CHUNK_NONCE" ]; then
  for _ in $(seq 1 30); do
    # The proxy creates the file as the dropped uid at mode 0600, so this must be read as root.
    if sudo test -s "$QFILE" && sudo grep -qF "$CHUNK_NONCE" "$QFILE" 2>/dev/null; then WRITE_OK=1; break; fi
    sleep 1
  done
fi
if [ "$WRITE_OK" = 1 ]; then
  echo "  TELL #4 SATISFIED — nonce#2 found in $QFILE (written by the dropped uid)"
  sudo ls -l "$QFILE" | tee "$OUT/quarantine-write.txt"
  sudo stat -c '  owner=%U:%G mode=%a size=%s' "$QFILE" | tee -a "$OUT/quarantine-write.txt"
else
  echo "  TELL #4 NOT SATISFIED — no quarantine file containing nonce#2 after 30s"
  echo "  (proceeding to teardown anyway: the bench must not be left with unbound cards)"
  { echo "TELL #4 NOT SATISFIED"; sudo ls -l /run/zerotrust-proxy/quarantine; } | tee "$OUT/quarantine-write.txt"
fi

say "TEARDOWN — serve never self-exits (it is the real production loop: on_verdict always returns"
say "Continue, so run_adapter_composed loops forever). Terminate it explicitly, or the post-run steps"
say "below — including the ixgbe rebind — never run and the bench is left with unbound cards."
sudo kill -TERM "$SERVE_PID" 2>/dev/null
for _ in $(seq 1 10); do sudo kill -0 "$SERVE_PID" 2>/dev/null || break; sleep 1; done
if sudo kill -0 "$SERVE_PID" 2>/dev/null; then
  echo "  serve ignored SIGTERM — escalating to SIGKILL"
  sudo kill -KILL "$SERVE_PID" 2>/dev/null; sleep 1
fi
sudo kill -0 "$SERVE_PID" 2>/dev/null && echo "  WARNING: serve STILL running (pid $SERVE_PID)" || echo "  serve stopped"
# s17_head self-exits on the display frame, but reap it if it is still waiting out its frame window.
# shellcheck disable=SC2015  # inert by construction: the `|| true` arm has no effect beyond
# swallowing a non-match, so the A&&B||C caveat cannot change any outcome here.
pkill -x s17_head 2>/dev/null && echo "  s17_head reaped (had not seen a display frame)" || true

say "POST-RUN"
sha256sum "$REPO/target/debug/serve" | tee "$OUT/binshas-after.txt"   # unchanged underneath?
echo "-- quarantine contents --"; sudo ls -l /run/zerotrust-proxy/quarantine | tee -a "$OUT/prerun-state.txt"
echo "-- rebinding ixgbe (leave the bench as found) --"
# REPORT THE ACTUAL FINAL STATE, NOT THE BIND WRITE'S EXIT STATUS.
#
# The write fails with EBUSY when the card is ALREADY bound — which is the normal case, because a
# binary that restored its own card on exit leaves nothing to rebind. Branching on that exit status
# therefore printed "check manually" on a perfectly clean run (observed 2026-07-25 for 0000:63:00.0,
# where the card was bound and routing fine the whole time). That is worse than noise: a teardown that
# cries wolf every run trains the operator to skim past the one message that matters when a card
# genuinely IS left unbound. So the write's status is deliberately discarded, and the driver link is
# re-read afterwards to report what is actually true.
for b in 0000:41:00.0 0000:63:00.0; do
  echo -n "$b" | sudo tee /sys/bus/pci/drivers/ixgbe/bind >/dev/null 2>&1 || true
  drv=$(basename "$(readlink "/sys/bus/pci/devices/$b/driver" 2>/dev/null)" 2>/dev/null)
  if [ "$drv" = ixgbe ]; then
    echo "  ok   $b bound to ixgbe"
  else
    echo "  *** $b IS NOT BOUND (driver=${drv:-<none>}) — THE BENCH IS LEFT WITH AN UNBOUND CARD ***"
    echo "      rebind manually:  echo -n $b | sudo tee /sys/bus/pci/drivers/ixgbe/bind"
  fi
done
say "ARTIFACTS in $OUT"; ls -1 "$OUT"
