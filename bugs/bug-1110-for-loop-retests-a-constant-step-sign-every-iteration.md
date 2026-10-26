# bug-1110: every `FOR` loop re-tests the sign of a constant `STEP` on each iteration, about half its loop cost

Last updated: 2026-10-24
Effort: medium
Severity: MED
Class: Other (Performance)

Status: Open
Regression Test: none yet; planned `tests/codegen/codegen_for_constant_step.rs` (Phase 1)

Found while measuring whether a narrower list stride is faster (`/tmp/list-stride-probe`,
see References). A sequential sum over a 128 MiB `List OF Integer` ran at the same speed
as over a 16 MiB `List OF Byte`. The loop was instruction-bound, not memory-bound.

A counted `FOR` loop tests its bound every iteration with a direction-agnostic
condition, `(step >= 0 AND i <= end) OR (step < 0 AND i >= end)`. When `STEP` is a
compile-time constant (including the default `1`), its sign is known at compile time:
a constant `STEP` of zero is already a compile error (`TYPE_FOR_STEP_ZERO`). The
optimizer still never removes the dead half of the test, at any level, on any target.
In `FOR i = 0 TO len(xs) - 1 : acc = acc + collections::get(xs, i) : NEXT` the
macos-aarch64 `-O2` loop executes 27 instructions per iteration, counted by hand from
the `objdump -d` listing of `_mfb_fn_sum1` below. 16 of them evaluate the step-sign
half of the test. The useful work (address, load, checked add, checked increment,
compare, branch) is about 11.

The same loop written as `WHILE i < n` ran in half the time per element (0.77 vs 1.53 ns
on the probe's quick run, M2 Max).

**The single correct behavior a fix produces:** a `FOR` loop whose `STEP` is a
compile-time constant tests only `i <= end` (positive step) or `i >= end` (negative
step) each iteration. The emitted loop has no `Boolean` `mov_imm` and no compare of the
step against zero. Loop semantics are unchanged.

References:

- `mfb spec language control-flow` (`src/docs/spec/language/10_control-flow.md`, the
  counted-loop paragraph): the bound test, the increment, `TYPE_FOR_STEP_ZERO`.
- Found in the list-stride probe, `/tmp/list-stride-probe/README.md` (one-off probe, not
  committed). Its `seq` vs `seqw` rows are the runtime measure.
- Sibling performance bug found in the same probe: bug-1111 (Byte sort).

## Failing Reproduction

```
mkdir -p /tmp/bug-1110/src && cd /tmp/bug-1110
cat > project.json <<'EOF'
{ "name": "bug1110", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
EOF
cat > src/main.mfb <<'EOF'
IMPORT io
IMPORT collections

FUNC sum1(xs AS List OF Integer) AS Integer
  MUT acc AS Integer = 0
  FOR i = 0 TO len(xs) - 1
    acc = acc + collections::get(xs, i)
  NEXT
  RETURN acc
END FUNC

FUNC main(args AS List OF String) AS Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO 999
    xs = collections::append(xs, i)
  NEXT
  io::print(toString(sum1(xs)))
  RETURN 0
END FUNC
EOF
mfb build -O 2 --ncode .
python3 -I -c 'import json;f=[f for f in json.load(open("bug1110.ncode"))["functions"] if f["symbol"]=="_mfb_fn_sum1"][0];print(sum(1 for i in f["instructions"] if i.get("op")=="mov_imm" and i.get("type")=="Boolean"))'
```

- Observed: `9` (macos-aarch64 `-O2`). The loop header is:

  ```
  mov x11,#1 ; mov x13,#0 ; cmp x11,x13      <- step >= 0, two constants
  mov x11,#1 ; cmp x11,#0 ; b.ne ...         <- Boolean `true`, branched on
  cmp x10,x12 ; b.le ...                     <- i <= end (the only live test)
  mov x13,#0/#1 ; mov x11,x13 ; cmp x11,#0 ; b.eq ... ; mov x11,#1 ; b ...
  ...                                        <- step < 0 half, also constant
  cmp x11,#0 ; b.eq for_end                  <- the OR's result
  ```

- Expected: `0`. The header is `cmp i, end ; b.gt for_end`.

Observed matrix (Boolean `mov_imm` count in `_mfb_fn_sum1`, release `mfb` at `9a696b152`):

| Target | -O1 | -O2 | -O3 |
| --- | --- | --- | --- |
| macos-aarch64 | 11 ✗ | 9 ✗ | 9 ✗ |
| linux-x86_64 | 11 ✗ | 9 ✗ | 9 ✗ |
| windows-x86_64 | 11 ✗ | 9 ✗ | 9 ✗ |
| linux-riscv64 | 11 ✗ | 11 ✗ | 11 ✗ |

The final binary matches the `.ncode` plan (the same sequence in `objdump -d` of
`_mfb_fn_sum1`, built with `--debug`), so the plan is a valid test surface.

Contrast: `WHILE i < n ... i = i + 1 ... END WHILE` lowers its condition directly and
has no step test.

## Root Cause

Two halves; either alone would fix the symptom.

1. **The test is synthesized after the NIR optimizer has run.**
   `src/codegen/engine/control/builder_control/loops.rs:lower_numeric_for` builds the
   whole `Or(And(step >= 0, iter <= end), And(step < 0, iter >= end))` as a fresh
   `NirValue` inside native lowering and lowers it through `lower_loop_condition`. Opt1
   (`src/optimizer/opt1`) only sees the IR `for` op, whose step is the `$for_step` local
   (`src/ir/lower/statement.rs:lower_statement_inner`). It never sees this condition, so
   NIR constant folding cannot drop the dead arm. The builder does not consult the
   step's constness either, even though it is a `NirValue::Const` (or a local with a
   known constant) for every literal `STEP` and for the default.
