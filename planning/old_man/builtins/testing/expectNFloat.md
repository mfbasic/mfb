# expectNFloat

Assert that two `Float` values are not equal, checking the operand type too

## Synopsis

```
expectNFloat(actual AS Float, expected AS Float)
```

## Package

testing

## Imports

None. The assertion builtins are always in scope and need no `IMPORT`
statement, but they are legal **only** inside a `TCASE` body — a call anywhere
else is rejected before any other front-end pass with
`TESTING_EXPECT_OUTSIDE_TCASE` (`2-208-0001`).
[[src/testing/desugar/placement.rs:validate_expect_placement]]

## Description

`expectNFloat` is the `Float`-typed inequality assertion and the exact mirror of
`expectFloat`. It evaluates `actual` and `expected` — in that order, exactly once
each — and passes when the two differ. It is spelled bare, with no `testing::`
qualifier, because it is a compiler-recognized builtin rather than a package
function. [[src/builtins/testing.rs:is_inequality_assert]]

What distinguishes it from the generic `expectNEqual` is a compile-time operand
check: both `actual` and `expected` must be exactly `Float`. Passing an `Integer`
or a `Fixed` where the generic form would silently accept it and compare
numerically is a compile error here, so the assertion pins the type and the
difference in one line. [[src/builtins/testing.rs:expect_operand_type]]

The check is the ordinary `Float` `=`, negated — an exact bit-value comparison
with no tolerance. Two `Float`s that differ by a single ULP therefore satisfy
`expectNFloat`, which makes it a weak assertion for numeric work: it confirms
only that a computation did not land on one specific value. Prefer asserting the
value you do expect with `expectFloat` where you can.
[[src/testing/desugar/expect.rs:expand_eq]]

`expectNFloat` is a statement-level assertion: it produces `Nothing` and cannot
be used as a subexpression. Once the type check has passed, it lowers through the
same expansion as every other inequality assertion — there is no runtime helper —
into a pair of `LET` bindings, a `=` comparison, and a `FAIL` when the comparison
succeeds. On failure it raises `error(77069001, "expected values to differ, but
both were <actual>")`. `77069001` is a reserved internal code the synthesized
`mfb test` driver recognizes, so the failure is reported as a test failure and not
as a crash. The raise unwinds out of the enclosing `TCASE`, so statements after
the failed assertion in that case do not run, while sibling cases and groups still
run to completion. [[src/testing/desugar/expect.rs:expand_expect]]
[[src/testing/desugar/driver.rs:assertion_detail]]

Both arguments are required; any count other than two is `TESTING_EXPECT_ARITY`
(`2-208-0002`), and an operand of the wrong type is
`TESTING_EXPECT_TYPE_MISMATCH` (`2-208-0008`), reported once per offending
operand. An operand whose type could not be inferred is skipped, to avoid
cascading diagnostics. [[src/syntaxcheck/inference.rs:check_expect_call]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `actual` | `Float` | The value produced by the code under test. Must be exactly `Float`. Evaluated first, exactly once. This is the value rendered in the failure detail. |
| `expected` | `Float` | The value `actual` must differ from, compared exactly. Must be exactly `Float`. |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `expectNFloat` is a statement-level assertion and yields no value. [[src/syntaxcheck/inference.rs:check_expect_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77069001` | *(reserved; deliberately absent from the errorCode:: registry)* | The assertion failed: `actual = expected`. The error carries the detail `expected values to differ, but both were <actual>` and is recognized by the `mfb test` driver as an assertion failure. [[src/builtins/testing.rs:TEST_ABORT_CODE]] |

A genuine runtime error raised while evaluating `actual` or `expected` is not
caught by the assertion. It propagates out of the `TCASE` and the driver reports
the case as failed with `runtime error [<code>] <message>` instead of an
assertion detail. [[src/testing/desugar/driver.rs:case_call]]

## Examples

Assert that an irrational result is not the naive value:

```
IMPORT math

FUNC main AS Integer
  RETURN 0
END FUNC

TESTING
  TGROUP "typed equality"
    TCASE "expectFloat / expectNFloat"
      expectFloat(math::sqrt(4.0), 2.0)
      expectNFloat(math::sqrt(2.0), 2.0)
    END TCASE
  END TGROUP
END TESTING
```

## See also

- `mfb man testing expectFloat`
- `mfb man testing expectNEqual`
- `mfb man testing expectNInteger`
- `mfb man testing expectNFixed`
- `mfb man testing expectNString`
- `mfb spec language test-framework`
