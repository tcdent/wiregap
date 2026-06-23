# Transport Stack — Scope, Decisions, Groundings, and Validation

### Companion to *Containment by Construction* and *The Console, Protocol & Multiplexer*

> This document records three things about the **hardware → driver → transport → protocol** stack:
> the **scope** of that layer (what it owns and where it stops), the **decisions** converged on while
> working through it, and the **documentary grounding** behind the hardware claims plus the **bench
> validation** still required. It is deliberately bounded: it does not design the application layer
> that rides the transport. Where a decision touches the line between transport and application, the
> line is stated, not crossed.
>
> The detailed, citation-level grounding for the hardware claims lives in the companion file
> *X520-Connectivity-Grounding.md*; this document summarizes those findings and adds the decisions and
> the validation plan.

---

## 1. Scope — what this layer is, and where it stops

The remit is a single vertical slice: the silicon, the userspace driver that brings it up and moves
bytes, the transport substrate that carries those bytes reliably, and the protocol that multiplexes
streams over it. The slice begins at the X520/82599 and ends at **the stream**.

**In scope — the transport delivers the lifecycle and identity of multiplexed streams:**

- a stream coming into existence (box-minted ID, appearing by emission)
- demultiplexing by discriminator (routing, zero security weight)
- per-stream sequencing and acknowledgment (stop-and-wait, every frame ACKed, retransmit on silence)
- flow control (absence of ACK as backpressure)
- teardown (close; ID never reused; restart as session boundary)
- the frame contract that makes all of the above possible (discriminator + flag byte, length from hardware, everything else opaque)

**Out of scope — what runs *inside* a stream is application, and ambiguous by design:**
the meaning of a stream (that it is a PTY, a file egress, or a control conversation), the verbs that
request new streams, PTY semantics, the file-selection workflow, the display payload's internal
layout, the headless terminal emulator on the box, and the head's rendering. These *consume* the
transport; defining them is not this layer's job. The transport carries a stream without knowing or
needing its purpose.

**The one residue that looks like application but belongs here for a security reason.** The proxy
performs exactly two operations on box-originated bytes as they land on trusted ground: it
*neutralizes* display content (a total function over the 256 byte values) and it *writes* file
content into a quarantine it owns. To apply the right treatment, a stream must carry, at its open, a
minimal signal of which treatment its bytes get. That signal is in scope — not because the transport
understands the stream's purpose, but because the byte-treatment is a trusted-ground safety operation.
It is the narrowest possible exception to "contents are opaque," and it is the only proxy-read
structure in the whole protocol.

---

## 2. Decisions and clarifications

The decisions below are stated with the *why* compressed to a line. Several are **clarifications** —
places where the initial framing was sharpened or corrected during the work; those are marked.

### 2.1 Hardware

The X520-DA2 is the Intel **82599ES**, PCIe 2.0 ×8, dual SFP+, driven over a passive **DAC** (which
puts the link in **SFI** mode — 10G serial, no external PHY). One port is used; the second is a
separate PCIe function left dark. The slice of the chip actually lit is thin: one RX ring, one TX
ring, the hardware CRC, and the MMIO BARs. Everything else stays dark — multiqueue/VMDq (64 queues
per port), SR-IOV, RSS, Flow Director, FCoE/iSCSI offload, DCB/QoS, 1588, all checksum/TSO offloads —
and each untouched feature is configuration space that cannot carry a bug in our code. The part is
end-of-life, which is why its datasheet is public and stable and why the published **errata
(specification update)** is mandatory reading rather than optional.

### 2.2 Driver and bring-up

Kernel-bypass via **VFIO** (`vfio-pci`), IOMMU-backed. The IOMMU is kept deliberately as a
*confinement the hardware enforces* — it bounds the NIC's DMA to mapped buffers — not as an
attacker-facing surface. Bring-up is a near-minimal fixed sequence (the equivalent in ixy is ~147
lines of actual code): disable interrupts → global reset → wait for EEPROM auto-read and DMA-init →
link setup (AUTOC, SFI mode) → clear stats → RX init → TX init → enable queues → wait for link.

