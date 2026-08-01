# plan-72-BB: delete duplicated free-function surface

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: every letter from plan-72-A through plan-72-AA is complete and
committed.

This final sub-plan removes the compatibility wrappers and direct package
dispatch chains after every per-package letter has proven the descriptor
registry equivalent. It has the largest review blast radius but the least
design uncertainty because each per-package letter has already landed its
descriptor and passed acceptance/byte-identity.

References: plan-72 overview, `src/builtins/mod.rs`,
`src/syntaxcheck/builtins.rs`, `src/ir/lower.rs`, `src/ir/verify/compat.rs`,
`src/target/shared/code/type_utils.rs`, `src/resolver/mod.rs`,
`src/syntaxcheck/mod.rs`, `src/monomorph/lower.rs`.

## Goal

- No production code calls any of the 26 builtin packages through
  duplicated helper functions for descriptor-owned metadata.
- The descriptor registry is the only compiler metadata authority for
  builtin package functions, types, source injection, implementation
  names, and defaults.
- Aggregate helpers in `src/builtins/mod.rs` no longer dispatch by
  explicit package chain; they iterate the registry.

## Non-goals

- Do not delete helper functions that represent genuine non-descriptor
  behavior outside the plan scope unless they are proven dead.
- Do not migrate any builtin package that a per-package letter did not
  already cover — every package must have landed its own letter first.

## Current State

Before A, there were 449 direct target helper call sites
(`rg -o 'builtins::[a-z_]+::[a-zA-Z0-9_]+' src | wc -l → 449`). BB must
re-run the census, prove the descriptor-owned metadata calls are zero, and
drive the aggregate dispatch chains through the registry.

## Phases

### Phase BB1 — remeasure and delete wrappers

- [ ] Re-run the direct-call census and update this file with the current
      count.
- [ ] Delete descriptor-owned wrapper functions from every
      `src/builtins/<pkg>.rs` once no production caller needs them.
- [ ] Keep only package-local constants or helpers required to build the
      descriptor or implement the package's resolver.
- [ ] Remove parity tests that compare descriptor to deleted legacy
      wrappers; replace them with descriptor invariant tests.

Acceptance: `rg -o 'builtins::[a-z_]+::(is_.*call|call_param_names|call_return_type_name|arity|resolve_call|implementation_name|argument_types|expected_arguments|default_argument_padding|param_types|builtin_type_fields|is_builtin_type|call_param_name_overloads|resolve_overload_target|is_overloaded)' src | wc -l → 0` for descriptor-owned metadata calls (excluding local descriptor construction).
Commit: —

### Phase BB2 — simplify aggregate dispatch

- [ ] Replace per-package arms in `src/builtins/mod.rs` aggregate helpers
      (`resolve_call_return_type`, `call_return_type_name`,
      `is_builtin_call`, `call_param_name_overloads`, `call_param_names`,
      `is_builtin_type`, `qualified_builtin_type`, `builtin_type_fields`)
      with registry iteration.
- [ ] Remove per-package rows from private compatibility tables in
      `src/syntaxcheck/builtins.rs` where registry descriptors now provide
      the same data.
- [ ] Route `src/ir/lower.rs:builtin_argument_types`,
      `normalize_builtin_call_arguments`, default padding, and
      `implementation_name` selection to descriptor APIs for every package.
- [ ] Route `src/ir/verify/compat.rs` and
      `src/target/shared/code/type_utils.rs:static_nir_value_type` through
      the descriptor return-type API.
- [ ] Route source injection in `src/resolver/mod.rs`,
      `src/syntaxcheck/mod.rs`, and `src/ir/lower.rs` through the
      descriptor `BuiltinSource` API in registry order.
- [ ] Audit comments citing the old helper names and update them to
      descriptor symbols.

Acceptance: `cargo test` passes and source grep finds no stale comment
claiming per-package metadata lives in free-function tables.
Commit: —

### Phase BB3 — docs, spec, and full gate

- [ ] Update `src/docs/spec/architecture/09_modules.md` to describe
      `BuiltinModule` descriptors as the compiler-owned metadata source
      for every builtin package.
- [ ] Update stdlib/spec citations that refer to deleted helper symbols,
      including datetime, money, term, and any other package whose
      citations moved.
- [ ] Run full validation from the overview.
- [ ] If any acceptance/golden output changes, apply AGENTS.md's
      golden-proof rules before touching expected output.

Acceptance: `cargo test` passes; `scripts/test-accept.sh target/debug/mfb
target/accept-actual` passes; byte-identity/artifact gate passes;
docs/spec citations resolve under the repo's citation checker if one
exists (`rg -n 'citation|dangling|spec' scripts` to locate it).
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate from the overview.
- Any doc/spec validation command found in `scripts/`.

## Corrections

Filled during execution.
