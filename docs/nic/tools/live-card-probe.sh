#!/usr/bin/env bash
#
# live-card-probe.sh — read-only diagnostic for an Intel 82599 (X520) NIC.
#
# Grounds the *static* open hardware questions in docs/nic/grounding/ against a
# LIVE, kernel-bound 82599 — including the one that may be this host's PRIMARY
# network interface — with ZERO risk of disturbing the link.
#
# ----------------------------------------------------------------------------
# THE LOAD-BEARING SAFETY INVARIANT (read this before reading anything else)
# ----------------------------------------------------------------------------
# This script is structurally incapable of disturbing the link. It performs
# ONLY read-only operations mediated by the in-tree kernel driver:
#
#   * ethtool QUERY subcommands only:  -i  -d  -e  -m  and plain `ethtool <if>`
#   * lspci   (PCI config-space READS via the kernel)
#   * sysfs   READS under /sys/...
#
# It NEVER:
#   * uses any ethtool set/change form (-s, -E, --change, --write, ...)
#   * writes NVM, registers, PCI config, or any ethtool setting
#   * unbinds a driver, binds vfio-pci, or detaches the device
#   * mmaps BAR0 / resource0 or issues any raw MMIO
#   * reads read-to-clear / RW1C registers itself — it rides `ethtool -d`,
#     which already excludes EICR and the statistics counters.
#
# Every device command this script issues is spelled out in the run_ro()
# allowlist below; there is no other path to the device. A reviewer can
# confirm read-only-ness by checking that (a) all device access goes through
# run_ro(), and (b) run_ro()'s first argument is always one of the allowlisted
# read verbs. Grep for `ethtool` in this file: every occurrence is a query.
#
# ----------------------------------------------------------------------------
# THE HONESTY CAVEAT (what this card can and cannot tell us)
# ----------------------------------------------------------------------------
# This card is POST-BRINGUP and KERNEL-CONFIGURED — NOT an idle/default card.
#
#   * `ethtool -e` (NVM/flash dump) is the AUTHORITATIVE pristine-default
#     source: bringup does not rewrite flash. LESM state and link-config
#     defaults are read from here as the primary evidence.
#   * `ethtool -d` (register dump) reflects the stock ixgbe driver's
#     POST-INIT register state, NOT the pristine power-on default. It is
#     CORROBORATION, not authority. For bits the driver does not synthesize
#     (the SFI PMA/PMD select and AUTOC2[19]) the live register is still
#     EEPROM-representative — but the NVM dump remains the authority.
#
# This grounds THIS specific board's static config (a strong prior + confirmed
# anchor for this card). It is one X520/82599 SKU; the prototype cards in the
# mail may differ — re-read them the same way. It answers NO dynamic bring-up
# question (minimal AUTOC writes, link-up form, throughput, buffer sizing):
# those require a card we can drive.
#
# Usage:
#   live-card-probe.sh [--help] [<ifname> | <pci-bdf>]
#
#   <ifname>   e.g. eth0, enp1s0f0   (preferred)
#   <pci-bdf>  e.g. 0000:01:00.0 or 01:00.0
#   (no arg)   list candidate ixgbe interfaces and exit
#
# Privilege: ethtool -e/-d/-m typically need root/CAP_NET_ADMIN; sysfs and
# lspci usually do not. Missing privilege => the affected check is reported as
# SKIPPED, not fatal. Missing ethtool/lspci => degrade gracefully.
#
# Companion spec: docs/nic/tools/live-card-probe.md
# ----------------------------------------------------------------------------

set -euo pipefail

# ============================================================================
# Constants — 82599 identity and register/NVM definitions
#   Sourced from reference/linux/.../ixgbe/{ixgbe_type.h,ixgbe_82599.c,
#   ixgbe_ethtool.c} and the 82599 datasheet 331520. Offsets cited inline.
# ============================================================================

readonly INTEL_VENDOR="8086"
# Known 82599 / X520 PCI device IDs (ixgbe_type.h IXGBE_DEV_ID_82599_*).
# Used only to confirm "this is an 82599" — driver name is the primary signal.
readonly DEV_IDS_82599="10fb 10f8 10f9 1557 154d 1528 10fc 1529 152a 10f7 1514 1517 000c 151c"

# ethtool -d register offsets (the address ethtool prints for each), from
# ixgbe_type.h. ethtool's ixgbe dumper labels each by name; we match either.
readonly AUTOC_OFF="0x042a0"    # IXGBE_AUTOC  — regs_buff[1064]
readonly LINKS_OFF="0x042a4"    # IXGBE_LINKS  — regs_buff[1065]
readonly AUTOC2_OFF="0x042a8"   # IXGBE_AUTOC2 — regs_buff[1066]

# AUTOC bit fields (ixgbe_type.h)
readonly AUTOC_LMS_SHIFT=13
readonly AUTOC_LMS_MASK=0x7        # (0x7 << 13)
readonly AUTOC_LMS_10G_SERIAL=0x3  # (0x3 << 13)>>13

