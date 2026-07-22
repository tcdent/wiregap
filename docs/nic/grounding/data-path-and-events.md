# Data path and the event model

> The per-frame hot path (RX/TX rings and descriptors), the hardware behaviors the reliability story
> rests on, and the event-driven idle-wakeup — a `MONITORX`/`MWAITX` park on the NIC's own `DD` writeback
> (§4) — that the whole event-driven application rides on. This is the surface that interprets the
> descriptor rings the firmware populates from the wire, so it is understood completely here.
>
> Grounded in `reference/ixy` (`ixgbe.c` rx/tx batch, `interrupts.c`, `libixy-vfio.c`) and the
> datasheet (§4.6.7/8 RX/TX init, §8.2.3.7.1 `FCTRL`).

## 1. RX init and the receive path

`init_rx` (`ixgbe.c:327`) and `ixgbe_rx_batch` (`:656`). The mandatory/quirk writes (kept):

- **Single 128 KB packet buffer**: `RXPBSIZE(0)=128KB`, buffers 1–7 zeroed (`:332`) — the no-DCB/VT
  consolidation.
- **Advanced one-buffer descriptors** + **`SRRCTL.DROP_EN`** (`:348`,`:351`): the NIC drops on RX
  when no descriptor is available rather than buffering. A dropped frame is never ACKed → the
  sender's retransmit timer fires → resend. This is the hardware leg of "RX overrun resolves into the
  same retransmit machinery."
- **CRC strip**: `HLREG0.RXCRCSTRP` + `RDRXCTL.CRCSTRIP` (`:338`).
- Ring base/length (`RDBAL/RDBAH/RDLEN`), head/tail init, and the **§4.6.7-tail quirk bits**
  `CTRL_EXT.NS_DIS` and the unnamed `DCA_RXCTRL` bit-12 clear (`:374`,`:377`) — reserved-init-1,
  must-be-0; not redundant.

**The receive hot path**: poll the descriptor's `DD` (descriptor-done) bit in host memory — no MMIO —
copy the descriptor, refill with a fresh buffer, advance `RDT` once per batch. **Frame length is read
from the RX descriptor** (`desc.wb.upper.length`, `:685`) — there is no software length field in our
framing. But "read from the descriptor" is **not** "trusted": that field is firmware-written from
box-influenced wire bytes, so it is **bounded** against the buffer size, never trusted as given (see the
adversary-influenced note below). **Multi-segment is explicitly unsupported** (`:679` errors on
non-EOP): single-buffer-per-frame, 2048-byte pktbuf, MTU 1518 — the multi-segment path is never
touched (jumbo would require revisiting this).

**The descriptor ring is adversary-influenced input; the driver treats it as such.** The firmware
populates these descriptors from the wire bytes the box controls, and the IOMMU does *not* guard their
*content* — it bounds only where the device can DMA, not what it writes into the rings we mapped (see
`kernel-bypass-and-dma.md §2`, vector 2). So the descriptor-interpreting code is the guard for that
vector and must hold against *any* content the firmware can produce: **bound the reported length to the
buffer size** rather than trust `desc.wb.upper.length`; **treat unexpected status bits** (a non-EOP, an
error flag) as a drop-and-refill, never a read past the buffer; never derive a memory access from an
unchecked descriptor field. This is the concrete reason the driver is held to reviewed-by-reading and
"understood completely" — it is the single piece of our code standing between adversarial ring content
and the trusted host. (A real build constraint, not yet implemented; reserved for the driver build.)

## 2. Integrity from hardware CRC — and it is a leave-at-default

`HLREG0.TXCRCEN` appends CRC on TX (`:388`); on RX the CRC is checked and stripped. **`FCTRL.SBP`
(Store Bad Packets, §8.2.3.7.1) defaults to `0b` = do-not-store**, so frames failing the Ethernet
CRC are **not posted to the ring** — hardware drops them, indistinguishable from a lost frame, into
the retransmit path. Integrity is inherited from the NIC; **`SBP` is left at default** (we never set
it). This is why the protocol needs no software integrity check.

## 3. TX init and the transmit path

