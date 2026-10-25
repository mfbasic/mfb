# plan-171-D: Conditional set and select (`cset`/`csel`, x86 `setcc`/`cmov`, rv64 `slt`)

Last updated: 2026-10-04
Effort: large (3h–1d)
Depends on: plan-171-C (rv64 `set_cc` uses `xori`, which C adds as `eor_imm`)

An integer comparison used as a value is a branch diamond today:
`lower_numeric_comparison_binary` (`src/codegen/engine/operators/builder_numeric.rs`)
spills both operands, then emits `cmp; b.<cc> cmp_true; mov_imm d, 0; b done;
cmp_true: mov_imm d, 1; done:`. `math::min`/`max` on Integer
(`lower_math_min_max`, `src/codegen/builtins/math/gen_math.rs`) is
`cmp; b.le|ge take_left; mov; b; mov`, `math::abs` (`lower_math_abs`) branches
on the sign, `math::clamp` (`lower_math_clamp`) has three branches. Neither IR
can express anything else: there is no select or conditional-set op
(`grep -n "Csel\|Cmov" src/arch/ops.rs` → empty, recorded in
`planning/optimizations.md` rows "If-conversion" and "Select/cmov formation";
the only `cset` is a raw `csinc` word inside the `add_carry`/`sub_borrow`
expansions, `src/arch/aarch64/encode/emitter.rs`).

This sub-plan adds conditional set and conditional select as first-class ops at
every layer — `CodeOp`s, flagless fused MIR ops, selection on three backends,
encoders, decoder, the flags analyses and folders — and converts the producers
above. Outcome: an integer comparison value, `min`, `max`, `abs` and `clamp`
compile without branches on AArch64 and x86-64, and to `slt`-family sequences on
riscv64; every output is unchanged.

References: plan-171-A's list, plus
- `src/codegen/engine/mir/mir.rs:fused_variant`, `fused_setter_codeop`, `FUSED_COND_FIELD`, `FUSED_SHARE_FIELD`, `lower_to_mir_owned`.
- `src/arch/aarch64/select.rs:select_aarch64`; `src/arch/x86_64/select.rs:select_x86`, `x86_float_branch`; `src/arch/riscv64/select.rs:expand_fused`, `int_branch`, `snapshot_lhs`.
- `src/optimizer/opt2/branches.rs` (known-compare fact), `flags.rs`, `knownbits.rs`, `sink.rs`/`licm.rs`/`pre.rs` (local `is_flag_setter`).
- `planning/optimizations.md` rows "If-conversion (predication)", "Select/cmov formation".

## Prerequisites

See plan-171-A §Prerequisites. In addition: plan-171-C complete
(`ls planning/plan-171-C-* 2>/dev/null` → no match).

## 1. Goal

- `CodeOp`s `cset` (`dst`, `cc`) and `csel` (`dst`, `lhs` = value if true,
  `rhs` = value if false, `cc`), `cc` ∈ {eq, ne, lt, ge, gt, le, lo, hs, hi, ls}.
- Fused MIR ops `set_cc`, `set_cc_imm`, `sel_cc`, `sel_cc_imm`: a compare plus
  its set/select reader, flagless (the MIR contract,
  `mfb spec architecture mir-instruction-set` "fused ops"). A `cset`/`csel` that
  is not directly preceded by its `cmp`/`cmp_imm` is a lowering bug
  (`lower_to_mir` asserts); builders get both from one constructor.
- Selection: AArch64 `cmp; cset|csel`; x86-64 `cmp; setcc r8; movzx` and
  `cmp; cmovcc` (with a `mov` for the false value when needed); riscv64 `slt`/`sltu`
  (+`xori`, `snez`) for set and a branch-free mask sequence for select (§4.3).
- Producers converted: integer comparison values, Integer/Fixed/Money
  `min`/`max`/`abs`/`clamp`, and the 0/1 diamonds the sweep finds.

### Non-goals

- Float comparisons stay branchy (Open Decision 1).
- Short-circuit `AND`/`OR` keep their branches (their right operand may trap or
  have effects; evaluating it unconditionally changes behavior).
- `abs`/`clamp` keep their existing trap checks (`INT64_MIN`, bounds) in the
  same order; only the value selection loses its branch.

## 2. Current State

- Boolean results from comparisons: diamond (above). Measured diamond count over
  the corpus: UNMEASURED (census pattern "0/1 branch diamond", plan-171-A §4.1;
  Phase 1).
