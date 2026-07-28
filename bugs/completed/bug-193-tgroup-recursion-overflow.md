# bug-193: nested TGROUP recurses without a depth cap → stack-overflow on deeply nested test groups

Last updated: 2026-07-14
Effort: small (<1h)
Severity: MEDIUM
Class: memory-safety (compile-time DoS)

Status: Fixed (2026-07-15) — `parse_test_group` is now a guarded wrapper around
`parse_test_group_inner` that enters the shared statement-block depth guard
(`enter_stmt`/`leave_stmt`, made `pub(super)`), so nested TGROUPs are bounded by the
same `MAX_STMT_DEPTH` cap as control flow: past the cap one bounded diagnostic
prints and the cursor collapses to `Eof` instead of overflowing the stack.
Regression Test: `tests/rt-error/parser_tgroup_depth` (300-deep TGROUP → single
`MFB_PARSE_BLOCK_TOO_DEEP`, exit 1).

`parse_test_group` self-recurses once per nested `TGROUP` with no depth bound, so
a deeply nested `TESTING` block overflows the native stack (same crash signature
as bug-183/bug-191: stack overflow, no diagnostic). TESTING blocks are parsed in
every mode, so this is reachable from any built/tested source.

## Failing Reproduction

```
TESTING
  TGROUP ""    (repeated N≈100k times)
    ...
  END TGROUP   (×N)
```
Observed: `fatal runtime error: stack overflow, aborting`. Expected: a bounded
parse diagnostic.

## Root Cause

`src/ast/testing.rs:53-54` `parse_test_group` recurses on nested groups with no
depth counter — a distinct unguarded recursive function not covered by bug-183's
statement-block scope.

## Non-goals

- Do not change accepted TESTING/TGROUP/TCASE syntax for valid inputs.

## Blast Radius

- `parse_test_group` only.

## Fix Design

Apply the same nesting-depth cap used for statements/expressions to
`parse_test_group` recursion and emit a bounded diagnostic when exceeded.