`init_tx` (`:386`) and `ixgbe_tx_batch` (`:743`). Mandatory writes: `HLREG0` CRC/pad enables,
`TXPBSIZE(0)=40KB`, **`DTXMXSZRQ=0xFFFF` + `RTTDCS.ARBDIS` clear** ("required when not using
DCB/VTd", `:397`), ring base/length, `TXDCTL` writeback thresholds (DPDK defaults), and
`DMATXCTL.TE` (enable DMA, `:431`).

**Two divergences from ixy worth carrying into the design:**

- **Per-descriptor cleanup is free for us.** Every TX descriptor sets **`RS`** (report-status, `:811`),
  so `DD` is available per descriptor. ixy nonetheless cleans in batches of `TX_CLEAN_BATCH=32` for
  PCIe efficiency, with a "clean the whole batch or nothing" stranding wrinkle (`:786–790`) that can
  hold up to 31 buffers if TX stops. At our rates we can drop the batching and clean per-descriptor,
  **eliminating the stranding wrinkle entirely**.
- **Buffers are ACK-bound, not TX-completion-bound.** ixy frees a buffer on TX `DD`. Our stop-and-wait
  may retransmit a frame until acknowledged, so **we free on ACK arrival**, not TX-completion — the
  buffer lifetime is longer. A genuine divergence from ixy's reclamation model.

## 4. The event model — the NIC's `DD` writeback, parked on via `MONITORX`/`MWAITX` (the load-bearing primitive)

The idle-wakeup is **not** an interrupt and **not** a poll — it is the NIC's **own per-frame event**, in
memory we already read, that the CPU parks on directly. On every received frame the NIC writes back the RX
descriptor (`DD` bit set) by DMA into the ring — a device write to memory we own. So the idle-wait needs no
kernel mechanism and no separate signal: we watch the very descriptor the NIC is about to write.

- **Arming**: `MONITORX` (userspace, ring-3) on the next RX descriptor's cache line — the line the NIC's
  `DD` writeback will hit. No MSI-X, no eventfd, no VFIO: the event is the DMA the NIC already performs.
- **Waiting**: `MWAITX` with a TSC deadline **halts the CPU** until (a) the NIC's `DD` write lands on the
  monitored line — a frame arrived — or (b) the deadline expires — a retransmit is due. That is exactly the
  idle-wait's two exit reasons ("block on the frame *or* the retransmit deadline, one loop, two reasons",
  `transport-stack.md §2.6`), in one instruction: `MWAITX(deadline)`.
- **Minimal slice we keep** ≈ the `MONITORX`/`MWAITX` park (`directpath/src/mwaitx.rs`) plus reading the
  `DD` bit — no VFIO layer, no eventfd/epoll plumbing. The idle-wait is a **check-before-wait**
  (`MONITORX`-recheck-`MWAITX`: read `DD`, arm, re-read, wait) on our own descriptor memory.

**Our control flow.** Busy-poll (busy-drain) while there is work, then park via `MWAITX` when the ring is
empty — never block-first, never adaptive mode-switching. `MWAITX`'s intrinsic max-wait bounds the park, so
a truly-missed write degrades to *latency* (the next re-poll catches it), never a hang.

*(This supersedes an earlier draft that described an MSI-X → eventfd → epoll VFIO idle-wait. That model
went with the VFIO/kernel-tooling approach the subsystem no longer uses: the pure-direct path delivers no
interrupt, and the NIC's own `DD` writeback — the signal the reactor already reads — is the minimal event
to park on. Grounded in `grounding/kernel-bypass-and-dma.md §4`.)*

## 5. One ring is the floor and the smaller total surface

RX/TX setup in every driver is **per-queue** (`RDBAL/RDLEN/SRRCTL/RXDCTL` looped per queue); one queue
is the loop running once. Choosing **one ring + a software frame-level session ID** over
hardware multiqueue keeps the register surface smallest — **no** RSS/RETA hash programming, no Flow
Director, no per-queue interrupt fan-out — and puts the demux in our reviewable software. Aggregate
throughput scales by running the one dumb codepath as **K parallel sessions**, never by making the
codepath cleverer. This confirms the explore docs' single-ring lean (`transport-stack.md §4.3 item 9`).

## 6. Bench-gated open items (this topic)

1. **Single-ring sufficiency**: confirm one RX/one TX ring fills the 10G pipe at the relevant frame
   sizes, retiring hardware multiqueue.
2. **Stop-and-wait arithmetic**: measure single-session frames/sec at the link's real RTT (expected
   ~200k/s, orders of magnitude past terminal rates) and confirm K parallel sessions scale aggregate.
3. **Per-descriptor cleanup affordability** at our rates (vs ixy's batching) — would let us drop the
   batching complexity and its stranding wrinkle.
4. **DMA buffer-pool sizing** — measure real held-buffer counts, do not copy a default.
5. **Idle-wakeup loop**: confirm the busy-drain + `MWAITX`-park loop on the `DD` line, and that the
   retransmit deadline composes into the `MWAITX` wait — the make-or-break being that the device's `DD`
   write reliably **wakes** the park (a CPU-capability + coherency question proven on silicon;
   `grounding/kernel-bypass-and-dma.md §5`).
