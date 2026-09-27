# bug-642: a type error inside a source-generic builtin (`collections::sort`) is reported at a line that does not exist in the user's file

Last updated: 2026-09-27
Effort: medium
Severity: MEDIUM
Class: Diagnostics (wrong location, no call-site attribution)

Status: Fixed (see STATUS below)
Regression Test: `tests/syntax/collections/bug642-generic-builtin-error-at-call-invalid`;
unit test `registry::tests::public_member_for_rewrite_names_the_member_a_user_calls`

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

## Root cause (confirmed 2026-09-27)

Reproduced at `a0082e4ae` exactly as described (`main.mfb:43` for an 8-line file),
and for the bug-744 lane's `List OF Nothing` (`got Nothing and Nothing`, same line).
The hypothesis holds, with one detail sharpened: monomorphization emits every
instantiation into the project's FIRST source file
(`Monomorphizer::into_project` → `first_file.items`), so `IrFunction::file` names
that file while the body's statement lines stay the package source's. `ir::verify`
reports at `(function.file, statement line)` and nothing threaded the user's call
through. The same holds for `ir::shape` and for the monomorphizer's own `report`
inside a body (which used the internal file's path).

Census of the `Body::Mfb` generic members whose body applies a type-dependent
rule, all reproduced at base with a nonexistent line: `sort` (`<`, line 43),
`sortBy` (key `<`, line 98), `distinct` (`TYPE_REQUIRES_COMPARABLE` naming
`collections.contains`, line 293). The call-level messages `instantiate_function`
reports (arity, inference) were at the right line but named the internal
`__collections_sort`.

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

- [x] Phase 1 — RED test: `tests/syntax/collections/bug642-generic-builtin-error-at-call-invalid`
(`sort` over `Boolean` and over `Nothing`, a second `sort` call reusing the
instantiation, `sortBy` with a `Boolean` key, `distinct` over `FUNC`s); at
`a0082e4ae` it reports lines 43/43/98/293 and only one `sort` site. Commit: see
Phase 2 (one commit).

- [x] Phase 2 — the monomorphizer records every user call that reaches an
internal-origin instantiation (`HirFunction::instantiated_at`, inherited by nested
instantiations), and `report`, `ir::shape::emit` and `ir::verify::emit` (via
`LinkSpans::instantiation_sites`) report once per call site, at its line, with
"In the body of `collections::sort`, instantiated by this call:". The member is
spelled as the user wrote it (`registry::public_member_for_rewrite`), also in
`instantiate_function`'s arity/inference messages. `sort`/`sortBy` man pages now say
the error is reported at the call; spec architecture monomorphization documents it.
Commit: COMMIT_642
