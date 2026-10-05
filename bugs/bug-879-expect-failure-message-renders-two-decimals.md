# bug-879: `mfb test` equality-failure detail renders Float/Fixed/Money with 2 decimals, so a failing `expectFloat` reads "expected 0.30, got 0.30"

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Footgun

Status: Open
Regression Test: tests/rt-behavior/testing/expect-float-failure-detail-rt (new) — name suggested

`expectFloat(0.1 + 0.2, 0.3)` correctly fails (the values differ), but the reported detail is
`X expected 0.30, got 0.30` — the two printed values are identical, so the report gives no clue why
the case failed. The same holds for `expectEqual`/`expectNEqual` on `Float`, and for `Fixed`/`Money`
values that differ beyond the second decimal; `expectNFloat`'s "expected values to differ, but both
were 0.30" can likewise hide distinct inputs.

Correct behavior: the detail renders each operand losslessly enough to distinguish two unequal
values (e.g. shortest round-trip for `Float`, full exact digits for `Fixed`), so that "expected X,
got Y" never shows X = Y for a failing equality.

References:

- src/docs/spec/language/22_test-framework.md "Running": "The failure `<detail>` reports the kind: an assertion mismatch (`expected …, got …`)".
- Found during B05 hardening of language/22_test-framework.

## Failing Reproduction

```
FUNC main() AS Nothing
END FUNC
TESTING
  TGROUP "g"
    TCASE "float"
      expectFloat(0.1 + 0.2, 0.3)
    END TCASE
  END TGROUP
END TESTING
```

`mfb test <proj>`

- Observed: `  * [F] float` / `    X expected 0.30, got 0.30  (src/main.mfb:6)`.
- Expected: e.g. `X expected 0.3, got 0.30000000000000004`.

Contrast: Integer/String mismatches print distinguishable values (`X expected 5, got 4`).

## Root Cause

`src/testing/desugar/expect.rs:expand_eq` builds the detail as `"expected " & toString(e) & ", got " & toString(a)`
using the one-argument `toString`, whose `Float`/`Fixed`/`Money` default is two decimal places.

## Goal

- For numeric non-integer operands the desugar renders with a precision that distinguishes any two unequal values.

### Non-goals (must NOT change)

- Pass/fail semantics (`=`/`<>`), and Integer/String/Byte rendering.

## Blast Radius

- `expand_eq` negated branch ("expected values to differ, but both were …") — same rendering; fix together.