*Clarification on minimality.* The way to get **smaller** than ixy is to remove its *optionality* —
the interrupt-mode machinery, the multi-queue loops collapsed to one queue, test conveniences — not to
trim the reset→link→ring→enable spine, which is already near-minimal and mostly either mandatory or
quirk-driven (the `RXPBSIZE` consolidation, `DTXMXSZRQ`, the `CTRL_EXT`/`DCA_RXCTRL` quirk bits are
*not* redundant). ixy itself models the right principle — it skips datasheet-suggested disables for
features it never enables.

*Clarification on MAC.* Use the **burned-in MAC** and the card's default unicast path: address frames
to the receiver's burned-in address, change no flags, generate nothing, run no promiscuous mode. This
lands in the code path the silicon was built for, and the reasoning is one sentence — *I addressed a
frame to that device; the device accepts frames addressed to it.* (An earlier line of thought toward
synthetic/locally-administered MACs, zeros, or promiscuous mode was dropped: it modified default card
behavior and required a justification the default path doesn't.)

*Clarification on interrupts.* The full ixy interrupt apparatus (~265 lines of code across the device,
policy, and VFIO layers) is mostly **adaptive mode-switching** that is not wanted. Strip all of it and
keep only a thin **idle-wakeup** slice (~60 lines): arm one interrupt + eventfd, and when the poll
loop finds the ring empty, block on the eventfd until a packet arrives. The interrupt is not "an
optimization we declined" — it is the **event primitive** the entire event-driven stack rides on (see
2.5). Per-packet operation remains poll-mode (busy-poll while there is work, which beats taking an
interrupt per frame); the interrupt is used only to sleep when idle.

### 2.3 Transport substrate — the link and reliability

The link's *one real failure mode* is **receiver overrun** — the receiver draining its descriptor ring
slower than the card fills it. Bit errors are caught by the NIC's hardware CRC (a corrupt frame is
dropped, indistinguishable from a lost one); a point-to-point cable has no routing, no reordering, and
no congestion, so those mechanisms simply do not exist here. The reliability answer derived for that
single failure is **per-stream stop-and-wait with a per-packet ACK**: in-order delivery falls out for
free (a requirement, since terminal state is serial — not a limitation), and aggregate throughput
scales by running the one dumb codepath as **K parallel streams**, never by making the codepath
cleverer.

The hot path is: poll the descriptor's **DD** (descriptor-done) bit in host memory (no MMIO), process,
refill, and write the tail register once per batch; when idle, block on the eventfd. The received
**frame length comes from the RX descriptor in hardware** — there is no software length field.

### 2.4 The frame contract

A **single frame shape**. The only frame-level fields are a **stream discriminator** (which stream —
pure demux, zero security weight, a misroute is cosmetic) and a **flag byte** (the stop-and-wait
alternation bit and an ack bit). Length comes from hardware. Everything else is **opaque payload**.

There is no length field, no traditional header, and no routing or inspection fields, because the
transport does no routing and needs no inspection beyond demux. Data, ACK, and piggybacked-ACK are
**flag states of the one shape**, not separate frame types — which is how "one frame type" survives
while still distinguishing an ACK from data.

### 2.5 Stream identity and lifecycle

The **box is the sole allocator** of stream IDs. The ID is a **monotonic counter**, generously sized
(64 bits retires exhaustion as a concern), **never reused**. This is the self-allocating, no-coordination
property a UUID would give, at a fraction of the bytes — and a single allocator means there is no
collision domain to negotiate.

Streams **appear by emission**: the box opens a stream by sending on a fresh ID, and the proxy binds it
**on first sight**. There is no negotiation, no setup handshake — existence-by-emission *is* the
handshake. Teardown is equally stateless: close the stream and never speak the ID again.

*Clarification on host-initiated streams.* The host can cause a stream to exist but never **allocates**
one: a request travels into the box (the safe direction — you may send the box anything), and the box
mints the ID and opens the stream back. "Host-initiated" means *cause*, not *allocation*. Correlating a
request to the stream it produces is a **payload convention** between the endpoints, not a transport
field.

*Clarification on restart.* A counter is per-box, so the proxy keys stream state to a **box session
that resets on restart**. A restarted box is a new session; the proxy drops its prior stream state, so
a counter reset never overlaps a stale binding.

