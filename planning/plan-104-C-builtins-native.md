# plan-104-C: Native ParameterType in codegen builtins + typed registry call boundary

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-104-B (the engine stores/oracle are typed; builtins receive
`ParameterType` from the engine seams and from NIR fields).

Convert the codegen **builtins** tree — the widest NIR type consumer (298
`.type_` reads, 16 of the 32 scalar-compare files) — from A's `.name()` shims to
native `ParameterType` operations, and give the registry's call-resolution
boundary a **typed entry point** so codegen stops rendering type names only for
the registry to re-parse them.

See plan-104-A §3 for the layering and the shared byte-identity gate; plan-104-A
§Prerequisites for the shared gate.

References:

- `src/codegen/builtins/` — the per-package native lowerings (the
  `abi_inline_self` bodies that read `NirValue` types).
- `src/codegen/builtins/mod.rs:325` — `resolve_call_return_type` (the string
  aggregate over the registry).
- `src/codegen/registry/mod.rs:2109` — `resolve_call(qualified, &[String],
  strict)`: the boundary that parses each arg string ("the conversion happens
  here — nothing inside the registry is a string").
- `src/codegen/builtins/general/mod.rs:236` (`function_parts`),
  `src/codegen/builtins/mod.rs:613` (`split_func_params_and_return`) — shared
  string type-vocabulary helpers whose call sites convert where the value is
  already typed.
- `.ai/resources-packages.md` — builtin-package authoring seams.

## Prerequisites

See plan-104-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-104-B complete | `static_nir_value_type` returns `Option<ParameterType>`; B's phases all `[x]` | MET 2026-08-24 (`rg 'fn static_nir_value_type' -A4` → `Option<ParameterType>`; B commits 7c0ecf8fa / 24ca75de7) |

## 1. Goal

- The builtins tree's 298 `.type_` reads
  (`rg -c '\.type_\b' src/codegen/builtins/` → 298) operate on `ParameterType`
  natively — scalar compares are variant matches, structural tests are variant
  matches, container constructions use `ParameterType::list_of(...)` etc.
- The registry exposes a typed resolution entry —
  `resolve_call_typed(qualified, &[ParameterType], strict) ->
  Option<ParameterType>` — and the string `resolve_call` becomes a thin wrapper
  (parse → typed → `name()`), byte-identical for its remaining string callers.
  Codegen-side callers (`type_utils`'s oracle from B, builtins dispatch) use the
  typed entry with **no parse**.
- The builtins scalar-compare census drops to 0 or each survivor is annotated as
  a deliberate string boundary.

### Non-goals (explicit constraints)

- No change to compiled output — byte-identical vs the plan-104 baseline.
- **The registry's string entry points survive** for their remaining string
  callers — measured: `resolve_call_return_type` is called from
  `src/syntaxcheck/builtins.rs` (7 sites), `src/monomorph/lower.rs` (2),
  `src/ir/lower.rs` (1) (`rg -n 'resolve_call_return_type\(' src/`), all of
  which still speak strings (their conversion is NOT this plan's scope — it
  would braid a front-end retype into a codegen plan). The wrapper delegates so
  there is exactly one algorithm.
- Registry descriptor definitions, matcher semantics (`unify`/`leaf_matches`/
  strict-`Nothing`/RES transparency), and overload sets are untouched — the
  typed entry point changes only who does the parsing, not what matches.
- `mangle`d/symbol strings, `SUPPORTED_RUNTIME_CALLS` gates, and error-message
  text stay strings (render `name()` at the seam).

## 2. Current State

Post-B, builtins still read NIR types through A's `.name()` shims, compare
scalars by string, and split structural types with the shared string helpers.
The registry boundary double-converts: codegen renders `name()` per argument →
`resolve_call` parses each back (`registry/mod.rs:2116`) — pure churn now that
both sides hold `ParameterType`.

### Measured populations

| What | Count | Command |
|---|---|---|
| builtins `.type_` reads | 298 | `rg -c '\.type_\b' src/codegen/builtins/ \| awk -F: '{s+=$2} END{print s}'` → 298 |
| builtins scalar-compare files | 16 | `rg -l '== "(Integer\|…\|Scalar)"' src/codegen/builtins/` → 16 files |
| codegen `format!("List OF …")` builds (mostly builtins) | 23 | plan-104-A census |
| codegen structural `strip_prefix` tests (mostly builtins) | 41 | plan-104-A census |
| `resolve_call_return_type` call sites (all crates) | 11 | `rg -n 'resolve_call_return_type\(' src/ \| grep -v 'fn ' \| wc -l` → 11 (7 syntaxcheck, 2 monomorph, 1 ir/lower, 1 codegen type_utils) |
| registry boundary parse sites removed for typed callers | 2 | `registry/mod.rs:2116,2145` (`.map(\|arg\| ParameterType::parse(arg))`) |

### Verified properties

- **Nothing inside the registry is a string** — its own doc comment at
  `registry/mod.rs:2109` states the conversion happens at `resolve_call`'s
  boundary; adding a typed entry above the parse is therefore a seam move, not
  an algorithm change. VERIFIED (read `resolve_call`/`CallShape`).
- **Typed args equal parsed strings by construction.** Codegen's
  `ParameterType` values originate from IR fields (parsed once at `ir::lower`)
  or from B's structural oracle rebuilds; `parse∘name = id` (plan-102's landed
  round-trip guarantee), so `resolve_call_typed(args)` sees exactly the trees
  `resolve_call(name(args))` would parse. VERIFIED by plan-102's gates + the
  round-trip unit tests.

## 3. Design Overview

Two moves:

1. **Typed registry entry.** `resolve_call_typed` takes `&[ParameterType]`,
   builds `CallShape` directly, returns `Option<ParameterType>` (no `name()` on
   the way out). The string `resolve_call` wraps it. `resolve_call_return_type`
   (the builtins aggregate) gets the same twin; its string form delegates.
   Codegen callers (post-B oracle, builtins dispatch) switch to the typed form.
2. **Builtins sweep.** File-by-file, compile-driven: shims → native matches;
   `format!("List OF {}", t)` → `ParameterType::list_of(t.clone())`; calls into
   shared string helpers (`function_parts`, `element_accepts_item`,
   `resource::state_type_name`) either take the typed value's structural
   equivalent (`ParameterType::Func` destructuring) or render `name()` at that
   call when the helper legitimately stays string (record which, per site).

**Correctness risk:** breadth. 298 reads across many per-function files, each a
small edit; the failure mode is a subtly wrong variant match in one builtin's
lowering, and the gate localizes it to that builtin's fixtures. Convert one
package at a time and run the scoped gate
(`scripts/artifact-gate.sh target/release/mfb <builtin>`) per package before the
full sweep's `all` run.

### Rejected alternatives

- **Retype `resolve_call_return_type`'s signature in place (no twin).**
  Rejected: 10 of its 11 callers are string-speaking front/middle-end passes
  outside this plan's scope — a signature change braids their conversion in.
- **Convert the shared string helpers (`function_parts`, …) to
  `ParameterType`.** Rejected here: they serve remaining string callers
  (syntaxcheck/monomorph); convert their *codegen* call sites instead, and let
  a later cleanup delete helpers whose last string caller disappears (measure
  then).

## Compatibility / Format Impact

None externally observable.

## Phases

### Phase 1 — typed registry entry

- [x] Add `resolve_call_typed` (registry) + the typed
      `resolve_call_return_type` twin (builtins aggregate); string forms become
      wrappers over the one `resolved_return_type` selection core (the string
      wrapper keeps its byte-verbatim `Arg(n)` echo of the caller's original
      string). Repoint `type_utils`'s oracle fallback (from B) to the typed
      entry. The three bespoke resolvers (`general`/`vector`/`strings`) keep a
      string pocket inside the typed twin — annotated deliberate boundary.
- [x] Tests: `typed_and_string_resolution_agree` (registry unit) — corpus incl.
      containers, `RES` echo (`List OF RES fs.File` preserved), FUNC params,
      `Unknown`, strict-`Nothing` rejection, and an unowned name. (Registry
      members key as `pkg.member` with a dot — the plan's `::` spellings
      resolve nothing.)

Acceptance: unit tests pass; `cargo test --no-fail-fast` green (exit 0, 0
FAILED); `artifact-gate all` no NEW diff (0 diffs).
Commit: 171879971

### Phase 2 — builtins sweep (per-package, scoped gate each)

- [x] Convert the builtins tree (compile-driven): **the sweep's mechanism was
      the `ValueResult.type_` flip to `ParameterType`** (see Corrections — the
      298 reads were `ValueResult` reads, not A-shims). Typed structural twins
      added for the container vocabulary (`typed_list_element_type`,
      `typed_map_type_parts`, `typed_is_collection_type`,
      `typed_callable_return_type`, …); scalar-literal compares on typed
      operands are variant `matches!`; FUNC checks on typed operands are
      structural `Func(..)` matches (isolation-exclusion preserved); compares
      the fixer had mis-shaped as per-compare parses were inverted to
      allocation-cheap `name()` renders; the fixer's stacked
      `parse(name())` round-trips were collapsed to clones (final same-line
      round-trip grep → 0). Note: the per-package scoped-gate cadence the plan
      prescribed was replaced by whole-tree `artifact-gate all` runs — the
      flip is a single crosscutting store change that cannot land
      per-package.
- [x] Tests: fixtures constructing type strings converted with the sweep
      (`cargo check --all-targets` → 0 errors/warnings).

Acceptance: `cargo test --no-fail-fast` green; `artifact-gate all` no NEW diff
vs baseline (0 diffs, verified mid-sweep and re-verified on the final tree);
builtins scalar-compare census
(`rg -n '== "(Integer|…)"' src/codegen/builtins/` → **25**, every survivor on a
`String`/`&str` carrier, none on a typed store — classified below); the two
registry boundary parse sites serve only string wrappers (`resolve_call`'s
wrapper parse + `rewrite_target`, whose callers are the string-speaking
front/middle end).

**Survivor classification (25):** ~7 compare mangled-name `$T` suffix fragments
(`target.strip_prefix("#collections_chunks$")` etc. — symbol strings, kept by
the plan's non-goals), 4 live in `general/mod.rs`'s bespoke string resolver
(the annotated Phase-1 pocket), and ~14 compare `String` locals derived through
the still-string helper chains in the collection lowerings — plan-104-D's
collection-tree conversion consumes them. Directional debt handed to D,
measured: 66 `ParameterType::parse` sites and 129 `name()` renders in
`src/codegen/builtins/` (mostly construction-site parses of helper-derived
strings), to be re-censused at D's closeout.
Commit: —

## Validation Plan

- Tests: registry typed/string agreement unit; builtins unit suite; full
  integration suite.
- Coverage check: every builtin package has byte-identity fixtures
  (`tests/byte-identity/<builtin>`) — the scoped gate per package is the
  coverage check.
- Runtime proof: `artifact-gate all` byte-identical vs baseline; `test-accept`
  no NEW mismatch (beyond the 2 documented pre-existing).
- Doc sync: none in C (D owns it).
- Acceptance: `cargo test --no-fail-fast`; `artifact-gate all`; `test-accept`;
  fmt both crates.

## Open Decisions

- **Typed entry naming:** `resolve_call_typed` vs overloading via a trait.
  Recommend the explicit `_typed` suffix; rename to the bare name when the last
  string caller converts in some future front-end plan. (§3)

## Corrections

- **The builtins' 298 `.type_` reads are `ValueResult.type_` reads, not A-phase
  shims.** Measured: `rg -n '\.type_\b' src/codegen/builtins/` receivers are
  `collection`/`value`/`list`/… — all `ValueResult`s from `lower_value`
  (`rg -c 'name\(\)' src/codegen/builtins/` → 15, i.e. almost no A-shims
  exist there). The plan's mechanism ("replace A's `.name()` shims") was
  therefore wrong: builtins always read codegen's own String-typed interchange
  struct. C's stated goal (native variant matches in builtins) is achievable
  only by flipping **`ValueResult.type_` to `ParameterType`** — a store no
  letter named (289 constructions across 117 files,
  `rg -c 'ValueResult \{' src/`). Done in C Phase 2 as the sweep's mechanism,
  with typed structural twins for the shared container splitters; the D trees
  take `.name()` shims where compile-driven and D finishes their native
  conversion per its own acceptance.

## Summary

The widest sweep of the feature, kept safe by per-package scoped gates and the
seam discipline the registry already documents ("nothing inside is a string").
The registry's matcher semantics are untouched; only the parsing moves to the
one place still holding strings.
