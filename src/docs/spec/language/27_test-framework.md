# 27. Test Framework (`TESTING` blocks + `mfb test`)

A `TESTING … END TESTING` block declares tests for the program it lives beside.
Under an ordinary `mfb build` the blocks are **dropped before codegen** — the
emitted binary is byte-identical to one whose blocks were deleted, so tests never
affect a release build. Under `mfb test` the blocks are compiled into a runnable
driver that executes every case, streams a pass/fail tree, and exits non-zero iff
any case failed.

```basic
TESTING
  TGROUP "arithmetic"
    TCASE "adds two numbers"
      expectEQ(add(2, 3), 5)
    END TCASE
    TCASE "parsing bad input traps"
      expectTrap(parseInt("nope"))
    END TCASE
  END TGROUP
END TESTING
```

## Structure

```
TESTING NEWLINE Group* END TESTING
Group := TGROUP <string> NEWLINE Case* END TGROUP
Case  := TCASE  <string> NEWLINE Statement* END TCASE
```

- A `TESTING` block appears anywhere a top-level declaration may appear, in any
  source file, any number of times. Blocks run in declaration order across files.
- A `TGROUP` contains only `TCASE` cases; a `TCASE` body is an ordinary statement
  block. Both take a **string-literal** description used verbatim in the report.
- Only `TESTING` is a reserved keyword. `TGROUP` and `TCASE` are contextual — they
  are recognized only inside a `TESTING` block, so existing programs that use them
  as identifiers are unaffected.

## Assertion builtins

Four compiler-lowered builtins are valid only inside a `TCASE` body (using one
elsewhere is `TESTING_EXPECT_OUTSIDE_TCASE`). Each produces `Nothing`; the first
failed assertion aborts its case, and sibling cases and groups continue.

| Builtin | Passes iff |
| --- | --- |
| `expectEqual(actual, expected)` | `actual = expected` (generic) |
| `expectNEqual(actual, expected)` | `actual <> expected` (generic) |
| `expectFloat`/`expectInteger`/`expectFixed`/`expectString`(a, b) | both are that type **and** `a = b` |
| `expectNFloat`/`expectNInteger`/`expectNFixed`/`expectNString`(a, b) | both are that type **and** `a <> b` |
| `expectTrap(expr)` | evaluating `expr` traps |
| `expectTrap(expr, code)` | evaluating `expr` traps **and** its `error.code = code` |
| `expectNTrap(expr)` | evaluating `expr` does **not** trap |

- `expectEqual`/`expectNEqual` reuse the language `=`/`<>` operators — the operands
  must be comparable with `=` (`TESTING_EXPECT_INCOMPARABLE`) and printable for the
  failure message: a scalar, `String`, or `Byte` (`TESTING_EXPECT_NOT_PRINTABLE`).
  `Integer` and `Float` compare numerically.
- The **typed** forms (`expectFloat`, `expectInteger`, `expectFixed`,
  `expectString`, and their `expectN…` counterparts) additionally require both
  operands to be exactly the named type (`TESTING_EXPECT_TYPE_MISMATCH`) — an
  exact type-and-value check that needs no `toString`.
- `expectTrap`/`expectNTrap` evaluate their argument under a trap guard, so the
  argument must be a genuinely-fallible call — the same constraint as an inline
  `TRAP` (§8.6). A non-call, an infallible call, or an inline-compiled builtin is
  rejected (`TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE` /
  `TESTING_EXPECT_TRAP_INLINE_BUILTIN`); wrap such an operation in a `FUNC`/`SUB`
  and pass that call instead.
- A failed assertion raises the reserved internal error code `77069001`
  (`7-706-9001`, trap/failure subsystem); it is not part of the `errorCode::`
  registry and is recognized only by the driver, which uses it to distinguish an
  assertion failure from a genuine runtime error.

## Running (`mfb test`)

`mfb test [path] [--coverage]` compiles the project with its `TESTING` blocks
retained, replaces the normal entry point with a synthesized driver, builds a
host executable, runs it, and adopts its exit status. The driver streams:

```
* <group description>
  * [P] <case description>
  * [F] <case description>
    X <detail>  (<file>:<line>)
...

Tests: <total>  Pass: <passed>  Fail: <failed>
```

- A case is `[P]` iff it ran with no failed assertion and no runtime trap.
- The failure `<detail>` reports the kind: an assertion mismatch
  (`expected …, got …`), a missing/unexpected trap, a trap-code mismatch, or a
  genuine `runtime error [<code>] <message>`. The `(<file>:<line>)` is the error's
  stamped origin (`Error.source`).
- A genuine runtime error inside a case is reported as a failure — not a crash —
  and the remaining cases still run.
- The process exits `0` iff `Fail: 0`, so `mfb test` is usable in CI.

## Coverage (`mfb test --coverage`)

`mfb test --coverage` additionally instruments the program's own statements,
writes the exercised hit counts at exit, and emits `coverage.html` — a file tree
with per-file line-coverage stats and color-coded, annotated source. Coverage is
gated entirely behind `--coverage`; it never affects a normal build or a plain
`mfb test`.

## See Also

- `./mfb spec diagnostics rule-codes` — the `TESTING_*` and
  `MFB_PARSE_TESTING_*` diagnostics.
- `./mfb man testing` — the assertion builtins.
- §8 `error-model` — the trap machinery the assertions reuse.
