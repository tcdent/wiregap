# wiregap

A containment system for AI agents. The agent runs on its own machine (the **box**) with no
network, no credentials, and no restrictions on what it does internally. Its only connection
to anything else is a single point-to-point cable to a second machine running a small,
human-reviewable program (the **proxy**). Nothing the box does has effect beyond its own
boundary unless the operator has approved that class of act in advance.

The design rests on one fact: the human operator answers for everything the system does. So
the system's job is to make sure the set of acts it can perform never exceeds the set the
operator approved. It does this by construction, not by inspection: the forbidden act is made
unreachable, never caught after the fact.

The isolation goes further than a firewall or a sandbox. The box is a separate physical
machine whose only link is one cable to the proxy, and that link carries no IP, no TCP, and
no kernel networking on the trusted side: the proxy drives the network card directly with
its own driver, written from the datasheet, and speaks its own wire protocol over it. That
protocol cannot express a control sequence, a redirect, or a destination the operator did
not approve. The box never holds a credential, the proxy never acts as a server the box can
attack through a general-purpose stack, and what the box may do is compiled into the proxy
rather than loaded from configuration. The operator's display shows typed screen cells and
interprets nothing the box sends.

`docs/principles.md` states the full set of assertions the design follows from. It is one
page and is the right place to start.

## Status

This is a working proof of the trusted middle, not a usable system yet.