### 2.6 The unified protocol — ACK, retransmit, flow control

**Every frame is acknowledged; every stream is stop-and-wait; one path, no per-stream branch.** This is
a deliberate choice of uniformity over special-casing — at single-digit-µs RTT a single stream clears
~200k frames/sec, far above any rate a pane produces, so acknowledging everything costs nothing.

*Clarification — the ACK is the event-driven completion marker.* A buffer is freed on **ACK arrival**,
not on TX-completion, because stop-and-wait may need to retransmit that exact frame until it is
acknowledged. So the ACK does double duty — flow control *and* buffer reclamation — and the meaningful
completion event is a packet on the wire, not a control structure. This is the concrete form of "it's
all just data": an ACK is just a data frame.

Retransmit fires on **silence** (a missing ACK past a deadline). This is the one event the arrival
interrupt cannot give you — nothing arriving means nothing fires — so it needs a timer, but the timer
**composes into the same wait**: block on the eventfd *or* the retransmit deadline, one loop, two exit
reasons. Backpressure is the **absence of an ACK** (withhold the ACK and the sender cannot advance) —
there is no separate flow-control message, and **802.3x PAUSE is declined** (it is a control frame
outside data, ethertype `0x8808`, and it head-of-line-blocks the whole link to relieve one stream).

Overflow resolves into the same machinery rather than a subsystem: the TX ring filling is made
**unreachable by sizing** (ring depth ≥ max concurrent streams), and RX overflow is handled by the
NIC's **`DROP_EN`** — a dropped frame is never ACKed, the sender's timer fires, it resends, identical
to the loss/CRC path. **Coalescing** (e.g., a retransmit carrying the *current* keyframe instead of
the stale one) lives **below the protocol, in the data source** — "resend N" and "send latest" are the
same shape for an idempotent keyframe — so it is not a protocol branch.

### 2.7 The stream abstraction and the proxy

A **stream is the transport primitive** — an addressed, sequenced, flow-controlled byte channel. It is
*not* a PTY or a file transfer; those are **application bindings** a stream is given, and the binding
plus its semantics ride in payload.

*Clarification — the proxy is a protocol transformation, not a verbatim relay.* The box↔proxy seam is
the security-relevant boundary; the proxy↔head seam is trusted-to-trusted, where the proxy may
transform, reframe, or re-serialize freely. The real invariant is directional and substantive:
**box-originated bytes must be made incapable of executing on trusted ground, and the proxy must not
itself inject malicious content downstream** — *not* the stricter "the proxy must never touch the
bytes," which was a phantom constraint. The proxy's two trusted-ground operations are total functions:
neutralize display content (byte-map over 256 values) and write file content verbatim into the
quarantine. Neither is parsing.

*Clarification — there is no "control stream" category.* The first stream the box brings up is special
only in being **first**; it runs the identical protocol as every other stream. Verbs, bindings, and any
control conversation are **payload the endpoints interpret** and the proxy does not. The host requests
new streams by interacting with an existing stream; the requested streams then appear by emission. No
second protocol or dedicated control format exists.

*Clarification — the file destination is the proxy's, not negotiated.* The proxy **owns** the
quarantine and decides placement; the box cannot address the trusted host's filesystem. What the box
supplies is a **constrained filename** — the spec's array-of-leaves — where each element must contain
**at least one alphanumeric**, may contain dots, and contains no separators (so `..` is not a rejected
path but simply *not a valid leaf*). The proxy joins those leaves under its own root; **traversal is
unsayable, not caught**. This is *filename*, not *destination* — the load-bearing security property is
that the proxy owns placement, and the leaf rule makes the box-supplied name safe to use without it
ever becoming an address. The frozen, proxy-read structure at a stream's open is therefore exactly:
the **binding-type discriminator** (which byte-treatment) plus, for a file stream, the **leaf-array
filename** — and nothing more.

---

## 3. Groundings and references

The hardware-and-driver claims above are grounded in primary documentation and reference
implementations. Detailed, sentence-level citations are in *X520-Connectivity-Grounding.md*; this is
the index, plus the empirical findings from reading the keystone reference directly.

### Primary — Intel 82599 documentation

