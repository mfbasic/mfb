# bug-816: Float/Fixed comparisons bypass the documented Fixed-first promotion

Last updated: 2026-10-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness
Status: Fixed
Regression Test: `tests/rt-behavior/arithmetic/float-fixed-comparison-promotion`, `tests/rt-error/arithmetic/float-fixed-comparison-overflow-rt`, `tests/rt-behavior/trap/inline-trap-fixed-comparison-promotion-rt`, `tests/syntax/trap/trap-local-after-fixed-comparison-invalid`, `tests/rt-behavior/types/with-fallible-comparison-source-order`, `tests/rt-behavior/control-flow/for-mixed-numeric-operands`

## Goal

The first comparison should agree with explicit Fixed conversion (TRUE); comparing Fixed with Float 3000000000.0 should raise ErrOverflow if the specified arithmetic promotion rule is followed. Observed instead: FALSE, TRUE, TRUE, exit 0.

## References and duplicate audit

The numeric review previously assumed arithmetic and comparison promotion agreed. This docs pass records the current discrepancy in the man/spec pages. Prior language types prose at b3546173c says comparisons use the same promotion as arithmetic.

The contract is in `src/docs/spec/language/04_types.md` and, for Money rounding,
`src/docs/spec/stdlib/13_money.md`. The documentation update distinguishes current
implementation limitations from the intended fix. Searches covered all live and
archived reports: `rg -n -i 'comparison.*(Float|Fixed)|Float.*compar.*Fixed|promotion.*compar|i64::MIN.*divisor' bugs --glob '*.md'`.

## Failing Reproduction

```sh
mkdir -p /tmp/mfb-doc-numeric/comparison/src
cat > /tmp/mfb-doc-numeric/comparison/project.json <<'JSON'
{"name": "comparison", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "entry": "main", "targets": ["native"]}
JSON
cat > /tmp/mfb-doc-numeric/comparison/src/main.mfb <<'BASIC'
IMPORT io
IMPORT collections
FUNC main() AS Nothing
  LET f AS Fixed = 1F
  LET x AS Float = 1.00000000001
  io::print(toString(f = x))
  io::print(toString(f = toFixed(x)))
  LET huge AS Float = 3000000000.0
  io::print(toString(f < huge))
END FUNC
BASIC
cargo build --release --bin mfb
target/release/mfb build -O0 /tmp/mfb-doc-numeric/comparison
/tmp/mfb-doc-numeric/comparison/build/comparison.out
```

The first comparison should agree with explicit Fixed conversion (TRUE); comparing Fixed with Float 3000000000.0 should raise ErrOverflow if the specified arithmetic promotion rule is followed. Observed instead: FALSE, TRUE, TRUE, exit 0.

Measured on macOS arm64 with the release binary, at both `-O0` and `-O1`:
`python3 /tmp/numeric-doc-probes.py target/release/mfb` generated and ran the above
project. The script is temporary; the complete durable reproduction is above.
Other targets have not been run. These are shared lowering paths.

## Root Cause

`src/codegen/engine/operators/builder_numeric.rs:lower_numeric_comparison_binary` explicitly chooses Float before Fixed, while `src/numeric.rs:typed_binary_result_type` gives Fixed priority for arithmetic. This is a separate promotion decision, not a failure in Float-to-Fixed rounding.

Inspection: `rg -n 'lower_numeric_comparison_binary|emit_money_divide_scalar|typed_binary_result_type' src/codegen/engine/operators/builder_numeric.rs src/codegen/builtins/money/gen_money_math.rs src/numeric.rs`.

## Blast Radius

- All six numeric comparison operators reach `lower_numeric_comparison_binary`; fix and test both Float/Fixed operand orders.
- Integer/Fixed comparisons already use checked Fixed conversion; preserve their errors.
- Float/Integer comparisons use Float intentionally; preserve large-Integer precision semantics.
- Arithmetic promotion is the reference contract; do not alter it to hide the comparison discrepancy.
- Money comparison and Map/Set/list element matching are independent; preserve them.

## Fix Design

Unify comparison operand conversion with the intended Fixed-first numeric promotion contract, retaining Boolean result typing and Money-only comparison. Add tests for both operand orders, all comparison operators, equal-after-rounding values, and range errors. If Float-first comparison is preferred as a language decision, approve that explicitly and make it a permanent documented exception rather than silently changing this bug's oracle.

### Non-goals

