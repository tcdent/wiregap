//! Emits the compile-time build identity the boot banner prints (self-pinning half (internal): the
//! git commit + dirty flag + the exact `rustc` that built the binary). Deterministic — no timestamp —
//! so `nic-review` re-derives the same `build-id` by checking out the same source and rebuilding. The
//! runner (`qemu-run.sh`) pins the external half (the `.efi` sha256 + the QEMU/OVMF invocation).

use std::process::Command;

fn main() {
    // git short SHA (12) + a `-dirty` marker when the working tree carries uncommitted changes — the
    // Slice-1 binary is built from an as-yet-uncommitted tree, so `-dirty` is the honest pin.
    let sha = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    let build_id = if dirty { format!("{sha}-dirty") } else { sha };
    println!("cargo:rustc-env=SUBSTRATE_BUILD_ID={build_id}");

    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let rustc_ver = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=SUBSTRATE_RUSTC={rustc_ver}");

    // Re-run when HEAD moves so the SHA stays current.
    println!("cargo:rerun-if-changed=../.git/HEAD");
}
