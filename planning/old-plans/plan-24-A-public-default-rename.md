# plan-24-A: PUBLIC rename + PUBLIC-by-default

Last updated: 2026-07-05
Overall Effort: x-large (whole plan-24: rename+default, EXPORT-in-package rule, PRIVATE file-scoping)
Effort: medium

Redesign the project visibility model. This sub-plan does the two mechanical-but-broad
pieces: rename the middle visibility level `PACKAGE` → `PUBLIC`, and flip the DEFAULT
visibility of every top-level declaration from `Private` to `Public`. After this,
declarations are project-wide visible unless explicitly marked `PRIVATE`, and a
multi-file executable can call across its own files with no annotations — which is the
behavioral outcome (the two-file benchmark split builds with zero `PUBLIC`/`PACKAGE`
markers).

It complements:

- `./mfb spec language modules-and-packages` (the visibility table; `src/docs/spec/language/13_modules-and-packages.md`)
- `./mfb spec language` bindings-and-scope, functions, types, grammar topics (default-visibility wording)

## 1. Goal

- Rename `Visibility::Package` → `Visibility::Public` end-to-end (enum, lexer keyword
  `PACKAGE`→`PUBLIC`, parser, AST/IR serialization strings `"package"`→`"public"`,
  resolver/syntaxcheck, NIR).
- Flip the default (no-modifier) visibility from `Private` to `Public` for every
  top-level decl kind: FUNC/SUB, TYPE/UNION/ENUM, top-level LET/MUT/RES, RESOURCE,
  FuncAlias.
- Field-visibility default follows: a field with no modifier defaults to the
  containing type's visibility (Export→Export, else Public).
- Fold in / remove the now-redundant interim patches: the monomorph `Private→Package`
  widening (`src/monomorph/lower.rs`) and the NIR `"package"→"public"` translation
  (`src/target/shared/nir/lower.rs`) — with default `Public`, collections instantiations
  are project-visible without widening, and IR emits `"public"` directly so NIR needs no
  translation.

### Non-goals (explicit constraints)

- No PRIVATE name-mangling yet (that is plan-24-C). Interim: same-name PRIVATE decls in
  different files still collide in `insert_top_level` — acceptable because default is now
  `Public` and explicit cross-file PRIVATE collisions are rare; C removes this limitation.
- No `EXPORT`-in-executable rule yet (plan-24-B).
- No change to value/copy/move semantics, layout/ABI, thread-transfer, or the `.mfp`
  export surface (only `Export` is written to `.mfp`; `Public`/`Private` are not — same as
  `Package`/`Private` were).
- The `#` internal-sigil machinery is untouched here.

## 2. Current State

- `Visibility { Private, Package, Export }` — `src/ast/types.rs:403`.
- Default is `Private`, hardcoded via `parse_visibility().unwrap_or(Visibility::Private)`
  at `src/ast/parser.rs:67,84,112`, `src/ast/items.rs:124,246,533,567`.
- Keyword lexing: `src/lexer.rs:99-102,674-681` (`Keyword::Package` ← `"PACKAGE"`).
- Parse: `parse_visibility` `src/ast/items.rs:412-422`.
- Field default: `effective_field_visibility` `src/syntaxcheck/helpers.rs:54-62`
  (Export→Export, Package|Private→Package).
- Resolution: `visible_from` — `Export|Package => true`, `Private => same file`
  (`src/resolver/mod.rs:491-495`, `src/syntaxcheck/mod.rs:1736-1745`).
- Serialization: `visibility_name` maps `Package→"package"` at `src/ir/json.rs:909-915`
  and `src/ast/serialize.rs:1257-1262`; `visibility_prefix` `src/ast/serialize.rs:1267`
  emits `"PACKAGE "`. Resource lowering `src/ir/lower.rs:475-479`.
- NIR validator accepts `"private"|"public"|"export"` (`src/target/shared/validate.rs:575,649`);
  the interim `nir_visibility` (`src/target/shared/nir/lower.rs`) maps `"package"→"public"`.
- Interim monomorph widening at `src/monomorph/lower.rs:260-261,276-277`.
- `.mfp` export gate keys on `Visibility::Export` only (`src/ir/lower.rs:120,134,457,476`) — unchanged.

## 3. Design Overview

Two independent edits that must land together (rename is a prerequisite for the default
flip to read cleanly):

1. **Rename** `Package`→`Public` everywhere it appears (enum variant, keyword, string
   literals `"package"`→`"public"`, prefix `"PACKAGE "`→`"PUBLIC "`). Because NIR already
   uses `"public"`, once IR emits `"public"` the `nir_visibility` translation collapses to
   the identity and is deleted; `validate.rs` already accepts `"public"`.
2. **Default flip**: change every `.unwrap_or(Visibility::Private)` on a top-level decl to
   `.unwrap_or(Visibility::Public)`, and flip `effective_field_visibility`'s fallback from
   `Package` to `Public`. `visible_from` logic is unchanged by the flip (Public and Export
   are both "always visible"); only the default assigned at parse time changes.

