# plan-171-I: Short branch and immediate encodings (rv64 B-type, x86 rel8/imm8); plan-171 closing review

Last updated: 2026-10-21
Effort: large (3h–1d)
Depends on: plan-171-H

Two backends never use their short encodings. riscv64 emits **every**
conditional branch as the 8-byte long form `b<inverse> rs1, rs2, +8; jal zero,
target` (`src/arch/riscv64/encode/emitter.rs:emit_rv_br`) — the 4-byte B-type
(±4 KiB) is never used, so each compare-and-branch is two instructions where
one suffices for nearly every in-function branch. x86-64 emits every `jcc`/`jmp`
as rel32 (`jmp_label`; only `branch_self` is `EB FE`), every ALU immediate as
imm32 (`enc_alu_imm32`, doc: "Always uses the imm32 form … for a
value-independent size"), every `mov_imm` as the 10-byte `movabs`
(`enc_mov_imm64`), and every stack access as `disp32` (`mem_disp32`). These are
the riscv64/x86-64 counterparts of AArch64's compact compare-and-branch and
immediate forms (sub-plans B and C).

This sub-plan adds the short forms with grow-only relaxation, then closes
plan-171 with the cross-form review and the family's final validation.

Outcome: riscv64 conditional branches within ±4 KiB are one 4-byte instruction;
x86-64 branches within ±127 bytes are 2 bytes, immediates that fit in 8 bits
use the `83` forms, `mov_imm` uses the shortest exact encoding, stack accesses
within ±127 use `disp8`; all outputs unchanged.

References: plan-171-A's list, plus
- `src/arch/encode_plan.rs:InstructionEncoder::instruction_size` (sizes are
  measured by encoding into a throwaway encoder — `sizing.rs`).
- `src/arch/riscv64/encode/relax.rs:relax_rv64_branches` (bug-453 hop islands).
- `src/arch/x86_64/encode/emitter.rs:jmp_label`, `patch_labels`, `enc_alu_imm32`,
  `enc_mov_imm64`, `mem_disp32`; `src/arch/x86_64/encode/sizing.rs`.
- `.ai/arch-abi.md` "riscv64 branch relaxation — `jal` is the widest jump there is".
- plan-169-A/B/C (Win64 unwind tables, constant-size prologues): a coordination
  point, not a dependency (§3).

## Prerequisites

See plan-171-A §Prerequisites. In addition: plan-171-H complete
(`ls planning/plan-171-H-* 2>/dev/null` → no match), and the "value-independent
size" rationale recorded by plan-171-B Phase 1 (`grep -n 'value-independent' planning/completed/plan-171-B-*.md` → its recorded finding).

## 1. Goal

- riscv64: `rv.br` and B's `tbz`/`tbnz` (F's table entries are `jal` and
  unaffected) carry a `form` field `short`|`long`; the encoder emits B-type for `short`;
  relaxation starts every conditional branch `short` and grows to `long` those
  out of ±4 KiB, to a fixpoint, before the existing hop-island pass.
- x86-64: `jcc`/`jmp` carry `form` `short`|`near` with the same grow-only pass
  (new `src/arch/x86_64/encode/relax.rs`); `cmp_imm`/`add_imm`/`sub_imm`/
  `*_imm` logical use `83 /digit ib` when the value fits `i8`; `mov_imm` uses
  `mov r32, imm32` (zero-extends) for `0..=u32::MAX`, `mov r/m64, simm32`
  (`C7 /0`) for sign-extendable values, else `movabs`; `[base + disp]` uses
  `disp8` when it fits.
- `mov_imm` never becomes `xor r, r` (it would write EFLAGS; `mov_imm` is
  flag-preserving per `opt2/plans/mark.rs:flag_preserving`).

### Non-goals

- No change to x86 **prologue** byte layout beyond what the `disp8` rule implies
  for its saves — and if plan-169 (Win64 unwind tables) has landed, its
  `UNWIND_INFO` describes prologue *offsets*; Phase 3 re-runs plan-169's unwind
  check on Windows so a shorter save instruction is proven to still be described.