- **Intel 82599 10 GbE Controller Datasheet** (rev 3.5, Nov 2024; doc 331520) — initialization chapter (the §4.6 family: init sequence, link init, statistics, interrupt, receive init, transmit init), register definitions (§8), AUTOC at §8.2.3.22.19, RSS at §7.1.2.8.
- **Intel 82599 Specification Update** (doc 331521) — the AUTOC/LESM link-setup errata and other bring-up corrections; mandatory because the part is EOL.

### X520-DA2 hardware

Vendor/board specifications confirm: Intel 82599ES, PCIe 2.0 (5.0 GT/s) ×8, dual SFP+, **SFP+ Direct
Attach Copper** (passive twinax) up to 10 m, network interfaces SFI/KR/XAUI/KX/KX4/BX/CX4, VMDq with
64 queues per port, SR-IOV. The DAC/SFI choice matches the simplest link mode the part offers.

### Reference driver implementations

- **ixy** (`emmericp/ixy`; "User Space Network Drivers," Emmerich et al., arXiv:1901.10664) — the keystone existence proof: a *complete* userspace driver for this exact 82599/X520 family in ~1,000 lines of C, built from the datasheet using DPDK and Snabb only to *understand* it (not as a source base), VFIO/IOMMU, runnable unprivileged. This is precisely the "learn from `ixgbe` behavior, use none of its source" posture, already executed on the target silicon.
- **ixy.rs** — confirms no kernel modules required except `vfio-pci`, and operation without root via the IOMMU.
- **DPDK `ixgbe` PMD** — the production-grade behavior reference, source annotated with the datasheet sections (RSS §7.1.2.8, statistics-init, the `TDBAL`/`TXDCTL` and `RDBAL`/`RXDCTL`/`SRRCTL` queue registers). Documents that a poll-mode driver accesses RX/TX descriptors directly **without interrupts** (except the Link-Status-Change interrupt).
- **Snabb** — the second behavior reference named by ixy's authors.
- **Linux `ixgbe`** — the authoritative behavior oracle; the *don't-copy* reference whose commit history explains why each bring-up step exists.

### Empirical findings from direct reading of the ixy source

(Measured by cloning the repo and counting the actual functions — these are observations of the code,
not web claims.)

- **Core bring-up** (`reset_and_init` + `init_link` + `init_rx`/`init_tx` + queue-start + `wait_for_link`, plus the poll-mode interrupt-*disable*): **~147 lines of code** (~224 with the datasheet-referencing comments).
- **Full interrupt apparatus**: **~265 lines of code** across the device layer (~113), the adaptive policy in `interrupts.c` (~28), the VFIO MSI/MSI-X/eventfd/epoll layer (~99), and the `rx_batch` integration (~25). The **idle-wakeup slice actually needed is ~60 lines** — the rest is adaptive mode-switching that is dropped.
- **Data path**: `rx_batch`/`tx_batch` do **zero offload** (no checksum/TSO/RSC); the only removable-for-poll-mode code is the RX-side interrupt machinery. TX cleanup is **batched** (`TX_CLEAN_BATCH = 32`) for PCIe efficiency, with a known sub-batch buffer-stranding wrinkle that generous mempools (or simple per-descriptor cleanup, affordable at our rates) neutralize.
- Ring depth default is 512 descriptors (max 4096).

### VFIO / IOMMU

- **Linux kernel VFIO documentation** and **IOMMUFD documentation** — the group/container interface (legacy) versus the cdev/iommufd interface (the direction new userspace should target).
- The control-plane sequence: open container → open group → `VFIO_GROUP_SET_CONTAINER` → `VFIO_SET_IOMMU(TYPE1)` → `VFIO_GROUP_GET_DEVICE_FD` → `VFIO_DEVICE_GET_REGION_INFO` → `mmap` BAR0. `VFIO_IOMMU_MAP_DMA` performs `pin_user_pages()` + `iommu_map()`.
- **IOMMU as confinement**: a device's DMA is bounded to explicitly mapped IOVAs; it cannot reach unmapped memory — the basis for "we gave a device direct memory access" *not* meaning "the device can scribble all of RAM."

### Kernel-bypass hot path