- MIR fusion: `lower_to_mir_owned` pairs a flag-setter with following flag-reading
  *branches* (`is_flag_reading_branch`); a second reader carries `share`.
  Selection splits at `cond` and re-emits setter + branch
  (`select_aarch64`). riscv64 has no flags: fused compares become `rv.br`
  (`expand_fused`); an *unfused* compare is emulated by snapshotting `lhs` to `gp`
  and `rhs` to a per-thread word (`.ai/arch-abi.md` "riscv64 flag-emulation
  reserved slots") — the path this plan must keep `cset`/`csel` off.
- riscv64 already has `rv.slt`/`rv.sltu` ops (`CodeOp::Slt`, `Sltu`).
- x86: `setcc` exists only inside `add_carry`/`sub_borrow` (`enc_setcc_to`); no
  `cmovcc` (0 hits).
- Opt2 passes that reason about flags: `flags.rs` (availability), `branches.rs`
  (known compare → folds a branch), `plans/mark.rs:flag_preserving`, local
  `is_flag_setter` in `sink.rs`, `licm.rs`, `pre.rs`. Which of them could
  separate a `cmp` from a non-terminator reader is UNVERIFIED (today every
  reader is a terminator, which pins its `cmp` in place); Phase 2 audits each.

### Verified properties

- *x86 `mov`, `setcc`, `movzx`, `cmovcc` do not write flags* — architectural; so
  `mov dst, f; cmp; cmovcc` and `cmp; setcc; movzx` are flag-safe in any order the
  expansion needs.
- *A fused op keeps its compare and reader adjacent through allocation* — the
  fused op is one MIR instruction until selection; after selection the two
  CodeInstructions are adjacent and spill code inserted between them
  (`ldr`/`str`/`mov`) preserves flags on AArch64 and x86 — the same property
  every `cmp; b.cc` pair already relies on.

## 3. Design Overview

The design mirrors the existing flagless compare-and-branch: the MIR never holds
a flag dependency; each backend expands the fused op into its own idiom. That
keeps riscv64 correct without touching its flag emulation, and keeps every MIR
optimizer row flag-unaware.

**Correctness risk:** (1) an opt2 pass that moves or deletes the `cmp` feeding a
`csel` after selection (today impossible because readers are terminators);
(2) x86 `cmov` operand aliasing; (3) rv64 mask arithmetic. All three get direct
unit tests; (1) gets an audit task per pass. **Design uncertainty:** riscv64
select profitability (Open Decision 2).

**Gate class:** behavior-changing. Expected diffs: every golden on every target
whose program compares integers into a value or calls `min`/`max`/`abs`/`clamp`
(the runtime's own helpers do, so effectively all).

**Rejected alternatives.**
- *Standalone flag-reading `cset`/`csel` in the MIR* — breaks the flagless MIR
  contract and forces riscv64 through flag emulation (a memory round trip per
  select).
- *Optimizer-only select by mask arithmetic on every target*
  (`planning/optimizations.md` If-conversion note) — 5 ALU ops where AArch64 and
  x86 need 1–2.
- *Branch diamond for riscv64 selects inside `select_riscv64`* — selection
  cannot mint labels; the mask sequence is label-free (§4.3).

## 4. Detailed Design

### 4.1 Builder surface

`abi::compare_and_set(dst, lhs, rhs_reg_or_imm, cc) -> [CodeInstruction; 2]` and
`abi::compare_and_select(dst, lhs, rhs, if_true, if_false, cc) -> [_; 2]`. The
pair is emitted together; `lower_to_mir` fuses it. Field layout of the fused op:
setter fields, `cond` = `"cset"`/`"csel"`, reader fields (`dst`, `cc`, and for
select `lhs`/`rhs`).

### 4.2 Selection and encoding

| | AArch64 | x86-64 | riscv64 |
|---|---|---|---|
| set | `cmp; cset d, cc` (`csinc d, xzr, xzr, !cc`) | `cmp; setcc d8; movzx d32, d8` | §4.3 |
| select | `cmp; csel d, t, f, cc` | `d==f`: `cmp; cmovcc d, t`; `d==t`: `cmp; cmov!cc d, f`; else `mov d, f; cmp; cmovcc d, t` (the encoder does the aliasing cases on physical registers) | §4.3 |

x86 condition codes map from the AArch64 names exactly as the existing jcc mapping
does (`lt`→`l`, `lo`→`b`, `hs`→`ae`, `hi`→`a`, `ls`→`be`, …).

### 4.3 riscv64

Set (`d` = 0/1): `lt`: `slt d,a,b`; `gt`: `slt d,b,a`; `ge`/`le`: the same then
`xori d,d,1`; `lo`/`hi`/`hs`/`ls`: `sltu` likewise; `eq`: `xor d,a,b; sltu d,zero,d;
xori d,d,1`; `ne`: `xor d,a,b; sltu d,zero,d` (no `sltiu` op is needed). An
immediate rhs uses `zero` for 0 and lowering scratch `t1` (never `t0` —
bug-749) otherwise. Select: `set c; sub m, zero, c; xor x, t, f; and x, x, m;
xor d, f, x` with `c`, `m`, `x` in lowering scratch `t1`/`t2` (both never touched
by spill code, `.ai/arch-abi.md`); the expansion is one selected unit with no
vreg-naming instruction between its scratch writes and reads.

### 4.4 Optimizer knowledge

- `flags.rs`/`flagslive` (B): `cset`/`csel` are readers; `flag_preserving`: yes.
- `branches.rs`: with a known compare fact, `cset` folds to `mov_imm 0|1` and
  `csel` to `mov d, t|f` (same proof as the branch fold).
- `knownbits.rs`: `cset` result has bits 63..1 known zero.
- `removable_op`: `cset`/`csel` are pure but **not** removable independently of
  their `cmp` (DCE removes the pair or nothing; a dead `csel` leaves a dead `cmp`
  that DCE already never removes — accept the dead compare rather than teach DCE
  pair removal; record the census count of dead compares in Corrections).

## Phases

> **NOTE — keep the checkboxes current** (same rules as plan-171-A).

### Phase 1 — measure

- [ ] Census on the corpus (three targets): 0/1 diamonds; `min`/`max`/`abs`/`clamp`
      call sites (`rg -c 'math::(min|max|abs|clamp)\(' examples benchmark tests --glob '*.mfb'`);
      record in §2.

Acceptance: §2 has no UNMEASURED.
  Check: `grep -c UNMEASURED planning/plan-171-D-*.md` → 0 (est. 15 min).
Commit: —

### Phase 2 — ops, fusion, selection, encoders (no producer; byte-identical)

- [ ] `ops.rs` (`Cset`, `Csel` + `cc` field), `mir.rs` (four fused ops in the
      fused group; `fused_variant` learns `cset`/`csel` readers; an unfused reader
      panics in `lower_to_mir` with a message naming the builder contract).
- [ ] `select_aarch64`, `select_x86`, `select_riscv64` expansions (§4.2, §4.3).
- [ ] AArch64 encoder (`csel`, `csinc`-as-`cset`) + decoder table rows (`cset`
      alias printing); x86 encoder (`setcc`/`movzx`, `cmovcc` with aliasing cases).
- [ ] Optimizer knowledge (§4.4); audit `sink.rs`, `licm.rs`, `pre.rs`, `gvn.rs`,
      `lvn.rs`, `tailduped.rs`, `dse.rs`, `stldfwd.rs`, `promote.rs` and the
      allocator's redundant-reload removal (`linear_scan.rs`, which treats
      `cmp`/`cmp_imm` as pure) for any transform that can separate or delete a
      `cmp` that feeds a non-terminator reader; fix each, or record why it cannot.
- [ ] Tests: per-backend selection unit tests (every `cc`, register and
      immediate rhs, all three x86 aliasing cases, rv64 with `a == b` registers);
      AArch64 decoder round trip; `branches.rs` fold test; MIR fuse/round-trip test.

Acceptance: round trips and unit tests pass; no emitted byte changes.
  Check: `cargo test --bin mfb mir:: select opt2:: aarch64::decode x86_64::encode` → pass (est. 5 min);
  `bash scripts/artifact-gate.sh target/release/mfb math` → `0 diff(s)` (est. 1 min).
Commit: —

### Phase 3 — producers: comparisons and math

- [ ] `lower_numeric_comparison_binary`: integer-class operands (Integer, Byte,
      Boolean, Fixed, Money, enum ordinals) end in `compare_and_set` instead of the
      diamond; Float and String paths unchanged.
- [ ] `gen_math.rs`: `lower_math_min_max`, `lower_math_abs` (after the
      `INT64_MIN` trap check), `lower_math_clamp` (after its bounds check) use
      `compare_and_select`.
- [ ] Fixtures: `tests/rt-behavior/operators/integer-compare-values` (each
      operator × Integer/Byte/Fixed/Money/Boolean × equal/less/greater, plus
      `i64::MIN`/`MAX` edges, unsigned-style `Byte` order) and
      `tests/rt-behavior/math/min-max-abs-clamp-select` (incl. `abs(INT64_MIN)`
      still raising — an `rt-error` twin `tests/rt-error/math/abs-int64-min-select`).
- [ ] `rt_codegen_forms`: `csel`/`cset` in aarch64, `cmov`/`setcc` names in the x86
      dump, no `b.` inside `math::min` on either; rv64 `slt` present.
- [ ] `rt_regalloc_levels_agree`: add both fixtures.
- [ ] Golden re-baseline (all targets).
- [ ] Runtime: macOS acceptance; 2223, 2227, 2229 `linux-runtime-proof.sh`; box 206 fixtures.
- [ ] Benchmark (macOS host): `run.sh 10`; compare `math`, `float`/`record`,
      `list (Fixed)` sort rows with `rank.py`.

Acceptance: those builtins and comparison values contain no branch on AArch64/x86;
all outputs and the `abs` trap unchanged on five targets.
  Check: `cargo test --release --test rt_codegen_forms select` → pass (est. 3 min); runtime proofs → 0 mismatches (est. 25–60 min each).
Commit: —

### Phase 4 — code-review sweep (this form)

- [ ] Census diff; list every remaining 0/1 diamond and two-move diamond by
      emitting function; read the 20 largest producers in `src/codegen/builtins/**`,
      `src/codegen/runtime/**`, `src/codegen/string/**`, `src/codegen/collection/**`
      and convert where the arms are pure values already computed (no trap, no
      memory access, no call between compare and join). "Makes sense" = both arm
      values are available before the compare without extra work; an arm that
      computes something expensive or trapping stays a branch (E's row does the
      same test automatically for user code).
- [ ] Golden re-baseline for the sweep's diffs.

Acceptance: every remaining diamond in the top-20 list has a recorded reason.
  Check: census `--diff` pasted in Corrections (est. 20 min).
Commit: —

### Phase 5 — spec and docs

- [ ] `mir-instruction-set` (four fused ops, `cond` values `cset`/`csel`),
      `aarch64-instruction-set`, `x86_64-instruction-set`, `riscv64-instruction-set`
      (the set and mask-select expansions, scratch choice).
- [ ] `planning/optimizations.md`: update "If-conversion" and "Select/cmov
      formation" rows (primitive landed; formation is sub-plan E).
- [ ] `.ai/codegen-invariants.md`: "`cset`/`csel` exist only fused with their
      compare; a pass that moves a `cmp` must check for a non-terminator reader".

Acceptance: specs render cleanly. Check: `cargo test --bin mfb spec` → pass (est. 2 min).
Commit: —

## Validation Plan

- Tests: selection/encoder units, MIR fuse tests, opt2 units, fixtures, levels-agree.
- Coverage: `tests/byte-identity/math` calls `min`/`max`/`abs`/`clamp`
  (`grep -n 'math::\(min\|max\|abs\|clamp\)' tests/byte-identity/math/src/main.mfb`);
  add calls there if any is missing (its `.ast`/`.ir` goldens move with it).
- Runtime proof: Phase 3's five targets.
- Doc sync: Phase 5.
- Final gate (once): full `cargo test --no-fail-fast` to a log; `EXIT=0`, 0 failures (est. 60–90 min).

## Open Decisions

1. Float comparison values — **recommend deferring to a follow-up plan**: x86
   needs parity-flag combination (`ucomisd` sets PF for unordered; the existing
   `x86_float_branch` shows the branch form) and AArch64 needs the `mi`/`ls`
   conditions this plan does not add to `cc`.
2. riscv64 select — **recommend the mask sequence** (§4.3; branch-free, no labels,
   7 instructions including the set) vs. leaving rv64 selects as producer-side
   diamonds (producers would branch on `active_backend()`, splitting every
   converted builder in two). Revisit if a `Zicond` target profile is added.

## Corrections

<Filled in during execution.>

## Summary

The real risk is an optimizer pass separating a compare from a non-branch reader,
which no pass has had to consider before; Phase 2 audits each one explicitly.
riscv64 gets real wins for set (`slt`) and a correct, label-free select.
