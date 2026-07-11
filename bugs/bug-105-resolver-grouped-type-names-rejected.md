# bug-105 — Resolver rejects parenthesized (grouped) type names outside thread types

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G2). **Reproduced with the
binary.**
**Severity:** MED — valid grouping syntax rejected as an unknown type.
**Class:** correctness.

## Finding

`src/resolver/resolution.rs:1193-1278` (`resolve_type_name`) has no
`strip_type_group` / paren handling. The parser explicitly supports grouped
types (`(T)` — ast/expr.rs:483-487) and emits them verbatim into type strings.
`thread_parts_full` strips the group for thread slots, and syntaxcheck's
`parse_type` strips it at top level, but `resolve_type_name` never does — so any
grouped type outside a thread position falls through to the bare-name arm and is
rejected as unknown.

## Trigger (reproduced)

- `LET xs AS List OF (Map OF String TO Integer) = []`
  → `SYMBOL_UNKNOWN_TYPE: Type '(Map OF String TO Integer)' is not a
  built-in…`
- `LET y AS (Integer) = 1` → same.

The unparenthesized spellings resolve fine, so valid grouping syntax is the
only casualty.

## Fix sketch

Apply the existing `strip_type_group` at the top of `resolve_type_name` (and
recursively for element/key/value positions), matching what `parse_type` and
`thread_parts_full` already do.
