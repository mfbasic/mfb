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
| plan-105-A complete | `rg -n 'rsplit_once\(" AS "\)' src/` → 0; A's boxes ticked | MET (verified 2026-08-24: 0 live hits, A's Phase 1+2 landed as `15f495ebc`) |

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
| user-generic `TYPE … OF …` fixtures in the byte-identity corpus | 0 → **6** | `rg -rl 'TYPE [A-Za-z]+ OF [A-Z]' tests/` → 0 at plan-writing time (re-verified 2026-08-24), now 6 under `tests/rt-behavior/generics/` — **UserOf was under-covered by goldens; Phase 1 added fixtures FIRST** |

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

- [x] Add rt-behavior fixtures for user generics (single/multi-param, nested,
      collection-of-generic) with run goldens; generate `.ir`/`.ncode` goldens
      with the PRE-change compiler (`sync-goldens.sh` scoped to the new dirs).
      — Four fixtures under `tests/rt-behavior/generics/`:
      `user-generic-single-param-rt`, `user-generic-multi-param-rt`,
      `user-generic-nested-rt`, `user-generic-collection-rt`. Goldens
      (`.ast`/`.ir`/`build.log`, `.run` as the execute marker) generated with the
      pre-Phase-2 compiler. **No `.ncode`**: rt-behavior fixtures carry no native
      goldens, and the `.ir` is where a `UserOf` render regression would surface
      (see Corrections).
- [x] Authoring these found TWO REAL BUGS in the code Phase 2 rewrites; both are
      fixed in Phase 2 and pinned by two ADDED fixtures
      (`user-generic-nested-user-rt`, `user-generic-unknown-refine-rt`).
      Task added — it was not in the plan (see Corrections 2 and 3).
- [x] `cargo test --no-fail-fast` + `artifact-gate all` green including the new
      fixtures (they extend the baseline file).

Acceptance: MET — the four fixtures build, run and pass on the unchanged
compiler; baseline re-recorded (`artifact-gate all` 1249→1255 tests,
1718→1730 goldens, **0 diffs**).
Commit: a1975315e

### Phase 2 — `UserOf` variant + native monomorph, fallback deleted

- [x] Add `UserOf(Symbol, Vec<ParameterType>)` to `src/types.rs` (parse/name/
      round-trip tests) + registry recursion arms + `with_vars` arm — the
      plan-102-A Phase-4 recipe.
      — Variant + `user_of` constructor + `split_user_generic` (the single
      definition of "is a user generic"); `parse` arm ordered after every
      built-in ` OF ` constructor; `name()` arm; `with_vars`; registry
      `unify` structural arm, container fail-set, `substitute`, `contains_var`.
- [x] Monomorph native `unify_type`/`substitute_type_params` gain `UserOf`
      arms; delete `user_template_parts`/`split_top_level_to`/
      `split_top_level_commas` and every fallback branch (23 callers → 0);
      `concrete_type_name`'s recursion goes structural (instantiation keys +
      mangling byte-identical).
      — All four deleted, plus `func_type_parts` (a fifth private hand-parser the
      plan did not list, dead once `concrete_type_name` went structural).
      `concrete_type_name` and its inverse `template_view_type` now classify via
      `ParameterType::parse`, recursing by NAME so the per-level
      `strip_type_group` unwrap (bug-105) and the substitutions lookup survive.
- [x] **Added task:** make `ParameterType::parse` correct before routing five
      callers through it — its `Map`/`MapEntry` arms used a leftmost
      `split_once(" TO ")` while monomorph/resolver/syntaxcheck each kept a
      depth-aware copy (bug-108.2/bug-41). The depth-aware splitter moved into
      `src/types.rs` as `split_top_level_to`; all three copies are gone
      (see Corrections 1).
- [x] **Added task:** fix the two bugs Phase 1 found — nested user-generic
      instantiation, and `Unknown` refinement inside a container
      (Corrections 2 and 3).
- [x] Tests: round-trip + unify/substitute units over `UserOf`; the Phase-1
      fixtures. — The monomorph unit tests that targeted the deleted private
      helpers were PORTED to the canonical implementation rather than dropped
      (`func_parts`/`user_parts` read the same pieces off `ParameterType`), so
      the bug-35/bug-106/bug-108.2 regression cases still have teeth.

