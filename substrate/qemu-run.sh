#!/usr/bin/env bash
# QEMU UEFI/OVMF serial-assert harness — Slice-1 surface (b). Builds the UEFI app, boots it under OVMF,
# captures the serial log, and self-pins everything nic-review needs to RE-DERIVE (rebuild at the same
# source) and RE-RUN (same invocation): the .efi sha256, the OVMF build + sha256, rustc, and the exact
# QEMU command line. The binary prints the internal half of the pin (build-id/rustc) over serial.
set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROFILE="${PROFILE:-debug}"
TARGET=x86_64-unknown-uefi
OVMF_CODE="${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE.fd}"
OVMF_VARS="${OVMF_VARS:-/usr/share/OVMF/OVMF_VARS.fd}"
WORK="$CRATE_DIR/target/qemu"
ESP="$WORK/esp"
LOG="${LOG:-$WORK/serial.log}"

# 1. Build the UEFI binary.
build_flags=()
[ "$PROFILE" = release ] && build_flags=(--release)
( cd "$CRATE_DIR" && cargo build --target "$TARGET" "${build_flags[@]}" )
EFI="$CRATE_DIR/target/$TARGET/$PROFILE/substrate.efi"

# 2. Stage a fresh ESP with the app at the removable-media fallback path, + a writable varstore copy.
rm -rf "$WORK"; mkdir -p "$ESP/EFI/BOOT"
cp "$EFI" "$ESP/EFI/BOOT/BOOTX64.EFI"
cp "$OVMF_VARS" "$WORK/OVMF_VARS.fd"

# 3. The exact QEMU invocation. q35 = PCIe/ECAM (MCFG) for the slice-2 read. QEMU 7.2 has no 82599
#    model, so the ECAM *mechanism* is validated against e1000e (82574L: PCIe, memory BAR0, MSI-X)
#    placed at a KNOWN BDF 00:03.0 (addr=3 on pcie.0); the binary targets exactly that BDF. netdev
#    none = a NIC that exists in config space without a backend. isa-debug-exit = deterministic exit.
QEMU=qemu-system-x86_64
cmd=( "$QEMU"
  -machine q35 -m 512M
  -drive "if=pflash,format=raw,readonly=on,file=$OVMF_CODE"
  -drive "if=pflash,format=raw,file=$WORK/OVMF_VARS.fd"
  -drive "format=raw,file=fat:rw:$ESP"
  -netdev "user,id=n0"
  -device "e1000e,netdev=n0,bus=pcie.0,addr=3"
  -device isa-debug-exit,iobase=0xf4,iosize=0x04
  -serial stdio -display none -no-reboot )

# 4. External pin block.
{
  echo "==== HARNESS PIN (qemu-run.sh) ===="
  echo "qemu         : $("$QEMU" --version | head -1)"
  echo "ovmf-code    : $OVMF_CODE  sha256=$(sha256sum "$OVMF_CODE" | cut -d' ' -f1)"
  echo "ovmf-vars    : $OVMF_VARS  sha256=$(sha256sum "$OVMF_VARS" | cut -d' ' -f1)"
  echo "efi          : $EFI  sha256=$(sha256sum "$EFI" | cut -d' ' -f1)"
  echo "rustc        : $(rustc --version)"
  echo "qemu-cmdline : ${cmd[*]}"
  echo "==== BEGIN SERIAL ===="
} | tee "$LOG"

# 5. Boot (bounded so a hang can't wedge the session; isa-debug-exit normally ends it in ~1s).
set +e
timeout 60 "${cmd[@]}" 2>&1 | tee -a "$LOG"
qemu_status=${PIPESTATUS[0]}
set -e
echo "==== END SERIAL (qemu host-exit=$qemu_status) ====" | tee -a "$LOG"

# isa-debug-exit pass code 0x10 -> host status (0x10<<1)|1 = 33.
if [ "$qemu_status" -eq 33 ] && grep -q "RESULT   : PASS" "$LOG"; then
  echo "HARNESS: PASS (qemu host-exit 33 + serial RESULT PASS)"
  exit 0
else
  echo "HARNESS: FAIL (qemu host-exit=$qemu_status; see $LOG)"
  exit 1
fi
