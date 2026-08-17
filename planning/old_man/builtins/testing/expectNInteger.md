# expectNInteger

Assert that two `Integer` values are not equal, checking the operand type too

## Synopsis

```
expectNInteger(actual AS Integer, expected AS Integer)
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

`expectNInteger` is the `Integer`-typed inequality assertion and the exact mirror
of `expectInteger`. It evaluates `actual` and `expected` — in that order, exactly
once each — and passes when the two differ. It is spelled bare, with no
`testing::` qualifier, because it is a compiler-recognized builtin rather than a
package function. [[src/builtins/testing.rs:is_inequality_assert]]

What distinguishes it from the generic `expectNEqual` is a compile-time operand
check: both `actual` and `expected` must be exactly `Integer`. Passing a `Float`,
a `Byte`, or a `Fixed` where the generic form would silently accept it and
compare numerically is a compile error here, so the assertion pins the type and
the difference in one line. Reach for `expectNEqual` instead when you deliberately
want a cross-type numeric comparison.
[[src/builtins/testing.rs:expect_operand_type]]

`expectNInteger` is a statement-level assertion: it produces `Nothing` and cannot
be used as a subexpression. Once the type check has passed, it lowers through
exactly the same expansion as every other inequality assertion — there is no
runtime helper — into a pair of `LET` bindings, a `=` comparison, and a `FAIL`
when the comparison succeeds. [[src/testing/desugar/expect.rs:expand_expect]]

On failure the expansion raises `error(77069001, "expected values to differ, but
both were <actual>")` — only `actual` is rendered, since the two values are equal
by definition at that point. `77069001` is a reserved internal code the
synthesized `mfb test` driver recognizes, so the failure is reported as a test
failure and not as a crash. The raise unwinds out of the enclosing `TCASE`, so
statements after the failed assertion in that case do not run, while sibling
cases and groups still run to completion. [[src/testing/desugar/expect.rs:expand_eq]]
[[src/testing/desugar/driver.rs:assertion_detail]]

Both arguments are required; any count other than two is `TESTING_EXPECT_ARITY`
(`2-208-0002`), and an operand of the wrong type is
`TESTING_EXPECT_TYPE_MISMATCH` (`2-208-0008`), reported once per offending
operand. An operand whose type could not be inferred is skipped, to avoid
cascading diagnostics. [[src/syntaxcheck/inference.rs:check_expect_call]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `actual` | `Integer` | The value produced by the code under test. Must be exactly `Integer`. Evaluated first, exactly once. This is the value rendered in the failure detail. |
| `expected` | `Integer` | The value `actual` must differ from. Must be exactly `Integer`. |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `expectNInteger` is a statement-level assertion and yields no value. [[src/syntaxcheck/inference.rs:check_expect_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77069001` | *(reserved; deliberately absent from the errorCode:: registry)* | The assertion failed: `actual = expected`. The error carries the detail `expected values to differ, but both were <actual>` and is recognized by the `mfb test` driver as an assertion failure. [[src/builtins/testing.rs:TEST_ABORT_CODE]] |

A genuine runtime error raised while evaluating `actual` or `expected` is not
caught by the assertion. It propagates out of the `TCASE` and the driver reports
the case as failed with `runtime error [<code>] <message>` instead of an
assertion detail. [[src/testing/desugar/driver.rs:case_call]]

## Examples

Pin the type while asserting a value moved:

```
FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC

TESTING
  TGROUP "typed equality"
    TCASE "expectInteger / expectNInteger"
      expectInteger(add(2, 3), 5)
      expectNInteger(add(2, 3), 6)
    END TCASE
  END TGROUP
END TESTING
```

## See also

- `mfb man testing expectInteger`
- `mfb man testing expectNEqual`
- `mfb man testing expectNFloat`
- `mfb man testing expectNFixed`
- `mfb man testing expectNString`
- `mfb spec language test-framework`
