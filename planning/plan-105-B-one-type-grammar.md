# plan-105-B: One type-grammar implementation (collapse the hand-parsers; add UserOf)

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-105-A (the driver plumbing is typed; this letter finishes the
review's "collapses the 5 type-string parsers" outcome).

Make `ParameterType::parse` (`src/types.rs`) the **only** implementation of the
type grammar outside the source-language parser itself. The architectural
review (`planning/Compiler Pipeline.md:25`) counts the hand-rolled
`strip_prefix` cascades that must be edited in lockstep whenever the grammar
grows; today they survive in the **resolver** (8 sites), **monomorph** (15 in
`lower.rs` + the `user_template_parts`/`split_top_level_*` string fallback with
23 callers), and **syntaxcheck** (7 sites; its private parser module is
retired wholesale in plan-106-C, but its inline cascades die here). The enabler
is a new `ParameterType::UserOf(Symbol, Vec<ParameterType>)` variant so a user
generic (`Pair OF Integer, String`) is structural instead of an opaque `Named`
whose name monomorph re-splits with strings.

See plan-105-A for the shared prerequisites/goal framing (this plan finishes
review Recommendation #1's "collapse the 5 parsers" outcome; the fifth listed
site, `ast/expr.rs:595`, is the **source-language grammar** — the tokenizer-side
parse that produces the AST — and is the one legitimate survivor, recorded as
such).

References:

- `planning/Compiler Pipeline.md:25` — the parser census and file:line list.
- `src/types.rs` — `ParameterType::parse`/`name` (the canonical grammar); the
  plan-102-A precedent for adding a variant (`MapEntryOf`/`ResultOf` wiring:
  parse arm + name arm + registry `unify`/`substitute`/`contains_var` + the
  container fail-set).
- `src/resolver/resolution.rs:1157,1171,1308-1340` — the 8 resolver cascades.
- `src/monomorph/lower.rs` (15 sites, dominated by `concrete_type_name`'s
  string recursion) and `src/monomorph/helpers.rs`
  (`user_template_parts`/`split_top_level_to`/`split_top_level_commas`).
- `planning/completed/plan-102-E-monomorph-typed.md` — where the user-generic
  string fallback was deliberately kept (no-new-variant decision); this plan
  reverses that decision WITH the byte-identity gate that made it scary.

## Prerequisites

See plan-105-A §Prerequisites (shared). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-105-A complete | `rg -n 'rsplit_once\(" AS "\)' src/` → 0; A's boxes ticked | NOT MET until A lands |

## 1. Goal

- `ParameterType::UserOf(Symbol, Vec<ParameterType>)` exists: `parse` produces
  it for `Name OF a, b` shapes that are not built-in containers; `name()`
  renders `Name OF a, b` byte-exactly; registry `unify`/`substitute`/
  `leaf_matches`/`contains_var` recurse into it; monomorph's native
  `unify_type`/`substitute_type_params` handle it structurally and the
  **string fallback path is deleted** (`user_template_parts`,
  `split_top_level_to`, `split_top_level_commas` — 23 callers today → 0).
- The resolver's 8 hand-parse sites use `ParameterType::parse` + structural
  matches (parse-in at the AST-domain boundary — the resolver legitimately
  reads source strings pre-elaboration; it just may not own a grammar copy).
- Monomorph's 15 `lower.rs` string sites are structural
  (`concrete_type_name`'s recursion becomes typed substitute + instantiate
  hooks; the mangling render stays `name()`-out).
- Syntaxcheck's 7 inline cascades use canonical parse (its `Type` enum and
  parser module are plan-106-C's scope — do not touch them here beyond the
  inline cascades).
- End-state census: `rg -n 'strip_prefix\("(List OF |Set OF |Map OF |RES |
  Result OF |MapEntry OF )' src/` hits **only** `src/types.rs` (the canonical
  parser) and `src/ast/` (the source grammar) — everything else 0.

### Non-goals (explicit constraints)

