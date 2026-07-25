# Capture coherence — binding a measurement to the bytes that produced it

A **capture** is a run on real hardware whose numbers become evidence in a gate. Coherence is
the property that every number in a capture binds to a specific binary, and that a reviewer can
verify the binding independently rather than taking the runner's word for it.

Sprints 5, 14 and 17 each rediscovered parts of this procedure, and Sprint 17's own gate
produced a *false* coherence mismatch because the recipe was scattered across three sprint
documents rather than written once. This is the one place.

## The coherence chain — four values, and what each rules out

For each binary in a capture, up to four independent measurements of the same bytes. They rule
out different failure modes, which is why more than one is needed:

| # | value | rules out |
|---|---|---|
| 1 | **build-time hash** — `sha256` of the binary on disk before the run | nothing on its own; it is the reference the others are compared against |
| 2 | **self-declared hash** — the binary hashes *itself* at startup and prints it | a capture whose numbers came from a *different* binary than the one hashed by a separate script |
| 3 | **running-bytes hash** — `sha256` of `/proc/<pid>/exe` while the process runs | a path replaced between build and run; this is the kernel's own handle on the executing image, not a filename |
| 4 | **reviewer's independent rebuild** | the runner's toolchain, source, or figures being wrong — this is the only value that ties *bytes back to source* |

**Values 1–3 prove numbers → bytes. Only value 4 proves bytes → source.** A binary can hash
itself perfectly and still be built from the wrong source; self-declaration does not replace the
reviewer's rebuild, and the two together are the standard.

### Self-declared vs external `/proc` read — the distinction that matters

Value 2 requires the binary to carry `benchtools::Provenance`, which prints a self-hash of
`std::env::current_exe()` at startup. Bench binaries do; **`serve` does not**, because
`benchtools` is deliberately off the production build (`proxy/Cargo.toml`, "the production lib
never links it") to keep verification tooling out of the TCB.

So `serve`'s bytes must be taken **externally**, by hashing `/proc/<pid>/exe` from the capture
script — value 3 in place of value 2. Sprint 17 did exactly this and got three of four chain
values rather than four. That is a standing property of the production binary, not a defect in
any capture, and it is an open escalation (Sprint 20, `FS#10`) rather than something a capture
can fix.

**Gotcha:** hash `current_exe()` *inside* the process. Passing the literal `/proc/self/exe` to a
child `sha256sum` hashes **sha256sum's own** executable, because the child resolves `/proc/self`.

## Building the binaries — three rules, each learned the hard way

### 1. One `cargo` invocation per binary. Never combine them.

Cargo unifies features across a single invocation. Building several binaries together compiles
the shared libraries with the *union* of their feature sets, producing different bytes than
building each alone.

Measured on this workspace (Sprint 17):

```
cargo build --offline --features serve,bench-capture --bin serve --bin s17_box --bin s17_head
    => serve = 3d0cbaab921721a434fa63fd081ca54100a2c85ad5dacf69af2aa03f9738c6f6

cargo build --offline --features serve         --bin serve
cargo build --offline --features bench-capture --bin s17_box
cargo build --offline --features bench-capture --bin s17_head
    => serve = 38e14a57bb497f1ddb3f4553d19e244dea30c55cc7083dedaf2cfbc2e8b88ff8
```

This is not a hash curiosity. `bench-capture` enables `directpath/bench`, so the combined build
links the **bench internals into the production binary** — precisely what the feature gate
exists to prevent. Record the exact per-binary invocation in the evidence chain, not just the
feature *names*.

### 2. Hashes are path-dependent without `--remap-path-prefix`.

A debug binary embeds its absolute build path (debuginfo and `panic!`/`file!()` location
strings). Two checkouts at different paths produce different hashes from identical source with
zero codegen difference — which reads as a coherence failure and is not one.

```
RUSTFLAGS="--remap-path-prefix <repo>=/build --remap-path-prefix <CARGO_HOME>=/cargo --remap-path-prefix <RUSTUP_HOME>=/rustup"
```

Two pins: the remapped hash is **profile-specific** (name the profile alongside it), and if
anything bakes `CARGO_ENCODED_RUSTFLAGS` into the binary the hash becomes sensitive to the
*spelling* of the flags — reproduce the capture header's rustflags string verbatim.

When a hash will not reproduce, check this **first**, before suspecting divergent source.

### 3. Target-directory dependence for `benchtools`-linked binaries.

A rebuild into a different `--target-dir` than the capture used can produce a different hash for
binaries that link `benchtools`. Reproduce into the same target directory, or expect a mismatch
that is an artifact of the rebuild rather than a real divergence. (Review manufactured a false
mismatch on Sprint 17's gate from exactly this, and caught it before reporting.)

## Commit the source before the run

A capture's provenance header records a `git` rev. **That rev is meaningless if the tree was
dirty at build time** — nobody can check out that rev and rebuild those bytes.

Worse, an *untracked* source file can disappear entirely. Sprint 17: `s17_head.rs` had never
been committed; a post-capture fix edited it in place, and the source for that binary's hash
ceased to exist in any form — no git history, no index entry, no copy on disk. The binary was
preserved and independently re-hashed, so the evidence held, but one of three binaries
permanently lost its bytes→source link.

**Commit the source before the measurement.** Then the header's git rev means something, and a
post-capture fix moves the tree forward instead of stranding evidence behind a file that no
longer exists.

**If the source is already gone: record the gap. Do not reconstruct the file** — even when the
prior text is recoverable or describable. A reconstructed source *looks* like evidence while
being an assertion: it may not rebuild to the pinned hash, and no downstream reader can tell by
inspection. Preserve the binary out-of-tree, obtain an independent re-hash, and state which
binaries remain re-derivable and which are anchored only on the preserved artifact.

## What to record in the evidence chain

- The exact per-binary `cargo` invocation, verbatim, and the toolchain (`rustc` version,
  profile, rustflags).
- Which of the four chain values exist **per binary**, and which do not. State gaps rather than
  averaging them into a claim of completeness.
- Whether `--remap-path-prefix` was used; if not, say the hashes are path-dependent.
- Whether the tree was clean at build time; if not, say the git rev does not pin the source and
  name the bin-sha as the anchor instead.
- The artifact location for anything preserved out-of-tree.

## Related

- `docs/proxy/tools/s17-capture.md` — a worked example: the Sprint 17 runner, its preconditions,
  its teardown, and the safety invariants for touching bench hardware.
- `docs/proxy/review-vocabulary.md` — `MB1`/`CB1` (coherence) are the gate bars this procedure
  discharges.
