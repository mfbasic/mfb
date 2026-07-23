# expectFixed

Assert that two `Fixed` values are equal, checking the operand type too

## Synopsis

```
expectFixed(actual AS Fixed, expected AS Fixed)
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

`expectFixed` is the `Fixed`-typed equality assertion. It evaluates `actual` and
`expected` — in that order, exactly once each — and passes when
`actual = expected`. It is spelled bare, with no `testing::` qualifier, because
it is a compiler-recognized builtin rather than a package function.
[[src/builtins/testing.rs:is_equality_assert]]

What distinguishes it from the generic `expectEqual` is a compile-time operand
check: both `actual` and `expected` must be exactly `Fixed`. Passing a `Float`,
an `Integer`, or a `Money` where the generic form would silently accept it and
compare numerically is a compile error here, so the assertion pins the type and
the value in one line. Reach for `expectEqual` instead when you deliberately want
a cross-type numeric comparison. [[src/builtins/testing.rs:expect_operand_type]]

Because `Fixed` is an exact scaled decimal, the comparison is exact and
predictable: it is the natural assertion for a computation whose result you want
pinned to a specific decimal value. A `Fixed` operand is written with the `F`
suffix (`1.5F`) or produced by `toFixed`. Both spellings are `Fixed`, so they mix
freely across the two arguments.

`expectFixed` is a statement-level assertion: it produces `Nothing` and cannot be
used as a subexpression. Once the type check has passed, it lowers through
exactly the same expansion as every other equality assertion — there is no
runtime helper — into a pair of `LET` bindings, a `=` comparison, and a `FAIL` on
mismatch. On failure it raises `error(77069001, "expected <expected>, got
<actual>")`, both values rendered with `toString`. `77069001` is a reserved
internal code the synthesized `mfb test` driver recognizes, so the failure is
reported as a test failure and not as a crash. The raise unwinds out of the
enclosing `TCASE`, so statements after the failed assertion in that case do not
run, while sibling cases and groups still run to completion.
[[src/testing/desugar/expect.rs:expand_expect]]
[[src/testing/desugar/driver.rs:assertion_detail]]

Both arguments are required; any count other than two is `TESTING_EXPECT_ARITY`
(`2-208-0002`), and an operand of the wrong type is
`TESTING_EXPECT_TYPE_MISMATCH` (`2-208-0008`), reported once per offending
operand. An operand whose type could not be inferred is skipped, to avoid
cascading diagnostics. [[src/syntaxcheck/inference.rs:check_expect_call]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `actual` | `Fixed` | The value produced by the code under test. Must be exactly `Fixed`. Evaluated first, exactly once. |
| `expected` | `Fixed` | The value `actual` must equal. Must be exactly `Fixed`. |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `expectFixed` is a statement-level assertion and yields no value. [[src/syntaxcheck/inference.rs:check_expect_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77069001` | *(reserved; deliberately absent from the errorCode:: registry)* | The assertion failed: `actual <> expected`. The error carries the detail `expected <expected>, got <actual>` and is recognized by the `mfb test` driver as an assertion failure. [[src/builtins/testing.rs:TEST_ABORT_CODE]] |

A genuine runtime error raised while evaluating `actual` or `expected` is not
caught by the assertion. It propagates out of the `TCASE` and the driver reports
the case as failed with `runtime error [<code>] <message>` instead of an
assertion detail. [[src/testing/desugar/driver.rs:case_call]]

## Examples

Assert exact decimal results, mixing the `F` suffix with `toFixed`:

```
IMPORT math

FUNC main AS Integer
  RETURN 0
END FUNC

TESTING
  TGROUP "typed equality"
    TCASE "expectFixed passes"
      expectFixed(math::abs(0 - 1.5F), 1.5F)
    END TCASE
    TCASE "toFixed normalizes the written form"
      expectFixed(toFixed("2.50"), toFixed("2.5"))
      expectFixed(toFixed(42), toFixed("42.0"))
    END TCASE
  END TGROUP
END TESTING
```

## See also

- `mfb man testing expectNFixed`
- `mfb man testing expectEqual`
- `mfb man testing expectFloat`
- `mfb man testing expectInteger`
- `mfb man testing expectString`
- `mfb spec language test-framework`
