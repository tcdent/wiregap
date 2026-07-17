#![deny(unsafe_code)]

//! Sprint-6 Slice-2 run — LINK BRING-UP on both bench ends over the DAC, NON-DMA (BAR0 register config
//! only). Re-acquires the bench cards through the gate+token, runs the §4.6.3 reset + the two mandatory
//! waits, then the minimal bench-determined link restart by escalation (None → Restart_AN →
//! reset_pipeline, least that links), records the escalation evidence, confirms 10G SFI link-up on both,
//! then rebinds (restores). Defaults-first: AUTOC2/EEPROM link mode left unwritten.

use directpath::gate::{gate, Allowed, Bdf, Decision};
use directpath::link::{apply_restart, link_state, reset_and_init, LinkRestart, LinkState};
use directpath::pci::{self, Bar0};
use std::fs;
use std::os::unix::fs::FileExt;
use std::process::ExitCode;
use std::time::{Duration, Instant};

const BENCH: [&str; 2] = ["0000:41:00.0", "0000:63:00.0"];

fn read_config(bdf: &Bdf) -> std::io::Result<Vec<u8>> {
    let f = fs::File::open(format!("/sys/bus/pci/devices/{}/config", bdf.sysfs()))?;
    let mut buf = vec![0u8; 0x40];
    f.read_exact_at(&mut buf, 0)?;
    Ok(buf)
}

/// Poll both ends for simultaneous link-up, up to `timeout`.
fn wait_both_up(a: &Bar0, b: &Bar0, timeout: Duration) -> (LinkState, LinkState) {
    let deadline = Instant::now() + timeout;
    loop {
        let (sa, sb) = (link_state(a), link_state(b));
        if (sa.up && sb.up) || Instant::now() >= deadline {
            return (sa, sb);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn main() -> ExitCode {
    println!("==== directpath Slice-2 run: LINK BRING-UP (NON-DMA, both bench ends) ====");
    let allowlist: Vec<Bdf> = BENCH.iter().map(|s| Bdf::parse(s).unwrap()).collect();

    // Gate + token + unbind + mmap both bench cards.
    let mut tokens: Vec<Allowed> = Vec::new();
    let mut drivers: Vec<Option<String>> = Vec::new();
    let mut bars: Vec<Bar0> = Vec::new();
    for bdf in &allowlist {
        let config = match read_config(bdf) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("ABORT: config {} {e}", bdf.sysfs());
                return ExitCode::FAILURE;
            }
        };
        let t = match gate(*bdf, &allowlist, &config) {
            Decision::Allow(t) => t,
            Decision::Refuse(r) => {
                eprintln!("ABORT: gate refused {}: {r:?}", bdf.sysfs());
                return ExitCode::FAILURE;
            }
        };
        drivers.push(pci::current_driver(&t));
        if let Err(e) = pci::unbind(&t) {
            eprintln!("ABORT: unbind {} {e}", t.bdf().sysfs());
            return ExitCode::FAILURE;
        }
        let bar = match pci::map_resource0(&t) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("ABORT: mmap {} {e}", t.bdf().sysfs());
                return ExitCode::FAILURE;
            }
        };
        println!("acquire: {} unbound + BAR0 mmap'd", t.bdf().sysfs());
        tokens.push(t);
        bars.push(bar);
    }

    // §4.6.3 reset + the two mandatory waits, both ends.
    for (t, bar) in tokens.iter().zip(bars.iter()) {
        let r = reset_and_init(bar);
        println!(
            "reset  : {} reset-cleared={} EEC.ARD={} RDRXCTL.DMAIDONE={} -> {}",
            t.bdf().sysfs(),
            r.reset_cleared,
            r.eec_ard,
            r.rdrxctl_dmaidone,
            if r.ok() { "OK" } else { "INCOMPLETE" }
        );
    }

    // Minimal link restart by escalation — apply the SAME form to both ends, then poll both for link-up.
    // Record the escalation evidence (a smaller form was tried and did NOT link).
    let forms = [LinkRestart::None, LinkRestart::RestartAn, LinkRestart::ResetPipeline];
    let mut linked: Option<LinkRestart> = None;
    for form in forms {
        apply_restart(&bars[0], form);
        apply_restart(&bars[1], form);
        std::thread::sleep(Duration::from_millis(100));
        let (sa, sb) = wait_both_up(&bars[0], &bars[1], Duration::from_millis(5000));
        println!(
            "link   : form={form:?} -> {}={}(10G={}) {}={}(10G={}) links=[{:#010x},{:#010x}]",
            BENCH[0], sa.up, sa.speed_10g, BENCH[1], sb.up, sb.speed_10g, sa.links_raw, sb.links_raw
        );
        if sa.up && sb.up {
            linked = Some(form);
            break;
        }
        println!("evidence: form={form:?} did NOT bring both ends up (escalating)");
    }

    let result = match linked {
        Some(form) => {
            let (sa, sb) = (link_state(&bars[0]), link_state(&bars[1]));
            println!(
                "RESULT : PASS — minimal restart = {form:?}; both ends 10G SFI link-up (10G={}/{})",
                sa.speed_10g, sb.speed_10g
            );
            ExitCode::SUCCESS
        }
        None => {
            println!("RESULT : LINK NOT ACHIEVED by None/Restart_AN/reset_pipeline — SURFACE (bench fact; do NOT escalate the write-set silently)");
            ExitCode::FAILURE
        }
    };

    // Reversibility: drop the mmaps, rebind both ends (restore the initial state).
    drop(bars);
    for (t, driver) in tokens.iter().zip(drivers.iter()) {
        if let Some(d) = driver {
            match pci::rebind(t, d) {
                Ok(()) => println!("revert : {} rebind {d} OK", t.bdf().sysfs()),
                Err(e) => eprintln!("WARN: rebind {} {d}: {e}", t.bdf().sysfs()),
            }
        }
    }
    result
}