| Part | State |
|---|---|
| Wire protocol and codec (`proto`) | Complete: all 22 message types, fuzzed, fully covered |
| Link: driver, transport, reactor (`driver`, `transport`, `reactor`, `directpath`) | Working on real hardware |
| Proxy (`proxy`) | Working; proven end to end on real hardware against the production Anthropic API |
| Head (the operator's display client) | Designed (`docs/head/architecture.md`); not built |
| Box-side agent runtime | Not built; only a scripted test driver exists |
| Wire numbers (message ids, error codes) | Provisional; not yet frozen |

The end-to-end proof ran with a deliberately invalid API credential, so the upstream answer
was a genuine 401. A successful, billed request has not been run.

## How it works

```
  box (untrusted)                 proxy host (trusted)                    outside
 ┌───────────────┐   10G DAC    ┌──────────────────────┐   TLS    ┌──────────────────┐
 │ agent runtime │◄────────────►│ proxy                │─────────►│ allowlisted APIs │
 │ any software  │  one cable,  │  - route register    │          └──────────────────┘
 │ no network    │  no kernel   │  - request recognizer│   socket ┌──────────────────┐
 └───────────────┘  networking  │  - file quarantine   │◄────────►│ head (operator)  │
                                └──────────────────────┘          └──────────────────┘
```

The proxy fronts three channels for the box:

- **Cloud egress.** The box asks the proxy to make a request. The proxy matches the
  destination against a compiled-in register of approved routes, then checks the request
  against that route's rules. A request that is not fully recognized is denied whole. The
  proxy holds the credential; the box never sees it. Responses flow back to the box verbatim.
- **Files.** The box can write files into a quarantine directory on the proxy host (mounted
  `noexec,nosuid,nodev`, written by an unprivileged uid) and read files from one readable
  root the operator chooses.
- **Terminal.** The box's screen is relayed to the head as typed display cells, and the
  operator's input is relayed back. The wire format cannot express control sequences, and the
  head displays without interpreting.

One rule organizes all three: bytes landing on trusted ground are gated; bytes landing on
the box are free.

## Design mentality

These are the working rules the code was written under. They explain choices that would
otherwise look odd.

- **Defend effect, not information.** The box may read anything and send anything through
  the channels it has. Disclosure is not treated as a harm; only effect is.
- **Rigor follows the boundary.** The box gets none, because its code can be rewritten by
  whatever runs inside it. All the care goes into the proxy.
- **Reviewability is the guarantee.** The security-critical core of the proxy is kept small
  enough for one person to read completely. Its checks are total functions over finite
  domains, verifiable by enumeration.
- **Borrow vetted ideas; exclude code that cannot be read.** The Linux network stack is
  bypassed because it is too large to review, not because it is unsound. The NIC is driven
  by a minimal userspace driver written from the datasheet.
- **UART simplicity at a higher bit rate.** The link has roughly a thousand times more
  capacity than the workload needs, and that headroom is spent on simplicity: stop-and-wait,
  an ACK per frame, a fixed retransmit interval, no sequence numbers, no congestion control.
- **No runtime configuration.** Routes, filesystem roots, and the credential's source are
  `const` Rust, validated at compile time. Changing what the box may do means editing
  source, reviewing it, and rebuilding. There is no config file to parse or tamper with.
- **Fail closed, by name.** The proxy starts only if every authority it needs validates. If
  one is missing, nothing is served and the proxy says which one.
- **Unsafe code is confined.** `proto` and `transport` forbid it. Where device memory forces
  it, it lives in a single named module per crate.

## Components

Seven independent Rust crates linked by path dependencies. There is no root workspace.

| Crate | What it is |
|---|---|
| `proto/` | The wire protocol codec. `no_std`, no dependencies, no unsafe. Any input yields a message or a typed rejection, never a panic. Feature-gated per service so a consumer compiles only the grammar it uses. |
| `transport/` | The session protocol: frames, session table, stop-and-wait with retransmit. `no_std`; all I/O sits behind traits. |
| `reactor/` | The single-threaded event loop that drives `transport`. |
| `driver/` | The NIC data path: receive and transmit descriptor rings, with the device-memory crossing confined to one module. |
| `directpath/` | Takes direct ownership of an Intel 82599 from userspace on running Linux, and exposes the link as a thin `open`/`send`/`recv` interface. |
| `proxy/` | The trusted core: route matching, request recognition, file read and write, terminal relay, the upstream TLS client, privilege drop, and the `serve` binary. |
| `benchtools/` | Measurement and provenance helpers for bench harnesses. Never linked into the proxy. |

`reference/` holds third-party grounding material (the 82599 datasheet, other drivers read
for comparison). Only its index, `reference/SOURCES.md`, is in version control.

Design documents live under `docs/`: `principles.md` first, then `docs/proxy/architecture.md`,
`docs/nic/architecture.md`, and `docs/proto/model.md`. `docs/teams/` holds the sprint
records the code was built against.

## Building and testing

Requires a recent stable Rust toolchain (edition 2024). Each crate builds on its own:

```
cd proto && cargo test
cd ../transport && cargo test
cd ../proxy && cargo test
```

A plain `cargo build` in `proxy/` produces the library only. Binaries that touch hardware
are behind features so the default build never links them:

```
cd proxy
cargo build --release --features serve --bin serve
```

Build each feature-gated binary in a separate `cargo` invocation. Cargo unifies features
across one invocation, and combining `serve` with `bench-capture` links bench internals into
the production binary.

## Deploying to hardware

There is no installer and no packaging. Deployment today means editing a few constants,
building `serve`, provisioning the host, and running it as root. The procedure below is the
one used for the end-to-end proof, which ran both cards in a single host. A two-machine
deployment follows the same steps on the proxy host but has not been run.

### Hardware

- Two Intel 82599-based 10G NICs (X520), one per side, joined by a passive DAC cable.
- An x86-64 CPU with `MONITORX`/`MWAITX` on the proxy side (AMD Zen and later). The idle
  wait is built on it; the Intel equivalent is not implemented.
- Linux with hugepages available and access to `/proc/self/pagemap`.
- A separate network interface on the proxy host for upstream access. The 82599 is taken
  away from the kernel and carries only the box link.

### What the proxy does to the host

`serve` unbinds its NIC from the kernel `ixgbe` driver, maps the device registers through
sysfs, and gives the card hugepage memory to read and write directly. No IOMMU is used, so
the card's firmware can in principle reach any host memory. This is a known, accepted
residual, described in `docs/nic/architecture.md` §2.1.

Cards are selected by exact PCI address, never by scanning for a device id. If the host has
another 82599 carrying its real network connection, a scan would match it too.

### Configure

Configuration is compile-time. Edit, then rebuild:

| Setting | Where |
|---|---|
| PCI address of the proxy's card | `PROXY_BDF` in `proxy/src/bin/serve.rs` |
| MAC address of the box's card | `PEER_BOX_MAC` in `proxy/src/bin/serve.rs` |
| Unprivileged uid/gid to drop to | `DROP_UID`, `DROP_GID` in `proxy/src/bin/serve.rs` |
| Head socket path | `HEAD_SOCKET_PATH` in `proxy/src/bin/serve.rs` |
| Approved routes, tool identities, filesystem roots, credential variable | `proxy/src/register.rs` |

### Provision

```
# Quarantine: where box-written files land. Must sit on a noexec,nosuid,nodev mount
# and be writable by the drop uid. /run is a suitable tmpfs on a stock systemd host.
mkdir -p /run/zerotrust-proxy/quarantine
chown 65534:65534 /run/zerotrust-proxy/quarantine

# Readable root: what the box may read from this host.
mkdir -p /srv/zerotrust-proxy/readable
```

Chown only the quarantine subdirectory, not `/run/zerotrust-proxy` itself. The startup
checks cannot detect a quarantine the drop uid cannot write to; that shows up as a denied
write later.

### Run

```
sudo ANTHROPIC_API_KEY=... ./proxy/target/release/serve
```

Startup order matters:

1. `serve` binds the head socket and waits for a head to connect.
2. Once a head is connected, `serve` validates its authorities, brings up the NIC, and drops
   privilege.
3. The box side can then start sending.

The head socket is currently root-owned, so only a root process can connect, and `serve`
accepts a single head connection for its lifetime.

`docs/proxy/tools/s17-capture.sh` runs the whole sequence with scripted stand-ins for the
head and the box, and restores the cards to the kernel driver afterward. Read
`docs/proxy/tools/s17-capture.md` before using it: it changes system state and has
host-specific PCI addresses in it.

### Restoring a card to the kernel

```
echo -n 0000:BB:DD.F | sudo tee /sys/bus/pci/drivers/ixgbe/bind
```
