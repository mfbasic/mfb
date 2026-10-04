# bug-813: Fixed literal validation rejects a representable maximum after binary64 rounding

Last updated: 2026-10-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness
Status: Open
Regression Test: proposed `tests/rt-behavior/conversions/fixed_literal_maximum`

The verifier converts a Fixed decimal literal to binary64 before checking its range. A valid decimal at the Q32.32 maximum rounds upward to 2147483648.0 in binary64 and is rejected before the decimal-to-carrier converter can accept it.

Correct behavior: validate the signed decimal using the same specified rounding and carrier-range decision as Fixed literal lowering. A literal that rounds to an in-range carrier must be accepted; one that rounds outside it must receive the appropriate range diagnostic. This intentionally replaces the current §4.12 binary64 acceptance rule, so the specification must change in the same fix.

References:

- `src/docs/spec/language/04_types.md`, Fixed representable range and §4.12 Fixed literal validation (the latter currently documents the problematic behavior).
- `src/numeric.rs:fixed_raw_from_decimal`, decimal-to-Q32.32 converter.
- `bugs/completed/bug-07-fixed-min-literal.md`, signed minimum handling to preserve.
- `bugs/completed/bug-91-fixed-literal-frac-digit-reject.md`, long fractional spellings; not a fix for this verifier bound.
- `bugs/completed/bug-367-negative-fixed-literal-stores-float-bits.md`, negative literal lowering; preserve its regression coverage.
- Found while reproducing numeric-spec review §5.3. Duplicate search: `rg -n -i 'literal.*(bound|f64)|fixed literal' bugs --glob '*.md'` across live and archived reports.

## Failing Reproduction

```sh
mkdir -p /tmp/mfb-numeric-review/fixed/src
cat > /tmp/mfb-numeric-review/fixed/project.json <<'JSON'
{"name": "numeric_fixed", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "entry": "main", "targets": ["native"]}
JSON
cat > /tmp/mfb-numeric-review/fixed/src/main.mfb <<'BASIC'
IMPORT io
FUNC main() AS Nothing
  LET maximum AS Fixed = 2147483647.99999999976716935634613037109375F
  io::print(toString(maximum))
END FUNC
BASIC
cargo build --bin mfb
target/debug/mfb build -O0 /tmp/mfb-numeric-review/fixed
```

Observed: build exit 1 with:

```text
error[2-203-0017 TYPE_FIXED_LITERAL_OVERFLOW]: numeric literal is outside the Fixed range
Numeric literal `2147483647.99999999976716935634613037109375` is outside the Fixed range.
```

Expected: build succeeds; the literal has raw carrier `9223372036854775807` (i64::MAX). A runtime regression should prove the value, for example by adding the smallest Fixed quantum and verifying `ErrOverflow`, alongside an exact raw-converter unit assertion.

Independent exact oracle:

```sh
python3 - <<'PY_ORACLE'
from fractions import Fraction
s = '2147483647.99999999976716935634613037109375'
print(Fraction(s) * 2**32)
print(float(s))
PY_ORACLE
```

Observed output:

```text
9223372036854775807
2147483648.0
```

Environment: `uname -sm` returned `Darwin arm64`; `cargo build --bin mfb` rebuilt current source at `git rev-parse HEAD` = `3aefe8332fdb921d55b75afcefb0f7424f12c475`. The command above reproduced the failure at `-O0`. The verifier is shared across targets, but other targets have not been executed.

## Root Cause

`src/ir/verify/values.rs:check_const_literal` parses Fixed text with `parse::<f64>()` and rejects values `>= 2147483648.0`. Binary64 loses the decimal distinction before that check. `check_negated_const_literal` has a corresponding binary64 lower-bound check; audit it for the opposite direction (acceptance of out-of-range magnitudes), rather than assuming the positive fix is sufficient.

`src/numeric.rs:fixed_raw_from_decimal` already lowers decimal text into the signed Fixed carrier using integer arithmetic and rounding. Validation and materialization therefore use different numerical representations and can disagree.

Inspection commands:

```sh
sed -n '490,505p;565,580p' src/ir/verify/values.rs
sed -n '219,315p' src/numeric.rs
rg -n 'fixed_raw_from_decimal|check_negated_const_literal' src
```

## Goal

The verifier and literal converter make the same signed, rounded range decision, accepting the exact maximum and minimum and rejecting rounded results outside the carrier range.

### Non-goals (must NOT change)

Do not widen Fixed, clamp overflowing literals, relax diagnostics indiscriminately, or alter Float-to-Fixed runtime conversion. Preserve Money literal behavior, tiny-literal handling, scientific notation safeguards, and negative-literal typing. No test is presumed wrong solely because this fix changes its output.

## Blast Radius

- `values.rs:check_const_literal` Fixed arm — reproduced and in scope.
- `values.rs:check_negated_const_literal` Fixed arm — same representation hazard; audit and unify signed validation in this fix.
- `numeric.rs:fixed_raw_from_decimal` — authoritative conversion seam to test, including rounding carry and long decimal spellings before reuse; do not assume its existing tests prove every boundary.
- Callers of `fixed_raw_from_decimal` in constant lowering/materialization — preserve carrier identity and test source and decoded-package verification.
- Float and Money verifier arms — independent representations/contracts; unchanged.
- Runtime `toFixed(Float)` and `toFixed(String)` — separate conversion paths, not covered by this literal acceptance fix.

## Fix Design

Use one signed decimal conversion result to decide Fixed literal acceptance and materialization. Apply the sign before conversion so the representable negative minimum does not require an unrepresentable positive intermediate. Preserve existing overflow/underflow diagnostic identities and source attribution. Specify nearest rounding with ties away from zero and range checking after rounding, consistent with the intended literal conversion contract.

Add exact maximum/minimum, adjacent in-range carriers, above/below endpoint values, and rounding-carry boundaries. Include suffixed and expected-type literals, negative forms, scientific notation, and long fractional spellings. Check the converter with independent rational arithmetic at boundaries before making it the validation authority.

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

Add converter and verifier unit tests plus the proposed runtime fixture and syntax fixtures for out-of-range literals. Preserve bug-07, bug-11, bug-91, and bug-367 regressions. Render the updated types specification to verify the new acceptance rule. Full-suite commands and runtime proof are required as listed above; successful compilation alone does not establish the stored maximum.

## Open Decisions

This report proposes post-rounding range validity explicitly. The current §4.12 binary64 rule must be replaced, not described as already implementing that policy.

## Summary

The defect is the validation/materialization disagreement. The risk is moving the disagreement to a different boundary; use one signed conversion decision and independent exact boundary tests.
