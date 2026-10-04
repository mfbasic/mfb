# bug-812: Money arithmetic and conversions through Fixed ignore the Money rounding mode

Last updated: 2026-10-04
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness
Status: Open
Regression Test: proposed `tests/rt-behavior/money/money_fixed_rounding`

Money/Fixed scaling and conversions silently discard fractional raw units instead of applying the documented Money rounding rule. Positive multiplication and division can understate an amount; negative multiplication floors while negative division truncates toward zero. Changing the rounding mode does not repair those paths.

The correct behavior is to compute the exact rational result on the stored carriers, round once to the destination carrier using the current thread's Commercial/Banker mode, then reject an out-of-range rounded carrier. Ordinary Fixed arithmetic must keep its independent semantics.

References:

- `src/docs/spec/language/04_types.md`, Money arithmetic rounding contract.
- `src/docs/spec/stdlib/13_money.md`, “The exact base-10 model” and “The rounding rule”: Money/Fixed scaling and `toMoney`/`toFixed` conversions consult the mode.
- `bugs/completed/bug-449-tomoney-string-parses-through-f64-not-exact-decimal.md` concerns the String conversion, not these Fixed paths.
- `bugs/completed/bug-230-money-divide-i64-min-divisor.md` concerns Integer division, not the Fixed helper dispatch.
- Found while reproducing numeric-spec review §5.2. Searched all `bugs/`, including completed/skipped reports, with `rg -n -i 'money.*fixed|fixed.*money|rounding mode' bugs --glob '*.md'`.

## Failing Reproduction

```sh
mkdir -p /tmp/mfb-numeric-review/money/src
cat > /tmp/mfb-numeric-review/money/project.json <<'JSON'
{"name": "numeric_money", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "entry": "main", "targets": ["native"]}
JSON
cat > /tmp/mfb-numeric-review/money/src/main.mfb <<'BASIC'
IMPORT io
IMPORT money
FUNC probe() AS Nothing
  LET a AS Money = 0.00003m
  LET b AS Money = -0.00003m
  LET h AS Fixed = 0.5F
  LET two AS Fixed = 2F
  io::print(toString(a * h, toByte(5)))
  io::print(toString(b * h, toByte(5)))
  io::print(toString(a / two, toByte(5)))
  io::print(toString(b / two, toByte(5)))
  LET rate AS Fixed = 0.0000152587890625F
  io::print(toString(toMoney(rate), toByte(5)))
  LET amount AS Money = 0.00001m
  io::print(toString(toFixed(amount), toByte(12)))
  io::print(toString(a / 2, toByte(5)))
END FUNC
FUNC main() AS Nothing
  money::setRounding(money::Rounding.Commercial)
  probe()
  money::setRounding(money::Rounding.Banker)
  probe()
END FUNC
BASIC
cargo build --bin mfb
target/debug/mfb build -O0 /tmp/mfb-numeric-review/money
/tmp/mfb-numeric-review/money/build/numeric_money.out
```

Observed: exit 0, with this seven-line block printed identically for Commercial and Banker:

```text
0.00001
-0.00002
0.00001
-0.00001
0.00001
0.000009999843
0.00002
```

Expected under either mode for these particular inputs:

| Expression | Expected carrier/value | Observed |
| --- | --- | --- |
| `a * h` | Money raw 2 (`0.00002`) | raw 1 |
| `b * h` | Money raw -2 (`-0.00002`) | raw -2, contrast case |
| `a / two` | Money raw 2 | raw 1 |
| `b / two` | Money raw -2 | raw -1 |
| `toMoney(rate)` | Money raw 2 | raw 1 |
| `toFixed(amount)` | Fixed raw 42950 | raw 42949, rendered `0.000009999843` |
| `a / 2` | Money raw 2 | raw 2, correct Integer-divisor contrast |

Oracle: `3 * (2^31) / 2^32 = 1.5`; both nearest modes choose raw 2. The Fixed rate is exactly `1/65536`, so `100000/65536 = 1.52587890625` rounds to Money raw 2. Money raw 1 converts to Fixed raw `2^32/100000 = 42949.67296`, which rounds to 42950. Assert carriers through exact value comparisons in regression fixtures, not just decimal formatting.

Environment: `uname -sm` returned `Darwin arm64`. `cargo build --bin mfb` rebuilt current source at `git rev-parse HEAD` = `3aefe8332fdb921d55b75afcefb0f7424f12c475`; the working changes were unrelated planning files. Both `target/debug/mfb build -O0 /tmp/mfb-numeric-review/money` and the same command with `-O1`, followed by the executable above, produced the displayed output. Other targets have not been executed.

## Root Cause

