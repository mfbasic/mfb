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

## The fix is NOT a port of bug-445's pass — read this first (2026-08-31, coordinator)

This document frames the work as "the AArch64 twin that riscv64 never got".
Structurally the hook is the same (a pass over `NativeCodePlan` before `encode`,
mirroring `relax_conditional_branches` at
`src/arch/aarch64/encode/relax.rs:68`, wired in `encode/mod.rs`). **The content
is not the same, and copying bug-445's pass fixes only half the failure.**

**On AArch64 the tight instruction is the conditional branch** (imm19). **On
riscv64 conditional branches are already handled** — `emit_rv_br`
(`src/arch/riscv64/encode/emitter.rs:749`) never emits a bare B-type (imm12,
±4 KiB). Its doc comment says so:

> `rv.br` — the flagless compare-and-branch, always emitted in the 8-byte long
> form so its size is deterministic and **it reaches ±1 MiB: an inverted
> conditional branch over an unconditional `jal` to the target.**

That is why the ±4 KiB B-type range never appears in this bug: it is structurally
avoided. But note *how* it is avoided — **the long form's escape hatch is itself
a `jal`.** So `rv.br` reaches ±1 MiB *because* it emits a `jal`, and `jal` at
±1 MiB is precisely what overflows here (`emitter.rs:875`).

**Consequence: two call sites overflow at the same threshold, not one.**

1. the standalone `jal` to the shared trap stub — the failure this doc reports;
2. the `jal` **inside** every `rv.br` long form whose target is >1 MiB away —
   same instruction, same limit, different emitter path.

A fix that relaxes only (1) will make the documented reproduction pass while
leaving (2) rejecting functions of essentially the same size. The bug would read
as fixed and would not be. Any relaxation (`auipc`+`jalr`, or a near
trampoline) must be applied at **both** sites, and the fixpoint must account for
`rv.br` growing from 8 bytes to whatever the relaxed form costs — which shifts
every later displacement and can push a previously in-range `jal` out of range.

**Acceptance must therefore cover a conditional branch across the boundary**, not
only a trap-stub call. `examples/ai_chat` and `examples/browser/app` (the doc's
repros) demonstrate (1); neither is evidence for (2). Add a fixture whose
`rv.br` target sits beyond ±1 MiB, or verify (2) explicitly on the built
artifact.

## Design: relax with a CHAIN of `jal zero` hops, not `auipc`+`jalr`

Worked out 2026-08-31 (coordinator) because the obvious mechanism runs straight
into a wall this project already hit once, and the way around it is not obvious.

**The wall.** Unlike AArch64, riscv64 has no wider unconditional branch to relax
*into*: `jal`'s ±1 MiB **is** the widest single-instruction jump. Reaching
further needs `auipc rd, %pcrel_hi(t)` + `jalr zero, %pcrel_lo(t)(rd)` — and
`auipc` needs a destination **register**. There is no free one:

* `t0`–`t2` are reserved *lowering* scratch (`select.rs:38-42`) — immediate
  materialization, overflow detection, the float-compare boolean. A `jal` that
  needs relaxing can sit **inside** such an expansion (see below), so they are
  not safely dead at the rewrite site.
* `gp` (x3) is the plan-99 flag register, holding a compare's left operand across
  the compare→branch span (`select.rs:49-55`).
* bug-381 already searched and found nothing else: *"rv64 has no free one
  (`tp`/x4 faults a dynamically-linked binary via TLS, and shrinking the
  allocatable pool to free a temporary destabilizes the allocator)"*. Do not
  re-litigate that; it cost a bug to establish.

**The way around it.** `jal zero, offset` writes **no** link register, so a jump
is register-free — only its *reach* is limited. Relax by chaining hops instead of
widening the instruction:

```text
    jal zero, far            jal zero, L1      ; ≤1 MiB
                    ==>    L1:
                             jal zero, far     ; ≤1 MiB from L1
```

Each hop must land within ±1 MiB of the previous, so place trampolines at ~1 MiB
intervals and route through as many as the distance needs. A function would have
to exceed ~2 MiB before a second hop is required, and the reported failure is
only 1107060 bytes (≈1.06 MiB) past the limit — one intermediate hop covers it.
This needs **no scratch register, no ABI reasoning, and no TLS hazard**, which is
what makes it preferable to `auipc`+`jalr` here even though the latter is what a
linker would emit.

**Refinement — an ADJACENT trampoline does not work here, and this is the single
biggest departure from bug-445.** AArch64's pass splices the veneer right next to
the branch, and that works because the veneer's `b` has *wider reach* (imm26,
±128 MiB) than the branch it replaces. riscv64 has no such instruction, so a
trampoline placed beside the far `jal` sits at essentially the same distance from
the target and is equally out of range. The hops must be placed **between** the
source and the target, at ≤1 MiB intervals — which makes this a different
algorithm, not a port with different constants.

An inserted hop is an island in the instruction stream, so it must be jumped
over rather than fallen into:

```text
    jal zero, far          ==>   jal zero, Lhop      ; ≤1 MiB to the island
    ...                                ...
                                 jal zero, Lover     ; skip the island
                               Lhop:
                                 jal zero, far       ; ≤1 MiB onward
                               Lover:
                                 ...
```

The implementation therefore needs a placement step the AArch64 pass has no
equivalent of: walk the offset table for an instruction boundary about 1 MiB
along the path to the target, and splice the three-instruction island there.
Prefer a boundary that is already a branch target or a function-level seam if one
is near, so the extra `jal zero, Lover` lands somewhere cold.

Structure the outer loop like the AArch64 twin regardless: a pass over
`NativeCodePlan` before `encode`, run to a fixpoint (inserting hops shifts later
displacements and can push a previously in-range `jal` out), and a strict no-op
when everything already fits, so every existing fixture stays byte-identical.
Termination needs an argument the AArch64 pass gets for free — there, a rewritten
branch targets a trampoline two instructions away and can never be rewritten
again. Here each rewrite shortens the *remaining* distance by ~1 MiB, so bound
the hop count per branch by `ceil(distance / 1 MiB)` and assert it, or a
pathological placement could oscillate.

**Both sites still need it** — see the section above: the standalone `jal` to the
trap stub *and* the `jal` inside every `rv.br` long form. The second is the one
that makes the scratch-register route dangerous, since that `jal` is emitted
mid-expansion where `t0`–`t2` liveness is not obvious; the chained-hop route is
immune to that question entirely, because it clobbers nothing.

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
