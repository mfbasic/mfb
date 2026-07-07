# testing

Assertion builtins for the built-in test framework (`TESTING` blocks + `mfb test`)

## Synopsis

```
expectEQ(actual, expected)
expectNQ(actual, expected)
expectTrap(expression)
expectTrap(expression, code)
expectNTrap(expression)
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

`expectEQ`/`expectNQ` reuse the language `=`/`<>` operators, so their operands
must be comparable with `=` and printable (a scalar, `String`, `Byte`, or
`List OF Byte`) for the failure message. `expectTrap`/`expectNTrap` evaluate their
argument under a trap guard, so the argument must be a genuinely-fallible call —
the same constraint as an inline `TRAP`.

A failed assertion raises a reserved internal error the `mfb test` driver
recognizes, so it is reported as a test failure rather than a crash, and is
distinguished from a genuine runtime error inside the case.

Run tests with `mfb test`, which streams a pass/fail tree and exits non-zero iff
any case failed. See `./mfb spec language test-framework` for the full model.

## See Also

- `./mfb spec language test-framework`
- `./mfb man general error`
