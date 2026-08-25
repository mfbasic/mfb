# bug-453: riscv64 backend rejects large functions — `jal` to a trap stub exceeds ±1 MiB and nothing relaxes it (bug-445's un-fixed twin)

Last updated: 2026-08-25
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (valid source rejected by codegen)

Status: Open
Regression Test: — (Phase 1 adds an encoder-level test in `src/arch/riscv64/encode/`, mirroring bug-445's `relax.rs` tests)

Cross-compiling a big-but-valid project to `linux-riscv64` fails at encode
time: a `jal` from user code to a shared in-function trap stub (`trap_0`) sits
more than 1 MiB away in a single large lowered function, `jal`'s imm20 cannot
encode the displacement, and the emitter hard-errors instead of relaxing. The
identical problem on AArch64 (imm19 conditional branches) was fixed by
bug-445's trampoline relaxation pass; riscv64 never got the twin. **The single
correct behavior a fix produces: any in-range-at-source `jal`/branch whose
displacement exceeds its immediate encoding is relaxed (via `auipc`+`jalr`
sequence or a near trampoline) so the function encodes; no size-dependent
rejection of valid programs.**

References:

- `bugs/completed/bug-445-aarch64-conditional-branch-no-veneer-rejects-large-function.md` —
  the AArch64 twin and its accepted fix shape (fixpoint relaxation pass over
  the code plan before `encode`).
- Memory note `mfb-large-function-branch-range.md` (AArch64 side, now fixed by
  bug-445; this file is the riscv64 remainder).
- Found during the optimizer worktree's examples verification (all-targets
  sweep after merging main, 2026-08-24).

## Failing Reproduction

```
target/release/mfb build --target linux-riscv64 -q examples/ai_chat
target/release/mfb build --target linux-riscv64 -q examples/browser/app
```

- Observed: `error: rv64 jal displacement 1107060 to 'trap_0' exceeds ±1 MiB`
  (ai_chat; browser/app fails identically), exit 1, no artifact.
- Expected: both projects produce a `linux-riscv64` executable, as they do for
  every other target.

Contrast cases (bound the bug):

| Environment | Details | Result |
| --- | --- | --- |
| linux-riscv64 | examples/ai_chat, examples/browser/app (one huge lowered function) | fails ✗ |
| linux-riscv64 | every other example (small functions) | works ✓ |
| macos-aarch64 / linux-aarch64 | same projects — bug-445's `relax_conditional_branches` fires | works ✓ |
| linux-x86_64 / windows-x86_64 | same projects — x86 `jmp`/`jcc` rel32 reach ±2 GiB | works ✓ |

## Root Cause

`src/arch/riscv64/encode/emitter.rs:875` — the `jal` emitter computes the
label displacement and returns a hard error when it exceeds imm20's ±1 MiB.
No relaxation stage exists for riscv64: bug-445 added
`src/arch/aarch64/encode/relax.rs` (`relax_conditional_branches`, run by the
two AArch64 targets on the code plan before `encode`), but the riscv64 target
calls `encode` directly on the raw plan. RISC-V's conditional branches
(`beq`-family, imm12 ±4 KiB — currently reached through fused `RvBr`) have an
even shorter reach and today survive only because lowering keeps them near;
they share the same latent hazard. x86 is immune (rel32); AArch64 is immune
since bug-445.

## Goal

- `mfb build --target linux-riscv64` succeeds on `examples/ai_chat` and
  `examples/browser/app`, and an encoder-level test proves a synthesized
  \>±1 MiB `jal` (and a >±4 KiB `RvBr`) encodes into a semantically identical
  relaxed sequence.

### Non-goals (must NOT change)

- In-range branches must stay **byte-identical** (bug-445 asserted this on
  AArch64; do the same — the relaxation must be a no-op on every current
  golden, so all `linux-riscv64` `.ncodesum` goldens stay untouched).
- No change to trap semantics, error codes, or `Error.source` locations — the
  trap stub is still reached, just via a longer encoding.
- Do NOT "fix" this by splitting user functions or capping lowering size;
  bug-445 explicitly rejected size-based workarounds in favor of relaxation.

## Blast Radius

