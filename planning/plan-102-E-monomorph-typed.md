# plan-102-E: Monomorph on ParameterType (typed unify/substitute)

Last updated: 2026-08-23
Effort: large (3h–1d)
Depends on: plan-102-D (monomorph already consumes/produces HIR; only its
`unify`/`substitute` internals still run the string algorithm behind a `name()`
shim).

Replace monomorph's string `unify_type`/`substitute_type_params` with native
`ParameterType` structural operations, and remove the `name()` shim introduced in
plan-102-D. Substitution maps become `HashMap<Symbol, ParameterType>` (or a
`ParameterType`-keyed structure); unification is structural variant matching with
integer-`Symbol` leaf comparisons; substitution builds `ParameterType` trees
instead of `format!`-ing strings. This is the Q3 payoff: the heaviest string-
manipulation stage above the IR stops touching strings.

See plan-102-A §3 for the full layering and the byte-identity gate.

References:

- `src/monomorph/helpers.rs` — `unify_type` (`:41`), `substitute_type_params`
  (`:171`), the string-keyed maps.
- `src/monomorph/mod.rs`, `src/monomorph/lower.rs` — the substitution-map types and
  the walk that threads them.
- `src/types.rs` — `ParameterType` (interned, complete post-A).

## Prerequisites

See plan-102-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-102-D complete | monomorph consumes/produces HIR (the `name()` shim is the only remaining string use in unify/substitute) | NOT MET until D lands |

## 1. Goal

- `unify_type` and `substitute_type_params` operate on `ParameterType`: structural
  match over variants, `Symbol` (integer) leaf comparison, tree-building
  substitution. The `name()` shim from plan-102-D is gone.
- monomorph's substitution/type maps are `ParameterType`-valued, not
  `HashMap<String, String>`.
- The count of type-name string operations in `src/monomorph/` drops sharply
  (measured before/after).

### Non-goals (explicit constraints)

- No change to compiled output — byte-identical `.ncode`/`.ncodesum`. Instantiation
  and mangling results are unchanged; only the internal representation changes.
- Name **mangling** (`name$Types`) still renders type names to strings at the mangle
  point — that is a deliberate string boundary (the mangled symbol is a string), not
  a hot-loop string op. Keep it; render via `name()` there.
- Do not change which overloads/instantiations are produced (that logic lives in
  `elaborate` after plan-102-D and is not touched here).

## 2. Current State

After plan-102-D, monomorph consumes generic HIR and produces concrete HIR, but
`unify_type`/`substitute_type_params` still run the string algorithm behind a
`name()` shim (D Phase 3). The string algorithm is the Q3 hotspot: `strip_prefix`
re-parsing, `format!` per substitution level, `HashMap<String, String>` with
`.to_string()` inserts (`src/monomorph/helpers.rs:51`).

### Measured populations (baseline to beat)

| What | Count (pre-E) | Command |
|---|---|---|
| monomorph `HashMap<String, String>` | 21 | `rg -c 'HashMap<String, String>' src/monomorph/ \| awk -F: '{s+=$2} END{print s}'` |
| monomorph `.to_string()` | 103 | `rg -c '\.to_string\(\)' src/monomorph/ \| awk -F: '{s+=$2} END{print s}'` |
| monomorph `strip_prefix("… OF ")` structural re-parse | UNMEASURED | measure at kickoff: `rg -c 'strip_prefix\("' src/monomorph/` |
| monomorph `format!("… OF …")` type builds | UNMEASURED | measure at kickoff: `rg -c 'format!\("(List OF\|Set OF\|Map OF\|Result OF\|MapEntry OF)' src/monomorph/` |

E's acceptance includes re-running these and recording the drop.

### Verified properties

- **`ParameterType` can represent every shape monomorph unifies** — plan-102-A added
  `MapEntryOf`/`ResultOf`, the two shapes `unify_type` handled that `parse` did not
  (`src/monomorph/helpers.rs:67,86`). So the typed rewrite has no representational
  gap. VERIFIED (contingent on A landing).

## 3. Design Overview

- `unify_type(pattern: &ParameterType, actual: &ParameterType, params: &SymbolSet,
  subs: &mut HashMap<Symbol, ParameterType>) -> bool`: match on variant pairs
  (`ListOf`/`MapOf`/`SetOf`/`Func`/`ThreadHandle`/`MapEntryOf`/`ResultOf`), recurse;
  a leaf whose `Symbol` is in `params` binds in `subs`; scalars compare by variant.
- `substitute_type_params(ty: &ParameterType, subs: &HashMap<Symbol, ParameterType>)
  -> ParameterType`: structural rebuild, substituting bound `Var`/`Named` leaves.