- No compressed riscv64 instructions (`C` extension) — a separate size project.

## 2. Current State

- riscv64: `emit_rv_br` always 8 bytes; `relax_rv64_branches` only handles the
  `jal` ±1 MiB limit (hop chains, never resizing a branch — "Relaxing never
  resizes the branch itself … the pass is a strict no-op in range").
- x86-64: fixed-size forms as above; `patch_labels` writes rel32 with no range
  check. No relaxation pass (`rg -c relax src/arch/x86_64` → 0).
- Why x86 chose value-independent sizes: recorded by plan-171-B Phase 1 (this
  plan must not start before it is known — Prerequisites).
- The size of each change (branches within ±4 KiB / ±127 B; immediates within
  i8; displacements within i8): UNMEASURED; Phase 1 measures with the census
  tool extended to compute final offsets from `-nobj`.

### Verified properties

- *Sizes are taken by encoding the instruction itself* — `sizing.rs` (aarch64,
  plan-171-A §2.3) and the shared `InstructionEncoder::instruction_size`
  contract; so a value-dependent immediate form is sizeable, and only a
  *distance*-dependent form needs a relaxation pass.
- *Grow-only relaxation terminates* — a branch only ever goes short → long, so
  the set of long branches grows monotonically and is bounded.

## 3. Design Overview

Two independent pieces per ISA: distance-dependent branch forms (need a
relaxation fixpoint) and value-dependent immediate/displacement forms (need
only the encoder and sizing). Correctness risk: the fixpoint (a branch left
short while out of range is an encode error, not a silent fault, because
`patch_labels` range-checks — add that check on x86) and x86 `mov r32, imm32`
zero-extension semantics (must only be chosen for values whose upper 32 bits
are zero). Gate class: behavior-changing; expected diffs: every `linux-riscv64`
golden (branches) and every `linux-x86_64`/`windows-x86_64` golden; no aarch64
golden.

Coordination with plan-169: plan-169-A needs prologues "at most 255 bytes";
shorter encodings only shrink them. No ordering dependency in either direction.

Rejected: shortening by peephole after encoding (needs re-patching every label
anyway — the relaxation pass is the same work, done once in the right place).

## Phases

> **NOTE — keep the checkboxes current** (same rules as plan-171-A).

### Phase 1 — measure

- [ ] Census over `-nobj`/`-ncode` (riscv64, x86): branch distances histogram,
      immediates fitting i8, `mov_imm` classes, `disp8`-eligible accesses;
      record in §2.

Acceptance: §2 has no UNMEASURED. Check: `grep -c UNMEASURED planning/plan-171-I-*.md` → 0 (est. 20 min).
Commit: —

### Phase 2 — riscv64 short branches

- [ ] `rv.br`/`tbz`/`tbnz` `form` field; `emit_rv_br` short arm (B-type imm12
      encoding); sizing; `patch_labels` range check for short.
- [ ] `relax.rs`: grow-only short→long fixpoint before the hop-island pass; the
      hop pass then sees only long forms (unchanged logic).
- [ ] Tests: in-range short, exactly ±4094/±4096 boundaries, a branch pushed out
      of range by another branch's growth, then a >1 MiB function (hop islands
      still work over long forms).
- [ ] Golden re-baseline (rv64); runtime: `linux-runtime-proof.sh` on 2229 and
      on 2232 (Debian riscv64 glibc — second libc world).

Acceptance: in-range conditional branches are 4 bytes; outputs unchanged on both riscv64 boxes.
  Check: `cargo test --bin mfb riscv64::encode` → pass (est. 3 min); both proofs → 0 mismatches (est. 60 min each — the only execution of rv64 branch encodings).
Commit: —

### Phase 3 — x86-64 short forms

- [ ] `src/arch/x86_64/encode/relax.rs` (rel8/rel32 fixpoint) wired where the
      x86 targets call `encode_plan`; `patch_labels` range check for rel8.