- `src/arch/riscv64/encode/emitter.rs:875` (`jal`) — fixed by this bug.
- riscv64 conditional branches (`RvBr` lowering → `beq`-family imm12) — same
  hazard at ±4 KiB, not yet observed to fail; IN SCOPE (relax through the same
  pass; RISC-V's standard sequence is the inverted-condition hop over a
  `jal`/`auipc+jalr`).
- `src/arch/aarch64/encode/relax.rs` — unaffected (already relaxed); reuse its
  fixpoint/label-bookkeeping shape rather than inventing a new one.
- x86 backends — unaffected (rel32 reach).

## Fix Design

Mirror bug-445: a `relax_rv64_branches` pass over the code plan before the
riscv64 `encode`, walking each function to a fixpoint:

- far `jal target` ⇒ `auipc t?, %hi(delta)` + `jalr` — or, if scratch-register
  discipline at encode time makes that awkward, a bug-445-style near
  trampoline `jal Ltramp` / `Ltramp: auipc+jalr far` placed after the
  function's terminator.
- far `RvBr cond, target` ⇒ invert condition to hop over an unconditional far
  form (the standard RISC-V assembler relaxation).

Correctness risk concentrates in label re-resolution while sizes change
(fixpoint, as in bug-445) and in picking a scratch register that is dead at
the branch (the AArch64 pass sidestepped scratch by using only `b`; riscv64's
far form needs a register — `t6`/the encoder's reserved scratch, documented in
`.ai/arch-abi.md`, is the candidate; confirm it is never live across a
branch).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Encoder-level test synthesizing a function with >1 MiB of padding between
      a `jal` and its label: assert today's hard error (pins the pre-fix
      behavior, flips to relaxed-encoding assertions in Phase 2).
- [ ] Audit `RvBr`/`beq`-family reach handling and document the scratch
      register's liveness guarantee.

Acceptance: test fails (errors) for the documented reason; audit verdicts
written above.
Commit: —

### Phase 2 — the fix

- [ ] `src/arch/riscv64/encode/relax.rs` (new): fixpoint relaxation for `jal`
      and `RvBr`, wired into `src/target/linux_riscv64` before `encode`.
- [ ] Flip the Phase 1 test to assert the relaxed sequence + byte-identity for
      in-range branches.

Acceptance: Phase 1 test passes; in-range-branch byte-identity test passes.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `artifact-gate.sh all` — expect **0 diffs** (relaxation is a no-op in
      range; any diff is a bug in the pass, not regen material).
- [ ] `cargo test --no-fail-fast`; full `test-accept.sh`.
- [ ] Rebuild `examples/ai_chat` + `examples/browser/app` for `linux-riscv64`;
      if a riscv64 runner is available (`.ai/remote_systems.md`), execute one.

Acceptance: suite green, gate 0 diffs, both examples build for riscv64.
Commit: —

## Validation Plan

- Regression tests: `src/arch/riscv64/encode/relax.rs` `#[cfg(test)]` (far
  `jal`, far `RvBr`, in-range byte-identity), mirroring bug-445's trio.
- Runtime proof: the two example builds above; remote execution if a rv64 host
  is reachable.
- Doc sync: `.ai/arch-abi.md` riscv64 section (record the relaxation + scratch
  choice); memory note `mfb-large-function-branch-range.md` gains the riscv64
  resolution when fixed.
- Full suite: `cargo test --no-fail-fast`, `scripts/artifact-gate.sh all`,
  `scripts/test-accept.sh`.

## Open Decisions

- Far-`jal` form: `auipc+jalr` inline (needs a dead scratch reg) vs.
  bug-445-style trampoline (no scratch for the hop, but the trampoline's far
  form still needs `auipc+jalr` unless within another `jal`'s reach).
  Recommended: trampoline-first (hop is a short `jal`, trampoline uses
  `auipc+jalr` with the encoder's reserved scratch), matching bug-445's
  keep-the-original-condition design.

## Summary

The engineering risk is the fixpoint size/label bookkeeping and the scratch
register's liveness argument; everything in-range must remain byte-identical,
so the gate is the proof. Trap semantics and all non-riscv64 targets are
untouched.
