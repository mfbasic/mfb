# expectTrap

Assert that evaluating an expression traps, optionally with a specific error code

## Synopsis

```
expectTrap(expression)
expectTrap(expression, code AS Integer)
```

## Package

testing

## Imports

None. The assertion builtins are always in scope and need no `IMPORT`
statement, but they are legal **only** inside a `TCASE` body — a call anywhere
else is rejected before any other front-end pass with
`TESTING_EXPECT_OUTSIDE_TCASE` (`2-208-0001`).
[[src/testing/desugar/placement.rs:validate_expect_placement]]

The two-argument form is usually written against an `errorCode::` constant,
which does require `IMPORT errorCode` in the file holding the `TESTING` block.

## Description

`expectTrap` is the failure-path assertion of the built-in test framework. It
evaluates `expression` under a trap guard and passes when the evaluation traps.
The expression's value is discarded — only whether it trapped matters. It is
spelled bare, with no `testing::` qualifier, because it is a compiler-recognized
builtin rather than a package function. [[src/builtins/testing.rs:EXPECT_TRAP]]

The trap guard is the ordinary inline-`TRAP` machinery, so `expectTrap` accepts
exactly what an inline `TRAP` accepts — the same uniform surface: user
`FUNC`/`SUB` calls, package functions, infallible built-ins such as `len`, the
index and range members, and the callback members like
`collections::transform`. There is exactly one rejection: a scrutinee with no
runtime call to trap, meaning a non-call expression or a package constant. That
is `TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE` (`2-208-0006`). Notably, an
*infallible* call is **not** rejected at compile time; the assertion simply
evaluates against the real runtime outcome and fails, because no trap occurs.
[[src/syntaxcheck/inference.rs:check_trap_guardable]]

With the two-argument form, `expectTrap` additionally requires the trap's
`error.code` to equal `code`. The expansion records the trapped error's `code`
into a temporary inside the guard's handler, then compares it after the guard.
`code` may be any `Integer` expression — an `errorCode::` constant, a literal, or
a computed value — and it is evaluated once, after the guarded expression, and
only in the two-argument form. [[src/testing/desugar/expect.rs:expand_trap]]

`expectTrap` is a statement-level assertion: it produces `Nothing` and cannot be
used as a subexpression. The compiler expands it in place — there is no runtime
helper — into a `Boolean` flag, an inline `TRAP` around `expression` whose
handler sets the flag (and captures the code) and then `RECOVER`s, and one or two
`IF` guards that `FAIL` when the outcome is wrong.
[[src/testing/desugar/expect.rs:expand_expect]]

On failure the expansion raises `error(77069001, <detail>)` with one of three
details, depending on the form and the outcome:

- `expected a trap, but none occurred` — one-argument form, no trap.
- `expected a trap with code <code>, but none occurred` — two-argument form, no
  trap at all.
- `expected trap code <code>, got <actual>` — two-argument form, a trap
  occurred but carried a different code.

`77069001` is a reserved internal code the synthesized `mfb test` driver
recognizes, so the failure is reported as a test failure and not as a crash. The
raise unwinds out of the enclosing `TCASE`, so statements after the failed
assertion in that case do not run, while sibling cases and groups still run to
completion. [[src/testing/desugar/driver.rs:assertion_detail]]

## Overloads

**`expectTrap(expression)`**

Passes iff evaluating `expression` traps, whatever the error code. Use this when
the fact of the failure is what the test is pinning down.

**`expectTrap(expression, code AS Integer)`**

Passes iff evaluating `expression` traps **and** the raised `error.code` equals
`code`. Use this to distinguish one failure mode from another — for example to
prove a parser rejects malformed input with `ErrInvalidFormat` rather than
falling over with some unrelated error. [[src/builtins/testing.rs:expect_arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `expression` | any call | The call to evaluate under a trap guard, expected to trap. Must be a call expression and not a package constant; its result type is unconstrained and its value is discarded. [[src/syntaxcheck/inference.rs:check_trap_guardable]] |
| `code` | `Integer` | Optional. The error code the trap must carry, compared against the trapped `error.code`. Any `Integer` expression; typically an `errorCode::` constant. [[src/syntaxcheck/inference.rs:check_expect_call]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `expectTrap` is a statement-level assertion and yields no value. [[src/syntaxcheck/inference.rs:check_expect_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77069001` | *(reserved; deliberately absent from the errorCode:: registry)* | The assertion failed: `expression` did not trap, or (two-argument form) it trapped with a code other than `code`. The error carries the corresponding detail and is recognized by the `mfb test` driver as an assertion failure. [[src/builtins/testing.rs:TEST_ABORT_CODE]] |

The trap raised by `expression` itself is *not* propagated — that is the passing
outcome, and the guard's handler `RECOVER`s from it. Only the assertion's own
reserved-code failure escapes the expansion.
[[src/testing/desugar/expect.rs:expand_trap]]

Compile-time rejections, which never reach runtime:
`TESTING_EXPECT_ARITY` (`2-208-0002`) for any argument count outside one to two,
`TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE` (`2-208-0006`) for a non-call or
package-constant `expression`, and `TESTING_EXPECT_CODE_TYPE` (`2-208-0005`) for
a `code` argument that is not `Integer`-compatible.

## Examples

Assert that a parse failure traps, and that it traps with the right code:

```
IMPORT errorCode

FUNC parseIt(s AS String) AS Integer
  RETURN toInt(s)
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC

TESTING
  TGROUP "traps"
    TCASE "expectTrap passes"
      expectTrap(parseIt("bad"))
    END TCASE
    TCASE "expectTrap with matching code passes"
      expectTrap(parseIt("bad"), errorCode::ErrInvalidFormat)
    END TCASE
  END TGROUP
END TESTING
```

Trap-guard a built-in member call — an out-of-range `collections::get`, and a
callback that fails partway through a `reduce`:

```
IMPORT collections

FUNC boomReduce(acc AS Integer, v AS Integer) AS Integer
  IF v = 2 THEN FAIL error(5, "reduce boom")
  RETURN acc + v
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC

TESTING
  TGROUP "trap parity"
    TCASE "expectTrap on out-of-range get"
      LET xs AS List OF Integer = [1, 2, 3]
      expectTrap(collections::get(xs, 99))
    END TCASE
    TCASE "expectTrap with matching code on failing reduce"
      LET xs AS List OF Integer = [1, 2, 3]
      expectTrap(collections::reduce(xs, 0, boomReduce), 5)
    END TCASE
  END TGROUP
END TESTING
```

## See also

- `mfb man testing expectNTrap`
- `mfb man testing expectEqual`
- `mfb man general error`
- `mfb spec language test-framework`
- `mfb spec language error-model`
