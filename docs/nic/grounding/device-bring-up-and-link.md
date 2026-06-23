# Device bring-up and link

> How the 82599 is brought from power-on to a live 10G SFI link over the passive DAC, and which
> registers the driver writes versus leaves at their documented default. Read through
> `driver-minimality.md`: the goal is the fewest writes that yield a correct, link-up device.
>
> Grounded in the datasheet (§4.6 init family), `reference/ixy` @ `0e00605`, and the production
> Intel base driver (`reference/linux/.../ixgbe_82599.c`, identical to DPDK's `base/`). Line
> references are to those sources.

## 1. The initialization sequence (datasheet §4.6.3 — authoritative)

The datasheet's nine major steps (p.166), which `ixy`'s `reset_and_init` (`ixgbe.c:446`) realizes
faithfully and which measure to **exactly 147 code lines** of bring-up (verified, not estimated):

1. Disable interrupts.
2. Issue global reset + general configuration (§4.6.3.2).
3. Wait for EEPROM auto-read completion (`EEC.ARD`).
4. Wait for DMA initialization done (`RDRXCTL.DMAIDONE`).
5. Set up the PHY and link (§4.6.4) — see §3 below.
6. Initialize statistical counters (read-to-clear).
7. Initialize receive (§4.6.7) — see `data-path-and-events.md`.
8. Initialize transmit (§4.6.8) — see `data-path-and-events.md`.
9. Enable interrupts (§4.6.3.1) — see `data-path-and-events.md`.

| Bring-up function (`ixgbe.c`) | code LOC |
|---|---|
| `reset_and_init` (446) | 28 |
| `init_link` (267) | 5 |
| `init_rx` (327) | 34 |
| `init_tx` (386) | 28 |
| `start_rx_queue` (277) | 23 |
| `start_tx_queue` (311) | 11 |
| `wait_for_link` (434) | 10 |
| `disable_interrupts`+`clear_interrupts` (78,99) | 8 |
| **Total** | **147** |

## 2. Reset and the mandatory waits

§4.6.3.2 defines **Global Reset = software reset + link reset**, requires a **≥10 ms wait** after,
and §4.6.3.1 the **double interrupt disable** (before *and* after reset) — all present in ixy. A
global reset **reloads AUTOC/AUTOC2 from the EEPROM**, so after reset the link-electrical config is
already in its provisioned state (relevant to §3). The two waits — `EEC.ARD` (EEPROM auto-read done)
and `RDRXCTL.DMAIDONE` (DMA init done) — are poll-until-set on hardware status bits; mandatory.

## 3. The link — SFI/DAC config is EEPROM-provided, and the driver leans on it

This is the load-bearing finding, confirmed across ixy, the datasheet, and the production driver.

**The datasheet prescribes the defaults-first posture.** §4.6.3.2: *"The link interconnect
configuration according to the electrical specification of the relevant electrical interface should
be set prior to the link setup. **This configuration is done through the EEPROM**."* The hardware
intends board NVM to carry the link-electrical config; software relies on it.

**The SFI selection lives in AUTOC2, and neither ixy nor the production driver writes it.** The
10G-serial PMA/PMD select (SFI/KR/XFI) is `AUTOC2[17:16]` (`AUTOC2_10G_SFI = 0x2<<16`). ixy's
`init_link` only touches **AUTOC** — it sets `LMS=10G_SERIAL` and writes the *parallel* PMA/PMD field
to XAUI, which is **dead in serial mode** — and never touches AUTOC2. The production
`ixgbe_setup_mac_link_82599` (`ixgbe_82599.c:779`) *reads* `pma_pmd_10g_serial` from AUTOC2 (`:796`,
comment *"Use stored value (EEPROM defaults)"* `:809`), makes no change for our case
(`pma_pmd==SFI && link_mode==10G_SERIAL`, `:841`), and guards the entire AUTOC write behind
**`if (autoc != current_autoc)`** (`:854`). For an EEPROM-SFI board, that condition is false → **no
reconfiguration write at all.** The production driver does *less* AUTOC work than ixy.

**AUTOC2 bit 19 (erratum 13) also comes from the EEPROM.** Erratum 13 (B0, NoFix): `AUTOC2[19]` must
be `1b` or the TX path can hang on an internal link-speed clock-switch; *"Intel drivers expect bit 19
to be set by EEPROM."* The production driver confirms this — on reset it reads-and-caches the
EEPROM-default AUTOC2 (`:1017`) and **preserves the upper bits, including bit 19, from that cached
original** (`:1043`); it never synthesizes bit 19.

**The link still needs a software restart — its minimal form is bench-gated.** Skipping
*reconfiguration* is not skipping *bring-up*: the link is kicked by `ixgbe_start_mac_link_82599` →
`ixgbe_reset_pipeline_82599` (`:2020`), which writes AUTOC with `Restart_AN` **and a toggled
`LMS[2]` bit**, waits on `ANLP1`, then restores the original LMS — this *is* the AUTOC/LESM speedlock
errata workaround (spec update 331521). Notes:

- **AN-complete polling is backplane-only** (`:862`); for SFI/serial the driver does not wait on AN.
- **ixy omits the whole dance** — one unconditional `Restart_AN`, no LMS[2] toggle, no LESM — and
  links on the X520. So the full sequence may be unnecessary on our board.
- The minimal restart is therefore a **bench determination** (§5): nothing / single `Restart_AN` /
  full errata-safe `reset_pipeline`, in increasing order, implementing the least that reliably links.

**Module detection is driver policy, not a hardware requirement.** The production drivers read the
cable's SFP+ EEPROM over I²C to identify `da_cu` (DAC) and enforce a qualified-module check
(`ixgbe_phy.c:1793–1831`, the "unsupported SFP+" refusal, bypassable via `allow_unsupported_sfp`).
This machinery exists for hot-plug / unknown / multi-speed optics and qualification enforcement —
none of which we have. **ixy performs zero I²C/SFP handling and links the DAC**, proving module ID is
not a hardware prerequisite for SFI link-up. Our fixed, known, qualified cable at fixed 10G needs
none of it.

**LESM firmware coordination — a surface the explore docs did not flag.** If the board NVM has LESM
(Link Establishment State Machine) firmware enabled (NVM FW-module word 0x2 ≠ 0/0xFFFF), on-chip
firmware *also* manipulates AUTOC, and `ixgbe_start_mac_link_82599` takes a **sw/fw semaphore**
(`acquire_swfw_sync`) around the restart. **Minimal posture: prefer LESM disabled** in NVM, removing
the coordination entirely and making the link path purely ours.

## 4. The candidate write-set (minimality sort applied)

- **Required by datasheet for our config:** the §4.6.3 reset + the two waits; (link) at most the
  minimal `Restart_AN` the bench requires.
- **Left at documented default (not written):** AUTOC2 link-electrical config (SFI + bit 19,
  EEPROM-set), flow-control registers (`FCTTV/FCRTL/FCRTH/FCCFG` — we decline 802.3x PAUSE),
  `FCTRL.SBP` (defaults to drop-bad-CRC — see `data-path-and-events.md`), and the entire dark-feature
  space (multiqueue/RSS/Flow-Director/FCoE/DCB/1588/offloads).
- **Actively depended-upon default that must be confirmed:** `AUTOC2[19]=1` (erratum 13).
- **Removable optionality vs ixy:** the unconditional AUTOC writes in `init_link` (the XAUI write is
  already dead), promisc mode (`:497`, test convenience), and the multi-queue loops (collapse to one
  RX/one TX ring).

## 5. Bench-gated open items (this topic)

These require silicon; the charge reserves them as open research. The production sequence above is the
fallback for each if the minimum proves insufficient.

1. **Does the link come up on the EEPROM default with no AUTOC write — or a single `Restart_AN` — or
   only the full errata-safe `reset_pipeline`?** Implement the least that reliably links.
2. **Is LESM enabled in our board's NVM?** If yes and not disable-able, the sw/fw semaphore becomes
   required scope.
3. **Confirm `AUTOC2[19]=1` and the FC-register reset defaults** by reading NVM-configured registers
   once before any writes (`transport-stack.md §4.1.2`) — diligence to understand the defaults, not
   to reassert them.
4. **Passive-DAC link-up needs no SFP+ module I²C/SDP handling** — confirm empirically (expected per
   ixy). An *optical* module would add SDP/I²C as scope. Bench warning: do not test link via
   single-card SFI loopback (spec update "SFP+ (SFI) back-to-back" clarification — fails for reasons
   unrelated to the driver; N/A to our two-card topology).
