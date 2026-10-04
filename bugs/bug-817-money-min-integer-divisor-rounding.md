# bug-817: Money divided by the minimum Integer skips required rounding

Last updated: 2026-10-04
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness
Status: Open
Regression Test: proposed `tests/rt-behavior/arithmetic/money-min-integer-divisor-rounding` plus runtime-error boundary fixtures

## Goal

Expected -0.00001; observed 0.00000, exit 0. The raw ratio is -5000000000000000000 / 9223372036854775808, whose magnitude exceeds one half. Both nearest modes must produce raw -1.

## References and duplicate audit

`bugs/completed/bug-230-money-divide-i64-min-divisor.md` fixes raw minimum/minimum but introduces the unconditional truncation branch. This is a distinct uncovered range of that branch. `bug-812` concerns Fixed operands, not this Integer divisor.

The contract is in `src/docs/spec/language/04_types.md` and, for Money rounding,
`src/docs/spec/stdlib/13_money.md`. The documentation update distinguishes current
implementation limitations from the intended fix. Searches covered all live and
archived reports: `rg -n -i 'comparison.*(Float|Fixed)|Float.*compar.*Fixed|promotion.*compar|i64::MIN.*divisor' bugs --glob '*.md'`.

## Failing Reproduction

```sh
mkdir -p /tmp/mfb-doc-numeric/money_min/src
cat > /tmp/mfb-doc-numeric/money_min/project.json <<'JSON'
{"name": "money_min", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "entry": "main", "targets": ["native"]}
JSON
cat > /tmp/mfb-doc-numeric/money_min/src/main.mfb <<'BASIC'
IMPORT io
IMPORT collections
FUNC main() AS Nothing
  LET m AS Money = 50000000000000m
  LET k AS Integer = -9223372036854775808
  io::print(toString(m / k, toByte(5)))
END FUNC
BASIC
cargo build --release --bin mfb
target/release/mfb build -O0 /tmp/mfb-doc-numeric/money_min
/tmp/mfb-doc-numeric/money_min/build/money_min.out
```

Expected -0.00001; observed 0.00000, exit 0. The raw ratio is -5000000000000000000 / 9223372036854775808, whose magnitude exceeds one half. Both nearest modes must produce raw -1.

Measured on macOS arm64 with the release binary, at both `-O0` and `-O1`:
`python3 /tmp/numeric-doc-probes.py target/release/mfb` generated and ran the above
project. The script is temporary; the complete durable reproduction is above.
Other targets have not been run. These are shared lowering paths.

## Root Cause

`src/codegen/builtins/money/gen_money_math.rs:emit_money_divide_scalar` special-cases i64::MIN by returning the truncated quotient. Its justification confuses a remainder smaller than the divisor with a remainder smaller than half the divisor. The existing exact minimum/minimum control does not exercise rounding.

Inspection: `rg -n 'lower_numeric_comparison_binary|emit_money_divide_scalar|typed_binary_result_type' src/codegen/engine/operators/builder_numeric.rs src/codegen/builtins/money/gen_money_math.rs src/numeric.rs`.

## Blast Radius

- Money/Integer `/` with divisor i64::MIN: reproduced, in scope.
- Integer divisors of other magnitudes and Byte divisors: separate ordinary rounding path; preserve as contrast coverage.
- Minimum raw Money divided by minimum Integer: preserve exact raw 1 from the earlier fix.
- Fixed and Float divisors, Money ratios, and `DIV`: separate branches, not changed.
- `emit_apply_rounding` callers must be audited if its comparison signedness/interface changes; do not alter unrelated rounding through a shared helper.

## Fix Design

Handle divisor magnitude 2^63 with unsigned quotient/remainder rounding or a dedicated correct half comparison. Keep exact quotient results and sign, and preserve Commercial versus Banker at a half: raw 2^62 rounds to raw -1 under Commercial and raw 0 under Banker when divided by i64::MIN. Cover both dividend signs, just below/above half, zero, and minimum/minimum. Do not feed magnitude 2^63 to the existing signed compare unchanged.

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

None: both supported nearest modes round the reproduced non-tie away from zero.
