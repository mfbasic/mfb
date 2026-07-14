# bug-89 — Parser infinite recursion → stack-overflow abort on `(`/`[` at EOF

**Status:** FIXED (commit e0fa88b8, 2026-07-11).
**Severity:** HIGH — compiler crash (SIGABRT, exit 134) on a ~25-byte user input.
**Class:** correctness / reachable crash on user input.

## Finding

`src/ast/parser.rs:256-261` — `FileParser::advance()` at `Eof` does not move
the cursor and returns `previous()`, i.e. the token *before* Eof, which was
already consumed. `parse_primary` (`src/ast/expr.rs:394-448`) then re-dispatches
on that stale token. If it is `(` (grouped-expression arm) or `[` (list-literal
arm), it calls `parse_expression` again with **zero cursor progress** →
unbounded mutual recursion → native stack overflow:

```
fatal runtime error: stack overflow, aborting   (exit 134)
```

No diagnostic is produced.

## Trigger (verified with target/debug/mfb)

A source file whose last bytes are an unclosed `(` or `[` with no trailing
newline:

```
FUNC main() AS Integer
LET a = (
```

Also reproduces with `foo(` (argument list), a bare `[` (list literal), and at
a top-level binding. Any editor crash/file truncation or hostile package source
kills the compiler outright.

## Distinct from audit-1 FE-01

FE-01 is input-proportional recursion depth (needs ~50k nested parens). This is
an **infinite** no-progress loop on a tiny input: the root cause is
`advance()`-at-Eof re-yielding the open delimiter, not missing depth limits.
FE-01's proposed depth guard would incidentally convert this into a diagnostic,
but the correct fix is to make `advance()`/`parse_primary` treat Eof as a hard
parse error (report + bail), never re-reading `previous()`.

## Fix sketch

In `parse_primary`'s LParen/LBracket arms (and any arm that recurses after
`advance()`), check `is_at_end()` before recursing, or make `advance()` at Eof
return the Eof token itself so the match falls into the error arm.

## Resolution

FIXED in commit e0fa88b8. parse_primary treats EOF as a hard MFB_PARSE_EXPECTED_EXPRESSION error instead of re-reading previous().

Regression test: `tests/syntax/lexical/bug89_parser_eof_open_delim` (fails on the unfixed compiler). Full
acceptance (871) and `cargo test` pass.
