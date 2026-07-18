# The device-memory crossing — a verified, cited account (platform-scoped)

> The soundness account for the one place the driver reads and writes memory the NIC concurrently mutates
> **outside the Rust abstract machine** — `driver/src/dma.rs`'s `VolatileRxRing`/`VolatileTxRing`. It is
> the project's longest-open hazard (`architecture.md §3.1`: "carries hazards we have **not** analyzed to
> resolution"; `concerns-to-watch.md`). This closes it as a **positive, platform-scoped** account: sound on
> the deployment platform (x86-64 + cache-coherent DMA + this 82599 over PCIe), with the residual **stated**.
> It is **not** a general/unconditional soundness claim, and it is certified by **reading against its cited
> primary specs — never by a run** ("path-works ≠ crossing-sound").
>
> Grounded in the primary specs (not reference implementations): the **82599 datasheet** 331520-rev3.5
> (§3.1.4.4 Ordering Rules, §3.1.4.5.3 Relaxed Ordering, §7.1.6.2 Advanced Rx Descriptor Write-Back), the
> **PCIe Base Specification** transaction-ordering rules (posted-write ordering), and the **Intel SDM
> Vol.3A §8.2** memory-ordering model (x86-TSO). Matched line-for-line to `dma.rs`.

## 0. The crossing, and what must hold

The NIC writes RX descriptors + packet payloads into host memory, and reads TX descriptors + packet buffers
out of it, concurrently with the CPU — a mutation the Rust/C++ abstract machine does not model. Two
directions, each with a soundness obligation:

- **RX-READ** — when `descriptor_done(slot)` observes `DD=1`, the subsequent `copy_out(slot, …)` must read
  the **complete, non-torn** packet the device wrote (no stale bytes, no partial write in flight).
- **WRITE** — when the device fetches a descriptor (RX `refill`'s `pkt_addr`; TX `post`'s `buffer_addr` +
  `cmd_type_len` + the buffer bytes), it must read the **fully-written** descriptor/buffer, never a
  half-written one.

Each obligation decomposes into a **device/fabric** ordering guarantee (cited hardware property) and a
**CPU** ordering guarantee (cited + made explicit in code). Visibility and sequence are **kept distinct**
(see §3).

## 1. RX-READ: `DD=1` ⇒ the payload is complete (`descriptor_done` → `copy_out`)

**Device/fabric half — the payload reaches memory before `DD` (CITED):**

- The device writes the payload write(s) and then the descriptor writeback. **Datasheet §3.1.4.4 item 2
  (Descriptor/Data Ordering):** *"The 82599 insures that a Rx descriptor is written back on PCIe only after
  the data that the descriptor relates to is written to the PCIe link."* — the payload precedes the
  descriptor/`DD` **on the link**.