# AUTOC2 bit fields (ixgbe_type.h)
readonly AUTOC2_SERIAL_PMA_PMD_SHIFT=16
readonly AUTOC2_SERIAL_PMA_PMD_MASK=0x3   # [17:16]
# values of the [17:16] field: 0=KR 1=XFI 2=SFI
readonly AUTOC2_PMA_SFI=2
# AUTOC2 bit 19 — erratum 13: must be 1, EEPROM-set.
readonly AUTOC2_BIT19_MASK=0x00080000

# LINKS status bits (ixgbe_type.h)
readonly LINKS_UP_MASK=0x40000000
readonly LINKS_SPEED_MASK=0x30000000   # [29:28] on 82599
readonly LINKS_SPEED_10G=0x30000000
readonly LINKS_SPEED_1G=0x20000000
readonly LINKS_SPEED_100=0x10000000

# NVM (flash) pointer-chase for LESM, from ixgbe_82599.c
# :ixgbe_verify_lesm_fw_enabled_82599() and ixgbe_type.h constants.
# All are 16-bit WORD offsets into the EEPROM.
#   word 0x0F            = Firmware Module pointer  (IXGBE_FW_PTR)
#   fw + 0x2            = LESM Parameters pointer   (IXGBE_FW_LESM_PARAMETERS_PTR)
#   lesm + 0x1          = LESM State word 1         (IXGBE_FW_LESM_STATE_1)
#   state & 0x8000      = LESM enabled              (IXGBE_FW_LESM_STATE_ENABLED)
readonly NVM_FW_PTR_WORD=0x0F
readonly NVM_LESM_PARAMS_PTR_OFF=0x2
readonly NVM_LESM_STATE_OFF=0x1
readonly NVM_LESM_ENABLED_BIT=0x8000

# ============================================================================
# Output helpers
# ============================================================================

# Plain ASCII verdict markers so the report is greppable and terminal-safe.
readonly MARK_OK="[ OK ]"
readonly MARK_INFO="[INFO]"
readonly MARK_WARN="[WARN]"
readonly MARK_SKIP="[SKIP]"

hr()      { printf '%s\n' "----------------------------------------------------------------------------"; }
section() { printf '\n== %s ==\n' "$1"; }
info()    { printf '  %s %s\n' "$MARK_INFO" "$*"; }
ok()      { printf '  %s %s\n' "$MARK_OK"   "$*"; }
warn()    { printf '  %s %s\n' "$MARK_WARN" "$*"; }
skip()    { printf '  %s %s\n' "$MARK_SKIP" "$*"; }

# expected -> observed -> verdict line. Args: label, expected, observed, verdict-marker, note
report() {
  local label="$1" expected="$2" observed="$3" mark="$4" note="${5:-}"
  printf '  %s %-28s expected=%-22s observed=%s\n' "$mark" "$label" "$expected" "$observed"
  [ -n "$note" ] && printf '         %s\n' "$note"
  return 0
}

die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# ============================================================================
# run_ro — THE ONLY GATEWAY TO THE DEVICE.
# Enforces, at runtime, that the verb is on the read-only allowlist. Any
# attempt to call a non-allowlisted ethtool form aborts the script. This is
# belt-and-suspenders: the call sites already only pass read verbs, but this
# makes a stray write structurally impossible to execute.
# ============================================================================
run_ro() {
  local cmd="$1"; shift
  case "$cmd" in
    lspci) command lspci "$@"; return $? ;;
    ethtool)
      # First non-flag token after `ethtool` is the subcommand (or, for a
      # bare `ethtool <if>`, it is the interface — which is also read-only).
      local sub="${1:-}"
      case "$sub" in
        # Allowlisted read-only query subcommands ONLY.
        -i|--driver|-d|--register-dump|-e|--eeprom-dump|-m|--module-info|--show-eeprom)
          command ethtool "$@"; return $? ;;
        # Bare `ethtool <if>` (no subcommand) — link query, read-only.
        -*)
          die "run_ro: refusing non-allowlisted ethtool subcommand: $sub" ;;
        *)
          command ethtool "$@"; return $? ;;
      esac ;;
    *)
      die "run_ro: refusing non-allowlisted command: $cmd" ;;
  esac
}

have() { command -v "$1" >/dev/null 2>&1; }

# ============================================================================
# Numeric helpers (bash arithmetic is 64-bit signed; all our values fit)
# ============================================================================

# hex string (with or without 0x) -> decimal, via bash base-16.
hex2dec() {
  local h="${1#0x}"; h="${h#0X}"
  [ -z "$h" ] && { echo 0; return; }
  printf '%d' "$((16#$h))"
}

# ============================================================================
# Argument / discovery
# ============================================================================

print_help() {
  sed -n '2,80p' "$0" | sed 's/^#\{0,1\} \{0,1\}//'
}

