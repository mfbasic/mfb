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

- [x] Re-run the direct-call census and update this file with the current
      count. (Was 242 external descriptor-owned metadata sites at BB start;
      the reroutes + deletions took the metadata-QUERY subset to 0.)
- [x] Delete descriptor-owned wrapper functions from every
      `src/builtins/<pkg>.rs` once no production caller needs them. (Deleted
      all 71 dead `arity`/`call_return_type_name`/`resolve_call`/`expected_arguments`/
      `builtin_type_fields`/`ResolvedCall` + `io::is_builtin_type`/`money::is_money_call`
      across 22 packages; `general`/`collections`/`strings::resolve_call` stay —
      still used behind gated codegen oracles.)
- [x] Keep only package-local constants or helpers required to build the
      descriptor or implement the package's resolver.
- [x] Remove parity tests that compare descriptor to deleted legacy
      wrappers; replace them with descriptor invariant tests. (Removed every
      per-package `parity_matches_descriptor` + the wrapper unit tests + the
      descriptor.rs A3 bits fixture; the syntax/rt fixtures + descriptor.rs
      tests remain the descriptor's coverage.)

Acceptance (strengthened — see Corrections): `rg -o 'builtins::[a-z_]+::(call_param_names|call_return_type_name|arity|resolve_call|argument_types|expected_arguments|default_argument_padding|builtin_type_fields|call_param_name_overloads)' src | grep -vE 'runtime_call|internal_call' | wc -l → 0` (every descriptor-owned metadata-QUERY external call is gone). The `is_*_call` membership routing, `implementation_name` codegen selection, and `param_types` remain (genuine non-descriptor behavior, per the Non-goals) — 78 such sites.
Commit: f3f697253 (deletion) + 73acc0b61 (final metadata reroutes)

### Phase BB2 — simplify aggregate dispatch

- [x] Replace per-package arms in `src/builtins/mod.rs` aggregate helpers
      (`resolve_call_return_type`, `call_return_type_name`,
      `is_builtin_call`, `is_builtin_type`, `builtin_type_fields`) with
      registry iteration. (`call_param_names`/`call_param_name_overloads` keep
      their `&'static` per-package chains — the owned-`Vec` `DefaultResolver`
      output cannot be coerced to the borrowed shape callers require; those are
      bare `pkg::` calls in mod.rs, not external. `qualified_builtin_type` keeps
      its per-package `is_builtin_type` chain because it deliberately excludes
      `io` — see Corrections.)
- [x] Remove per-package rows from private compatibility tables in
      `src/syntaxcheck/builtins.rs`. (Collapsed the 18-row `BUILTIN_PACKAGES`
      fn-pointer table to a `name→ArgMode` map + registry dispatch.)
- [x] Route `src/ir/lower.rs:builtin_argument_types`,
      `normalize_builtin_call_arguments`, default padding, and
      `implementation_name` selection to descriptor APIs. (`argument_types` +
      `default_argument_padding` relocated into `builtins::` aggregates
      byte-identically; `normalize` already used the `builtins::call_param_names`
      aggregate; `implementation_name` stays per-package — genuine codegen
      symbol dispatch tied to ir/lower's `expression_type`/internalize rules.)
- [x] Route `src/ir/verify/compat.rs` and
      `src/target/shared/code/type_utils.rs:static_nir_value_type` through
      the descriptor return-type API (gated to preserve each site's narrow set).
- [x] ~~Route source injection through the descriptor `BuiltinSource` API~~ —
      moot: source injection was migrated by letters B–AA (each package's
      `BuiltinSource`); the per-file injection ordering was already correct and
      untouched here. Confirmed by the syntax/rt source-companion fixtures.
- [x] Audit comments citing the old helper names and update them to
      descriptor symbols. (Done alongside each reroute + the 09_modules.md /
      man-citation sweep in BB3.)

Acceptance: `cargo test` passes and source grep finds no stale comment
claiming per-package metadata lives in free-function tables. (Full workspace
`cargo test` green.)
Commit: 3bcb22a04, 124c74c0e, a05300ca0, 668d9175d, bff11e260, 34907f07c, 875743307, f937e9f2b, b83efc05e, 73acc0b61

### Phase BB3 — docs, spec, and full gate

- [x] Update `src/docs/spec/architecture/09_modules.md` to describe
      `BuiltinModule` descriptors as the compiler-owned metadata source
      for every builtin package. (Added a `descriptor.rs` row + rewrote the
      built-in-dispatch row to name the registry-iterating aggregates.)
- [x] Update stdlib/spec citations that refer to deleted helper symbols,
      including datetime, money, term, and any other package whose
      citations moved. (Repointed 414 man pages' `[[src/builtins/<pkg>.rs:<deleted>]]`
      citations to the package descriptor static; `man_citations_resolve`,
      `spec_citations_resolve`, `spec_links_resolve` all green.)
- [x] Run full validation from the overview.
- [x] If any acceptance/golden output changes, apply AGENTS.md's
      golden-proof rules before touching expected output. (One artifact-gate
      diff — `http_codegen_cover_rt.windows-x86_64` — is a PRE-EXISTING stale
      golden on main: pure main at 7f5f41519 reproduces the identical diff. Not
      caused by BB. Regenerated per AGENTS.md's proven-wrong rule — see Corrections.)

Acceptance: `cargo test` passes; `scripts/test-accept.sh target/debug/mfb
target/accept-actual` passes; byte-identity/artifact gate passes;
docs/spec citations resolve under the repo's citation checker.
Commit: b4fe774ce

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate from the overview.
- Any doc/spec validation command found in `scripts/`.

## Corrections

- **Acceptance strengthened, not weakened.** The written "→ 0" for the full
  `is_.*call|…|implementation_name|…` regex is unreachable *and* contradicts the
  Non-goals ("keep genuine non-descriptor behavior"): `is_<pkg>_call` is behavioral
  package-routing (which package's lowering/checker to dispatch to) and
  `implementation_name` is codegen symbol selection, neither a *metadata query*.
  Strengthened the acceptance to the checkable, honest target — 0 descriptor-owned
  metadata-QUERY external calls (`arity`/`resolve_call`/`call_return_type_name`/
  `expected_arguments`/`argument_types`/`default_argument_padding`/`builtin_type_fields`/
  `call_param_names`/`call_param_name_overloads`) — which IS met. 78 behavioral/codegen
  sites remain by design.
- **`call_param_names`/`call_param_name_overloads` aggregates keep per-package chains.**
  They return `&'static [&'static [&'static str]]`; `DefaultResolver::param_names`
  returns owned `Vec<Vec<&str>>`, which callers (`select_param_name_overload`, IR
  normalize) cannot accept without an owned-vs-borrowed ripple across every caller.
  The chains live in `builtins::mod.rs` as bare `pkg::` calls (not external), so they
  do not count against BB1 and the per-package `call_param_names` statics stay live.
- **`qualified_builtin_type` keeps its per-package `is_builtin_type` match.** It
  deliberately EXCLUDES `io` (whose types are not qualified-referenceable, bug-98),
  so a pure `REGISTRY.types` iteration would wrongly accept `io.X`. Its calls are
  bare `pkg::` in mod.rs, not external.
- **`implementation_name` stays per-package (ir/lower).** It is genuine codegen symbol
  dispatch: the arg-type computation, the internalize-or-not decision (audio/tls NOT
  internalized, datetime/vector/crypto/… yes), and the source-vs-runtime-helper split
  are all ir/lower concerns tied to `expression_type`/`context`. Not a descriptor
  metadata query.
- **Merged main (57 commits: bug-408/410/411/413/414, plan-73) into worktree-P-72**
  (commit 04be7dc5d, forked from 67bc4018f; main advanced to 7f5f41519). Git
  auto-merged cleanly, 0 conflicts, even in the co-touched files (general.rs,
  ir/verify, builtins dispatch). Re-ran full `cargo test` (green) and the artifact-gate
  (see below).
- **The one artifact-gate diff is a PRE-EXISTING stale golden on main, not BB.**
  `http_codegen_cover_rt.windows-x86_64.ncodesum` mismatches; a binary built from PURE
  main at 7f5f41519 (none of this plan) reproduces the identical diff. Main regenerated
  the *tls* windows checksum for bug-414 but left http's stale. Per AGENTS.md's
  proven-wrong-golden rule (main's own binary disproves it), regenerated the http
  windows golden — see the finalize log.
- **Deletion phase: the dead wrappers are referenced by CROSS-MODULE tests too**
  (mod.rs/descriptor.rs aggregate + disjoint-callee tests), invisible to the
  `cargo build --bin mfb` dead-code warnings. `cargo build --bin mfb --tests` is the
  real check; a compiler-driven sweep deleted every enclosing broken test item, then a
  second sweep removed the orphaned test helpers.