No carrier/layout changes, no weakening tests to accept today's wrong answer,
no Float approximation replacing exact Money arithmetic, and no change to
collection key equality. This report files work; it does not implement it.

## Phases

### Phase 1 — failing tests and audit

- [ ] Add runtime success/error fixtures for the reproduction, contrasting paths, and boundary matrix.
- [ ] Confirm RED before implementation and audit shared callers.

Acceptance: tests fail for the documented wrong conversion/rounding, not formatting.
Commit: —

### Phase 2 — fix and documentation

- [ ] Implement the design and preserve sibling contracts.
- [ ] Update the current-limitation prose in the numeric man/spec topics in the same change.

Acceptance: reproduction and contrast cases produce the expected results.
Commit: —

### Phase 3 — full verification

- [ ] Run `cargo test` and `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- [ ] Before any golden edit, run the full suite and establish the required historical/contract/dependent/oracle evidence; correct only disproved lines.
- [ ] Run the reproduction on supported targets at `-O0` and `-O1`, recording results.

Acceptance: runtime proof and full gates agree; no unrelated expected-output changes.
Commit: —

## Validation Plan

The durable reproduction above is the runtime oracle; implement the expanded
boundary matrix described in Fix Design. Render changed man/spec topics and
check citations. Compile success alone is insufficient.

## Open Decisions

Whether to retain the historical Fixed-first comparison contract or deliberately specify Float-first comparison must be settled before implementing a semantic change. Recommended: restore the previously specified promotion rule.

## Resolution

Open decision settled as recommended: comparisons follow the arithmetic
promotion table (Fixed-first). RED fixtures in 4baadd56f; fix in the commit that
archives this document.

- `builder_numeric.rs:lower_numeric_comparison_binary` now takes
  `promoted_binary_type(Add, …)`, so a Float/Fixed pair compares as Fixed through
  the existing checked `load_numeric_as_fixed` (ErrOverflow out of range,
  ErrInvalidFormat for a non-finite Float). Integer/Float stays Float; Money
  stays Money/Money.
- A comparison can therefore raise, and four censuses said it could not
  (Integer/Fixed comparisons already raised, so all four were pre-existing
  holes): `ir::fallible::comparison_can_raise` (new, shared) now feeds the HIR
  escape walk (function fallibility), `lower::trap_hoist_kind` (inline TRAP
  coverage), `trap_init` (function-level handler initialization, now typed with
  globals), and codegen `nir_value_cannot_fail` (mixed `WITH` reordering).
  `binary_may_consume_float_into_exact` registers the ErrInvalidFormat message
  for comparisons.
- Counted FOR: the IR bound `$for_end`/`$for_step` as the loop type with an
  unconverted operand, so a Float bound/step of a Fixed loop, an Integer bound
  of a Fixed loop, or an Integer start of a Float loop ran on raw bits (0, 1, 3
  or ~10^9 iterations, pre-existing). The temps now keep their operand type and
  the bound/step tests and increment promote; the start is converted by
  `numeric_value_as`; test failures carry the FOR location. The HIR escape walk
  now treats every FOR as able to raise (its increment is checked), matching
  `trap_init`.

Evidence: the six fixtures fail at the parent (build.log mismatch each, via
`scripts/test-accept.sh <parent mfb>`) and pass after; e.g. `1F = 1.00000000001`
FALSE→TRUE, `1F < 3000000000.0` TRUE→ErrOverflow. Runtime fixtures pass at -O0
and -O1 on macOS arm64, 2226 (aarch64 glibc), 2229 (riscv64 musl) and 206
(Windows x86-64), output byte-compared with the macOS run. Targeted acceptance
over 205 related fixtures (arithmetic, control-flow, trap, conversions, money,
operators, types, match, `*fixed*`, `*float*`, `*compar*`, `*toString*`) passes.
An `-ir` sweep of all 1604 fixtures shows no IR or diagnostic change outside the
new fixtures. Spec/man updated: `language types` §4.1, `language error-model`,
`language control-flow`, `mfb man types comparisons`/`numeric`.

Golden moves (native code only): `byte-identity/vector` `.ncodesum` on all five
targets, measured by hashing `mfb build -ncode -target <t>` per goldened
`.ncodesum` with the parent and with this fix plus the MATCH-pattern fix that
follows it (157 files; the other 152 unchanged by these two commits); no
`.ir`/`build.log` golden moves.

Found alongside, filed as bug-820: a Float-suffixed literal (`1.5f`) is accepted
into a Fixed/Money slot and stored as its f64 bits.
