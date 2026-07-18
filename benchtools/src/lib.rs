//! Isolated verification/measurement tooling for the nic bench harnesses. NOT part of the production stack
//! (driver/reactor/transport) or the directpath integration lib — the harness bins depend on this; the
//! production modules do not reference it. Three concerns:
//!
//! - [`Provenance`] — the **coherence anchor**: the binary hashes its OWN `/proc/self/exe` at startup, so a
//!   capture's self-declared bin-sha is, by construction, the hash of the exact bytes that produced the
//!   numbers. This makes the shell-computed-hash-drift class of evidence incoherence impossible.
//! - [`affinity::pin`] — pin to one core so the MWAITX idle-wait/wake is not perturbed by migration.
//! - [`msr::Aperf`] — the APERF idle instrument (active cycles freeze in a C-state) via `/dev/cpu/<n>/msr`.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Self-documenting provenance of the RUNNING binary — computed live at startup.
pub struct Provenance {
    /// sha256 of `/proc/self/exe` (the actual executing bytes) — the coherence anchor.
    pub bin_sha256: String,
    pub rustc: &'static str,
    pub profile: &'static str,
    pub rustflags: &'static str,
    pub git: &'static str,
    pub host: String,
    pub unix_ts: u64,
}

impl Provenance {
    /// Capture the running binary's provenance. The bin-sha is `sha256sum` of THIS process's own executable
    /// path (`std::env::current_exe()`, resolved IN this process — NOT the literal "/proc/self/exe", which a
    /// child `sha256sum` would resolve to ITS OWN exe). Same tool a reviewer uses, on the exact executing
    /// bytes, so the declared hash byte-matches the delivered binary with no separate build step to drift.
    pub fn capture() -> Self {
        let exe = std::env::current_exe().ok();
        let bin_sha256 = exe
            .as_ref()
            .and_then(|p| Command::new("sha256sum").arg(p).output().ok())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.split_whitespace().next().map(str::to_string))
            .unwrap_or_else(|| "sha-unavailable".into());
        let host = Command::new("hostname")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "?".into());
        let unix_ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        Self {
            bin_sha256,
            rustc: env!("BENCH_RUSTC"),
            profile: env!("BENCH_PROFILE"),
            rustflags: env!("BENCH_RUSTFLAGS"),
            git: env!("BENCH_GIT"),
            host,
            unix_ts,
        }
    }

    /// A self-documenting header block for the capture log. Every number in the capture binds to this hash.
    pub fn header(&self, tool: &str) -> String {
        format!(
            "=== {tool} — self-declared provenance (the bin-sha is the RUNNING binary) ===\n\
             bin-sha256(current_exe): {}\n\
             rustc: {}\n\
             profile: {}\n\
             rustflags: {}\n\
             git: {}   host: {}   unix-ts: {}\n\
             reproduce: rebuild this bin with the same rustc + profile + rustflags, `sha256sum` must match the bin-sha above.",
            self.bin_sha256, self.rustc, self.profile, self.rustflags, self.git, self.host, self.unix_ts
        )
    }
}

pub mod affinity {
    /// Pin the calling thread to `core` (so MWAITX idle/wake timing is not perturbed by scheduler migration).
    pub fn pin(core: usize) -> std::io::Result<()> {
        // SAFETY: cpu_set_t is zero-initialized then populated via the documented CPU_ZERO/CPU_SET macros;
        // sched_setaffinity is passed the correct set size. Errors are surfaced, not ignored.
        unsafe {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            libc::CPU_ZERO(&mut set);
            libc::CPU_SET(core, &mut set);
            if libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

pub mod msr {
    use std::os::unix::fs::FileExt;

    const IA32_APERF: u64 = 0xE8;

    /// APERF reader for one core via `/dev/cpu/<core>/msr` (needs `modprobe msr` + root). APERF counts
    /// active (C0) cycles and freezes in a C-state, so `active(APERF) << wall(TSC)` across a wait proves the
    /// core genuinely idled (a spin gives `APERF ≈ TSC`).
    pub struct Aperf(std::fs::File);

    impl Aperf {
        pub fn open(core: usize) -> std::io::Result<Self> {
            Ok(Self(std::fs::File::open(format!("/dev/cpu/{core}/msr"))?))
        }

        /// Read IA32_APERF; `None` if the MSR read fails.
        pub fn read(&self) -> Option<u64> {
            let mut b = [0u8; 8];
            self.0.read_at(&mut b, IA32_APERF).ok().map(|_| u64::from_le_bytes(b))
        }
    }
}