# List ixgbe-bound interfaces by walking sysfs. Read-only.
list_ixgbe_ifaces() {
  local found=0 nd drv ifn
  if [ -d /sys/class/net ]; then
    for nd in /sys/class/net/*; do
      [ -e "$nd/device/driver" ] || continue
      drv="$(basename "$(readlink -f "$nd/device/driver" 2>/dev/null)" 2>/dev/null || true)"
      if [ "$drv" = "ixgbe" ]; then
        ifn="$(basename "$nd")"
        printf '  %s  (driver=ixgbe, pci=%s)\n' \
          "$ifn" "$(basename "$(readlink -f "$nd/device" 2>/dev/null)" 2>/dev/null || echo '?')"
        found=1
      fi
    done
  fi
  return $((found == 0))
}

# Resolve an arg that may be an ifname OR a pci bdf into both forms.
# Sets globals: IFNAME, BDF  (either may be empty if unresolvable).
IFNAME=""
BDF=""
resolve_target() {
  local arg="$1"
  # Looks like a PCI BDF?  [dddd:]bb:dd.f
  if printf '%s' "$arg" | grep -Eq '^([0-9a-fA-F]{4}:)?[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-7]$'; then
    # normalize to full domain form
    if printf '%s' "$arg" | grep -Eq '^[0-9a-fA-F]{2}:'; then
      BDF="0000:$arg"
    else
      BDF="$arg"
    fi
    # find the ifname (if any) bound to this bdf
    local nd
    for nd in /sys/class/net/*; do
      [ -e "$nd/device" ] || continue
      if [ "$(basename "$(readlink -f "$nd/device" 2>/dev/null)" 2>/dev/null)" = "$BDF" ]; then
        IFNAME="$(basename "$nd")"
        break
      fi
    done
  else
    IFNAME="$arg"
    if [ -e "/sys/class/net/$IFNAME/device" ]; then
      BDF="$(basename "$(readlink -f "/sys/class/net/$IFNAME/device" 2>/dev/null)" 2>/dev/null || true)"
    fi
  fi
}

# ============================================================================
# Identity confirmation — MUST pass before any 82599-specific offset is read.
# ============================================================================

# Globals populated by confirm_identity()
DRV_NAME=""
PCI_VENDOR=""
PCI_DEVICE=""
PCI_REVISION=""

is_82599_devid() {
  local d="$1" id
  for id in $DEV_IDS_82599; do
    [ "$d" = "$id" ] && return 0
  done
  return 1
}

confirm_identity() {
  section "Identity confirmation"

  # (a) driver name via ethtool -i (preferred) or sysfs.
  if [ -n "$IFNAME" ] && have ethtool; then
    DRV_NAME="$(run_ro ethtool -i "$IFNAME" 2>/dev/null | awk -F': ' '/^driver:/{print $2; exit}')" || true
  fi
  if [ -z "$DRV_NAME" ] && [ -n "$IFNAME" ] && [ -e "/sys/class/net/$IFNAME/device/driver" ]; then
    DRV_NAME="$(basename "$(readlink -f "/sys/class/net/$IFNAME/device/driver" 2>/dev/null)" 2>/dev/null || true)"
  fi
  if [ -z "$DRV_NAME" ] && [ -n "$BDF" ] && [ -e "/sys/bus/pci/devices/$BDF/driver" ]; then
    DRV_NAME="$(basename "$(readlink -f "/sys/bus/pci/devices/$BDF/driver" 2>/dev/null)" 2>/dev/null || true)"
  fi

  # (b) PCI vendor/device/revision via sysfs (no privilege needed).
  if [ -n "$BDF" ] && [ -d "/sys/bus/pci/devices/$BDF" ]; then
    [ -r "/sys/bus/pci/devices/$BDF/vendor" ]   && PCI_VENDOR="$(sed 's/^0x//' "/sys/bus/pci/devices/$BDF/vendor" 2>/dev/null || true)"
    [ -r "/sys/bus/pci/devices/$BDF/device" ]   && PCI_DEVICE="$(sed 's/^0x//' "/sys/bus/pci/devices/$BDF/device" 2>/dev/null || true)"
    [ -r "/sys/bus/pci/devices/$BDF/revision" ] && PCI_REVISION="$(sed 's/^0x//' "/sys/bus/pci/devices/$BDF/revision" 2>/dev/null || true)"
  fi

  info "interface = ${IFNAME:-<none>}   pci-bdf = ${BDF:-<unknown>}"
  info "bound driver = ${DRV_NAME:-<unknown>}"
  info "pci id = ${PCI_VENDOR:-????}:${PCI_DEVICE:-????}  revision = ${PCI_REVISION:-??}"

  # Verdict: require ixgbe driver OR a known 82599 device id.
  local driver_ok=1 devid_ok=1
  [ "$DRV_NAME" = "ixgbe" ] && driver_ok=0
  if [ "${PCI_VENDOR,,}" = "$INTEL_VENDOR" ] && [ -n "$PCI_DEVICE" ] && is_82599_devid "${PCI_DEVICE,,}"; then
    devid_ok=0
  fi

  if [ $driver_ok -eq 0 ] || [ $devid_ok -eq 0 ]; then
    if [ $devid_ok -eq 0 ]; then
      ok "confirmed 82599 (vendor:device ${PCI_VENDOR}:${PCI_DEVICE}, driver=${DRV_NAME:-?})"
    else
      ok "driver is ixgbe; device id ${PCI_VENDOR:-?}:${PCI_DEVICE:-?} not in known-82599 list (newer/variant SKU). Proceeding cautiously."
    fi
    return 0
  fi

  warn "This device is NOT an 82599/ixgbe (driver=${DRV_NAME:-?}, id=${PCI_VENDOR:-?}:${PCI_DEVICE:-?})."
  warn "Refusing to read 82599-specific register/NVM offsets off a foreign NIC."
  return 1
}

# ============================================================================
# IOMMU group composition + PCIe functions of the card
# Grounds kernel-bypass-and-dma.md §5.1.
# ============================================================================
check_iommu_and_functions() {
  section "IOMMU group composition + PCIe functions  [kernel-bypass-and-dma.md §5.1]"
  [ -z "$BDF" ] && { skip "no PCI BDF resolved; cannot inspect IOMMU group."; return 0; }

  local grp="/sys/bus/pci/devices/$BDF/iommu_group"
  if [ -d "$grp/devices" ]; then
    local gid members
    gid="$(basename "$(readlink -f "$grp" 2>/dev/null)" 2>/dev/null || echo '?')"
    members="$(ls -1 "$grp/devices" 2>/dev/null | tr '\n' ' ' | sed 's/ $//')"
    report "iommu group" "(observe)" "group $gid = { $members }" "$MARK_INFO" \
      "If the second (dark) function shares this group, bind BOTH functions to vfio-pci to own the group."
  else
    skip "no iommu_group for $BDF — IOMMU/VT-d likely disabled in BIOS or not bound. (Enable intel_iommu=on for VFIO.)"
  fi

  # All PCIe functions of this physical card (same bus:device, differing .function).
  local base="${BDF%.*}"   # e.g. 0000:01:00
  local fns
  fns="$(ls -1d /sys/bus/pci/devices/${base}.* 2>/dev/null | xargs -n1 basename 2>/dev/null | tr '\n' ' ' | sed 's/ $//')" || true
  [ -n "$fns" ] && info "card PCIe functions (this bus:device): $fns"
}

# ============================================================================
# Baseline: link state + firmware/driver version
# ============================================================================
check_baseline() {
  section "Baseline: driver / link / firmware"
  if [ -z "$IFNAME" ]; then
    skip "no interface name; link/firmware query needs an ifname (have BDF only)."
    return 0
  fi
  if ! have ethtool; then
    skip "ethtool not installed; cannot query link/driver info."
    return 0
  fi

  local drvinfo
  if drvinfo="$(run_ro ethtool -i "$IFNAME" 2>/dev/null)"; then
    printf '%s\n' "$drvinfo" | awk -F': ' '
      /^driver:/   {printf "  '"$MARK_INFO"' driver       = %s\n",$2}
      /^version:/  {printf "  '"$MARK_INFO"' drv version  = %s\n",$2}
      /^firmware-version:/ {printf "  '"$MARK_INFO"' firmware     = %s\n",$2}
      /^bus-info:/ {printf "  '"$MARK_INFO"' bus-info     = %s\n",$2}'
  else
    skip "ethtool -i failed (permissions or unsupported)."
  fi

  # Plain `ethtool <if>` — link state, speed. Read-only.
  local linkinfo
  if linkinfo="$(run_ro ethtool "$IFNAME" 2>/dev/null)"; then
    local speed link
    speed="$(printf '%s\n' "$linkinfo" | awk -F': ' '/Speed:/{print $2; exit}')"
    link="$(printf '%s\n' "$linkinfo" | awk -F': ' '/Link detected:/{print $2; exit}')"
    info "link detected = ${link:-?}   speed = ${speed:-?}"
  else
    skip "plain ethtool <if> failed."
  fi
}

# ============================================================================
# NVM / EEPROM dump  — THE AUTHORITATIVE pristine-default source.
# Grounds device-bring-up-and-link.md §5.2 (LESM) + link-config defaults.
#
# Parses the `ethtool -e <if>` text dump:
#     Offset      Values
#     ------      ------
#     0x0000:     11 22 33 44 ...
# On x86 the kernel emits each NVM word little-endian, so:
#     word(N) = bytes[2N] | (bytes[2N+1] << 8)
# (Confirmed in ixgbe_get_eeprom(): words are le16_to_cpu()'d then memcpy'd
#  into the byte buffer ethtool prints; host is little-endian.)
# ============================================================================

# Holds the flattened byte array of the EEPROM dump, indexed by byte offset.
# Implemented as an associative array EEPROM_BYTE[offset]=hexbyte.
declare -A EEPROM_BYTE
EEPROM_LOADED=0

# Parse `ethtool -e` text into EEPROM_BYTE[]. Returns 0 on success.
parse_eeprom_dump() {
  local dump="$1"
  local line off vals first b i
  EEPROM_BYTE=()
  while IFS= read -r line; do
    # Match lines like "0x0010: 11 22 33 ..." (colon-terminated hex offset).
    case "$line" in
      0x*:*|0X*:*)
        off="${line%%:*}"
        vals="${line#*:}"
        # strip the offset's 0x and convert to decimal base
        first="$(hex2dec "$off")"
        i=0
        for b in $vals; do
          # only accept 2-hex-digit byte tokens
          case "$b" in
            [0-9a-fA-F][0-9a-fA-F])
              EEPROM_BYTE[$((first + i))]="$b"
              i=$((i + 1)) ;;
            *) : ;;  # ignore stray tokens
          esac
        done
        ;;
      *) : ;;
    esac
  done <<EOF
$dump
EOF
  [ "${#EEPROM_BYTE[@]}" -gt 0 ] && { EEPROM_LOADED=1; return 0; }
  return 1
}

# Read a 16-bit NVM word at WORD offset $1 from the parsed dump.
# Echoes a 0x-prefixed 4-hex-digit value, or empty string if out of range.
nvm_word() {
  local woff_dec; woff_dec="$(hex2dec "$1")"
  local byte_lo=$((woff_dec * 2)) byte_hi=$((woff_dec * 2 + 1))
  local lo="${EEPROM_BYTE[$byte_lo]:-}" hi="${EEPROM_BYTE[$byte_hi]:-}"
  [ -z "$lo" ] || [ -z "$hi" ] && { echo ""; return 1; }
  printf '0x%02x%02x' "$((16#$hi))" "$((16#$lo))"
}

check_nvm() {
  section "NVM / EEPROM dump (ethtool -e)  — AUTHORITATIVE pristine default  [device-bring-up-and-link.md §5.2]"
  if [ -z "$IFNAME" ]; then skip "no interface name; ethtool -e needs an ifname."; return 0; fi
  if ! have ethtool; then skip "ethtool not installed."; return 0; fi

  local dump rc=0
  dump="$(run_ro ethtool -e "$IFNAME" 2>/dev/null)" || rc=$?
  if [ $rc -ne 0 ] || [ -z "$dump" ]; then
    if [ "$(id -u)" -ne 0 ]; then
      skip "ethtool -e failed — needs root/CAP_NET_ADMIN. Re-run as root to read NVM (the LESM authority)."
    else
      skip "ethtool -e failed or empty (driver may not support get_eeprom dump)."
    fi
    return 0
  fi

  if ! parse_eeprom_dump "$dump"; then
    skip "could not parse ethtool -e output (unrecognized format). Raw words unavailable."
    return 0
  fi
  info "parsed ${#EEPROM_BYTE[@]} EEPROM bytes from ethtool -e dump."

  # --- LESM detection: the authoritative pointer-chase from ixgbe_82599.c ---
  #   fw_offset        = word(0x0F)
  #   lesm_param_off   = word(fw_offset + 0x2)
  #   lesm_state       = word(lesm_param_off + 0x1)
  #   enabled          = lesm_state & 0x8000
  local fw_ptr fw_dec lesm_pp_word lesm_pp_dec lesm_state
  fw_ptr="$(nvm_word "$NVM_FW_PTR_WORD" || true)"
  info "NVM[0x0F] Firmware-Module pointer = ${fw_ptr:-<unreadable>}"

  if [ -z "$fw_ptr" ]; then
    report "LESM enabled (NVM)" "!=0/0xFFFF chain" "FW ptr unreadable" "$MARK_SKIP" \
      "Dump did not cover word 0x0F. Re-dump full EEPROM to evaluate."
  elif [ "$fw_ptr" = "0x0000" ] || [ "$fw_ptr" = "0xffff" ]; then
    report "LESM enabled (NVM)" "chain valid" "FW ptr=$fw_ptr -> DISABLED" "$MARK_OK" \
      "Firmware Module pointer absent => LESM disabled. Minimal posture (no sw/fw semaphore). Matches preferred default."
  else
    fw_dec="$(hex2dec "$fw_ptr")"
    lesm_pp_word="$(printf '0x%x' $((fw_dec + $(hex2dec "$NVM_LESM_PARAMS_PTR_OFF"))) )"
    lesm_pp_word="$(nvm_word "$lesm_pp_word" || true)"
    info "NVM[fw+0x2] LESM-Parameters pointer = ${lesm_pp_word:-<unreadable>}"
    if [ -z "$lesm_pp_word" ]; then
      report "LESM enabled (NVM)" "chain valid" "LESM-param ptr unreadable" "$MARK_SKIP" \
        "Dump did not cover word $(printf '0x%x' $((fw_dec + 2))). Re-dump full EEPROM."
    elif [ "$lesm_pp_word" = "0x0000" ] || [ "$lesm_pp_word" = "0xffff" ]; then
      report "LESM enabled (NVM)" "chain valid" "LESM-param ptr=$lesm_pp_word -> DISABLED" "$MARK_OK" \
        "No LESM parameters block => LESM disabled. Minimal posture; no semaphore needed."
    else
      lesm_pp_dec="$(hex2dec "$lesm_pp_word")"
      lesm_state="$(nvm_word "$(printf '0x%x' $((lesm_pp_dec + $(hex2dec "$NVM_LESM_STATE_OFF"))))" || true)"
      info "NVM[lesm+0x1] LESM state word = ${lesm_state:-<unreadable>}"
      if [ -z "$lesm_state" ]; then
        report "LESM enabled (NVM)" "state bit 0x8000" "state word unreadable" "$MARK_SKIP" \
          "Dump did not cover the LESM state word. Re-dump full EEPROM."
      else
        local st_dec; st_dec="$(hex2dec "$lesm_state")"
        if [ $((st_dec & $(hex2dec "$NVM_LESM_ENABLED_BIT"))) -ne 0 ]; then
          report "LESM enabled (NVM)" "(observe)" "state=$lesm_state, bit15 SET -> ENABLED" "$MARK_WARN" \
            "LESM is ENABLED. On-chip FW also drives AUTOC; the link path must take the sw/fw semaphore (acquire_swfw_sync). Prefer disabling LESM in NVM if possible."
        else
          report "LESM enabled (NVM)" "(observe)" "state=$lesm_state, bit15 clear -> DISABLED" "$MARK_OK" \
            "LESM disabled. No sw/fw semaphore needed; link path is purely ours. Matches preferred minimal posture."
        fi
      fi
    fi
  fi

  # --- A few link-config-relevant words, presented raw with offsets ---
  # These offsets are board/SKU specific in the NVM map; we present raw hex and
  # state honestly that the AUTHORITY for the live link-electrical config is the
  # AUTOC2 read below corroborated by these flash words, not a fabricated map.
  info "Selected NVM words (raw; interpret against the 82599 NVM map for this SKU):"
  local w
  for w in 0x00 0x01 0x02 0x03 0x0F 0x10 0x20 0x28 0x30; do
    local v; v="$(nvm_word "$w" || true)"
    [ -n "$v" ] && printf '         NVM[%s] = %s\n' "$w" "$v"
  done
  info "Note: the link-electrical config (SFI + AUTOC2[19]) is loaded from flash into AUTOC2 at reset;"
  info "      the register read below is EEPROM-representative for those bits even post-init."
}

# ============================================================================
# Register dump (ethtool -d) — POST-INIT state, CORROBORATION not authority.
# Grounds device-bring-up-and-link.md §3,§5.3.
#
# Parses ethtool's ixgbe register dump. ethtool labels registers by NAME and/or
# prints the register offset. We match on offset (0x042A0/4/8) first, then fall
# back to name tokens (AUTOC/LINKS/AUTOC2). If neither matches, degrade: print
# the raw lines we think are relevant and say the format was unrecognized.
# ============================================================================

# Extract a 32-bit register value from `ethtool -d` text given its offset hex
# (e.g. 0x042a0) and/or a name token (e.g. AUTOC2). Echoes 0xXXXXXXXX or "".
extract_reg() {
  local dump="$1" off="$2" name="$3"
  local val=""
  # Strategy 1: match a line containing the offset followed by a hex value.
  # ethtool prints e.g.:  "0x042A0: AUTOC (...)    0x00003D00"
  val="$(printf '%s\n' "$dump" | grep -iE "(^|[^0-9a-fA-Fx])${off}([^0-9a-fA-F]|:)" \
          | grep -oiE '0x[0-9a-fA-F]{8}' | tail -n1 || true)"
  if [ -n "$val" ]; then echo "$val"; return 0; fi
  # Strategy 2: match by register NAME token, take the last 0xXXXXXXXX on the line.
  val="$(printf '%s\n' "$dump" | grep -iE "(^|[^A-Za-z])${name}([^A-Za-z0-9]|$)" \
          | grep -oiE '0x[0-9a-fA-F]{8}' | tail -n1 || true)"
  [ -n "$val" ] && { echo "$val"; return 0; }
  echo ""
  return 1
}

check_registers() {
  section "Register dump (ethtool -d)  — POST-INIT, corroboration only  [device-bring-up-and-link.md §3,§5.3]"
  if [ -z "$IFNAME" ]; then skip "no interface name; ethtool -d needs an ifname."; return 0; fi
  if ! have ethtool; then skip "ethtool not installed."; return 0; fi

  local dump rc=0
  dump="$(run_ro ethtool -d "$IFNAME" 2>/dev/null)" || rc=$?
  if [ $rc -ne 0 ] || [ -z "$dump" ]; then
    if [ "$(id -u)" -ne 0 ]; then
      skip "ethtool -d failed — needs root/CAP_NET_ADMIN. Re-run as root to read registers."
    else
      skip "ethtool -d failed or empty."
    fi
    return 0
  fi
  info "These readings are the stock ixgbe driver's POST-INIT register state — NOT pristine defaults."

  local autoc autoc2 links
  autoc="$(extract_reg "$dump" "$AUTOC_OFF" "AUTOC" || true)"
  links="$(extract_reg "$dump" "$LINKS_OFF" "LINKS" || true)"
  autoc2="$(extract_reg "$dump" "$AUTOC2_OFF" "AUTOC2" || true)"

  if [ -z "$autoc" ] && [ -z "$autoc2" ] && [ -z "$links" ]; then
    skip "Could not locate AUTOC/AUTOC2/LINKS in ethtool -d output (format not recognized)."
    info "Relevant raw lines (best-effort) for manual inspection:"
    printf '%s\n' "$dump" | grep -iE 'AUTOC|LINKS|0x042a[048]' | sed 's/^/         /' || true
    return 0
  fi

  info "raw: AUTOC=${autoc:-?}  AUTOC2=${autoc2:-?}  LINKS=${links:-?}"

  # --- AUTOC.LMS — expect 10G_SERIAL (0x3) ---
  if [ -n "$autoc" ]; then
    local a; a="$(hex2dec "$autoc")"
    local lms=$(( (a >> AUTOC_LMS_SHIFT) & $(hex2dec "$AUTOC_LMS_MASK") ))
    if [ "$lms" -eq "$(hex2dec "$AUTOC_LMS_10G_SERIAL")" ]; then
      report "AUTOC.LMS" "10G_SERIAL(0x3)" "$(printf '0x%x' "$lms") = 10G_SERIAL" "$MARK_OK" \
        "Link mode select = 10G serial, as expected for SFI/DAC."
    else
      report "AUTOC.LMS" "10G_SERIAL(0x3)" "$(printf '0x%x' "$lms")" "$MARK_WARN" \
        "Live LMS is not 10G_SERIAL (post-init driver value)."
    fi
  fi

  # --- AUTOC2 serial PMA/PMD select [17:16] — expect SFI (0x2) ---
  if [ -n "$autoc2" ]; then
    local a2; a2="$(hex2dec "$autoc2")"
    local pma=$(( (a2 >> AUTOC2_SERIAL_PMA_PMD_SHIFT) & $(hex2dec "$AUTOC2_SERIAL_PMA_PMD_MASK") ))
    local pma_name
    case "$pma" in
      0) pma_name="KR" ;; 1) pma_name="XFI" ;; 2) pma_name="SFI" ;; *) pma_name="?" ;;
    esac
    if [ "$pma" -eq "$AUTOC2_PMA_SFI" ]; then
      report "AUTOC2 serial PMA/PMD" "SFI(0x2)" "$(printf '0x%x' "$pma") = SFI" "$MARK_OK" \
        "SFI selected. Driver does NOT synthesize this field => EEPROM-set; corroborates the flash default."
    else
      report "AUTOC2 serial PMA/PMD" "SFI(0x2)" "$(printf '0x%x' "$pma") = $pma_name" "$MARK_WARN" \
        "Serial PMA/PMD is not SFI."
    fi

    # --- AUTOC2 bit 19 — erratum 13 — expect 1 (EEPROM-set) ---
    if [ $((a2 & $(hex2dec "$AUTOC2_BIT19_MASK"))) -ne 0 ]; then
      report "AUTOC2[19] (erratum 13)" "1 (EEPROM-set)" "1" "$MARK_OK" \
        "Bit 19 set. Driver preserves it from cached EEPROM-default AUTOC2; favorable. (NVM dump is the authority.)"
    else
      report "AUTOC2[19] (erratum 13)" "1 (EEPROM-set)" "0" "$MARK_WARN" \
        "Bit 19 CLEAR. Erratum 13 (B0, NoFix): TX path can hang on link-speed clock switch. Expected EEPROM to set it."
    fi
  fi

  # --- LINKS speed/up ---
  if [ -n "$links" ]; then
    local l; l="$(hex2dec "$links")"
    local up="down" spd="unknown"
    [ $((l & $(hex2dec "$LINKS_UP_MASK"))) -ne 0 ] && up="up"
    case $((l & $(hex2dec "$LINKS_SPEED_MASK"))) in
      $(hex2dec "$LINKS_SPEED_10G")) spd="10G" ;;
      $(hex2dec "$LINKS_SPEED_1G"))  spd="1G"  ;;
      $(hex2dec "$LINKS_SPEED_100")) spd="100M";;
      *) spd="other/none" ;;
    esac
    report "LINKS (speed/up)" "up,10G" "link=$up, speed=$spd" \
      "$([ "$up" = "up" ] && [ "$spd" = "10G" ] && echo "$MARK_OK" || echo "$MARK_INFO")" \
      "Live link status (post-init; reflects current cable/peer state)."
  fi
}

# ============================================================================
# Module EEPROM (ethtool -m) — SFF-8472 identity. device-bring-up-and-link.md §5.4
# ============================================================================
check_module() {
  section "Module EEPROM (ethtool -m, SFF-8472 identity)  [device-bring-up-and-link.md §5.4]"
  if [ -z "$IFNAME" ]; then skip "no interface name; ethtool -m needs an ifname."; return 0; fi
  if ! have ethtool; then skip "ethtool not installed."; return 0; fi

  local mod rc=0
  mod="$(run_ro ethtool -m "$IFNAME" 2>/dev/null)" || rc=$?
  if [ $rc -ne 0 ] || [ -z "$mod" ]; then
    if [ "$(id -u)" -ne 0 ]; then
      skip "ethtool -m failed — needs root/CAP_NET_ADMIN, or module has no readable I2C EEPROM (passive DAC may be sparse)."
    else
      skip "ethtool -m failed/empty — passive DAC may expose little/no SFF data, or driver declines I2C."
    fi
    return 0
  fi

  local ident conn vendor pn cabletype
  ident="$(printf '%s\n' "$mod" | awk -F': ' '/Identifier/{print $2; exit}')"
  conn="$(printf '%s\n' "$mod"  | awk -F': ' '/Connector/{print $2; exit}')"
  vendor="$(printf '%s\n' "$mod"| awk -F': ' '/Vendor name/{print $2; exit}')"
  pn="$(printf '%s\n' "$mod"    | awk -F': ' '/Vendor PN/{print $2; exit}')"
  cabletype="$(printf '%s\n' "$mod" | awk -F': ' '/[Cc]able/{print $0; exit}')"

  report "SFF-8472 identity" "passive DAC (da_cu)" "${ident:-?} / ${conn:-?}" "$MARK_INFO" \
    "vendor=${vendor:-?} PN=${pn:-?}"
  [ -n "$cabletype" ] && info "cable: $(printf '%s' "$cabletype" | sed 's/^[[:space:]]*//')"
  info "(Confirms what the passive DAC presents; grounds the da_cu assumption. Module ID is driver policy, not a link prerequisite.)"
}

