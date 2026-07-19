# bug-351: macOS linker panics on an out-of-range relocation offset where Linux returns a diagnostic

Last updated: 2026-07-18
Effort: small (<1h)
Severity: LOW
Class: Correctness (robustness / diagnostics quality)

Status: Open
Regression Test: tests/ (new) — an `EncodedImage` whose relocation offset exceeds
`text.len()` must yield `Err` on **both** platforms (per the instruction already
recorded in `bugs/bug-335-linker-binary-repr-cleanup.md:60-90`)

The macOS linker's relocation patch helpers index the text buffer directly and
panic on an out-of-range offset; the Linux twins bounds-check and return `Err`. The
same relocation stream drives both `patch_relocations` implementations, so on
identical malformed input the macOS build aborts the compiler with a Rust panic
while the Linux build prints an actionable error. This is one-sided-fix residue:
the Linux side was hardened by bug-225 and the macOS twin — byte-identical before
that commit — was never updated.

**Reachability is the whole story here, and it is narrow.** Relocation offsets are
recorded by this compiler's own encoder immediately before the 4-byte word they
annotate, so `offset + 4 <= text.len()` holds *by construction*. `EncodedImage`
has no deserialization path — a downloaded `.mfp` carries IR, never relocations —
so **this is not reachable from untrusted input**. It fires only if an internal
codegen bug produces an inconsistent offset, which is exactly when a clear
diagnostic matters most and a panic serves worst.

The single correct behavior a fix produces: an out-of-range relocation offset
returns the same bounds error on macOS as it does on Linux — a diagnostic, never a
panic — so an internal codegen defect surfaces as a readable message on both
platforms.

References:

- `bugs/completed-bugs/bug-225-linux-linker-robustness.md` — hardened the Linux
  half only (commit `08d998b0`, "fix(linux-link): bounds-check reloc read/write +
  drop vestigial param").
- `bugs/bug-335-linker-binary-repr-cleanup.md:60-90` — **already documents this
  exact asymmetry** and explicitly instructs that it "deserves its own bug with its
  own regression test… File it before or alongside item A4; this cleanup must not
  be the changelog entry for a panic fix." This document is that filing.
- Found during the cleanup review of the linker surface.

## Failing Reproduction

There is no user-facing reproduction — no input a user can supply reaches this
(§Root Cause, reachability). The reproduction is a unit-level one, and it is the
regression test the fix needs:

```rust
// Construct an EncodedImage whose text is 8 bytes but whose relocation
// offset is 6 (so offset + 4 = 10 > 8), then link it on each platform.
let image = EncodedImage { text: vec![0u8; 8], relocations: vec![
    EncodedRelocation { offset: 6, /* … */ }
], /* … */ };
```

- Observed (macOS, `src/os/macos/link/mod.rs:616`): thread panics —
  `range end index 10 out of range for slice of length 8` — the compiler aborts.
- Observed (Linux, `src/os/linux/link/mod.rs:608-625`): `Err`, message
  `"linux linker: relocation offset 6 + 4 exceeds text length 8"`.
- Expected: both return `Err` with an equivalent message.

Contrast cases that are correct today: every Linux read/write site
(`src/os/linux/link/mod.rs:178,184,185,194,197,213,227,228,243,246,261,275,292,298`
plus the riscv64 helpers at `466,468,474,475`) propagates with `?`. The Linux
`.expect("slice length")` at `src/os/linux/link/mod.rs:615` is *infallible* — it
converts an already-bounds-checked 4-byte slice — and is not a hazard.

| Environment | Path | Result |
| --- | --- | --- |
| macOS | `src/os/macos/link/mod.rs:615-621` | panics ✗ |
| Linux (aarch64/x86_64/riscv64) | `src/os/linux/link/mod.rs:608-625` | returns `Err` ✓ |

## Root Cause

`src/os/macos/link/mod.rs:615-621` — `read_u32` slices `bytes[offset..offset + 4]`
and `write_u32` does `bytes[offset..offset + 4].copy_from_slice(...)`. Both return
plain values (`u32` / `()`), not `Result`, so their ten call sites inside
`patch_relocations` (`src/os/macos/link/mod.rs:250`; calls at
`:272,285,286,302,305,321,333,334,349,352`) cannot propagate a failure and do not
use `?`. Out-of-range indexing is a Rust panic.

The Linux twins (`src/os/linux/link/mod.rs:608-625`) use `.get()`/`.get_mut()` with
`ok_or_else`, return `Result<u32, String>` / `Result<(), String>`, and every call
site `?`-propagates through `patch_relocations` (`src/os/linux/link/mod.rs:165`).

**Why this is not a security issue.** Both implementations index `text`, which is
`image.text.clone()` with import stubs appended (macOS `mod.rs:133-144`, Linux
`mod.rs:111-124`) — appending only grows it. `relocation.offset` originates as
`EncodedRelocation.offset` (`src/arch/aarch64/encode/mod.rs:74-80`), set in the
emitter as `let offset = self.text.len();` *immediately before* emitting the word it
annotates (`src/arch/aarch64/encode/emitter.rs:1096-1099,1107-1108,1142-1147`, and
the riscv64/x86_64 equivalents). So the invariant holds structurally, and nothing
upstream needs to — or does — re-validate it.

`EncodedImage` has exactly one producer: `arch::{aarch64,riscv64,x86_64}::encode::encode()`,
called from `src/target/macos_aarch64/mod.rs:296`,
`src/target/linux_aarch64/mod.rs:319`, `src/target/linux_riscv64/mod.rs:322`,
`src/target/linux_x86_64/mod.rs:335`. There is **no deserialization into
`EncodedImage`** anywhere in `src/`. The `.mfp` container carries no relocations at
all (`src/target/package_mfp/mod.rs` contains zero occurrences of `relocation`,
`EncodedImage`, or any text-offset concept) — a downloaded package contributes IR,
which is re-lowered and re-encoded locally in-process. An attacker-supplied package
therefore cannot plant an out-of-range offset without first inducing a codegen bug.

Hence LOW: identical input, worse failure mode on one platform, internal-only reach.

## Goal

- An out-of-range relocation offset returns `Err` on macOS, with a message
  matching the Linux wording modulo the platform prefix.
- No `patch_relocations` path on either platform can panic on relocation data.

### Non-goals (must NOT change)

- The relocation encoding, offset derivation, or the `EncodedRelocation` shape.
- Linux behavior — it is already correct; the macOS side moves to meet it.
- Emitted binaries: this is an error-path-only change and must be
  **byte-identical** on every target. Any artifact churn means the fix is wrong.
- Do NOT close this by silently folding the two helpers together as part of the
  `bug-335` item-A4 cleanup — that bug explicitly forbids it, because a panic fix
  must not land as a refactor changelog entry. Fix it here, with a test.

## Blast Radius

Search: all range-slice and unwrap-family sites in both linkers.

- `src/os/macos/link/mod.rs:616` (`read_u32`) — fixed by this bug.
- `src/os/macos/link/mod.rs:620` (`write_u32`) — fixed by this bug.
- The ten macOS `patch_relocations` call sites
  (`:272,285,286,302,305,321,333,334,349,352`) — must gain `?`; mechanical.
- `src/os/macos/link/mod.rs:435` — `.expect("initializer text symbol validated
  before encoding")`, guarded by a prior validation as its message states.
  Unaffected.
- `src/os/linux/link/mod.rs:615` — `.expect("slice length")` on an
  already-bounds-checked slice; infallible. Unaffected.
- The macOS linker has **no other** range-slice, `unwrap()`, `panic!`, or bare-index
  site; the Linux side has none. Count: 2 hazardous sites, both macOS, both fixed
  here.

## Fix Design

Port the Linux shape verbatim: make the macOS `read_u32`/`write_u32` return
`Result<u32, String>`/`Result<(), String>` using `.get()`/`.get_mut()` +
`ok_or_else`, with the message `"macos linker: relocation offset {offset} + 4
exceeds text length {len}"`, and add `?` at the ten call sites. `patch_relocations`
already returns a `Result` on both platforms, so no signature churn propagates
outward.

Rejected alternatives:

- **Hoist one shared helper now.** That is `bug-335` item A4's job, and doing it
  here would make the panic fix invisible in the diff — the outcome bug-335
  explicitly warns against. Fix in place; let the cleanup dedupe afterwards, with
  the regression test already guarding the behavior.
- **Assert the invariant at the encoder instead.** The invariant genuinely holds
  there; asserting it does not help the case this bug is about, which is precisely
  when a codegen bug has broken it.
- **Leave it — it is unreachable.** It is unreachable *today*, from *untrusted*
  input. It is reachable from the class of bug the linker exists to catch, and the
  asymmetry means macOS developers get a stack trace where Linux developers get a
  sentence.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a linker test constructing an `EncodedImage` with a relocation offset
      past `text.len()`, asserting `Err` on both platforms. Confirm it **panics**
      on macOS today and passes on Linux.
- [ ] Confirm the blast-radius list above is complete (2 hazardous sites, both
      macOS); record the verdict per site here.

Acceptance: the test panics on macOS for the documented reason and passes on Linux;
the audit list is complete.
Commit: —

### Phase 2 — the fix

- [ ] Convert the macOS `read_u32`/`write_u32` to the checked, `Result`-returning
      Linux shape; add `?` at all ten call sites.

Acceptance: the Phase 1 test passes on both platforms; no other behavior moves.
Commit: —

### Phase 3 — validation

- [ ] `scripts/artifact-gate.sh` — confirm **zero** artifact churn on every target
      (error-path-only change).
- [ ] Full acceptance suite on macOS and Linux.
- [ ] Cross-link this bug from `bugs/bug-335-linker-binary-repr-cleanup.md:60-90`
      so the later dedupe knows the behavior is now test-guarded.

Acceptance: full suite green; zero golden/artifact deltas.
Commit: —

## Validation Plan

- Regression test(s): the out-of-range-relocation linker test, asserting `Err` on
  both platforms — the test bug-335 already specified.
- Runtime proof: a normal macOS build produces a byte-identical binary (the happy
  path is untouched), and the crafted image yields a message instead of a panic.
- Doc sync: none expected — no spec documents the panic; `bug-335` gains a
  cross-link.
- Full suite: `scripts/artifact-gate.sh` then `tests/test-accept.sh` on macOS and
  Linux.

## Open Decisions

- Fix in place now vs. fold into the `bug-335` A4 helper hoist. Recommend **fix in
  place**, per bug-335's own explicit instruction; the hoist then inherits a tested
  contract.

## Summary

A bug-225 hardening landed on the Linux linker and never reached its macOS twin, so
identical malformed relocation data panics on one platform and diagnoses on the
other. Severity is LOW because the offsets are in-bounds by construction and
`EncodedImage` is never deserialized — `.mfp` packages carry no relocations, so
untrusted input cannot reach it. The fix is a mechanical port of the Linux shape;
the real value is the regression test, which bug-335 already asked for and which
must land before that cleanup collapses the two helpers together.