2. **Opt2 cannot fold a branch on a Boolean immediate.**
   `src/optimizer/opt2/constant_folding.rs:fold_one`, `CodeOp::MovImm` arm: "Only
   Integer immediates enter the domain". A `mov_imm type=Boolean value=true` returns
   `Step::KillDst`, so the `cmp_imm x,0 ; b.ne` that follows has an unknown input. SCCP
   and branch folding (`opt2/sccp.rs`, `opt2/branches.rs`, whose literal parser
   `branches::bits` reads only numeric text) therefore never decide it. The lowering
   already folded `step >= 0` to the Boolean constant `true`, but that constant is
   invisible to every pass after it.

## Goal

- At `-O1` and above, on all four targets, a constant-`STEP` `FOR` loop emits no
  `Boolean` `mov_imm`, no step-versus-zero compare, and exactly one bound compare per
  iteration in its header (the new test counts them in the `.ncode` plan).
- The probe's `seq` row (FOR) runs within noise of its `seqw` row (WHILE).

### Non-goals (must NOT change)

- `FOR` semantics: each operand is evaluated once; the bound is inclusive; the
  increment is checked and its overflow is attributed to the `FOR` location;
  `CONTINUE FOR` takes the increment path. The max-Integer-to-max-Integer case still
  runs its body once and then raises `ErrOverflow` on the increment.
- A non-constant `STEP` keeps the direction-agnostic test: its sign is only known at
  run time. A fix that assumes a positive step for a variable step is a miscompile.
- `-O0` may keep the generic test. `-O0` is the control in the codegen tests.
- `Float`/`Fixed`/`Byte` loops: the promotion and the checked conversion of `end`/`step`
  into the loop's type (bug-816) must still happen exactly where they do today.
- No test may be weakened to make the count pass (e.g. by testing only `-O3`).

## Blast Radius

