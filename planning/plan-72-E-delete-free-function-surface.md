# plan-72-E: delete duplicated five-package free-function surface

Last updated: 2026-07-31
Effort: medium (1h-2h)
Depends on: plan-72-D

This final sub-plan removes the compatibility wrappers and direct target-package
dispatch chains after the registry has proven equivalent. It has the largest
review blast radius but the least design uncertainty.

References: plan-72 overview, `src/builtins/mod.rs`, `src/syntaxcheck/builtins.rs`,
`src/ir/lower.rs`, `src/ir/verify/compat.rs`, `src/target/shared/code/type_utils.rs`.

## Goal

- No production code calls the five target packages through duplicated helper
  functions for descriptor-owned metadata.
- The descriptor registry is the only compiler metadata authority for target
  package functions, types, source injection, implementation names, and defaults.

## Non-goals

- Do not delete helper functions that represent genuine non-descriptor behavior
  outside the plan scope unless they are proven dead.
- Do not migrate other builtin packages opportunistically.

## Current State

Before A, there were 70 direct target helper call sites (`rg -o
'builtins::(strings|datetime|encoding|money|term)::...' src | wc -l → 70`).
E must remeasure the remaining count after D and drive descriptor-owned metadata
call sites to zero.

## Phases

### Phase E1 — remeasure and delete wrappers

- [ ] Re-run the direct-call census and update this file with the current count.
- [ ] Delete descriptor-owned wrapper functions from
      `src/builtins/{strings,datetime,encoding,money,term}.rs` once no production
      caller needs them.
- [ ] Keep only package-local constants or helpers that are required to build the
      descriptor or implement the resolver.
- [ ] Remove parity tests that compare descriptor to deleted legacy wrappers;
      replace them with descriptor invariant tests.

Acceptance: `rg -o 'builtins::(strings|datetime|encoding|money|term)::(is_.*call|call_param_names|call_return_type_name|arity|resolve_call|implementation_name|argument_types|expected_arguments|default_argument_padding|param_types|builtin_type_fields|is_builtin_type|call_param_name_overloads|resolve_overload_target|is_overloaded)' src | wc -l → 0` for descriptor-owned metadata calls, excluding local descriptor construction if any.
Commit: —

### Phase E2 — simplify aggregate dispatch

- [ ] Replace target-specific arms in `src/builtins/mod.rs` aggregate helpers
      with registry iteration.
- [ ] Remove target package rows from private compatibility tables in
      `src/syntaxcheck/builtins.rs` where registry descriptors now provide the
      same data.
- [ ] Audit comments citing the old helper names and update them to descriptor
      symbols.

Acceptance: `cargo test` passes and source grep finds no stale comment claiming
      target package metadata lives in free-function tables.
Commit: —

### Phase E3 — docs, spec, and full gate

- [ ] Update `src/docs/spec/architecture/09_modules.md` to describe
      `BuiltinModule` descriptors as the compiler-owned metadata source for the
      target packages.
- [ ] Update stdlib/spec citations that refer to deleted helper symbols, including
      datetime and money topics if those citations moved.
- [ ] Run full validation from the overview.
- [ ] If any acceptance/golden output changes, apply AGENTS.md's golden-proof
      rules before touching expected output.

Acceptance: `cargo test` passes; `scripts/test-accept.sh target/debug/mfb target/accept-actual`
passes; byte-identity/artifact gate passes; docs/spec citations resolve under the
repo's citation checker if one exists (`rg -n 'citation|dangling|spec' scripts`
to locate it).
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate from the overview.
- Any doc/spec validation command found in `scripts/`.

## Corrections

Filled during execution.