- No change to compiled output — byte-identical vs the plan-105 baseline.
  `UserOf` must be provably neutral: `parse(s).name() == s` for every user
  generic spelling, and every consumer that previously matched
  `Named("X OF …")` behavior (registry leaf matching, type_index name-keyed
  lookups via `.name()`) behaves identically. The gate decides.
- No `.mfp`/`.ir`/`.nir` byte changes (renders unchanged).
- Do NOT retype the resolver's AST-domain data model or syntaxcheck's `Type`
  enum (106's scope). This letter only collapses grammar *implementations*.
- The HIR `parse↔name` round-trip invariant is load-bearing (memory:
  `hir-parse-name-roundtrip-load-bearing`) — `UserOf` must preserve
  `ParameterType::parse(s).name() == s` for all existing corpus spellings; the
  `src/types.rs` round-trip unit tests extend to `UserOf` shapes.

## 2. Current State

`parse` folds `Pair OF Integer, String` into `Named` (deliberate plan-102-E
decision), so monomorph keeps a string algorithm for exactly that shape, and
the review's lockstep-edit hazard ("adding one type constructor means editing
all five in lockstep") still stands for the resolver/monomorph/syntaxcheck
copies.

### Measured populations

| What | Count | Command |
|---|---|---|
| resolver hand-parse sites | 8 | `rg -n 'strip_prefix\("(List OF \|Set OF \|Map OF \|RES \|Result OF \|MapEntry OF )' src/resolver/*.rs` → 8 (all in `resolution.rs`) |
| monomorph `lower.rs` string type sites | 15 | same pattern over `src/monomorph/lower.rs` → 15 |
| `user_template_parts`/`split_top_level_*` callers | 23 | `rg -n 'user_template_parts\|split_top_level' src/monomorph/*.rs \| grep -v 'fn ' \| wc -l` → 23 |
| syntaxcheck inline cascades | 7 | same strip pattern over `src/syntaxcheck/` → 7 |
| user-generic `TYPE … OF …` fixtures in the byte-identity corpus | 0 | `rg -rl 'TYPE [A-Za-z]+ OF [A-Z]' tests/` → 0 — **UserOf is under-covered by goldens; Phase 1 adds fixtures FIRST** |

### Verified properties

- **plan-102-A's variant-addition recipe is proven** (MapEntryOf/ResultOf
  landed byte-identically with the same wiring surface). VERIFIED (landed).
- **Generic user types flow through monomorph's string fallback today** —
  `unify_type`'s `Named`-leaf fallback and `concrete_type_name`'s
  `user_template_parts` split (read `src/monomorph/helpers.rs`,
  `lower.rs::concrete_type_name`). So `UserOf` + native arms subsumes exactly
  that code. VERIFIED (read during plan-102-E).
- **Golden coverage for user generics is ZERO** (measured above) — byte-identity
  alone cannot certify `UserOf`. Phase 1 therefore adds rt-behavior + golden
  fixtures for generic user types (single/multi-param, nested) BEFORE the
  variant lands, so the gate has teeth. This is the plan's design-uncertainty
  concentration and it is scheduled first.

## 3. Design Overview

Order: fixtures → variant → consumers. Byte-identity is the gate; the new
fixtures extend the corpus so the gate actually covers the changed shape.

1. **Fixtures first** (`tests/rt-behavior/generics/…` + goldens via
   `sync-goldens.sh` on the PRE-change compiler): generic record + generic
   function over it, multi-param (`Pair OF A, B`), nested
   (`Box OF Pair OF Integer, String`), collection-of-generic.
2. **`UserOf` variant**: parse arm (a `Name OF …` head that is not a built-in
   container → split top-level commas structurally — the ONE place that logic
   lives now), name arm, registry recursion arms, `with_vars` arm, monomorph
   native arms; DELETE `user_template_parts`/`split_top_level_*` and the
   fallback branches.
3. **Consumer sweeps**: resolver 8 → parse+match; monomorph 15 → structural;
   syntaxcheck 7 → parse+match.

**Correctness risk:** `concrete_type_name` — it both substitutes AND triggers
user-template *instantiation*; its typed rewrite must preserve instantiation
keys and mangled names exactly (mangle renders `name()`, unchanged). The new
fixtures + the full gate cover it.

