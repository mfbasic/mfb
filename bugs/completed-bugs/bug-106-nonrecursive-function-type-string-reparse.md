# bug-106 — Function-type string re-parsers in resolver/syntaxcheck aren't nesting-aware → nested function-typed params rejected

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G2). **Reproduced with the
binary.**
**Severity:** MED — valid higher-order types rejected with no workaround.
**Class:** correctness.

## Finding

`src/resolver/resolution.rs:1287-1302` (`resolve_function_type_name` — leftmost
`split_once(") AS ")` + `params.split(", ")`); resolution.rs:1255-1261
(template-arg `args.split(", ")`, same flaw); `src/syntaxcheck/types.rs:76-93`
(`parse_function_type`, same flaw — latent, degrades to `Type::Unknown`).

A `FUNC(...) AS R` whose parameter list itself contains a `FUNC(...) AS …`
splits at the **inner** `") AS "`, producing garbage names. The resolver then
rejects a valid higher-order type; there is no workaround (parenthesizing the
inner type also mis-splits, and bug-105 blocks it anyway). The depth-aware
helper already exists (`builtins::split_func_params_and_return`, the bug-26/35
fix) but these callers don't use it. The syntaxcheck copy is currently
unreachable-for-error only because the resolver rejects first.

## Trigger (reproduced)

`FUNC apply(f AS FUNC(FUNC(Integer) AS Integer, Integer) AS Integer) AS
Integer`
→ `SYMBOL_UNKNOWN_TYPE: Function type 'FUNC(Integer' is malformed.` + `Type
'Integer, Integer) AS Integer' is not a built-in…`

The parser accepts the type (ast/expr.rs:605-631 builds it recursively).

## Fix sketch

Replace the leftmost-split logic in both resolver sites and the syntaxcheck
copy with the depth-aware `split_func_params_and_return` (bug-26/35 helper).

## Prior art

Same class as fixed bug-26 (builtins/general.rs) and bug-35 (monomorph) — but
these are different, still-broken sites not covered by those docs.
