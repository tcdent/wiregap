# Canonical Sources — NIC / Transport Subsystem Grounding

> Primary documentation and reference implementations the NIC subsystem design is grounded
> against. Everything here is pulled from a canonical upstream and pinned to a revision so
> the grounding is traceable and re-checkable. This directory is *reference material we read*,
> not code we ship — none of it enters the trusted base; it is the diligence behind what we
> will build from the datasheet ourselves.
>
> Captured 2026-06-20 by zero.nic-design.

## Hardware truth (primary)

| File | Doc | Rev | Source |
|---|---|---|---|
| `datasheets/82599-datasheet-331520-rev3.5.pdf` | Intel 82599 10 GbE Controller Datasheet | 3.5 (Nov 2024), 331520-005 | https://cdrdv2-public.intel.com/331520/331520_Intel%C2%AE%2082599%2010%20GbE%20Controller%20Datasheet_rev3_5.pdf |
| `datasheets/82599-specupdate-331521-rev4.3.pdf` | Intel 82599 Specification Update (errata) | 4.3 (Jul 2024), 331521-012 | https://cdrdv2-public.intel.com/331521/331521_82599_SpecUpdate_Rev4.3.pdf |

The datasheet is the device's external architecture: the §4.6 init family (init sequence, link
init, statistics, interrupt, RX/TX init), register definitions (§8), AUTOC at §8.2.3.22.19. The
spec update carries the AUTOC/LESM link-setup errata — mandatory reading because the part is EOL.

## Reference implementations (pinned commits)

| Dir | Repo | Commit | Date | Why canonical |
|---|---|---|---|---|
| `ixy/` | github.com/emmericp/ixy | `0e00605` | 2021-02-13 | **The keystone.** A complete userspace driver for this exact 82599/X520 family in ~1k LOC C, built from the datasheet, VFIO/IOMMU, runnable unprivileged. The "learn from ixgbe behavior, use none of its source" posture already executed on the target silicon. Paper: arXiv:1901.10664. |
| `ixy.rs/` | github.com/ixy-languages/ixy.rs | `d3fc461` | 2023-12-08 | Confirms no kernel modules except `vfio-pci`, operation without root via IOMMU. |
| `dpdk/` (sparse: `drivers/net/intel/ixgbe`, `doc/guides/nics`) | github.com/DPDK/dpdk | `d5f7bc5` | 2026-06-19 | Production-grade behavior reference; poll-mode descriptor access without interrupts (except LSC). |
| `linux/` (sparse: `drivers/net/ethernet/intel/ixgbe`, `drivers/vfio`, `Documentation/{driver-api,userspace-api}`) | github.com/torvalds/linux | `1a3746c` | 2026-06-19 | The authoritative behavior oracle and the *don't-copy* reference whose history explains why each bring-up step exists. Also holds the VFIO/IOMMUFD docs. |
| `snabb/` (sparse: `src/apps/intel_mp`, `src/apps/intel`, `src/lib/hardware`) | github.com/snabbco/snabb | `f327292` | 2024-08-30 | The second behavior reference named by ixy's authors. |

## Reading posture

These are **behavior oracles and an existence proof**, not a source base. The driver we design
is built from the datasheet; the reference drivers tell us *why each step exists* and *what is
mandatory vs quirk-driven vs optional*. Where the explore docs assert an empirical claim about
these sources (line counts, which registers are load-bearing), it is verified here against the
pinned source rather than inherited.