- Remove the plan-102-D `name()` shim; delete the string helpers
  (`user_template_parts`, `func_type_parts`, `split_top_level_to`, …) once no caller
  remains (measure).
- Mangling renders `name()` at the mangle site only.

This is a self-contained algorithm swap behind an already-typed interface (HIR in,
HIR out), so it is a single large sub-plan, not x-large.

### Rejected alternatives

- **Keep `HashMap<String, String>` substitutions, only type the leaves.** Rejected:
  leaves the `.to_string()`/`format!` churn; half the win.

## Compatibility / Format Impact

None externally observable. Mangled symbol strings are unchanged (rendered via
`name()` at the same point).

## Phases

### Phase 1 — typed `unify_type`

- [x] Rewrite `unify_type` to `ParameterType` structural matching with a
      `HashMap<Symbol, ParameterType>` binding map (`src/monomorph/helpers.rs`).
      (Native arms for every represented shape — scalars, `ListOf`/`SetOf`/`ResultOf`/
      `MapOf`/`MapEntryOf`/`Func`/`ThreadHandle`/`Res`; a leaf `Var`/`Named` whose
      `Symbol` is in the template-param set binds/checks; a **user-generic** `Named`
      (`Pair OF Integer, String` — no distinct variant, per the no-new-variant
      decision) falls back to the string algorithm via `user_template_parts` and
      re-parses, preserving results with zero `parse` behavior change.)
- [x] Tests: unify cases over every variant incl. `MapEntryOf`/`ResultOf`/`Func`/
      `ThreadHandle`; a `Var` binding-consistency case. (The historical string-level
      cases run through `unify_str`/`substitute_str` test adapters that route through
      the native functions via the byte-exact `parse`↔`name` round-trip — every
      assertion preserved; container/thread/func/binding-consistency all covered.)

Acceptance: unify unit tests pass; `cargo test` green. VERIFIED (3625 bin unit tests).
Commit: — (landed with Phase 2 as one atomic algorithm swap)

### Phase 2 — typed `substitute_type_params` + remove the shim

- [x] Rewrite `substitute_type_params` to build `ParameterType` trees; thread
      `HashMap<Symbol, ParameterType>` through the monomorph walk
      (`src/monomorph/mod.rs`, `lower.rs`); delete the plan-102-D `name()` shim at the
      unify/substitute boundaries. (The walk's substitution maps are `Symbol`-keyed
      `ParameterType` values end to end; `concrete_type_name` — which additionally
      *instantiates* user templates — keeps its string recursion but looks up
      substitutions by `Symbol` and renders `.name()` at its boundary.)
- [x] Mangling renders `name()` only at the mangle point. (`mangle_name` and the
      `name<args>` instantiation keys stay string — the deliberate boundary.)
- [x] Tests: full suite. (Sole failure = the recorded `artifact_gate_all` baseline.)

Acceptance: `artifact-gate all` no NEW diff vs the plan-102-A baseline; `cargo
test` green; `test-accept` no NEW mismatch; the §2 census re-run over
`src/monomorph/` shows the counts dropped (record the numbers). **VERIFIED** — gate
`diff` vs baseline IDENTICAL; census (commands per §2): `HashMap<String,String>`
21→**12**, `.to_string()` 103→**100**, `strip_prefix("` 17 (post; the survivors are
the deliberate string boundaries — `concrete_type_name`'s instantiation recursion +
the user-generic fallback), `format!(" OF ")` builds **8** (post). The unify/
substitute hot pair itself is fully native; the residual string ops live in the
mangling/instantiation-key/user-generic boundaries the Non-goals §2 explicitly keep.
Commit: —

## Validation Plan

- Tests: unify/substitute units; the generics/monomorph fixture set; full suite.
- Coverage check: every generic instantiation flows through the rewritten
  unify/substitute.
- Runtime proof: `artifact-gate all` byte-identical (modulo baseline) — instantiation
  results unchanged.
- Perf proof: record the monomorph string-op census drop; optionally a
  before/after compile-time sample on a generics-heavy fixture (the Q3 motivation).
- Doc sync: `.ai/codegen-invariants.md` monomorph notes.
- Acceptance: `cargo test`; `artifact-gate all`; `test-accept`; fmt both crates.

## Open Decisions

- **Substitution map key: `Symbol` vs `ParameterType`.** Recommend `Symbol`
  (variable names are `Var(Symbol)`), so lookup is an integer hash. (§3)

## Corrections

<Filled in during execution.>

## Summary

The Q3 payoff, isolated behind a typed interface: monomorph's hot loop stops doing
string surgery. Low design risk (the interface is fixed by D; the shapes are
complete via A), gated by byte-identity, with a measured string-op reduction as the
success signal.
