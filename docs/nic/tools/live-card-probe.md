# Tool specification: `live-card-probe`

> An ad-hoc, **read-only** diagnostic that grounds the NIC subsystem's *static* open questions
> against a real 82599 — including the one currently installed as this host's **primary network
> interface** — without any risk of bringing the link down. It is not in the hot path and is not
> part of the shipped trusted base; it is an instrument we run to answer our own questions before
> we have prototype cards to drive freely.
>
> Companion to the grounding notes in `docs/nic/grounding/`. Each reading it takes maps to a named
> open item there.

## 1. Purpose

Several grounding open items are *static* facts about a specific board — what the NVM configures,
what the cable presents, how the PCI/IOMMU topology looks — that we previously filed as
"bench-gated, needs silicon." A read-only probe of an installed card answers them **now**, turning
assumptions into confirmed anchors and giving a strong prior before the prototype cards arrive.

## 2. The load-bearing constraint: structural no-disruption

The target card is the host's **primary, live network interface**. The tool's first property is
that it **cannot** disturb that link — not "is careful not to," but *has no code path that could*.
This is the project's own "make it impossible, not caught" applied to the instrument.

**The tool performs only read-only operations mediated by the in-tree kernel driver.** It is
forbidden, by construction, from:

- writing **any** register, NVM word, PCI config byte, or `ethtool` setting (no `set_*`);
- unbinding the kernel driver, binding `vfio-pci`, or otherwise detaching the device;
- `mmap`-ing BAR0 / `resource0` or issuing **any** raw MMIO;
- reading **read-to-clear / RW1C** registers (the statistics counters, `EICR`) — these have side
  effects the live driver depends on. (The kernel `ethtool -d` dump already excludes these, which
  is why we ride it rather than reading registers ourselves.)

Every operation below is one the kernel driver already executes routinely on live production NICs,
so the probe adds **zero** new device interaction beyond what `ethtool`/`sysfs` already do safely.

## 3. What it reads, and which open item each answers

All via kernel-mediated read-only interfaces — no raw device access.

| Reading | Mechanism | Grounds (open item) |
|---|---|---|
| Device + subsystem ID, **stepping/revision** | `lspci -nnvvv`, sysfs | Confirm it is an 82599ES and its stepping → whether the B0 errata (e.g. #13) apply |
| **IOMMU group composition**; the two PCIe functions | `/sys/bus/pci/devices/<bdf>/iommu_group/devices/`, `lspci` | `kernel-bypass-and-dma.md §5.1` — does our port share a group with the dark function? |
| Current driver binding, link state, FW version | `ethtool -i`, `ethtool <if>` | Baseline; confirms `ixgbe` is the bound driver |
| **AUTOC2[19]** (erratum 13), **AUTOC2 serial PMA/PMD** (SFI select), **AUTOC.LMS**, **LINKS** speed | `ethtool -d` (regs_buff 1064/1065/1066) | `device-bring-up-and-link.md §3,§5.3` — is the link-electrical config (SFI + bit 19) EEPROM-set as expected? |
| **NVM / EEPROM contents** — LESM FW-module word 0x2, link-interconnect config words | `ethtool -e` (read-only `get_eeprom`) | `device-bring-up-and-link.md §5.2` (LESM enabled?), confirms EEPROM drives link config |
| **SFP+/DAC module EEPROM** (SFF-8472 identity) | `ethtool -m` (`get_module_eeprom`) | `device-bring-up-and-link.md §5.4` — what the passive DAC presents; confirms `da_cu` data |

## 4. Output

A single structured report (human-readable, also machine-parseable) that, for each open item,
prints **expected → observed → verdict**, e.g.:

```
[erratum-13 AUTOC2[19]]   expected=1 (EEPROM-set)   observed=1   ✓ default favorable
[LESM enabled (NVM 0x2)]  expected≠0x0000/0xFFFF?   observed=0xFFFF → LESM DISABLED  ✓ no sw/fw semaphore needed
[link mode]               expected=10G_SERIAL/SFI   observed=LMS=10G_SERIAL, AUTOC2 serial=SFI  ✓
[IOMMU group]             observed: group 21 = {port0, port1}  → bind both functions to own group
[module]                  observed: SFF-8472 id = passive DAC (da_cu), <vendor>/<len>m
```

The report is a grounding artifact; its findings fold back into the relevant `grounding/` note as
confirmed anchors (replacing "bench-confirm" with the observed value + the card it came from).

## 5. Caveats — what this card can and cannot tell us

- **The attached cable is production-identical (operator-confirmed).** The passive DAC on this live
  card is the same brand and model we will use in production, so the `ethtool -m` reading grounds the
  *exact* production cable's SFF-8472 identity (`da_cu` classification, what it presents to the NIC),
  and the fact that **the link is up right now** is empirical confirmation that our exact DAC links on
  real 82599 silicon — and whether the stock driver accepted it as qualified or needed
  `allow_unsupported_sfp` (informs `device-bring-up-and-link.md §3`, module-detection-is-policy). The
  cable transfers to production directly; the *board* may not (next bullet).
- **It grounds the family/board *pattern*, not necessarily the prototype target.** This is one
  specific X520/82599 board; the cards "in the mail" may be a different vendor SKU with different
  NVM. Findings here are a **strong prior and a confirmed anchor for *this* board** — re-read the
  prototype cards the same way once they arrive (or directly once VFIO-bound).
- **Only *static* questions are answerable read-only.** The *dynamic* link-bring-up questions —
  does the link come up on the EEPROM default with no AUTOC write, the minimal `Restart_AN` form,
  single-ring throughput, buffer-pool sizing — require actually *driving* the device and so stay
  for prototype bring-up on the mailed cards, not this live one.
- **First action is identity confirmation.** The tool verifies the device is an 82599/`ixgbe`
  before reading any 82599-specific register offset, and degrades gracefully (reports "not an
  82599") rather than misreading another NIC.

## 6. Explicitly out of scope (and why)

- **No raw MMIO tier.** Because `ethtool -d` already exposes AUTOC/AUTOC2/LINKS, raw BAR0 reads buy
  nothing for the current questions and would forfeit the structural no-disruption guarantee. If a
  future question ever needs a register `ethtool -d` omits, adding a raw-read path is a **deliberate,
  operator-gated** change scoped to a verified read-only allowlist (never a read-to-clear register)
  — not a default capability of this tool.
- **No NVM/register writes, ever.** Writing the EEPROM or any register is prototype-card work on a
  device we own and can afford to disrupt — never this primary interface.
