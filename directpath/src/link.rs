//! 82599 link bring-up (datasheet §4.6.3) — NON-DMA: only BAR0 register config, the device configures
//! itself / its PHY, nothing touches host RAM. The reset + the two mandatory waits, then the **minimal**
//! bench-determined link restart by escalation. **Defaults-first**: AUTOC/AUTOC2 (the SFI link mode) are
//! left EEPROM-provided — we do NOT override the link mode (only `Restart_AN` / the pipeline-reset
//! toggle), so the escalation starts from the least intervention. Register offsets/constants from the
//! 82599 datasheet (`reference/ixy` `ixgbe_type.h`).

use crate::pci::Bar0;
use std::time::{Duration, Instant};

const CTRL: usize = 0x00000;
const EIMC: usize = 0x00888;
const EEC: usize = 0x10010;
const RDRXCTL: usize = 0x02F00;
const AUTOC: usize = 0x042A0;
const LINKS: usize = 0x042A4;

const IRQ_CLEAR_MASK: u32 = 0xFFFF_FFFF;
const CTRL_RST_MASK: u32 = 0x0400_0008; // CTRL_LNK_RST | CTRL_RST
const EEC_ARD: u32 = 0x0000_0200; // EEPROM auto-read done (bit 9)
const RDRXCTL_DMAIDONE: u32 = 0x0000_0008; // DMA init cycle done (bit 3)
const AUTOC_AN_RESTART: u32 = 0x0000_1000; // restart auto-negotiation (bit 12)
const AUTOC_LMS_TOGGLE: u32 = 0x4 << 13; // reset_pipeline: toggle LMS[2]
const LINKS_UP: u32 = 0x4000_0000; // link up (bit 30)
const LINKS_SPEED_10G: u32 = 0x3000_0000; // 10G speed (bits 28-29)

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn wait_mask(bar: &Bar0, off: usize, mask: u32, want_set: bool, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        let hit = (bar.read32(off) & mask == mask) == want_set;
        if hit {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        sleep_ms(1);
    }
}

/// The §4.6.3 reset + mandatory-wait outcome (observed).
pub struct ResetOutcome {
    pub reset_cleared: bool,
    pub eec_ard: bool,
    pub rdrxctl_dmaidone: bool,
}

impl ResetOutcome {
    pub fn ok(&self) -> bool {
        self.reset_cleared && self.eec_ard && self.rdrxctl_dmaidone
    }
}

/// §4.6.3: disable interrupts → global reset → wait reset clear → disable interrupts again → wait
/// `EEC.ARD` + `RDRXCTL.DMAIDONE`. Defaults-first: does NOT write AUTOC/AUTOC2 (leaves the EEPROM SFI
/// link config).
pub fn reset_and_init(bar: &Bar0) -> ResetOutcome {
    bar.write32(EIMC, IRQ_CLEAR_MASK); // disable interrupts
    bar.write32(CTRL, CTRL_RST_MASK); // global software+link reset
    let reset_cleared = wait_mask(bar, CTRL, CTRL_RST_MASK, false, Duration::from_millis(1000));
    sleep_ms(10); // datasheet post-reset settle
    bar.write32(EIMC, IRQ_CLEAR_MASK); // §4.6.3.1 disable interrupts again
    let eec_ard = wait_mask(bar, EEC, EEC_ARD, true, Duration::from_millis(1000));
    let rdrxctl_dmaidone = wait_mask(bar, RDRXCTL, RDRXCTL_DMAIDONE, true, Duration::from_millis(1000));
    ResetOutcome { reset_cleared, eec_ard, rdrxctl_dmaidone }
}

/// The link-restart escalation forms, least-intervention first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkRestart {
    /// No restart write — observe whether the reset + EEPROM defaults link on their own.
    None,
    /// `AUTOC.Restart_AN` — restart auto-negotiation.
    RestartAn,
    /// The pipeline reset — write AUTOC with LMS[2] toggled + Restart_AN, then the original LMS.
    ResetPipeline,
}

/// Apply a link-restart form. Defaults-first: never overrides the LMS/PMD link mode (only Restart_AN /
/// the LMS toggle-and-restore).
pub fn apply_restart(bar: &Bar0, form: LinkRestart) {
    match form {
        LinkRestart::None => {}
        LinkRestart::RestartAn => {
            let autoc = bar.read32(AUTOC);
            bar.write32(AUTOC, autoc | AUTOC_AN_RESTART);
        }
        LinkRestart::ResetPipeline => {
            let autoc = bar.read32(AUTOC) | AUTOC_AN_RESTART;
            bar.write32(AUTOC, autoc ^ AUTOC_LMS_TOGGLE); // toggled LMS[2] + restart
            bar.write32(AUTOC, autoc); // original LMS + restart
        }
    }
}

/// Observed link state.
#[derive(Clone, Copy, Debug)]
pub struct LinkState {
    pub up: bool,
    pub speed_10g: bool,
    pub links_raw: u32,
}

pub fn link_state(bar: &Bar0) -> LinkState {
    let links = bar.read32(LINKS);
    LinkState {
        up: links & LINKS_UP != 0,
        speed_10g: links & LINKS_SPEED_10G == LINKS_SPEED_10G,
        links_raw: links,
    }
}
