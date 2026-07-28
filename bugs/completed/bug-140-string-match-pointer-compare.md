# bug-140 — String MATCH patterns compare pointers, never bytes → every string CASE is dead

**Status:** FIXED (commit e0fa88b8, 2026-07-11).
**Severity:** HIGH — `MATCH` on a String silently never matches any string CASE;
control always falls to CASE ELSE.
**Class:** correctness.

## Finding

`src/target/shared/code/builder_value_semantics.rs:768-772`
(`lower_match_compare`, fallback arm). The literal-pattern fallback emits
`compare_registers(matched.location, pattern.location)`. For `String`
scrutinees these are block pointers, so equality is pointer identity, not
content. The spec (src/docs/spec/language/09_pattern-matching.md:28) explicitly
allows string literal patterns.

## Trigger (reproduced)

```
MATCH s
  CASE "abc"
    ...
  CASE ELSE
    ...
```
A probe returning `pick(s)` printed `no-match` for both a concatenation-built
`"abc"` AND a literal `"abc"` argument (parameter deep-copy changes the
pointer). Every string CASE arm is dead. No test in tests/ exercises `MATCH
<String>` with a string CASE.

## Fix

In `lower_match_compare`, dispatch String (and other block-typed) patterns to a
byte-content comparison (the `emit_compare_bytes`/string-equality helper), not
`compare_registers`.

## Resolution

FIXED in commit e0fa88b8. lower_match_compare routes String/record scrutinees through emit_comparable_values_match_branch (byte compare).

Regression test: `tests/rt-behavior/control-flow/bug140_string_match_content` (fails on the unfixed compiler). Full
acceptance (871) and `cargo test` pass.