`src/codegen/builtins/money/gen_money_math.rs:emit_money_multiply` calls `emit_fixed_multiply`; `emit_money_divide_scalar` calls `emit_fixed_divide`. `src/codegen/engine/convert/builder_conversions.rs:lower_to_money` and `lower_to_fixed` use the same helpers for Fixed/Money conversion.

`src/codegen/engine/operators/builder_numeric.rs:emit_fixed_multiply` forms the signed wide product and drops its low 32 bits, yielding a floor for negative inexact products. `emit_fixed_divide` generates 32 fractional bits of the magnitude and restores the sign, truncating toward zero. Neither calls `emit_apply_rounding` or reads the rounding-mode word. This is a helper-selection defect; changing the ordinary Fixed helpers globally would alter unrelated language behavior.

Inspection commands:

```sh
sed -n '157,263p' src/codegen/builtins/money/gen_money_math.rs
sed -n '1479,1633p' src/codegen/engine/operators/builder_numeric.rs
rg -n 'emit_fixed_multiply\(|emit_fixed_divide\(' src/codegen
```

## Goal

All Money/Fixed arithmetic and explicit conversions settle the exact carrier ratio once using the Money rounding mode, including negative values, halfway cases, and rounded overflow.

### Non-goals (must NOT change)

Do not change pure Fixed arithmetic, Float approximation rules, Money literal rounding, presentation rounding, or thread inheritance. Do not redefine the spec to excuse the current silent wrong answers. Do not use Float intermediates as a replacement for exact rescaling.

## Blast Radius

Caller inventory command: `rg -n 'emit_fixed_multiply\(|emit_fixed_divide\(' src/codegen`.

- `gen_money_math.rs:emit_money_multiply` — reproduced, fixed by this bug; cover both operand orders.
- `gen_money_math.rs:emit_money_divide_scalar` — reproduced, fixed by this bug.
- `builder_conversions.rs:lower_to_money` Fixed arm — reproduced, fixed by this bug.
- `builder_conversions.rs:lower_to_fixed` Money arm — reproduced, fixed by this bug.
- `builder_numeric.rs:emit_fixed_binary` and `emit_fixed_pow` — ordinary Fixed semantics, preserve unchanged.
- `gen_fixed_math.rs` callers (atan/power paths) — pure Fixed kernels, preserve unchanged.
- `gen_money_math.rs` Integer division — the small-divisor control above passes; retain regression coverage without claiming all Integer edge cases validated.
- Float scaling and String conversion — separate rounding paths; run existing coverage, do not redirect them to Fixed arithmetic.

## Fix Design

Add Money-specific exact rescaling that retains quotient, remainder, divisor magnitude, and result sign through the rounding decision. Multiplication must retain the full signed wide product; division and conversions must avoid intermediate overflow where the final rounded carrier fits. Apply the existing Commercial/Banker rule safely to those quantities and range-check after a possible rounding carry. Route only the four Money/Fixed sites to it.

Expand the regression matrix to even/odd ties that distinguish modes, non-ties on both sides, negative values, signed endpoints, divide-by-zero, and an overflow caused by rounding. Verify expressions through locals and record fields, and both multiplication orders. No existing test/golden is presumed wrong.

## Phases

### Phase 1 — failing tests and audit

- [ ] Add the proposed regression fixtures and confirm RED against the current compiler.
- [ ] Complete the caller audit and preserve contrast cases before changing implementation.

Acceptance: failures demonstrate the documented defect, not formatting or unrelated errors.
Commit: —

### Phase 2 — implementation and specification

- [ ] Implement the Fix Design and update the specification in the same change.
- [ ] Make the regression fixtures pass without weakening existing tests.

Acceptance: the original reproduction produces the expected behavior and sibling contracts remain intact.
Commit: —

### Phase 3 — full validation

- [ ] Run the full suite before changing any existing golden; establish the four required evidence points for each disproved expectation.
- [ ] Correct only proven-wrong golden lines, and inspect every unexpected artifact difference.
- [ ] Run `cargo test` and `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- [ ] Execute the reproduction at `-O0` and `-O1` on all supported native targets; record results.

Acceptance: runtime values, diagnostic behavior, and the full validation gates agree with the corrected contract.
Commit: —

## Validation Plan

The proposed Money fixture proves each arithmetic/conversion path at runtime; add invalid-input fixtures for overflow and zero division. Preserve existing `money_operations`, conversion, and Fixed arithmetic fixtures. Update the language types and Money stdlib specifications with explicit carrier formulas. Use the Phase 3 full-suite commands and compare generated artifacts before accepting changed expectations.

## Open Decisions

None for the reproduced failures: the existing spec requires mode-sensitive rounding. The exact helper interface is an implementation choice; it must preserve signed full-range arithmetic.

## Summary

The main risk is fixing shared arithmetic by changing pure Fixed behavior. Keep Money settlement explicit and verify the four consumers independently.