# ============================================================================
# Header / footer caveats
# ============================================================================
print_header() {
  hr
  printf '  live-card-probe — read-only 82599/X520 static-config grounding\n'
  printf '  THIS IS A READ-ONLY PROBE. It cannot change the device or the link.\n'
  hr
  printf '  Honesty caveat: this card is POST-BRINGUP, KERNEL-CONFIGURED — not idle/default.\n'
  printf '    * ethtool -e (NVM/flash) = AUTHORITATIVE pristine default (LESM, link-config).\n'
  printf '    * ethtool -d (registers) = POST-INIT driver state; corroboration, not authority.\n'
  hr
}

print_footer() {
  hr
  printf '  Scope of this grounding:\n'
  printf '    * Confirms THIS board'\''s STATIC config (strong prior + confirmed anchor for THIS card).\n'
  printf '    * One X520/82599 SKU — prototype cards in the mail may differ; re-read them the same way.\n'
  printf '    * Answers NO dynamic bring-up question (minimal AUTOC writes, link-up form,\n'
  printf '      throughput, buffer sizing). Those require a card we can drive.\n'
  printf '    * NVM (ethtool -e) is the authority for LESM + link-config defaults; registers\n'
  printf '      (ethtool -d) corroborate but reflect the stock driver'\''s post-init state.\n'
  hr
}

