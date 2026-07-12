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
      expectInteger(add(2, 3), 5)
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
Group  := TGROUP <string> NEWLINE Member* END TGROUP
Member := Case | Group
Case   := TCASE  <string> NEWLINE Statement* END TCASE
```

- A `TESTING` block appears anywhere a top-level declaration may appear, in any
  source file, any number of times. Blocks run in declaration order across files.
- A `TGROUP` contains `TCASE` cases and/or nested `TGROUP` sub-groups, in
  declaration order; nesting may be **arbitrarily deep** and cases and sub-groups
  may interleave within one group. A `TCASE` body is an ordinary statement block.
  Both `TGROUP` and `TCASE` take a **string-literal** description used verbatim in
  the report.
- A `TGROUP` with no `TCASE` anywhere in its subtree emits nothing in the report;
  cases still run and report in declaration order regardless of nesting depth.
- Only `TESTING` is a reserved keyword. `TGROUP` and `TCASE` are contextual — they
  are recognized only inside a `TESTING` block, so existing programs that use them
  as identifiers are unaffected.

## Assertion builtins

The assertion builtins are valid only inside a `TCASE` body (using one
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
  failure message: a scalar, `String`, `Byte`, or `List OF Byte` (`TESTING_EXPECT_NOT_PRINTABLE`).
  `Integer` and `Float` compare numerically.
- The **typed** forms (`expectFloat`, `expectInteger`, `expectFixed`,
  `expectString`, and their `expectN…` counterparts) additionally require both
  operands to be exactly the named type (`TESTING_EXPECT_TYPE_MISMATCH`) — an
  exact type-and-value check that needs no `toString`.
- `expectTrap`/`expectNTrap` evaluate their argument under a trap guard built on
  the inline-`TRAP` machinery, so they accept **any call** it accepts — the same
  uniform surface (§8.6): infallible built-ins, the index/range members, the
  callback members, and user `FUNC`/`SUB` calls. The only rejection is a scrutinee
  with no runtime call to trap — a non-call or a package constant
  (`TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE`). On an infallible callee the assertion
  simply evaluates against the real outcome: `expectTrap` always fails at runtime
  (no trap occurs) and `expectNTrap` always passes — exactly as for an infallible
  user `FUNC`.
- A failed assertion raises the reserved internal error code `77069001`
  (`7-706-9001`, trap/failure subsystem); it is not part of the `errorCode::`
  registry and is recognized only by the driver, which uses it to distinguish an
  assertion failure from a genuine runtime error.

## Running (`mfb test`)

`mfb test [path] [--coverage]` compiles the project with its `TESTING` blocks
retained, replaces the normal entry point with a synthesized driver, builds a
host executable, runs it, and adopts its exit status. The driver streams a tree
that indents two columns per nesting level, so a nested `TGROUP` sits under its
parent and its cases sit under it:

```
* <group description>
  * [P] <case description>
  * [F] <case description>
    X <detail>  (<file>:<line>)
  * <nested group description>
    * [P] <case description>
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