- [ ] `83 /digit ib` for i8 immediates (`cmp`, `add`, `sub`, `and`, `or`, `xor`).
- [ ] `mov_imm` three-way choice (§1); never `xor`.
- [ ] `disp8` for memory operands (incl. `rsp`/`r12` SIB and `rbp`/`r13` cases).
- [ ] Tests: encoder units for each boundary (−128/127, `u32::MAX`, `i32::MIN`,
      `0x8000_0000` as `mov r32` vs `C7`), relaxation units.
- [ ] Golden re-baseline (x86 targets).
- [ ] Runtime: `linux-runtime-proof.sh` on 2227 (musl) and 2228 (glibc);
      Windows: the acceptance runnable fixtures on box 2230; if plan-169 has
      landed, its unwind-table check (`mfb man debug backtraces` examples via
      `bash scripts/man-debug-examples.sh target/release/mfb windows`) on box 206.

Acceptance: short forms used where they fit; outputs unchanged on SysV and Win64.
  Check: `cargo test --bin mfb x86_64::encode` → pass (est. 3 min); runtime proofs → 0 mismatches (est. 60–120 min each, emulated — the only x86 execution).
Commit: —

### Phase 4 — plan-171 cross-form review sweep

- [ ] Census `--diff` from plan-171-A's baseline TSVs to now, per target, for
      every pattern; a table of instructions-per-program before/after for the
      examples and benchmark corpus goes into Corrections.
- [ ] For each pattern still above zero, a one-line reason per remaining class
      (e.g. "flags live", "non-encodable mask", "guarded MATCH").
- [ ] Hand-written emitters outside `finalize_frame` (B Phase 5's list) re-checked
      against all forms (pairs, logical immediates, selects, tables, G's
      addressing/multiply-add/bitfield/`csinc`/`csneg` forms, H's shared exits).
- [ ] `planning/optimizations.md`: every row plan-171 touched is current.
- [ ] Benchmark (macOS host): full `./benchmark/language/run.sh 10` vs.
      `benchmark/language/baseline/` with `rank.py`; regenerate `benchmark/language/baseline/`
      only if the user asks (it is the committed reference).

Acceptance: every remaining candidate class has a recorded reason.
  Check: Corrections contains the per-target census table (est. 30 min).
Commit: —

### Phase 5 — spec and docs

- [ ] `riscv64-instruction-set` (short/long forms, relaxation order),
      `x86_64-instruction-set` (rel8, imm8, `mov_imm`, disp8), `linker`
      pipeline topic where relaxation is listed (`mfb spec linker pipeline`).
- [ ] `.ai/arch-abi.md` riscv64 relaxation entry: two passes now (grow, then hops).

Acceptance: specs render. Check: `cargo test --bin mfb spec` → pass (est. 2 min).
Commit: —

## Validation Plan (closes plan-171)

- Tests: every letter's unit tests and `rt_codegen_forms`; levels-agree.
- Coverage: per-target `.ncodesum` goldens moved only on the predicted targets
  (classify `git status --porcelain` by target infix).
- Runtime proof, all five targets, once at the end of the family: macOS
  acceptance; `linux-runtime-proof.sh` on 2223 (aarch64), 2227/2228 (x86_64),
  2229/2232 (riscv64); Windows fixtures on 2230.
- Doc sync: every letter's Phase-docs; `mfb man optimizations`.
- Final gate (once): `cargo test --no-fail-fast > /tmp/171i.log 2>&1; echo EXIT=$?` → `EXIT=0`; `grep -c '^failures:' /tmp/171i.log` → 0; `bash scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)` (est. 90 min).

## Open Decisions

1. x86 `disp8` — **recommend including it** (every stack access shrinks by 3
   bytes) vs. deferring for plan-169 churn; it is a pure encoding change and
   plan-169 only requires prologue length bounds, which shrink.

## Corrections

<Filled in during execution.>

## Summary

Distance-dependent forms are the only new mechanism (a grow-only fixpoint per
ISA); everything else is encoder-local. Phase 4 is where plan-171 proves, by
census, that the forms are actually used everywhere they apply.
