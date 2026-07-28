# bug-98 — builtins LOW cluster: qualified type accepts any package pairing; `resolve_replace_list` indexes before length check

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G4). Two independent LOW
findings in `src/builtins/`, batched per goal-02.

## 1. `qualified_builtin_type` accepts any (builtin-package, builtin-type) cross pairing

`src/builtins/mod.rs:87-97` — the function checks `is_builtin_import(package)`
and `is_builtin_type(member)` independently, never that the type belongs to
that package. It is wired into the parser (`ast/expr.rs:294,695`) and
syntaxcheck (`types.rs:19`), so a wrong-package qualification resolves
silently to the real type.

Trigger: `LET x AS csv.Thread = ...` or `LET u AS io.Url = ...` compiles,
treating the annotation as `Thread`/`Url` — an invalid program (wrong package,
possibly not even imported) is accepted rather than diagnosed. No valid
program miscompiles.

Fix: key the check on a (package → types) table — the module's own doc comment
("net.Url, http.Response") already names the intended pairs.

## 2. `resolve_replace_list` indexes `arg_types[0]` before its length check

`src/builtins/general.rs:360-367` — `list_element(&arg_types[0])` runs before
the `arg_types.len() == 3` test, so an empty slice panics with
index-out-of-bounds. Its siblings (`resolve_find_list`, `resolve_mid_list`,
`resolve_get`, …) all length-check first.

Unreachable today: all four `collections::resolve_call` callers
(syntaxcheck/builtins.rs:1532, ir/verify/mod.rs:3308, ir/lower.rs,
type_utils.rs) reject arity ≠ 3 first. Latent: any new caller or arity-table
drift turns `collections.replace()` with zero args into a compiler panic
instead of a diagnostic.

Fix: hoist the length check above the indexing, matching the siblings.
