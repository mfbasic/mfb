# expectEqual

Assert that two values are equal, using the language `=` operator

## Synopsis

```
expectEqual(actual, expected)
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

`expectEqual` is the generic equality assertion of the built-in test framework.
It evaluates `actual` and `expected` — in that order, exactly once each — and
passes when `actual = expected`. It is spelled bare, with no `testing::`
qualifier, because it is a compiler-recognized builtin rather than a package
function. [[src/builtins/testing.rs:is_expect_call]]

The comparison is the language `=` operator, not a special test-only equality,
so `expectEqual` accepts exactly the operand pairs `=` accepts: any two numeric
operands (so `Integer` and `Float` compare numerically across types), or two
operands of compatible comparable type. It additionally requires both operands
to be printable, because the failure detail renders them with `toString`. Use
`expectEqual` when you want a value check that tolerates numeric type mixing;
use the typed `expectInteger`, `expectFloat`, `expectFixed`, or `expectString`
when you want the operand type asserted too.
[[src/syntaxcheck/inference.rs:check_expect_call]]

`expectEqual` is a statement-level assertion: it produces `Nothing` and cannot
be used as a subexpression. The compiler expands it in place — there is no
runtime helper — into a pair of `LET` bindings for the two operands, a `=`
comparison, and a `FAIL` on mismatch.
[[src/testing/desugar/expect.rs:expand_expect]]

On failure the expansion raises `error(77069001, "expected <expected>, got
<actual>")`, where both values are rendered with `toString`. `77069001` is a
reserved internal code that the synthesized `mfb test` driver recognizes, so the
failure is reported as a test failure and not as a crash. The raise unwinds out
of the enclosing `TCASE`, so statements after the failed assertion in that case
do not run, while sibling cases and groups still run to completion. The driver
prints the case as `* [F] <description>` followed by an indented detail line
`X expected <expected>, got <actual>  (<file>:<line>)`, and exits non-zero
because at least one case failed. [[src/testing/desugar/expect.rs:expand_eq]]
[[src/testing/desugar/driver.rs:assertion_detail]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `actual` | any comparable, printable type | The value produced by the code under test. Evaluated first, exactly once. |
| `expected` | any comparable, printable type | The value `actual` must equal. Must be comparable with `actual` under `=`. |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `expectEqual` is a statement-level assertion and yields no value. [[src/syntaxcheck/inference.rs:check_expect_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77069001` | *(reserved; deliberately absent from the errorCode:: registry)* | The assertion failed: `actual <> expected`. The error carries the detail `expected <expected>, got <actual>` and is recognized by the `mfb test` driver as an assertion failure. [[src/builtins/testing.rs:TEST_ABORT_CODE]] |

A genuine runtime error raised while evaluating `actual` or `expected` is not
caught by the assertion. It propagates out of the `TCASE` and the driver reports
the case as failed with `runtime error [<code>] <message>` instead of an
assertion detail. [[src/testing/desugar/driver.rs:case_call]]

## Type checking

`expectEqual` takes exactly two arguments; any other count is
`TESTING_EXPECT_ARITY` (`2-208-0002`).
[[src/builtins/testing.rs:expect_arity]]

The operands must be comparable with `=`. Two numeric operands (`Integer`,
`Float`, `Fixed`, `Money`, `Byte`) always qualify and compare numerically; two
non-numeric operands qualify when one type is compatible with the other and both
are comparable. Anything else is `TESTING_EXPECT_INCOMPARABLE` (`2-208-0003`).
[[src/syntaxcheck/inference.rs:infer_binary]]

Both operands must also be printable, so the failure detail can render them:
`Integer`, `Float`, `Fixed`, `Money`, `Boolean`, `String`, `Byte`, `Scalar`, or
`List OF Byte`. Any other type — a record, a map, a `List` of anything but
`Byte` — is `TESTING_EXPECT_NOT_PRINTABLE` (`2-208-0004`). An operand whose type
could not be inferred is treated as acceptable, to avoid cascading diagnostics.
[[src/syntaxcheck/inference.rs:is_printable]]

## Examples

Assert a function's result, mixing a `Byte` against an `Integer` literal:

```
IMPORT strings

FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC

TESTING
  TGROUP "equality"
    TCASE "expectEqual passes"
      expectEqual(add(2, 3), 5)
    END TCASE
    TCASE "expectEqual reports the mismatch"
      expectEqual(add(2, 3), 6)
    END TCASE
    TCASE "Boolean results compare directly"
      expectEqual(strings::contains("Hello", "ell"), TRUE)
    END TCASE
  END TGROUP
END TESTING
```

The second case above fails and the driver prints:

```text
* equality
  * [P] expectEqual passes
  * [F] expectEqual reports the mismatch
    X expected 6, got 5  (src/main.mfb:17)
  * [P] Boolean results compare directly
```

## See also

- `mfb man testing expectNEqual`
- `mfb man testing expectInteger`
- `mfb man testing expectFloat`
- `mfb man testing expectFixed`
- `mfb man testing expectString`
- `mfb man testing expectTrap`
- `mfb spec language test-framework`