### Rejected alternatives

- **Keep the string fallback and only collapse resolver/syntaxcheck.**
  Rejected: leaves the review's lockstep hazard alive inside monomorph and
  blocks plan-106's no-strings end state.
- **`UserOf(String, …)` with an owned name.** Rejected: `Symbol` is the
  established interned leaf (plan-102-A); no reason to regress.

## Compatibility / Format Impact

None. All renders byte-identical (`name()` reproduces the exact spellings).

## Phases

### Phase 1 — user-generic fixtures (gate coverage first)

- [ ] Add rt-behavior fixtures for user generics (single/multi-param, nested,
      collection-of-generic) with run goldens; generate `.ir`/`.ncode` goldens
      with the PRE-change compiler (`sync-goldens.sh` scoped to the new dirs).
- [ ] `cargo test --no-fail-fast` + `artifact-gate all` green including the new
      fixtures (they extend the baseline file).

Acceptance: new fixtures pass on the unchanged compiler; baseline re-recorded.
Commit: —

### Phase 2 — `UserOf` variant + native monomorph, fallback deleted

- [ ] Add `UserOf(Symbol, Vec<ParameterType>)` to `src/types.rs` (parse/name/
      round-trip tests) + registry recursion arms + `with_vars` arm — the
      plan-102-A Phase-4 recipe.
- [ ] Monomorph native `unify_type`/`substitute_type_params` gain `UserOf`
      arms; delete `user_template_parts`/`split_top_level_to`/
      `split_top_level_commas` and every fallback branch (23 callers → 0);
      `concrete_type_name`'s recursion goes structural (instantiation keys +
      mangling byte-identical).
- [ ] Tests: round-trip + unify/substitute units over `UserOf`; the Phase-1
      fixtures.

Acceptance: `cargo test --no-fail-fast` green; `artifact-gate all` no NEW diff
(incl. Phase-1 fixtures); `rg -n 'user_template_parts\|split_top_level' src/monomorph/`
→ 0.
Commit: —

### Phase 3 — resolver + syntaxcheck cascades onto canonical parse

- [ ] Replace the resolver's 8 sites with `ParameterType::parse` + structural
      matches (`resolution.rs`).
- [ ] Replace syntaxcheck's 7 inline cascades likewise (its parser module
      untouched — 106-C).
- [ ] Tests: diagnostic golden corpus (every `*-invalid` fixture) — codes,
      wording, order unchanged.

Acceptance: `cargo test --no-fail-fast` green; `artifact-gate all` no NEW diff;
the end-state census — `rg -n 'strip_prefix\("(List OF |Set OF |Map OF |RES |Result OF |MapEntry OF )' src/`
hits only `src/types.rs` and `src/ast/` — holds and is recorded here.
Commit: —

## Validation Plan

- Tests: `UserOf` round-trip/unify units; the new generics fixtures; full
  diagnostic corpus.
- Coverage check: Phase 1 exists precisely because the corpus had zero
  user-generic fixtures — after it, the gate covers the changed grammar.
- Runtime proof: `artifact-gate all`; `test-accept` (no NEW mismatch).
- Doc sync: `.ai/codegen-invariants.md` monomorph notes (fallback deleted);
  memory `hir-parse-name-roundtrip-load-bearing` gets updated (user generics
  now HAVE a variant).
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- **Does `Thread OF` parsing subsume via `UserOf`?** No — thread types keep
  their dedicated variant (they carry the RES/STATE planes). `UserOf` applies
  only to non-builtin heads; the parse arm ordering (builtins first) enforces
  it. Recommendation recorded so the implementer doesn't re-litigate.

## Corrections

<Filled in during execution.>

## Summary

This reverses plan-102-E's "no new variant" deferral with the safety that
deferral lacked: golden coverage added FIRST, then one grammar implementation.
Risk concentrates in `concrete_type_name`'s instantiation-key fidelity; the
fifth "parser" (`ast/expr.rs`) is the source grammar and stays by design.
