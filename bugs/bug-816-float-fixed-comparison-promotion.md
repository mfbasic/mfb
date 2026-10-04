# bug-816: Float/Fixed comparisons bypass the documented Fixed-first promotion

Last updated: 2026-10-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness
Status: Open
Regression Test: proposed `tests/rt-behavior/arithmetic/float-fixed-comparison-promotion` plus runtime-error boundary fixtures

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
