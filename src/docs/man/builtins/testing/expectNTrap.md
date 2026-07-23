# expectNTrap

Assert that evaluating an expression does not trap

## Synopsis

```
expectNTrap(expression)
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

`expectNTrap` is the success-path counterpart of `expectTrap`. It evaluates
`expression` under a trap guard and passes when the evaluation completes without
trapping. The expression's value is discarded — only the absence of a trap
matters. It is spelled bare, with no `testing::` qualifier, because it is a
compiler-recognized builtin rather than a package function.
[[src/builtins/testing.rs:EXPECT_NTRAP]]

The trap guard is the ordinary inline-`TRAP` machinery, so `expectNTrap` accepts
exactly what an inline `TRAP` accepts — the same uniform surface: user
`FUNC`/`SUB` calls, package functions, infallible built-ins such as `len`, the
index and range members, and the callback members like
`collections::transform`. There is exactly one rejection: a scrutinee with no
runtime call to trap, meaning a non-call expression or a package constant. That
is `TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE` (`2-208-0006`). An *infallible* call
is deliberately **not** rejected — `expectNTrap(len(xs))` compiles and passes,
which makes the assertion usable as a regression guard on a call that is
expected to stay infallible. [[src/syntaxcheck/inference.rs:check_trap_guardable]]

Unlike `expectTrap`, `expectNTrap` takes exactly one argument: there is no
expected-code form, because a passing case has no error to inspect. Any other
argument count is `TESTING_EXPECT_ARITY` (`2-208-0002`).
[[src/builtins/testing.rs:expect_arity]]

`expectNTrap` is a statement-level assertion: it produces `Nothing` and cannot be
used as a subexpression. The compiler expands it in place — there is no runtime
helper — into a single inline `TRAP` around `expression` whose handler is itself
the failure path. No trap means the guard falls through and the assertion passes;
the expansion is therefore leaner than `expectTrap`'s, with no flag and no
post-guard `IF`. Because the handler diverges by raising, it needs no `RECOVER`.
[[src/testing/desugar/expect.rs:expand_ntrap]]

On failure — that is, when `expression` did trap — the handler raises
`error(77069001, "unexpected trap: <message>")`, where `<message>` is the
trapped error's own message, so the underlying failure is visible in the report.
`77069001` is a reserved internal code the synthesized `mfb test` driver
recognizes, so the failure is reported as a test failure and not as a crash. Note
that the original error's `code` is not carried through; the reported code is the
reserved one and the original detail survives only as text. The raise unwinds out
of the enclosing `TCASE`, so statements after the failed assertion in that case do
not run, while sibling cases and groups still run to completion.
[[src/testing/desugar/driver.rs:assertion_detail]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `expression` | any call | The call to evaluate under a trap guard, expected to succeed. Must be a call expression and not a package constant; its result type is unconstrained and its value is discarded. [[src/syntaxcheck/inference.rs:check_trap_guardable]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `expectNTrap` is a statement-level assertion and yields no value. [[src/syntaxcheck/inference.rs:check_expect_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77069001` | *(reserved; deliberately absent from the errorCode:: registry)* | The assertion failed: evaluating `expression` trapped. The error carries the detail `unexpected trap: <message>` and is recognized by the `mfb test` driver as an assertion failure. [[src/builtins/testing.rs:TEST_ABORT_CODE]] |

The trap raised by `expression` is not propagated as itself: the guard's handler
intercepts it and replaces it with the reserved-code assertion failure carrying
its message. [[src/testing/desugar/expect.rs:expand_ntrap]]

Compile-time rejections, which never reach runtime: `TESTING_EXPECT_ARITY`
(`2-208-0002`) for any argument count other than one, and
`TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE` (`2-208-0006`) for a non-call or
package-constant `expression`.

## Examples

Assert that valid input parses without trapping:

```
FUNC parseIt(s AS String) AS Integer
  RETURN toInt(s)
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC

TESTING
  TGROUP "traps"
    TCASE "expectNTrap passes"
      expectNTrap(parseIt("42"))
    END TCASE
    TCASE "expectNTrap on a trapping call fails"
      expectNTrap(parseIt("bad"))
    END TCASE
  END TGROUP
END TESTING
```

Guard an infallible built-in and a succeeding callback member — both compile and
both pass:

```
IMPORT collections

FUNC ok(v AS Integer) AS Integer
  RETURN v * 2
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC

TESTING
  TGROUP "trap parity"
    TCASE "expectNTrap on infallible len (never traps)"
      LET xs AS List OF Integer = [1, 2, 3]
      expectNTrap(len(xs))
    END TCASE
    TCASE "expectNTrap on succeeding transform callback"
      LET xs AS List OF Integer = [1, 2, 3]
      expectNTrap(collections::transform(xs, ok))
    END TCASE
  END TGROUP
END TESTING
```

## See also

- `mfb man testing expectTrap`
- `mfb man testing expectEqual`
- `mfb man general error`
- `mfb spec language test-framework`
- `mfb spec language error-model`
