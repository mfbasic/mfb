# expectInteger

Assert that two `Integer` values are equal, checking the operand type too

## Synopsis

```
expectInteger(actual AS Integer, expected AS Integer)
```

## Package

testing

## Imports

None. The assertion builtins are always in scope and need no `IMPORT`
statement, but they are legal **only** inside a `TCASE` body — a call anywhere
else is rejected before any other front-end pass with
`TESTING_EXPECT_OUTSIDE_TCASE` (`2-208-0001`).
[[src/testing/desugar.rs:validate_expect_placement]]

## Description

`expectInteger` is the `Integer`-typed equality assertion. It evaluates `actual`
and `expected` — in that order, exactly once each — and passes when
`actual = expected`. It is spelled bare, with no `testing::` qualifier, because
it is a compiler-recognized builtin rather than a package function.
[[src/builtins/testing.rs:is_equality_assert]]

What distinguishes it from the generic `expectEqual` is a compile-time operand
check: both `actual` and `expected` must be exactly `Integer`. Passing a `Float`,
a `Byte`, or a `Fixed` where the generic form would silently accept it and
compare numerically is a compile error here, so the assertion pins the type and
the value in one line. Reach for `expectEqual` instead when you deliberately want
a cross-type numeric comparison. [[src/builtins/testing.rs:expect_operand_type]]

`expectInteger` is a statement-level assertion: it produces `Nothing` and cannot
be used as a subexpression. Once the type check has passed, it lowers through
exactly the same expansion as every other equality assertion — there is no
runtime helper — into a pair of `LET` bindings for the two operands, a `=`
comparison, and a `FAIL` on mismatch.
[[src/testing/desugar.rs:expand_expect]]

On failure the expansion raises `error(77069001, "expected <expected>, got
<actual>")`, both values rendered with `toString`. `77069001` is a reserved
internal code the synthesized `mfb test` driver recognizes, so the failure is
reported as a test failure and not as a crash. The raise unwinds out of the
enclosing `TCASE`, so statements after the failed assertion in that case do not
run, while sibling cases and groups still run to completion.
[[src/testing/desugar.rs:expand_eq]] [[src/testing/desugar.rs:assertion_detail]]

Both arguments are required; any count other than two is `TESTING_EXPECT_ARITY`
(`2-208-0002`), and an operand of the wrong type is
`TESTING_EXPECT_TYPE_MISMATCH` (`2-208-0008`), reported once per offending
operand. An operand whose type could not be inferred is skipped, to avoid
cascading diagnostics. [[src/syntaxcheck/inference.rs:check_expect_call]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `actual` | `Integer` | The value produced by the code under test. Must be exactly `Integer`. Evaluated first, exactly once. |
| `expected` | `Integer` | The value `actual` must equal. Must be exactly `Integer`. |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `expectInteger` is a statement-level assertion and yields no value. [[src/syntaxcheck/inference.rs:check_expect_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77069001` | *(reserved; deliberately absent from the errorCode:: registry)* | The assertion failed: `actual <> expected`. The error carries the detail `expected <expected>, got <actual>` and is recognized by the `mfb test` driver as an assertion failure. [[src/builtins/testing.rs:TEST_ABORT_CODE]] |

A genuine runtime error raised while evaluating `actual` or `expected` is not
caught by the assertion. It propagates out of the `TCASE` and the driver reports
the case as failed with `runtime error [<code>] <message>` instead of an
assertion detail. [[src/testing/desugar.rs:case_call]]

## Examples

Assert integer results, including a conversion round trip:

```
FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC

TESTING
  TGROUP "typed equality"
    TCASE "expectInteger passes"
      expectInteger(add(2, 3), 5)
    END TCASE
    TCASE "toInt of a Byte round-trips"
      expectInteger(toInt(toByte(7)), 7)
    END TCASE
  END TGROUP
END TESTING
```

## See also

- `mfb man testing expectNInteger`
- `mfb man testing expectEqual`
- `mfb man testing expectFloat`
- `mfb man testing expectFixed`
- `mfb man testing expectString`
- `mfb spec language test-framework`
