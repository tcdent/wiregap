#!/usr/bin/env bash
#
# s17-capture.sh — staged runner for the Sprint 17 live end-to-end capture.
#
# Companion notes: docs/proxy/tools/s17-capture.md (read that first).
#
#   usage: sudo -v && docs/proxy/tools/s17-capture.sh
#
# ------------------------------------------------------------------------------------------------
# DO NOT RUN THIS AS-IS — IT IS STALE AS OF SPRINT 18 (2026-07-25)
# ------------------------------------------------------------------------------------------------
# Sprint 18 replaced the upstream client with a verbatim TLS pipe, and that CHANGED WHAT REACHES THE
# BOX. `net-response` now carries the ENTIRE upstream response — status line, headers, body — where
# it previously carried the body alone. Two consequences for this script:
#
#   * An invalid credential no longer produces a proxy-invented `net-error`. It arrives as the
#     upstream's own verbatim 401. This run therefore exercises the RELAY path, not the error path —
#     a better capture than the one designed here, but a DIFFERENT one.
#   * Any tell that pattern-matched a response body must be re-derived against the new byte stream.
#     The credential-echo gate below still holds (it greps raw bytes), but tells #2/#3 do not.
#
# It is committed in this state deliberately: the preconditions, the verify-before-kill ordering, the
# teardown, and the `ixgbe` rebind took several review iterations to get right and must not be
# re-derived from scratch. Revalidate the tells before the next run; do not delete and rewrite.
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
mkdir -p "$OUT"
say() { printf '\n=== %s ===\n' "$*"; }

say "PRECONDITIONS (nothing is touched until all pass)"
fail=0
for d in /run/zerotrust-proxy/quarantine /srv/zerotrust-proxy/readable; do
  [ -d "$d" ] && echo "  ok   $d" || { echo "  MISSING $d"; fail=1; }
done
sudo -u '#65534' test -w /run/zerotrust-proxy/quarantine \
  && echo "  ok   quarantine writable by uid 65534" || { echo "  NOT writable by 65534"; fail=1; }
[ -n "$BOGUS_KEY" ] && echo "  ok   deliberately-invalid credential set (non-empty, so the ladder reaches Served)" \
  || { echo "  bogus credential is empty — the upstream call would never be attempted"; fail=1; }
for b in 0000:41:00.0 0000:63:00.0; do
  drv=$(basename "$(readlink /sys/bus/pci/devices/$b/driver 2>/dev/null)" 2>/dev/null)
  echo "  bench $b driver=${drv:-<unbound>}"
done
prod=$(basename "$(readlink /sys/bus/pci/devices/0000:61:00.0/driver 2>/dev/null)" 2>/dev/null)
[ "$prod" = ixgbe ] && echo "  ok   production 61:00.0 still ixgbe (never in any allowlist)" \
  || { echo "  PRODUCTION NIC NOT ixgbe — STOP"; fail=1; }
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
{ ls -ld /run/zerotrust-proxy /run/zerotrust-proxy/quarantine /srv/zerotrust-proxy/readable
  findmnt -no TARGET,OPTIONS --target /run/zerotrust-proxy/quarantine; } | tee "$OUT/prerun-state.txt"
sha256sum "$REPO/target/debug/serve" "$REPO/target/debug/s17_box" "$REPO/target/debug/s17_head" \
  | tee "$OUT/binshas-before.txt"

say "1/3  serve  (blocks on accept; deliberately-INVALID credential via env — no billed call)"
export ANTHROPIC_API_KEY="$BOGUS_KEY"
sudo -E "$REPO/target/debug/serve" >"$OUT/serve.log" 2>&1 &
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
[ -n "$SERVE_PID" ] || { echo "serve did not start (or pid unverifiable) — see $OUT/serve.log"; cat "$OUT/serve.log"; exit 1; }
# THE RUNNING-BYTES TELL: hash the kernel's own handle on the executing image, not a path. Read as root,
# while serve is blocked on accept() (before bring_up) — a stable, no-race window.
{ echo "pid: $SERVE_PID"
  echo -n "cmdline: "; sudo tr '\0' ' ' < "/proc/$SERVE_PID/cmdline"; echo
  echo -n "exe -> "; sudo readlink "/proc/$SERVE_PID/exe"
  sudo sha256sum "/proc/$SERVE_PID/exe"
} | tee "$OUT/serve-proc-exe-sha.txt"

say "2/3  s17_head  (must be root — socket is root-owned; must connect BEFORE the box)"
sudo "$REPO/target/debug/s17_head" >"$OUT/head.log" 2>&1 &
for _ in $(seq 1 60); do grep -q "CONNECTED" "$OUT/head.log" 2>/dev/null && break; sleep 1; done
grep -q "CONNECTED" "$OUT/head.log" || { echo "head never connected — see $OUT/head.log"; }

say "3/3  s17_box  (drives the round trip — real TLS+HTTP to the real host, rejected 401, unbilled)"
RESP_BODY="$OUT/response-body.bin"
sudo "$REPO/target/debug/s17_box" "$RESP_BODY" >"$OUT/box.log" 2>&1
echo "box exit: $?"

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
pkill -x s17_head 2>/dev/null && echo "  s17_head reaped (had not seen a display frame)" || true

say "POST-RUN"
sha256sum "$REPO/target/debug/serve" | tee "$OUT/binshas-after.txt"   # unchanged underneath?
echo "-- quarantine contents --"; sudo ls -l /run/zerotrust-proxy/quarantine | tee -a "$OUT/prerun-state.txt"
echo "-- rebinding ixgbe (leave the bench as found) --"
for b in 0000:41:00.0 0000:63:00.0; do
  echo -n "$b" | sudo tee /sys/bus/pci/drivers/ixgbe/bind >/dev/null 2>&1 && echo "  rebound $b" || echo "  $b rebind: check manually"
done
say "ARTIFACTS in $OUT"; ls -1 "$OUT"
