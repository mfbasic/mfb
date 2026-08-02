# plan-72-L: general package descriptor

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: plan-72-A

Migrate `src/builtins/general.rs` (815 LOC, 6 metadata helpers, 0 source
glue, 0 builtin types, 0 custom-resolver helpers, 26 fixtures) to a
`pub(crate) static GENERAL: BuiltinModule`.

References: plan-72 overview, `src/builtins/general.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/general/`.

## Goal

- `general::GENERAL` descriptor exists and mirrors every current metadata
  helper.
- Legacy free functions in `general.rs` become wrappers over `GENERAL`.
- Parity tests cover every function name, arity, return type, and argument
  type.

## Non-goals

- Do not add builtin types or a source companion (general has neither
  today).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/general.rs` is a large but data-shaped module with 6
descriptor-owned helpers and no source, type, or resolver customization.
Fixture load is 26 projects.

## Phases

### Phase L1 — descriptor and wrappers

- [x] Add `pub(crate) static GENERAL: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default. Done: 18 functions. The 6 numeric
      narrowing conversions carry `ReturnType::Fixed`; every other function
      carries `ReturnType::Custom` (so `call_return_type_name` returns `None`
      for them, matching legacy). `error` uses empty overloads (arity `None`).
      Parameter *types* are illustrative — resolution is owned by the bespoke
      `resolve_call` (accepted type-SET matching the descriptor cannot express).
- [x] Rewrite the 6 metadata helpers as wrappers over `GENERAL`.
      `is_general_call`→`contains`, `arity`→`arity`, `call_return_type_name`
      →`return_type_name` delegate to `DefaultResolver`; `call_param_names`
      (borrowed + covers `error`), `resolve_call` (type-set logic), and
      `expected_arguments` (bespoke phrasing) stay hand-authored statics pinned
      by parity where derivable.
- [x] Register `GENERAL` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests for every `general.*` name and unknown-name behavior
      (`parity_matches_descriptor`): 17 regular names through the harness,
      `error` checked separately, registry stays collision-free.

Acceptance: `cargo test` passes; every `general.*` fixture runs clean
under `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: 4a436687b

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `general` fixtures per the overview.

## Corrections

- **`error` is an irregular member the descriptor cannot model uniformly.** Legacy
  `is_general_call("error")` is `true` and `call_param_names("error")` returns
  `[["code"],["message"]]`, but `arity("error")` returns `None` (its 2-argument
  count is validated by `resolve_call`, not the generic arity gate). No non-empty
  overload can yield `None` arity, and a 2-parameter overload would flip
  `arity("error")` to `Some((2,2))` — a behavior change. Resolved by giving `error`
  an EMPTY overload list: `contains`/`arity`/`return_type_name` derive correctly
  (`true`/`None`/`None`), and its parameter names remain in the hand-authored
  `call_param_names` static (which BB must handle specially). Evidence: original
  `general.rs` `arity` match (no `ERROR` arm → `None`) vs `call_param_names` `ERROR`
  arm. `error` is excluded from the parity harness's `calls` list (its
  `DefaultResolver::param_names` is `None` ≠ the static's `Some`) and checked with
  explicit assertions instead.
- **general's function names are unqualified** (`"len"`, `"error"`, not
  `"general.len"`). They are added to the shared `BuiltinRegistry`; the parity test
  asserts `REGISTRY.duplicate_function_name() == None`, confirming no collision with
  any qualified package function.