- DPDK's PMD model: RX/TX descriptors accessed directly, **no interrupts** (except LSC), **ring buffers shared directly** between userspace and the NIC, **eliminating context-switch overhead**.
- The **QEMU/VMexit model does not apply**: this is a bare-metal userspace process, not a guest, so the MMIO tail-write is a direct CPU store to the mapped BAR and does not trap. (This distinction is the difference between the kernel-free hot-path claim being exactly true and merely approximately true.)

---

## 4. Next steps — bench validation

The decisions above rest on a chain of hardware behavior asserted from documentation but not yet
verified on silicon. The validation plan below is the frontier of *this* layer: not further protocol
design (the transport is converged and self-consistent), but confirming the foundation it stands on.
Each item is a concrete thing to measure or confirm.

### 4.1 Device bring-up

1. Bind the device to `vfio-pci`; confirm it leaves the kernel `ixgbe` driver and the NIC disappears from the host stack.
2. **Read back NVM-configured registers *before* any writes** (`AUTOC`, `RXPBSIZE`, link mode, etc.) to see what the board's flash already sets — this directly resolves which bring-up writes are redundant, especially the `AUTOC` link override that ixy itself questions.
3. Run the minimal reset → link → ring → enable sequence and confirm **link-up on the passive DAC via AUTOC pokes alone** (SFI mode), with no SFP+ module I²C/SDP handling. *If* module detection turns out to be required, that is added driver scope — flag and size it.
4. Confirm whether the spec-update **AUTOC link-setup errata** applies on this stepping and fold it in if so.

### 4.2 Kernel-bypass and hot path

5. `mmap` BAR0 via VFIO; confirm direct userspace MMIO register writes (the tail pointer) take effect.
6. `VFIO_IOMMU_MAP_DMA` the descriptor rings and packet buffers; confirm DMA works and that the IOMMU bounds it (a deliberately out-of-range access is blocked).
7. Confirm **bare-metal operation (no VM)** so there is no VMexit, and verify **zero syscalls in the per-frame loop** under steady-state RX/TX (`strace`/`perf`).
8. Wire one **MSI (or MSI-X) + eventfd**; confirm the **idle-wakeup** loop — busy-poll while there is work, block on the eventfd when the ring is empty, wake on arrival — behaves and that the retransmit deadline composes into the same wait.

### 4.3 Ring architecture and throughput

9. **Single-ring sufficiency**: confirm one RX/one TX ring fills the 10 G pipe at the relevant frame sizes. Expectation from the analysis is that a single ring suffices and hardware multiqueue is retired; this is the empirical confirmation of that fork.
10. **Stop-and-wait arithmetic**: measure single-stream frames/sec at the link's actual RTT, confirm it clears terminal rates by orders of magnitude, and confirm **K parallel streams** scale aggregate throughput.
11. **Cleanup**: validate that **simple per-descriptor cleanup** is affordable at these rates (versus ixy's batched cleanup), which would let the design drop the batching complexity and its stranding wrinkle entirely.

### 4.4 Sizing and forks to settle on hardware

12. **DMA buffer pool size**: size against the **ACK-hold buffer lifetime** — buffers live until their ACK arrives, not until TX-completion — across the maximum concurrent stream count. Measure real held-buffer counts rather than copying a default.
13. **MTU / jumbo fork**: measure whether jumbo frames (9 K) meaningfully help the *file-transfer* path against the buffer-memory cost; keep single-buffer-per-frame so the multi-segment path is never touched.
14. **Ring-ownership / threading**: decide single-producer (all PTY captures funnel into one TX thread — no locking, consistent with why single-ring was simple) versus synchronized producers for the one TX ring.

### 4.5 Currency

15. Pull the **latest 82599 specification update** and apply any relevant errata to the bring-up sequence.
16. Target **iommufd / cdev** rather than the legacy VFIO group/container ABI for new code; the architecture is unchanged either way.

---

*The transport is converged: streams come into existence, are identified, sequenced, acknowledged,
flow-controlled, and torn down — and what rides inside them is the application's concern, opaque to the
wire. What remains is not protocol design but validation of the hardware-and-driver foundation the
transport asserts. The bench is where the stack the machine owns stops being reasoned about and starts
being measured.*