Correctness risk is concentrated in the golden churn: the default flip rewrites the
`visibility` field of essentially every function/type/binding in every AST/IR/NIR dump
from `"private"` to `"public"`. These are legitimate, expected updates — the validation
step must distinguish them from real regressions.

## Layout / ABI Impact

None. Visibility is not part of value layout, copy/transfer, or the native ABI. Native
codegen does not branch on visibility (only `validate.rs` reads it). `.mfp` export surface
is gated on `Export` only and is unchanged. Byte-identical native output for existing
single-file programs is expected (visibility never reaches the encoder).

## Phases

### Phase 1 — Rename PACKAGE → PUBLIC

Pure rename; no behavior change (Package and Public resolve identically). Lands alone.

- [ ] `src/ast/types.rs:403` — rename enum variant `Package` → `Public`.
- [ ] `src/lexer.rs` — `Keyword::Package`→`Keyword::Public`; match `"PUBLIC"` (drop `"PACKAGE"`).
- [ ] `src/ast/items.rs:412-422` — `parse_visibility`: `Keyword::Public => Visibility::Public`.
- [ ] `src/ir/json.rs:909`, `src/ast/serialize.rs:1257` — `visibility_name`: `Public => "public"`.
- [ ] `src/ast/serialize.rs:1267` — `visibility_prefix`: `Public => "PUBLIC "`.
- [ ] `src/ir/lower.rs:475-479` — resource visibility: `Public => "public"`.
- [ ] `src/resolver/mod.rs:491`, `src/syntaxcheck/mod.rs:1742` — `visible_from` match arm rename.
- [ ] `src/syntaxcheck/helpers.rs:54` — `effective_field_visibility` arm rename.
- [ ] Delete interim `nir_visibility` translation (`src/target/shared/nir/lower.rs`); restore
      direct `.clone()`/`as_deref()` copies now that IR emits `"public"`.
- [ ] Revert interim monomorph widening (`src/monomorph/lower.rs:254-263,276-277`) — keep the
      `into_project` structure but drop the `Private→Package` `.map`.
- [ ] Migrate the one test `.mfb` using `PACKAGE` → `PUBLIC` (grep `^\s*PACKAGE ` under tests/).
- [ ] Docs: `src/docs/spec/language/13_modules-and-packages.md` visibility table
      `PACKAGE`→`PUBLIC`; grep `src/docs/**` for other `PACKAGE`-as-visibility mentions.

Acceptance: `cargo build`; `scripts/test-accept.sh` green after regenerating goldens whose
only diff is `"package"`→`"public"` / `PACKAGE `→`PUBLIC ` strings. No `"package"` visibility
string remains in the codebase (`grep -rn '"package"' src/ | grep -i visib` empty).
Commit: —

### Phase 2 — Flip default to PUBLIC

Change the parse-time default and field default. Highest churn.

- [ ] `src/ast/parser.rs:67,84,112` and `src/ast/items.rs:124,246,533,567` —
      `.unwrap_or(Visibility::Private)` → `.unwrap_or(Visibility::Public)` for every top-level
      decl kind (binding, function, type, resource, func-alias).
- [ ] `src/syntaxcheck/helpers.rs:54-62` — `effective_field_visibility` fallback
      `Package|Private => Package` becomes `_ => Public` (Export→Export retained).
- [ ] Audit `Visibility::Private` construction sites that are NOT parse-defaults
      (e.g. `src/escape.rs` synthesized functions, `src/target/shared/nir/lower.rs:142`
      global-initializer) — these must stay `Private` deliberately; confirm each.
- [ ] Tests: add `tests/visibility-default-public-valid` (two files, one calls the other's
      unmarked FUNC → runs) and keep an explicit-`PRIVATE`-still-file-local invalid case.
- [ ] Docs: update `13_modules-and-packages.md` ("default is `Public`"), plus
      `bindings-and-scope`, `functions`, `types`, and `grammar` topics under
      `src/docs/spec/language/**` wherever they state the default is Private.

Acceptance: the two-file `benchmark/mfb` split (list.mfb + main.mfb) builds and runs with
NO `PUBLIC`/`PACKAGE`/`EXPORT` markers on the moved functions and produces the 33 correct
non-random list checksums. `scripts/test-accept.sh` green after golden regeneration; each
regenerated diff is confirmed to be a `private`→`public` default flip, not a behavior change.
Commit: —

## Validation Plan

- Function tests: n/a (no builtin function changed); language tests under
  `tests/visibility-*` cover default-public and explicit-private.
- Runtime proof: two-file benchmark split builds + correct checksums with no visibility markers.
- Doc sync: `src/docs/spec/language/13_modules-and-packages.md` (+ scope/functions/types/grammar).
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- Whether to keep `PACKAGE` as a deprecated keyword alias — recommend NO (clean rename;
  only one test uses it). (§Phase 1)

## Summary

Broad but mechanical. Risk is entirely in proving the large golden churn is the intended
`private→public` default flip and nothing else. Native output stays byte-identical.
Unblocks plan-24-B (EXPORT rule) and plan-24-C (PRIVATE file-scoping).