# ============================================================================
# main
# ============================================================================
main() {
  if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
    print_help
    exit 0
  fi

  # Tool availability (degrade gracefully).
  have ethtool || warn "ethtool not found — ethtool-based checks will be SKIPPED." >&2
  have lspci   || true  # lspci is optional; sysfs covers PCI identity.

  if [ $# -eq 0 ]; then
    print_header
    section "No target given — candidate ixgbe interfaces"
    if list_ixgbe_ifaces; then
      printf '\n  Re-run with one of the above, e.g.:  %s <ifname>\n' "$(basename "$0")"
    else
      warn "No ixgbe-bound interfaces found under /sys/class/net."
      info "You can also pass a PCI BDF directly, e.g.:  $(basename "$0") 0000:01:00.0"
    fi
    print_footer
    exit 0
  fi

  resolve_target "$1"
  if [ -z "$IFNAME" ] && [ -z "$BDF" ]; then
    die "could not resolve '$1' to an interface or PCI device. Run with no args to list candidates."
  fi

  print_header

  # IDENTITY GATE — must pass before any 82599-specific offset is read.
  if ! confirm_identity; then
    print_footer
    exit 2
  fi

  check_iommu_and_functions
  check_baseline
  check_nvm          # authoritative
  check_registers    # corroboration
  check_module

  print_footer
}

main "$@"
