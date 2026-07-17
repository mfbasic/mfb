# bug-267: aarch64 object-encoder `branch_imm26`/`branch_imm19` silently truncate an out-of-range branch → miscompile

Last updated: 2026-07-17
Effort: small (<1h)
Severity: LOW
Class: Correctness (build-time codegen)

Status: Open
Regression Test: (none yet)

The aarch64 object encoder's `branch_imm26` (`B`/`BL`, ±128 MiB) and
`branch_imm19` (conditional/compare branches, ±1 MiB) compute the branch delta and
mask it to the field width with **no reach check**. An intra-function branch whose
target exceeds the encodable range wraps silently to a wrong target — a
miscompile, not a diagnostic. This is the same silent-truncation class the linker
copies eliminated (LNK-06, now returning `Err`); the object-encoder twins were
left unfixed. It is not attacker-controlled (the compiler's own control flow), but
a very large function could produce a wrong branch with no error. The single
correct behavior a fix produces: an out-of-reach branch is a hard encoder error,
never a truncated encoding.

References:

- `planning/audit-2-linker-hardening.md` (LNK-11).
- `src/arch/aarch64/encode/sizing.rs:138-141` (`branch_imm26`, `& 0x03ff_ffff`),
  `:143-146` (`branch_imm19`, `& 0x0007_ffff`) — mask with no reach check
  (verified current).
- Fixed sibling for contrast: the linker relocation encoders reach-check and
  return `Err` (LNK-06; `src/os/linux/link/mod.rs:507,524,538`).

## Failing Reproduction

A single function whose body forces an unconditional branch span `> ±128 MiB`
(or a conditional branch `> ±1 MiB`). Observed: the encoder masks the delta and
emits a branch to a wrong address — silent miscompile. Expected: the encoder
returns an `Err` ("branch target out of range"), as the linker relocation path
already does.

Contrast: the linker's own `read_u32`/relocation writers now bounds-check and
`Err` (LNK-06 / bug-225); only these object-encoder immediate builders truncate.

## Root Cause

`branch_imm26`/`branch_imm19` (`arch/aarch64/encode/sizing.rs:138-146`) do
`((delta / 4) as i32 as u32) & MASK` and return the raw `u32`, discarding any
high bits that indicate the target was unreachable. There is no signed-range
assertion before the mask, unlike the linker copies fixed under LNK-06.

## Goal

- `branch_imm26`/`branch_imm19` return `Result` (or assert) that the signed,
  word-scaled delta fits the field (±2^27 bytes / ±2^20 bytes respectively) and
  produce an encoder error on overflow instead of a truncated encoding.

### Non-goals (must NOT change)

- The encoding of in-range branches (bit-identical output for every currently
  valid program).
- The linker-side relocation path (already fixed).

## Fix Design

Add a signed-range check in each helper before masking: verify
`delta` is a multiple of 4 and `(delta >> 2)` fits 26/19 signed bits; on failure,
propagate an `Err` up the encode path (threading `?` through the callers, as the
linker copies do). Because valid programs are unaffected, the artifact-gate diff
should be empty.