To audit in Phase 1 (searches to run: `grep -rn "BinaryOp::Or\|BinaryOp::And" src/codegen`
for other conditions synthesized inside lowering; the Boolean `mov_imm` count over every
function of the benchmark suite's `.ncode`):

- `loops.rs:lower_numeric_for`: fixed by this bug.
- Other lowering-synthesized Boolean conditions (`for_each`, the collection helpers'
  loops, the `bool_and`/`bool_or` short-circuit lowering in
  `src/codegen/engine/operators/builder_numeric/operator_lowering.rs`): the same
  "Boolean constant invisible to opt2" hazard. Verdict per site to be recorded in the
  audit.
- Source-level constant conditions (`WHILE TRUE`, `IF x AND TRUE`): opt1 folds what it
  sees in NIR; whatever reaches opt2 as a Boolean immediate is affected by root cause 2.
  Verdict to be recorded.
- Every `FOR` loop in every package (`src/codegen/builtins/**/*.mfb`-style bodies,
  `packages/`) and the benchmark suite: improved by the fix, no semantic change.

## Fix Design

Do both halves, because they cover different cases:

1. In `lower_numeric_for`, when `step` is a constant (a `NirValue::Const`, or a local
   whose `LocalValue::constant` is known), build only the live arm: `iter <= end` for a
   positive step and `iter >= end` for a negative one. The comparison still goes through
   the same promoted `BinaryOp` lowering, so bug-816's checked conversion is untouched.
2. In `opt2/constant_folding.rs:fold_one`, admit `Boolean` immediates as the integers
   0/1, so a branch on a Boolean constant folds anywhere. This is the general fix for
   the blast-radius sites. The risk is in the passes that consume the lattice: confirm
   that none of them treats a value as Integer-typed beyond its bits.

Rejected: moving the condition synthesis into IR lowering so that opt1 folds it. It
changes the `.ir` golden surface of every `FOR` loop for no runtime gain over (1).

Expected generated-output shift: every `.ncode`/`.ncodesum` golden that contains a
`FOR` loop shrinks. That is the intended change, and it is broad. Diff one fixture by
hand first and confirm that only the loop header changed.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/codegen/codegen_for_constant_step.rs`, following
      `tests/codegen/codegen_place_forward.rs` (`common::build_ncode_opt`). For
      `STEP` 1, 2 and -1 at `-O1`: the loop function has zero `Boolean` `mov_imm`.
      At `-O0` (the control) it has some. A variable-`STEP` case keeps both arms at
      every level. Confirm the test fails today.
- [ ] Complete the blast-radius audit above with a verdict per site.

Acceptance: the new test fails for the documented reason; the audit list has a verdict
per site.
Commit: —

### Phase 2 — the fix

- [ ] `loops.rs:lower_numeric_for`: emit only the live arm for a constant step.
- [ ] `opt2/constant_folding.rs:fold_one`: Boolean immediates enter the domain.
- [ ] Apply the opt2 change's effect to any in-scope audit site.

Acceptance: the Phase 1 test passes; `tests/rt-behavior` loop fixtures (including the
max-Integer increment case) still pass; nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate the shifted `.ncode`/`.ncodesum` goldens. Diff and confirm the delta is
      only loop headers and folded Boolean branches. Per AGENTS.md, run
      `scripts/artifact-gate.sh <mfb> all` and a full `scripts/test-accept.sh` before
      re-baselining.
- [ ] Landing tier plus the codegen, optimizer and loop targeted gates.
- [ ] Re-run the probe's `seq`/`seqw` rows and the benchmark suite's list group;
      record the before/after numbers here.

Acceptance: landing tier and targeted gates green; golden deltas are only the intended
change; `seq` within noise of `seqw`.
Commit: —

## Validation Plan

- Regression test: `tests/codegen/codegen_for_constant_step.rs`.
- Runtime proof: the probe's `seq` vs `seqw` rows, and `benchmark/language` before/after.
- Doc sync: none expected. The spec describes the loop's semantics, not its test
  sequence; recheck the `lower_numeric_for` citation in `10_control-flow.md`.
- Landing gate: the landing tier plus the named test, `tests/codegen`, the opt2 unit
  tests, and the scoped artifact gate.
- Full tier: required before landing (broad golden shift), per `.ai/testing-gates.md`.

## Open Decisions

- Land both halves together, or (1) first? Recommended: together. (1) alone leaves the
  Boolean-constant hazard for other sites; (2) alone may still leave the dead `cmp`
  that the lowering emits for `step >= 0` with two constants.

## Summary

The fix is small; the risk is in the opt2 lattice change (half 2) and in the
broad, intended `.ncode` golden shift. Loop semantics, error attribution and the
variable-step path stay untouched.