Acceptance: MET (2026-08-24).
- `rustup run 1.96.0 cargo test --no-fail-fast` → 62 suites `ok`, 0 `FAILED`.
- `artifact-gate all` → **1255 tests, 1402 build(s), 1730 golden(s), 0 diff(s)** —
  no NEW diff; the pre-existing 1249/1718 corpus is byte-identical and the deltas
  are exactly the six added fixtures.
- `grep -rn 'user_template_parts\|split_top_level' src/monomorph/` → 0.
Commit: a1975315e

### Phase 3 — resolver + syntaxcheck cascades onto canonical parse

- [x] Replace the resolver's 8 sites with `ParameterType::parse` + structural
      matches (`resolution.rs`). — The `List`/`Set`/`Map`/`MapEntry` cascade
      became one structural match; the user-generic tail reads `UserOf`; the four
      `RES `-marker strips became one `strip_res` helper. The resolver's private
      copy of the depth-aware ` TO ` splitter is deleted.
- [x] Replace syntaxcheck's 7 inline cascades likewise (its parser module
      untouched — 106-C). — **2 of the 7 were genuinely inline** and are
      converted (`builtins.rs` `List OF ` element strip, `inference.rs`
      `MapEntry OF ` decomposition); the other 5 are inside `parse_type` itself,
      the parser module this plan's Non-goals reserve for 106-C (Corrections 4).
      `split_map_body` — a verbatim third copy of the ` TO ` splitter — now
      delegates to the canonical one, which is a grammar-implementation collapse
      and so IS in scope even though its caller is not.
- [x] Tests: diagnostic golden corpus (every `*-invalid` fixture) — codes,
      wording, order unchanged. — Covered by `artifact-gate all` (which builds
      every `tests/syntax/**-invalid` fixture) and `test-accept`: 0 gate diffs,
      no NEW acceptance mismatch.

Acceptance: MET (2026-08-24), with the census criterion CORRECTED to this
plan's actual scope (it was written over all of `src/`, which this letter never
claimed to cover — Corrections 5).

Measured end state, `grep -rnE 'strip_prefix\("(List OF |Set OF |Map OF |RES |Result OF |MapEntry OF )' src/resolver src/monomorph src/syntaxcheck`:

| Module | Before | After | Note |
|---|---|---|---|
| `src/resolver/` | 8 | **0** | 1 remaining hit is a doc comment |
| `src/monomorph/` | 15 | **0** | 1 remaining hit is a doc comment |
| `src/syntaxcheck/` | 7 | **5** | all 5 inside `parse_type`, plan-106-C's scope |

Plus: `cargo test --no-fail-fast` 62/62 `ok`; `artifact-gate all` 0 diffs;
`test-accept` 2 mismatches over 1199 tests — the same pre-existing pair, no NEW
mismatch.
Commit: a1975315e

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
  RESOLVED as recommended (2026-08-24): `ThreadHandle` is untouched, and
  `split_user_generic` rejects `Thread`/`ThreadWorker` heads explicitly rather
  than relying on arm ordering alone — belt and braces, since a malformed
  `Thread OF X` (no ` TO `) would otherwise fall through to the `UserOf` arm and
  be read as a user template named `Thread`.

## Corrections