- Both are **PCIe posted writes** (datasheet: *"The descriptors are written back to host memory using PCIe
  posted writes"*), and §3.1.4.4 binds the 82599 to the **PCIe ordering rules** (PCI simple device model).
  Under the PCIe Base Spec, a posted request **must not pass** another posted request in the same direction
  — so the two writes reach host **memory** in issue order: payload, then `DD`.
- **Relaxed ordering does not break this — it is predicated on it.** §3.1.4.5.3 item 3: *"The 82599 cannot
  relax ordering for descriptor writes"*; item 2 permits receive **data** writes to bypass **each other**
  *"because software does not process this data until their associated descriptor writes are done."* The
  non-relaxable descriptor write stays ordered after the data, and the datasheet's own optimization assumes
  exactly the `DD`-gates-payload contract. So the guarantee holds **with or without** relaxed ordering.

**CPU half — the payload read is ordered after the `DD` observation (CITED + code):**

- `descriptor_done` reads `DD` with an **`Acquire` atomic load** (`AtomicU32::load(Acquire)` — realized as
  an atomic, not `read_volatile`, because acquire ordering is defined for atomic operations, §4). The
  `Acquire` orders **every later load in this single-threaded reactor** after it — including `copy_out`'s
  payload reads (program order: `descriptor_done` → `status`/`reported_len` → `copy_out`). **SDM Vol.3A
  §8.2.2:** *"Reads are not reordered with other reads"* (x86-TSO) makes LOAD-LOAD order hold; the `Acquire`
  states it **explicitly and portably** (on weaker ISAs it is the required barrier).
- **Cache-coherent DMA** (x86 platform) makes the device's coherent writes **visible** to the CPU load
  without an explicit cache operation.

**No-tearing:** `DD` is a naturally-aligned `u32` — a single-word read, **atomic** on x86 (SDM §8.1.1), so
the `DD` observation itself cannot tear. The multi-byte **payload** is read **only after** `DD=1`, and the
device wrote the entire payload **before** that `DD` (above) — so **no device write to that buffer is in
flight during the read**. Non-tearing follows from the ordering, not from an atomic payload read.

## 2. WRITE: the doorbell publishes a fully-written descriptor (`post`/`refill` → TDT/RDT)

**CPU half — the stores are globally visible before the doorbell (CITED + code):**

- `post` writes the packet buffer bytes + `buffer_addr` + `olinfo_status` + `cmd_type_len`; `refill` writes
  `pkt_addr` + `hdr_addr`. All are stores to **write-back (WB) cacheable** host memory. **SDM §8.2.2:**
  *"Writes are not reordered with other writes"* (x86-TSO STORE-STORE) — they retire to memory in program
  order.
- A **`fence(Release)`** precedes the doorbell store, and the doorbell (`TDT`/`RDT`) is a write to the
  **UC (uncacheable) MMIO** BAR: a UC write is strongly ordered and **drains the store buffer**, so all
  prior WB stores are globally visible before the doorbell write completes. On x86 the `Release` fence is a
  compiler barrier only (the UC write already enforces the CPU ordering); it is the **explicit, portable**
  form and pairs — *in the hardware sense* — with the device's fetch-after-doorbell.

**Device half — the device reads the descriptor only after the doorbell (CITED):**

- The device advances its ring only when the tail (`TDT`/`RDT`) moves; it fetches descriptors it did not
  previously own **after** the doorbell it observes (§7.2.3.4 Tx / §7.1.9 Rx descriptor fetch). Its DMA read
  of the descriptor + buffer is **coherent** (visibility), and — the tail write being ordered after the
  descriptor stores — it reads the **complete** descriptor/buffer.

## 2a. TX buffer reuse — the reverse race, closed by CONTROL FLOW (not a barrier)

The write direction (§2) covers publishing a *new* descriptor. There is a complementary TX obligation: the
CPU must not overwrite a TX buffer while the device is still DMA-**reading** it for the in-flight transmit.
This is closed by **control flow**, not a memory barrier:

- The device sets the TX-completion `DD` **after** it has read the buffer and transmitted it (§7.2.3 — the
  descriptor is done, buffer read complete). So `DD=1` ⇒ the device's read of that buffer is finished.
- `TxPath` (the guard, `tx.rs`) reclaims a slot **only** when `descriptor_done`/`is_free` observes `DD=1`
  (the completion), and only reclaimed (free) slots are reused. So the reuse **path** is control-dependent
  on observing `DD=1`.
- The reuse **store** (`post` writing new bytes) is therefore reached only *after* the branch on `DD=1`
  resolves. x86 stores are **never speculative** — a store commits only at instruction retirement, after the
  controlling branch is resolved (SDM Vol.3A §8.2.3.4/§11.10 store-buffer semantics) — so no reuse byte is
  written to the buffer before the device's read of the old contents completed.

So the TX `descriptor_done` needs **no acquire**: it gates only reclaim, and there is **no dependent
device-data load** after it (unlike RX, where the payload read follows). The ordering is the control
dependency `observe DD=1 → reclaim → reuse-store` plus non-speculative stores — a fully-articulated leg, not
a residual.

## 3. The two distinctions this account keeps (the papering traps)

- **Coherence ≠ ordering.** Cache-coherent DMA provides **visibility** — the CPU and device see each other's
  writes without explicit cache flushes. It says **nothing** about the **sequence**. The sequence
  (payload-before-`DD`; stores-before-doorbell) is provided by the **datasheet + PCIe posted-write ordering
  + x86-TSO**, cited above. Conflating the two would be the exact paper this account avoids.
- **Reference implementation ≠ authority.** `reference/ixy` and Linux `ixgbe` drive this crossing with no
  explicit fences and it works — but that is **path-works**, not the spec guarantee. This account rests on
  the cited primary specs; a green run (7a's live transmission) is evidence the path works, **not** that the
  crossing is sound.

## 4. Why `AtomicU32`/`fence`, not `read_volatile`/`write_volatile` + a comment

`read_volatile`/`write_volatile` give no-elision, no-reorder-**among-volatiles**, and single-width access —
**not** an acquire/release and **not** atomicity. A `fence(Acquire)` is defined to pair with an **atomic**
load; a `read_volatile` followed by a fence does **not** formally establish the acquire. So the `DD`-gating
read is an `AtomicU32::load(Acquire)` (the acquire is on the load itself), and the doorbell is preceded by a
`fence(Release)`. On x86 these compile to no additional instructions (TSO + UC-MMIO already order); their
value is the **explicit, portable, formally-defined** ordering and the annotated intent.

## 5. The residual — STATED, not papered

This is a **hardware-level** argument, sound **on the stated platform**:

- **Platform scope:** x86-64 (SDM memory model) + cache-coherent DMA + this **82599** over **PCIe** with its
  cited ordering guarantees. On a non-coherent-DMA platform, a weaker-ordered fabric, or a device without
  the §3.1.4.4.2 guarantee, the account does **not** carry — it would need re-derivation against that
  platform's specs.
- **Language-model gap:** Rust's (and C/C++'s) abstract machine does **not** model memory an external device
  mutates. The `Acquire`/`Release` primitives here synchronize — *in the hardware sense* — with the
  **device**, which is outside the abstract machine; there is no in-language release store to pair with. So
  this is a cited **hardware** argument, **not** a language-machine proof. That is the irreducible residual,
  and it is why the crossing is *platform-scoped-sound*, not *unconditionally sound*.

The **control-flow** half — poll `DD`, and only on `DD=1` read + copy; a `DD`-clear descriptor is never
read or delivered — is the separately-**testable** half, proven on the software sim (`rx.rs` gates 2/3,
byte-frozen). The **ordering effect** in this document is a **reasoning artifact** (a no-op on x86),
certified by reading the placement + intent against the citations above.

## Citations (primary)

- **82599 datasheet 331520-rev3.5:** §3.1.4.4 Ordering Rules (item 2 Descriptor/Data Ordering); §3.1.4.5.3
  Relaxed Ordering (items 2–3); §7.1.6.2 Advanced Rx Descriptor Write-Back; §7.1.9 / §7.2.3.4 descriptor
  fetch.
- **PCIe Base Specification:** transaction-ordering rules — a Posted Request must not pass another Posted
  Request (producer/consumer ordering).
- **Intel SDM Vol.3A §8.1.1** (aligned-access atomicity), **§8.2.2** (memory-ordering: reads not reordered
  with reads, writes not reordered with writes — x86-TSO), **§8.2.5** (serializing / UC ordering).
