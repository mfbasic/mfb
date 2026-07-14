# bug-103 — Monomorph can't type globals or builtin-call args → generic/overloaded calls with them falsely rejected

**Status:** FIXED (commit e0fa88b8, 2026-07-11).
**Severity:** HIGH — valid programs rejected with a misleading
"not a top-level function" / "cannot infer template arguments" error.
**Class:** correctness.

## Finding

`src/monomorph/lower.rs:950-953` (the `Expression::Call` arm `arg_types`
`filter_map`), lower.rs:1392-1432 (`function_context` — registers only project
functions, never top-level bindings), lower.rs:1521 (`expression_type` Call arm
— `function_returns` has no builtin/package members); `src/monomorph/mod.rs:59-69`
(`FunctionContext` has no globals table).

`expression_type` returns `None` for (a) an identifier naming a top-level
`LET`/`MUT` binding and (b) any call to a builtin/package function. The Call
arm then `filter_map`s these out of `arg_types`, **silently shifting the
remaining argument types left**. Template unification then zips misaligned
types (spurious `TYPE_CALL_ARGUMENT_MISMATCH: cannot infer template arguments
from …`), or the arg list is too short and instantiation/overload selection
returns `None`, leaving the callee as its bare name — which no longer exists
after mangling — so the post-monomorph resolver reports the nonsensical
`SYMBOL_UNKNOWN_IDENTIFIER: Callable X is not a top-level function`.

## Trigger (all reproduced with the compiled binary)

1. `LET G AS Integer = 7` + `show OF T(value AS T)` + `show(G)` → "Callable
   `show` is not a top-level function".
2. `show OF T(label AS String, value AS T)` + `show(NAME, 42)` (NAME a global
   String) → "cannot infer template arguments from `Integer`" (misaligned zip).
3. `show(math::abs(-7))` → "not a top-level function".
4. param-overloaded `pick(Integer)/pick(String)` + `pick(NAME)` → same.

All are valid per spec/03_templates.md (types inferred from explicit argument
types; the global carries an explicit `AS Integer`).

## Fix sketch

Give `FunctionContext` a globals table (name → declared type) populated from
top-level `LET`/`MUT`, and make `expression_type`'s Call arm consult the
builtin/package return-type resolver (the same one syntaxcheck uses) so builtin
calls yield a type. Never `filter_map` unknown arg types out of `arg_types` —
that silent left-shift is the proximate cause; on an untyped arg, unification
should bail with a proper diagnostic, not misalign.

## Prior art

bug-36 covered untyped-`[]` ambiguity only; audit-1-frontend FE-02 is recursion
depth. Neither covers this.

## Resolution

FIXED in commit e0fa88b8. FunctionContext gained a globals table + builtin-return resolver; the Call arm no longer filter_maps unknown arg types.

Regression test: `tests/rt-behavior/functions/bug103_generic_global_builtin_args` (fails on the unfixed compiler). Full
acceptance (871) and `cargo test` pass.
