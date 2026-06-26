# Data path and the event model

> The per-frame hot path (RX/TX rings and descriptors), the hardware behaviors the reliability story
> rests on, and the interrupt-driven idle-wakeup that is the event primitive the whole event-driven
> application rides on. This is the surface that interprets the descriptor rings the firmware
> populates from the wire, so it is understood completely here.
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

## 4. The event model — interrupt → eventfd → epoll (the load-bearing primitive)

The idle-wakeup interrupt is **not an optimization we declined** — it is the event primitive the
entire event-driven application rides on for free. Understood completely:

- **Arming**: one MSI-X vector → `eventfd(0,0)` → `VFIO_DEVICE_SET_IRQS` with
  `DATA_EVENTFD|ACTION_TRIGGER` (`libixy-vfio.c:95`); the eventfd joins an epoll instance (`:210`).
  Device-side, `IVAR`/`GPIE`/`EIMS` map the queue's RX cause to the vector (`ixgbe.c:158`).
- **Waiting**: `vfio_epoll_wait` (`:181`) blocks on epoll; on wake it `read()`s the eventfd to clear
  the latch. **The `timeout` (ms) argument is exactly where the retransmit deadline composes into the
  idle-wait** — "block on the eventfd *or* the retransmit deadline, one loop, two exit reasons"
  (`transport-stack.md §2.6`) is directly realizable as `epoll_wait(..., deadline_ms)`.
- **Minimal slice we keep** ≈ 60 LOC: `vfio_enable_msix` (21) + `vfio_epoll_ctl` (8) +
  `vfio_epoll_wait` (17) in the VFIO layer, plus the device-side `IVAR`/`EIMS` arming.

**Our control flow inverts ixy's.** ixy calls `vfio_epoll_wait` at the *top* of `rx_batch` (`:667`)
— it **blocks first, then polls** — and toggles poll/interrupt via an adaptive moving-average rate
estimator (`interrupts.c:27`, ~20 LOC). Our design is the inverse: **busy-poll while there is work,
block on the eventfd only when the ring is empty**, with **no** adaptive mode-switching. So the
~60-LOC slice is a **re-derivation** from ixy's primitives, not a copy; the entire adaptive policy and
the `instr_counter`-throttled toggling (`:715–734`) are dropped.

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
5. **Idle-wakeup loop**: confirm the poll-then-block loop and that the retransmit deadline composes
   into the same `epoll_wait` (`transport-stack.md §4.2`).