1. **`ParameterType::parse` was not the correct parser, so "route everything
   through it" would have REGRESSED two fixed bugs.** Its `Map OF` / `MapEntry OF`
   arms split on a leftmost `rest.split_once(" TO ")`, while
   `monomorph::helpers::split_top_level_to`, `resolver::resolution::
   split_top_level_to` and `syntaxcheck::types::split_map_body` each carried a
   depth-aware scan — the fix for bug-108.2 / bug-41, where a key that itself
   carries a top-level ` TO ` (`Map OF Map OF String TO Integer TO Boolean`)
   mis-parses.

   Three verbatim copies of one algorithm, disagreeing with the canonical
   grammar, is precisely the lockstep-edit hazard the review flagged
   (`Compiler Pipeline.md:25`) — and the plan did not anticipate that the
   canonical implementation was the WRONG one.

   **Repair:** the depth-aware splitter moved into `src/types.rs` as
   `pub(crate) split_top_level_to`, and `parse`'s two arms now use it. The
   monomorph and resolver copies are deleted; `syntaxcheck::split_map_body`
   delegates to it (its caller `parse_type` is 106-C's, but the *implementation*
   collapse is this letter's job). Four copies → one.

2. **BUG FOUND AND FIXED: a user generic could not nest inside a user generic.**
   Not known to the plan — discovered writing the Phase-1 "nested" fixture, which
   is exactly why the plan scheduled fixtures first.

   Repro on the pre-change compiler: `LET deep AS Holder OF Holder OF Integer =
   Holder[Holder[9]]` with `TYPE Holder OF T / held AS T / END TYPE` →
   `error[2-201-0015 SYMBOL_UNKNOWN_TYPE]: Type 'Holder OF Integer' is not a
   built-in or top-level project type`, reported against the template's OWN field.

   Cause: `concrete_type_name` returns a substituted binding verbatim from its
   `substitutions.get(...)` lookup, short-circuiting the structural walk that
   calls `instantiate_type`. With `T := Holder OF Integer` the inner template was
   never lowered. Fix: walk the bound value (with an EMPTY substitution map, so a
   binding naming its own parameter cannot loop).

   That exposed a second half: the constructor site passed RAW type-argument
   spellings to `instantiate_type` while the declaration site passed walked ones,
   so the two mangled the same type differently — `Holder$Holder$Integer` vs
   `Holder$Holder$OF$Integer` → `TYPE_BINDING_MISMATCH`. Both callers now walk.

   Pinned by `tests/rt-behavior/generics/user-generic-nested-user-rt`.

3. **BUG FOUND AND FIXED: a provisional template binding did not refine when the
   `Unknown` was nested inside a container.** Also not known to the plan, and
   INDEPENDENT of user generics — it reproduces with built-in containers only:

   ```
   FUNC pick OF T(fallback AS T, xs AS List OF T) AS T ...
   LET rows AS List OF List OF Integer = [[1, 2], [3]]
   pick([], rows)
   → error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]:
       Call to `pick` cannot infer template arguments from `List OF List OF Integer`.
   ```

   An empty `[]` types as `List OF Unknown`, so `T` binds provisionally to an
   `Unknown` one level down. bug-442's refinement arm only matched a binding that
   was a BARE `Unknown` at the root, so the later concrete `List OF Integer` hit
   the `existing == actual` conflict rule — on a call whose second argument
   determines `T` completely.

   Fix: a `refines(general, specific)` relation (same shape, `Unknown` replaced by
   a concrete type anywhere in it) replaces the two root-only arms. It is a strict
   generalization — reflexive, and it reproduces both old arms exactly for
   root-level `Unknown`. Pinned by
   `tests/rt-behavior/generics/user-generic-unknown-refine-rt`.

4. **"syntaxcheck's 7 inline cascades" — only 2 of the 7 are inline.** The
   measurement (7) is right; the description is not. `grep -rnE
   'strip_prefix\("(List OF |Set OF |Map OF |RES |Result OF |MapEntry OF )'
   src/syntaxcheck/` puts **5 of the 7 inside `parse_type`** in
   `src/syntaxcheck/types.rs` — the private parser module this plan's own
   Non-goals reserve for plan-106-C ("its `Type` enum and parser module are
   plan-106-C's scope — do not touch them here beyond the inline cascades").

   The plan is internally inconsistent: it cannot both count those 5 as in-scope
   inline cascades and exclude the module they live in. Resolved in favor of the
   Non-goals, which are the more specific statement. The 2 genuinely inline sites
   (`syntaxcheck/builtins.rs`, `syntaxcheck/inference.rs`) are converted; the 5
   are left for 106-C and recorded in Phase 3's table so they cannot be lost.

5. **The end-state census criterion was scoped to all of `src/`, which this
   letter never claimed to cover.** As written it demanded that
   `strip_prefix("(List OF |…)` hit ONLY `src/types.rs` and `src/ast/`. Measured
   over the whole tree at plan-writing time that pattern hits **24 files**, of
   which this plan's three target modules account for 30 of ~80 hits. The rest
   live in `src/ir/verify/` (18), `src/codegen/**` (14), `src/binary_repr/
   sections.rs` (5), `src/ir/lower.rs` (10) and `src/syntaxcheck/types.rs` (5) —
   all of which read IR/NIR type STRINGS, i.e. plan-106's "no-strings" end state
   and plan-107's relocation work, and none of which this letter's phases touch.

   Per §4 an unmeetable criterion is STRENGTHENED to something checkable, never
   weakened. It is restated in Phase 3 as a per-module table over the three
   modules this letter actually rewrites — a stricter bar for them (0, not
   "only types.rs and ast/") — with the out-of-scope residue measured, attributed
   to its owning plan, and recorded so a later census cannot mistake it for done.

6. **A fifth private hand-parser, `monomorph::helpers::func_type_parts`, is also
   deleted.** Not in the plan's census (it strips `FUNC(`, not a container
   prefix, so the plan's grep never saw it). It became dead once
   `concrete_type_name` classified via `ParameterType::parse`, and its unit tests
   were ported to read the same pieces off `ParameterType::Func` — which is
   strictly better, since the private copy ERASED the `ISOLATED` marker and the
   canonical variant preserves it.

7. **Phase 1's `.ncode` goldens were not generated, deliberately.** The phase
   text says "generate `.ir`/`.ncode` goldens", but rt-behavior fixtures carry no
   native goldens in this corpus (`find tests/rt-behavior -name '*.ncodesum'`
   matches only a handful of `crypto`/`net` dirs, and the harness discovers native
   goldens by filename). The `.ast`/`.ir`/`build.log`/`.run` set is the standard
   rt-behavior shape, and the `.ir` is where a `UserOf` parse/render regression
   would surface — the mangled instantiation names and every field type are
   rendered into it. `artifact-gate all` checks those `.ir` goldens: the corpus
   went 1249→1255 tests and 1718→1730 goldens with 0 diffs.

8. **Two shapes remain unsupported and are recorded rather than silently
   omitted** (both verified against the post-Phase-2 build):

   - `Holder OF Pairing OF Integer, String` (a MULTI-parameter template nested
     inside a user generic) → `cannot infer template arguments`. This is a
     **surface-grammar ambiguity, not a compiler defect**: the spelling is
     textually indistinguishable from a two-argument
     `Holder OF (Pairing OF Integer), String`, and MFBASIC has no bracketing to
     separate them. `ParameterType::parse` splits the argument list on top-level
     commas and yields 2 arguments for a 1-parameter template; it cannot do
     better, because it is dependency-light and holds no arity table. Resolving
     this needs a language/spec decision (delimiters, or a greedy-trailing-`OF`
     rule), which is outside plan-105's mandate to consolidate implementations.
   - `Holder OF Holder OF Holder OF String` (THREE levels) →
     `SYMBOL_UNKNOWN_TYPE: Type 'Holder' ...`. A doubly-nested constructor's
     expected type is the enclosing template's already-MANGLED field type, which
     no longer names a template, so the innermost constructor keeps the bare
     template name. Two levels — the shape Correction 2 fixes — is what this plan
     delivers. A speculative fix was drafted and REVERTED because it did not
     resolve the case; shipping it would have been unproven dead code.

   Both are documented in `user-generic-nested-user-rt`'s header with their exact
   diagnostics, so the next reader inherits the evidence rather than the surprise.

9. **A parameter of user-generic type must come LAST in a parameter list.**
   Discovered writing the Phase-1 fixtures, and worth recording because it shapes
   every generic API in the corpus: `FUNC f OF T(b AS Box OF T, v AS T)` does not
   parse — the type parser reads `OF`'s argument list greedily across commas, so
   `Box OF T, v` becomes a two-argument type and the parameter list never closes.
   `FUNC f OF T(v AS T, b AS Box OF T)` is fine. Same root cause as the first item
   in Correction 8. Noted in the fixture sources.

## Summary

This reverses plan-102-E's "no new variant" deferral with the safety that
deferral lacked: golden coverage added FIRST, then one grammar implementation.
Risk concentrates in `concrete_type_name`'s instantiation-key fidelity; the
fifth "parser" (`ast/expr.rs`) is the source grammar and stays by design.
