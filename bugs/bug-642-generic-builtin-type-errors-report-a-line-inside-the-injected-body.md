# bug-642: a type error inside a source-generic builtin (`collections::sort`) is reported at a line that does not exist in the user's file

Last updated: 2026-09-16
Effort: medium
Severity: MEDIUM
Class: Diagnostics (wrong location, no call-site attribution)

Status: Open
Regression Test: none yet — see Phase 1

`collections::sort` is a source-generic member: its MFBASIC body is injected into
the project and instantiated per element type. When the element type does not
support `<` (for example `Boolean`), the build fails with a diagnostic that points
at a line **inside the injected body**, but attributes it to the user's file:

```
Building init_colsortb (executable) for macos-aarch64
   6 |   LET sorted = collections::sort(flags)
   7 |   io::print(toString(len(sorted)))
   8 | END SUB
colsortb/src/main.mfb:43 error[2-203-0001 TYPE_BINARY_OPERATOR_MISMATCH]: binary operator operands have incompatible types
               Operator `<` requires numeric or String operands, got Boolean and Boolean.
```

The user's `main.mfb` is 8 lines long. Line 43 does not exist, no caret marks the
excerpt, and the message never names `sort` or the call on line 6. A developer
must guess which call in their program caused it. The same shape applies to
`collections::sortBy` with a non-orderable key type (its man page says the error is
"reported against the key comparison inside the merge").

**The single correct behavior a fix produces:** a type error that arises while
instantiating a source-generic builtin for a user's call is reported at that
call's location in the user's file (`main.mfb:6` here), and names the builtin and
the unsatisfied requirement (for example: "`collections::sort` requires an element
type that `<` accepts; got `Boolean`"). No diagnostic cites a line number that is
not in the named file.

Found by plan-125-C Phase 4 while verifying the `sort` man page's claim about this
diagnostic (`/tmp/p125-ex/colsortb`). **Filed, not fixed**, by user instruction
during a documentation-only plan ("file all bugs, make no fixes"). The `sort` and
`sortBy` pages currently describe this as-is.

## Reproduction

```basic
IMPORT io
IMPORT collections

SUB main()
  LET flags AS List OF Boolean = [TRUE, FALSE]
  LET sorted = collections::sort(flags)
  io::print(toString(len(sorted)))
END SUB
```

Observed (macos-aarch64, `worktree-P-125`): the diagnostic above, at
`main.mfb:43`.
Expected: an error at `main.mfb:6` naming `collections::sort` and the element-type
requirement.

## Root cause

Hypothesis, to confirm in Phase 1. The injected generic body is parsed as part of
the project (the `collections` package's source is injected at parse time so the
monomorphizer can instantiate it; see `src/codegen/builtins/collections/mod.rs`).
The binary-operator check runs on the instantiated body, and its span carries the
body's own line number while the diagnostic file is taken from the caller's
module. The instantiation site (the user's call) is not threaded through to the
check, so there is nothing better to report.

Confirm by locating where `TYPE_BINARY_OPERATOR_MISMATCH` is emitted for an
instantiated generic, and what span and file it receives.

## Non-goals

- Changing which element types `sort`/`sortBy` accept.
- Suppressing the error: the call must still fail at build time.
- Rewording the operator message alone. The location is the defect; a clearer
  message at a nonexistent line would not fix it.

## Blast-radius audit

- `collections::sortBy` (non-orderable key), and any other source-generic member
  whose body applies an operator or call that depends on the instantiated type:
  census every `Body::Mfb` generic member in `src/codegen/builtins/`.
- Diagnostics other than `TYPE_BINARY_OPERATOR_MISMATCH` raised inside an
  instantiated generic body (comparability, call-argument mismatches).
- Compare with `collections::distinct` on a non-comparable element type, whose
  diagnostic names `collections.contains` (plan-125-C Phase 4 probe
  `/tmp/p125-ex/coldist`), to see whether it has the same location problem.

## Fix

Phase 1 — RED test: the reproduction above expects the error at `main.mfb:6`
naming `collections::sort`. Run the census. Commit:

Phase 2 — thread the instantiation call site into diagnostics raised while
checking an instantiated generic body, and word them against the builtin (GREEN);
full suite and diagnostic goldens. Update the `sort` and `sortBy` man pages, which
document the current location. Commit:
