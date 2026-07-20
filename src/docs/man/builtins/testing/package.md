# testing

Assertion builtins for the built-in test framework (`TESTING` blocks + `mfb test`)

## Synopsis

```
expectEqual(actual, expected)     expectNEqual(actual, expected)
expectFloat(actual, expected)     expectNFloat(actual, expected)
expectInteger(actual, expected)   expectNInteger(actual, expected)
expectFixed(actual, expected)     expectNFixed(actual, expected)
expectString(actual, expected)    expectNString(actual, expected)
expectTrap(expression)            expectNTrap(expression)
expectTrap(expression, code)
```

## Imports

The assertion builtins are always in scope inside a `TCASE` body and need no
`IMPORT` statement. They are valid **only** inside a `TCASE`; using one anywhere
else is a compile error (`TESTING_EXPECT_OUTSIDE_TCASE`).
[[src/builtins/testing.rs:is_expect_call]]

## Description

The `testing` builtins are the assertions of the built-in test framework. They
appear inside the `TCASE` bodies of a `TESTING … END TESTING` block and are
compiler-lowered — there is no runtime helper. Each produces `Nothing`; the first
failed assertion aborts its case (sibling cases and groups still run).

`expectEqual`/`expectNEqual` reuse the language `=`/`<>` operators, so their
operands must be comparable with `=` and printable (a scalar, `String`, or
`Byte`) for the failure message; `Integer` and `Float` compare numerically. The
typed forms `expectFloat`/`expectInteger`/`expectFixed`/`expectString` (and their
`expectN…` inequality counterparts) additionally require both operands to be
exactly the named type — an exact type-and-value check that needs no `toString`.
`expectTrap`/`expectNTrap` evaluate their argument under a trap guard, so the
argument must be a genuinely-fallible call — the same constraint as an inline
`TRAP`.

A failed assertion raises a reserved internal error the `mfb test` driver
recognizes, so it is reported as a test failure rather than a crash, and is
distinguished from a genuine runtime error inside the case.

Run tests with `mfb test`, which streams a pass/fail tree and exits non-zero iff
any case failed. See `./mfb spec language test-framework` for the full model.

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77069001` | *(reserved; unnamed)* | an assertion fails. The desugaring emits `FAIL error(77069001, <detail>)`, so a failed expectation is an ordinary raised error carrying the failure detail. [[src/builtins/testing.rs:TEST_ABORT_CODE]] [[src/testing/desugar.rs:fail_test]] |

The code sits in the `7-706-*` trap/failure subsystem but is deliberately absent
from the `errorCode::` registry, so user code can neither name it nor collide
with it short of a deliberate `FAIL error(77069001, …)`. That is what lets the
synthesized driver tell an assertion failure from a genuine runtime error: it
wraps each case in a `TRAP` and branches on this code.

`expectTrap` and `expectNTrap` additionally *catch* an error raised by the
expression under test — that is their whole job — and then raise the code above
if the outcome was not the expected one.
