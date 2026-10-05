# bug-840: language spec §4/§8 cite drifted `table.rs` line numbers and a deleted `resolve_type_name`; the §4.2 `Node` example uses the keyword `next` as a field name

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (spec/doc accuracy)

Status: Open
Regression Test: none (doc-only; `mfb spec` citation sweep)

**Update (2026-10-05):** **Re-checked at HEAD 846aa55fd after plan-165/167.** The five line citations
(`table.rs:642`, `:480`, `:486`, `:312`, `:318`) are still in `04_types.md` / `08_error-model.md`.
They now land on bare `},` / `Rule {` lines and name no rule at all. The `next` keyword example
is unchanged. The bug stands. Use the symbol citation `[[src/rules/table.rs:RULES]]`.

There are three spec defects in the `language` topics. All three are SPEC bugs; the code
is correct.

1. **Line-number citations into `src/rules/table.rs` point at the wrong rules.** Each one
   names a line that today holds a different rule:

   | Spec site | Citation | Line now holds | Rule actually at |
   |---|---|---|---|
   | 04_types.md §4.2 (recursive record) | `[[src/rules/table.rs:642]]` | `TYPE_SET_ELEMENT_MISMATCH` (2-203-0054) | `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION` @ 722 |
   | 04_types.md §4.4 (read-only Error) | `[[src/rules/table.rs:480]]` | `TYPE_DEFAULT_VALUE_MISMATCH` | `TYPE_READ_ONLY_RECORD_UPDATE` @ 543 |
   | 04_types.md §4.4 | `[[src/rules/table.rs:486]]` | `TYPE_DUPLICATE_ENUM_MEMBER` | `TYPE_READ_ONLY_RECORD_CONSTRUCTOR` @ 549 |
   | 08_error-model.md rule 4 | `[[src/rules/table.rs:312]]` | `SYMBOL_LAMBDA_PARAMETER_DEFAULT` | `TYPE_PROPAGATE_REQUIRES_TRAP` @ 381 |
   | 08_error-model.md rule 4 | `[[src/rules/table.rs:318]]` | `SYMBOL_SHADOWS_TOP_LEVEL_BINDING` | `TYPE_FAIL_REQUIRES_ERROR` @ 375 |

   Measured with `sed -n '<line-6>,<line+1>p' src/rules/table.rs` and
   `grep -n 'name: "TYPE_READ_ONLY_RECORD\|TYPE_RECURSIVE_RECORD\|TYPE_FAIL_REQUIRES\|TYPE_PROPAGATE_REQUIRES' src/rules/table.rs`
   at HEAD 031f64353. Line citations drift whenever a rule is inserted above them. The
   same section already uses the stable form `[[src/rules/table.rs:RULES]]` (§4.2
   field-assignment paragraph). These are the only three spec lines tree-wide that cite
   `table.rs:<number>` (`grep -rn "table.rs:[0-9]" src/docs/spec | wc -l` returns 3).

2. **Dangling symbol.** 08_error-model.md §8.5b cites
   `[[src/resolver/resolution.rs:resolve_type_name]]`. That function no longer exists.
   `src/resolver/resolution.rs` documents above `fn resolve_type` that it "replaces
   `resolve_type_name(&str)`". The `TYPE_RESULT_NOT_USER_VISIBLE` emit is at
   `resolution.rs:1649`, inside the `resolve_type` path. `grep -rn "fn resolve_type_name" src`
   finds only an unrelated test in `src/codegen/builtins/general/mod.rs`.

3. **The §4.2 rejected-record example does not show the documented diagnostic.**
   04_types.md §4.2 says the `Node` example is "rejected: `next` always demands another
   Node" with `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION`. But `next` is the `NEXT`
   keyword, so the example actually fails to parse:
   `error[1-102-0003 MFB_PARSE_INVALID_IDENTIFIER]: identifier is invalid / Field name must be an identifier.`
   Renaming the field (`nxt AS Node`) yields the documented
   `error[2-203-0065 TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION]`.

References:

- `src/docs/spec/language/04_types.md` §4.2, §4.4; `src/docs/spec/language/08_error-model.md` §8.5b, §8.6 rule 4.
- `.ai/specifications.md` (citation rules).

## Failing Reproduction

```basic
IMPORT io
TYPE Node
  value AS Integer
  next  AS Node
END TYPE
FUNC main() AS Nothing
  io::print("ok")
END FUNC
```
- Observed: `error[1-102-0003 MFB_PARSE_INVALID_IDENTIFIER]` … `Field name must be an identifier.`
- Expected (per spec text): `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION`.

The citations are verified with the commands above.

## Root Cause

Spec drift. Rules were inserted into `RULES` above the cited lines, `resolve_type_name`
was replaced by `resolve_type` (plan-105-B), and the example predates `NEXT` reservation
or was never compiled.

## Goal

- Replace the five line citations with `[[src/rules/table.rs:RULES]]` (or with the
  emitting function, e.g. the verifier site that reports each rule).
- Repoint §8.5b at `[[src/resolver/resolution.rs:resolve_type]]`.
- Rename the example field (e.g. `child AS Node`) so the example produces the diagnostic
  it documents.

### Non-goals

- No code change.

## Validation Plan

- `mfb spec language types` / `error-model` render. Compile the corrected example and
  confirm 2-203-0065.

## Summary

These are doc-only edits.
