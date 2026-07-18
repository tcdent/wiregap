//! Embed BUILD-time provenance (rustc, profile, rustflags, git rev) as env vars the binary reads at
//! runtime, so a capture self-documents exactly how its binary was produced. The RUN-time bin-sha (hash of
//! /proc/self/exe) is computed by the library at startup and is the coherence anchor.

use std::process::Command;

fn env_or(k: &str, default: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| default.into())
}

fn main() {
    let rustc_bin = env_or("RUSTC", "rustc");
    let rustc = Command::new(rustc_bin)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=BENCH_RUSTC={rustc}");

    println!("cargo:rustc-env=BENCH_PROFILE={}", env_or("PROFILE", "unknown"));

    // The encoded rustflags carry the --remap-path-prefix that makes the hash host-independent; decode the
    // \x1f unit separators to spaces for a readable, reproducible record.
    let flags = std::env::var("CARGO_ENCODED_RUSTFLAGS")
        .unwrap_or_default()
        .replace('\x1f', " ");
    let flags = if flags.trim().is_empty() { "(none)".to_string() } else { flags };
    println!("cargo:rustc-env=BENCH_RUSTFLAGS={flags}");

    let git = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "nogit".into());
    println!("cargo:rustc-env=BENCH_GIT={git}");

    // Rebuild provenance whenever the flags/env change so the embedded record stays truthful.
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");
    println!("cargo:rerun-if-changed=build.rs");
}
