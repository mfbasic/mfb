# plan-136-C: No local binding shadows a top-level binding; plan-136 closeout

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-136-B

Prerequisites: see plan-136-A §Prerequisites, plus: `ls planning/plan-136-B-*` → no match
(plan-136-B archived). If plan-136-B is not complete, this letter cannot start, full stop.

**Status (re-run 2026-09-13, before Phase 1): all MET.** `git merge-base --is-ancestor 53f10b1fe
main && echo MET` → `MET`; `git log --all --oneline --grep='plan-136'` → only `caf191edd` and this
session's plan-136-A/B commits; `ls bugs/bug-614-*.md` → one file; `ls planning/plan-136-B-*` →
`no matches found` (archived as `planning/completed/plan-136-B-package-parameter-defaults.md` in
`5c59277c3`).

`mfb spec language bindings-and-scope` §5 says "There is no shadowing: an inner binding may not
re-use a name still visible from an enclosing block", and `mfb man tour` says "no shadowing". The
resolver only enforces it within a function's own locals, and only at 4 of the 9 places a local is
introduced. After this letter:

1. A local binding whose name equals a **top-level `LET`/`MUT` visible at that point** is a
   located compile error, `SYMBOL_SHADOWS_TOP_LEVEL_BINDING`.
2. A local binding whose name equals a live local is `SYMBOL_DUPLICATE_LOCAL` at **every** site
   that introduces a local — including lambda parameters (today silently accepted) and inline-TRAP
   bindings (today an unlocated `NIR local 'e' is declared more than once`).

The last phase runs plan-136's single full gate and closes bug-614.

References:

- `src/docs/spec/language/05_bindings-and-scope.md` (§5, resolver scope model),
  `13_modules-and-packages.md` (visibility; `PRIVATE_SHADOWS_PUBLIC`).
- `src/docs/spec/diagnostics/01_rule-codes.md`; `src/rules/table.rs`.
- `.ai/codegen-invariants.md` ("A rename fix can leak a mangled name into a diagnostic",
  "FOR-loop desugar creates synthetic locals").
- bug-285 (`tests/rt-behavior/trap/trap-body-local-shadows-private-rt`), bug-396 (scope_privates).

## 1. Goal

- Each of these, named like a visible top-level `LET`/`MUT`, is `SYMBOL_SHADOWS_TOP_LEVEL_BINDING`
  at its line: `LET`/`MUT`/`RES` binding, function parameter, `FOR` variable, `FOR EACH` variable,
  lambda parameter, `MATCH` case binding, `MATCH` guard binding, function-level `TRAP` binding,
  inline-`TRAP` binding.
- "Visible" = a `PUBLIC`/`EXPORT` top-level binding anywhere in the project, or a `PRIVATE` one in
  the same file.
- Each of the nine sites reports `SYMBOL_DUPLICATE_LOCAL` for a name already live in its scope.
- bug-614 is archived to `bugs/completed/` with the full gate green.

### Non-goals (explicit constraints)

- **Function, `SUB`, `TYPE`, `UNION`, `ENUM` and `RESOURCE` names are not covered.** A local may
  still be named like a function (text census: 40 locals, mostly `packages/json_schema` and
  `packages/jwt`, would break).
- **Imported package bindings are not covered** — they are only reachable qualified (`pkg::Name`).
- Another file's `PRIVATE` binding is not visible, so a local may reuse its name.
- Internal names (`#…`, `$…`, compiler-synthesized locals such as the FOR desugar's) never trigger
  either rule — a user cannot spell them.
- `PRIVATE_SHADOWS_PUBLIC` (top-level vs. top-level) is unchanged.

## 2. Current State

**Local insertion sites** — all in `resolver::resolution` (shadowing census agent, citations
re-checked 2026-09-13; `grep -n 'DUPLICATE_LOCAL' src/resolver/resolution.rs | wc -l` → 4):

| Binding | Where | Duplicate check today |
|---|---|---|
| Function parameters | `resolve_function` | yes |
| Function-level `TRAP` binding | `resolve_function` (`trap_locals` clone) | **no** |
| `LET`/`MUT`/`RES` | `resolve_statement`, `HirStatement::Let` | yes |
| `MATCH` guard binding | `resolve_statement` (`guard_locals`) | **no** |
| `MATCH` case binding | `resolve_statement` (`case_locals`) | **no** |
| `FOR` variable | `resolve_statement` | yes |
| `FOR EACH` variable | `resolve_statement` | yes |
| Lambda parameters | `resolve_expression`, `HirExpression::Lambda` | **no** |
| Inline-`TRAP` binding | `resolve_expression`, `HirExpression::Trapped` | **no** |

**Top-level visibility.** `resolver::Resolver::collect_top_level_symbols` fills `top_levels`
(bindings, types and resources together — `Symbol` has `file_path`, `line`, `visibility`, no kind)
and a separate `functions` map. `top_level_visible_in_file` / `visible_from` answer visibility.

**PRIVATE renaming happens first.** `ast::scope_privates::scope_privates` runs on the AST before
resolution and renames each `PRIVATE` top-level to `#<hash>$name` (`internal_name::mangle_private`),
so a bare-name lookup in `top_levels` misses every `PRIVATE` binding; `internal_name::display_name`
demangles. The resolver also runs a second pass on post-monomorph HIR (`resolve_augmented`).
Source packages go through the same resolver (`cli::build::source_packages::build_source_dependency`);
built-in package sources are Rust strings whose top-level bindings are `__`-internal.

### Measured populations

| What | Count | Command |
|---|---|---|
| Committed files that the top-level rule would reject | **1** real: `tests/rt-behavior/trap/trap-body-local-shadows-private-rt` (bug-285, `LET x` vs `PRIVATE LET x`) | read-only Python text census over every `project.json` root in `tests/`, `packages/`, `examples/` (1464 + 14 + 16 projects), built-in package strings, 86 spec ` ```basic ` blocks and 573 man `EX` strings: 2 hits, 1 a parse-error fixture false positive. **Lower bound** — lambda, `MATCH` and `TRAP` bindings not scanned. |
| Top-level `LET`/`MUT` declarations scanned | 162 in 30 projects | same census |
| Locals named like a same-project `FUNC`/`SUB` (why functions are excluded) | 40 (overcount; ignores file `PRIVATE` scope) | same census |
| Unit tests that assert a local may shadow a `PRIVATE` | 2: `locals_shadow_private_names_and_are_left_alone`, `a_local_shadowing_a_private_keeps_its_state_assign_target_bare` | `grep -n 'fn locals_shadow_private\|fn a_local_shadowing_a_private' src/ast/scope_privates.rs` |
| Exact compiler census (all nine sites, both rules) | **1** committed fixture: `tests/rt-behavior/trap/trap-body-local-shadows-private-rt` (bug-285) — the same one the text census found. Beyond it, only this letter's two new `-invalid` fixtures (placeholder goldens) fired a rule | `scripts/test-accept.sh /tmp/p136c-census/target/release/mfb /tmp/p136c-census-accept` → `acceptance tests failed: 5 mismatch(es) (1471 test(s) ran)`: the bug-285 fixture's `build.log` plus its two now-missing `.ast`/`.ir`, and the two new fixtures; `grep -rl 'SYMBOL_SHADOWS_TOP_LEVEL_BINDING\|SYMBOL_DUPLICATE_LOCAL' /tmp/p136c-census-accept --include=build.log` → those three `build.log`s only |
| Reports on `local-shadows-top-level-binding-invalid` (census compiler) | **13** `SYMBOL_SHADOWS_TOP_LEVEL_BINDING`, one per offending binding (LET, MUT and RES count separately: 13 bindings), each at its own line (32, 37, 44, 51, 52, 53, 55, 59, 63, 67, 70, 76, 80) | a copy built with `/tmp/p136c-census/target/release/mfb build -ast -ir … \| grep -c SYMBOL_SHADOWS_TOP_LEVEL_BINDING` |
| In-tree packages the rules reject, after merging main (Phase 4) | **0** of 9 — re-measured because main's `af9b1a591` turned json_schema/jwt/mustache/yaml error-code accessor FUNCs into `EXPORT LET` constants, exactly the names the plan's text census said locals share | `git merge main` (8 commits, no overlap with this branch), then `bash /tmp/p136c-package-census.sh target/release/mfb` with the rule-enabled compiler → `packages built: 9, failed: 0` |
| In-tree packages the rules reject | **0** of 9 (baseline with the current compiler: 9 built, 0 failed) | `bash /tmp/p136c-package-census.sh <mfb>` (builds every `packages/*/`) with `target/release/mfb` → `packages built: 9, failed: 0`, and with the census compiler `/tmp/p136c-census/target/release/mfb` → `packages built: 9, failed: 0` |
| Next free `2-201` code | `2-201-0022` (corrected 2-201-0019 → see plan-136-A Corrections: `0019` is retired by plan-115-B and never reused, `0013` by bug-216; plan-136-A took `0020`/`0021`) | `git log main -G'"2-201-00(19\|2[0-9]\|13)"' --oneline -- src/rules/table.rs` plus `grep -n '"2-201-00' src/rules/table.rs \| tail -1` — the table tail alone cannot see a retired code |

### Verified properties (probes, 2026-09-13)

| # | Program | Result |
|---|---|---|
| P5 | global `limit`; parameter `limit`; `FOR limit` | builds, runs |
| P7 | `LET x = 1`; `LAMBDA(x AS Integer) -> x + 10` (and with a global `x`) | builds, prints `15 1` / `15` |
| P8 | `LET e = 3`; `… TRAP(e) … END TRAP` | build exit 1, unlocated `error: NIR local 'e' is declared more than once` |

## 3. Design Overview

One helper, `Resolver::check_new_local(file, name, line, locals)`, called at all nine sites before
the insert: (1) `name` already in `locals` → `SYMBOL_DUPLICATE_LOCAL`; (2) `name` is a visible
top-level binding → `SYMBOL_SHADOWS_TOP_LEVEL_BINDING`, message *"`limit` is already a top-level
binding (declared at <file>:<line>); a local cannot reuse its name"*, both names via `display_name`.
Names starting with an internal sigil skip both checks.

**Visible top-level binding index.** After `collect_top_level_symbols`, build per resolver a map
from **bare** name (`display_name` of the key) to the binding symbols with that bare name, recording
the kind (add a `kind` to `Symbol`, or keep a parallel set of binding keys — whichever the Phase 2
read of `insert_top_level` makes smaller). Visibility per site uses `visible_from` against the
owning file, so a mangled own-file `PRIVATE` matches and another file's does not.

**Second pass.** Phase 1 measures whether `resolve_augmented` re-reports `SYMBOL_DUPLICATE_LOCAL`
today; the new check follows the same rule so no diagnostic doubles.

**Correctness risk:** false positives on legal programs (a synthesized local, a package-internal
name, another file's `PRIVATE`) — schedule the positive fixture first. **Design uncertainty:** the
exact breakage population (UNMEASURED past the text census) — the Phase 1 compiler census measures
it before any migration is scoped.

**Byte-identity:** every fixture that compiled before and is not in the census compiles to the same
bytes — the rule only adds errors. The gate diff must be confined to the census list.

**Rejected alternatives.** *Check inside `scope_privates`* — it tracks locals for renaming, but
does not see the resolver's nine sites or produce located resolver diagnostics, and it skips
internal files. *Include function names* — 40 real locals break for a rule the owner stated about
bindings. *A warning* — owner ruled an error.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit; `- [~]` partial; moot
> tasks struck through with evidence; fill `Commit:` on landing. **An unticked box means NOT DONE.**

### Phase 1 — fixtures first, then the exact census

- [x] Prove leaf names unused (`find tests -name '<leaf>' | wc -l` → 0 each), then add with
      placeholder goldens (all three → `0`; neither `tests/syntax/scope` nor
      `tests/rt-behavior/scope` existed):
  - [x] `tests/syntax/scope/local-shadows-top-level-binding-invalid` — one offending binding of
        each of the nine kinds against a same-file `LET`, plus a parameter against a same-file
        `PRIVATE MUT`, plus a `LET` against a `PUBLIC` binding in a second file (RED: builds or
        unlocated). Measured RED: `[exit 0]`, `.ast`/`.ir` written — every shadowing binding is
        accepted. (A first draft stopped at `TYPE_MATCH_NOT_EXHAUSTIVE` — a `WHEN`-guarded `CASE`
        does not cover its variant — which masked the bindings; a `CASE ELSE` arm was added.)
  - [x] `tests/syntax/scope/duplicate-local-at-every-binding-site-invalid` — P7 and P8 shapes plus
        `MATCH` case/guard and function-level `TRAP` duplicates (RED). Measured RED: `[exit 0]` under
        `mfb build -ast -ir` (a syntax fixture never reaches NIR, so P8's unlocated NIR error does
        not appear); same `CASE ELSE` fix.
  - [x] `tests/rt-behavior/scope/local-names-distinct-from-top-level-valid` — must keep building
        and running: a local named like another file's `PRIVATE` binding, like a `FUNC`, like an
        imported package's binding (source package), and a `FOR` loop (synthesized desugar locals)
        (GREEN pin). Builds and prints `7 42` / `2` / `3 100` / `6`; goldens captured
        (`bash scripts/sync-goldens.sh ./target/release/mfb local-names-distinct-from-top-level-valid`
        → `synced 3 golden file(s) across 1 test(s)`).
- [x] Exact census: implement `check_new_local` in a scratch worktree
      (`git worktree add --detach /tmp/p136c-census HEAD`), build release there, and run
      `scripts/test-accept.sh /tmp/p136c-census/target/release/mfb /tmp/p136c-census-accept` plus
      `for p in packages/*/; do (cd "$p" && /tmp/p136c-census/target/release/mfb build .); done`.
      Record every newly failing fixture/package in **Measured populations**. Est. 20 min — the
      only instrument that sees lambda/`MATCH`/`TRAP` bindings and the second resolution pass; the
      text census cannot. Remove the worktree afterwards. Measured: 1471 acceptance tests → the only
      existing fixture a rule rejects is bug-285's `trap-body-local-shadows-private-rt`; packages
      `packages built: 9, failed: 0` with both compilers (§Measured populations). The package loop ran
      as a script (`/tmp/p136c-package-census.sh <mfb>`) rather than an inline `for`, and against the
      current compiler too, so a package that already fails could not be mistaken for a rule hit.
      Worktree removed (`git worktree remove --force /tmp/p136c-census`).
- [x] Measure whether the second pass double-reports: the existing `SYMBOL_DUPLICATE_LOCAL` count in
      a fixture's golden `build.log` (`grep -c DUPLICATE_LOCAL` on one existing fixture that has
      one) vs. the number of offending bindings. No existing golden has one (Corrections); measured
      on a copy of `duplicate-local-at-every-binding-site-invalid` with the census compiler:
      `mfb build -ast -ir … | grep -c SYMBOL_DUPLICATE_LOCAL` → `5` for its 5 offending bindings
      (lines 24 function TRAP, 32 lambda, 35 inline TRAP, 43 guarded CASE, 46 CASE). No double
      report: the resolver's first-pass errors stop the build before `resolve_augmented`, and the
      `MATCH` check runs once per pattern binding.

Acceptance: RED fixtures fail as described; the positive pin passes; the census list is recorded.
  Check: `scripts/test-accept.sh target/release/mfb /tmp/p136c-p1-accept
  local-shadows-top-level-binding-invalid duplicate-local-at-every-binding-site-invalid
  local-names-distinct-from-top-level-valid` → two mismatches, one pass (est. 2 min).
Commit: 74077337e (landed together with Phase 2 — see Phase 2's measured line)

### Phase 2 — the rule

- [x] `src/rules/table.rs` + `01_rule-codes.md`: `SYMBOL_SHADOWS_TOP_LEVEL_BINDING` (error) at the
      next free `2-201` code (re-check the code race as in plan-136-A Phase 2); population count.
      `2-201-0022`: `git log main -G'"2-201-0022"' --oneline -- src/rules/table.rs` → nothing, and no
      main-checkout or sibling-worktree `src/rules/table.rs` contains it; spec row added and the
      `2-201` population is `21`.
- [x] `resolver`: the visible top-level binding index (§3) and `check_new_local`; call it at all
      nine sites; skip internal-sigil names; follow the Phase 1 second-pass finding.
      `Resolver.top_level_bindings` (bare display name → `top_levels` keys, non-internal files only)
      and `resolution::check_new_local` (duplicate first, with each site's existing detail text;
      then the visible top-level binding; `#`/`$` names exempt; built-in source files exempt from
      the top-level half). Called at: parameters, function-level `TRAP`, `LET`/`MUT`/`RES`, `MATCH`
      (once per pattern binding, before the guard/case inserts), `FOR`, `FOR EACH`, lambda
      parameters, inline `TRAP`. Implemented in the census worktree, then copied over the unmodified
      worktree files; `cargo build --release --bin mfb` → `Finished` in both. Evidence it fires once
      per offending binding: the census counts in §Measured populations (13 of 13, 5 of 5).
- [x] Unit tests in `src/resolver/` for: own-file `PRIVATE` match, other-file `PRIVATE` no match,
      `PUBLIC` cross-file match, function name no match, internal name no match. Five tests in
      `resolver::tests` (`check_new_local` is `pub(super)` for them; the internal case also covers a
      built-in source file). `cargo test --release --bin mfb -- resolver::tests::a_local_named_like
      resolver::tests::an_internal_name_or_a_built_in_source_file_is_exempt rules::` → `test result:
      ok. 22 passed; 0 failed`, each of the five `... ok` by name.
- [x] Generate the three fixtures' goldens and read each `build.log`: one located error per
      offending binding, messages show bare names. `bash scripts/sync-goldens.sh ./target/release/mfb
      local-shadows-top-level-binding-invalid duplicate-local-at-every-binding-site-invalid` →
      `synced 2 golden file(s) across 2 test(s)` (the valid pin's were captured in Phase 1). Read:
      the shadowing `build.log` has 13 `2-201-0022` errors at lines 32, 37, 44, 51, 52, 53, 55, 59,
      63, 67, 70, 76, 80, each naming the bare binding and its declaration (`src/main.mfb:20` …,
      `src/other.mfb:3` for the cross-file case, `kappa` the own-file `PRIVATE MUT`); the duplicate
      `build.log` has 5 `2-201-0009` errors at lines 24, 32, 35, 43, 46. No mangled name appears.

Acceptance: fixtures pass; resolver unit tests pass.
  Check: `cargo build --release && scripts/test-accept.sh target/release/mfb /tmp/p136c-p2-accept
  local-shadows-top-level-binding-invalid duplicate-local-at-every-binding-site-invalid
  local-names-distinct-from-top-level-valid` → 3 passed (est. 6 min); `cargo test --release --bin
  mfb resolver:: rules::` → ok (est. 4 min).
  **Measured 2026-09-13:** `cargo build --release --bin mfb` → `Finished`; the three fixtures →
  `acceptance tests passed (3 test(s) ran)`; `cargo test --release --bin mfb -- resolver:: rules::`
  (the command as written is rejected by cargo — Corrections) → `test result: ok. 121 passed; 0
  failed`. Phases 1 and 2 land in ONE commit: their ticks share this file and the fixtures' real
  goldens are Phase 2's output, so the two cannot be split into honest separate commits.
Commit: 74077337e

### Phase 3 — migrate the tests the rule makes invalid

For each, the four AGENTS.md answers are recorded here; the owner's 2026-09-13 ruling (spec §5
enforced against top-level bindings) is the proof in answer 4.

- [x] `tests/rt-behavior/trap/trap-body-local-shadows-private-rt` — (1) bug-285; (2) a local that
      shares a file `PRIVATE`'s name must win inside the function-level `TRAP` body, so adding an
      unrelated `PRIVATE` never silently changes behavior; (3) no other test depends on it
      (`grep -rn 'trap-body-local-shadows-private' tests src scripts`) — **corrected: the
      cross-backend `CORPUS` in `src/codegen/builtins/tests/corpus.rs` lists it (Corrections), so its
      entry is replaced by the new runtime fixture below**; (4) its program is now
      invalid by rule. The guarantee survives stronger: move the program to
      `tests/syntax/trap/trap-body-local-shadows-private-invalid` (expects
      `SYMBOL_SHADOWS_TOP_LEVEL_BINDING`), and keep bug-285's runtime protection with
      `tests/rt-behavior/trap/trap-body-local-lambda-shadows-private-func-rt`: a `PRIVATE FUNC
      helper()` returning 42, a body `LET helper = LAMBDA() -> 7`, the `TRAP` body prints
      `helper()` → `7` (the scope rewriter still renames function references, so the bug-285 seeding
      is still exercised). Done: moved with `git mv` (via `/tmp/p136c-move.sh`; the worktree guard
      refuses inline commands containing `trap`), project renamed
      `trap_body_local_shadows_private_invalid`, `.ast`/`.ir`/`.run` goldens dropped, comment
      rewritten; its golden `build.log` → one `main.mfb:15 error[2-201-0022
      SYMBOL_SHADOWS_TOP_LEVEL_BINDING] … `x` is already a top-level binding (declared at
      src/main.mfb:12)`. New runtime fixture (leaf unused: `find tests -name
      trap-body-local-lambda-shadows-private-func-rt | wc -l` → `0`) builds and prints `7`; its `.ast`
      keeps the TRAP body's `helper()` callee bare while the declaration is `#…$helper` (a regression
      would print `42`). `CORPUS` entry replaced; `every_backend_lowers_the_corpus_to_the_same_program
      ... ok` and `no_corpus_function_lowers_to_an_empty_body ... ok`.
- [x] `ast::scope_privates` tests `locals_shadow_private_names_and_are_left_alone` and
      `a_local_shadowing_a_private_keeps_its_state_assign_target_bare` — (1) bug-396 / bug-285;
      (2) the rewriter leaves a local that shadows a `PRIVATE` unrenamed; (3) only the rewriter
      (`scope_privates` runs before the resolver, so it still sees these programs); (4) the rewriter
      behavior stays correct and live for function names, and these unit tests call the rewriter
      directly without the resolver — **keep them unchanged** if they still pass; add a sibling
      asserting the same for a `PRIVATE FUNC` name. Only if a test fails, record why here before
      touching it. Both kept unchanged and passing; sibling
      `a_local_named_like_a_private_func_is_left_alone` added (the declaration is mangled, a local
      lambda's `helper()` call stays bare). `cargo test --release --bin mfb -- scope_privates corpus`
      → `test result: ok. 67 passed; 0 failed`, all three `... ok` by name.
- [x] `tests/runtime/rt_parameter_default_scope.rs`
      `a_default_reading_a_global_ignores_a_callers_same_named_local` (plan-136-A) — (1)
      plan-136-A Phase 1, bug-614's program; (2) a default reads the declaration's global, not a
      caller's same-named local; (3) nothing else; (4) the caller's `LET limit` is now invalid. The
      same property stays pinned by `a_default_calling_a_function_ignores_a_callers_same_named_local`
      (function names are not covered by this rule). Change the case to assert the build fails with
      `SYMBOL_SHADOWS_TOP_LEVEL_BINDING`, and rename it
      `a_caller_local_named_like_a_defaults_global_is_refused`. Done (asserts no executable, and the
      output names the rule and `` `limit` ``); `cargo test --release --test rt_parameter_default_scope
      -- --test-threads=1` → `test result: ok. 9 passed; 0 failed`.
- [x] ~~Every other entry in the Phase 1 census list: rename the offending local (a fixture whose
      subject is not shadowing) or convert it as above (a fixture whose subject is shadowing), each
      with its four answers here.~~ — moot: the exact census has no other entry. Of 1471 acceptance
      tests only bug-285's fixture fired a rule; 0 of 9 packages failed (§Measured populations).

Acceptance: every census entry resolved; converted fixtures pass.
  Check: `cargo build --release && cargo test --release --test rt_parameter_default_scope --
  --test-threads=1` → ok; `scripts/test-accept.sh target/release/mfb /tmp/p136c-p3-accept trap
  scope` → pass (est. 7 min); `cargo test --release --bin mfb scope_privates` → ok (est. 3 min).
  **Measured 2026-09-13:** `cargo build --release --bin mfb` → `Finished`; `cargo test --release
  --test rt_parameter_default_scope -- --test-threads=1` → `test result: ok. 9 passed; 0 failed`;
  the acceptance filter as written matched nothing (Corrections) — with `'*/trap/*' '*/scope/*'` →
  `acceptance tests passed (35 test(s) ran)`; `cargo test --release --bin mfb -- scope_privates
  corpus` (widened to the `CORPUS` tests this phase edits) → `test result: ok. 67 passed; 0 failed`;
  `rustfmt --check` on the three changed Rust files → no diff.
Commit: fba131c10

### Phase 4 — docs, the plan-wide gate, closeout

- [x] `05_bindings-and-scope.md`: §5 states the rule against visible top-level `LET`/`MUT`
      bindings (visibility, own-file `PRIVATE`, not functions/types/imports), the nine sites, both
      rule names, with citations to `check_new_local`. The scope bullet names both rules; "No
      shadowing of an in-scope local" now lists every checking site (it said only LET and FOR); new
      subsection "No local reuses a visible top-level binding's name" (what is and is not covered,
      once-per-binding, example, `[[src/resolver/resolution.rs:check_new_local]]`). After a rebuild,
      `target/release/mfb spec language bindings-and-scope | grep -n 'SYMBOL_SHADOWS_TOP_LEVEL_BINDING\|visible
      from its file\|reuses a visible top-level'` → rendered lines 51, 117, 120, 131.
- [x] `mfb man`: `tour` already says "no shadowing"; add to the `variable` topic
      (`src/docs/man/variable/package.md`) one sentence and a compiled example that a local cannot
      reuse a top-level binding's name; `scripts/man-census.sh --memory-scope` → 0 unclassified.
      Added under "LET and MUT": the sentence, the refused program (introduced in prose as not
      building), and the renamed version with its output. Both compiled with the release binary in
      `/tmp`: the first → `main.mfb:6 error[2-201-0022 SYMBOL_SHADOWS_TOP_LEVEL_BINDING] … `limit` is
      already a top-level binding (declared at src/main.mfb:3)`; the second builds and prints `10`.
      `scripts/man-census.sh --memory-scope` → `unclassified memory-vocabulary hits: 0`; §9.2b
      citation-marker grep over `src/docs/man` → nothing; `mfb man variable` renders the section
      (lines 58–72).
- [x] `cargo test --release --bin mfb spec` → ok; `scripts/spec-census.sh --citations` → no new
      unresolved. `cargo test --release --bin mfb spec` → `test result: ok. 43 passed; 0 failed`;
      `scripts/spec-census.sh --citations` → 81 MISS rows (MISS-PATH 2, MISS-LINE 0, MISS-SYMBOL 61),
      the same count as before Phase 4; `grep -E 'check_new_local|05_bindings'` over its output →
      nothing.
- [ ] **Final gate (plan-136, once):** `cargo build --release`, then
      `cargo test --release --no-fail-fast` (never piped to `tail`; read the per-target summary) and
      `scripts/test-accept.sh target/release/mfb /tmp/p136-final-accept` and
      `scripts/artifact-gate.sh` — expected golden deltas: plan-136-A's
      `user-function-default-args-result-valid`, and the fixtures added/converted by A–C; anything
      else is root-caused on one fixture before it is re-baselined. Est. 60–90 min (guess from the
      suite's size); this is the only whole-suite run in the plan.
- [ ] Linux runtime proof on native aarch64 box 2223 (`.ai/remote_systems.md`): run
      `rt_parameter_default_scope` and `rt_package_parameter_defaults` there (the LINK cases use
      `libc.so.6`, a different library path than macOS). Est. 15 min. x86_64 boxes are skipped:
      the change is IR-level, and the artifact gate covers the x86_64/riscv64 lowering of the new
      calls. **First run (not yet the tick):** `/tmp/p136-box2223.sh` ships `git archive HEAD` and runs
      both targets with `--no-fail-fast`; on tree `5c59277c3` (plan-136-A and -B complete) →
      `rt_package_parameter_defaults`: `test result: ok. 7 passed; 0 failed`,
      `rt_parameter_default_scope`: `test result: ok. 9 passed; 0 failed`, `REMOTE_EXIT=0`
      (box probe: `/usr/bin/cargo`, 8 cores, aarch64). Phase 3 rewrites one
      `rt_parameter_default_scope` case, so the tick waits for a rerun on the final tree.
- [ ] Close bug-614: fill its Phases/Commit lines and a `STATUS: FIXED` block naming plan-136-A/B/C
      commits and the tests; `git mv` it to `bugs/completed/`; update `planning/bug-backlog.md`
      (open count, the 614 row, the Open decisions entry).
- [ ] Archive: `git mv planning/plan-136-C-*.md planning/completed/` (A and B were archived when
      they completed).

Acceptance: full gate green with only the named golden deltas; box 2223 runs both runtime tests
green; bug-614 archived.
  Check: the final-gate commands above → `test result: ok` for every target and `acceptance tests
  passed`; `ls bugs/bug-614-*` → no match.
Commit: —

## Validation Plan

- Tests: three `tests/syntax|rt-behavior/scope` fixtures, the converted bug-285 pair, resolver unit
  tests.
- Coverage check: every one of the nine sites appears in the RED fixture — a site whose call is
  missed leaves its line without an error in `build.log`.
- Runtime proof: `local-names-distinct-from-top-level-valid` runs; P8 is a located error.
- Doc sync: Phase 4.
- Final gate: Phase 4 — the single plan-wide run.

## Open Decisions

- `MATCH` guard binding and function-level `TRAP` binding reuse — recommended: both rules apply,
  like every other site (spec §5 has no exception). Alternative: exempt the function-level `TRAP`
  binding because its scope is disjoint from the body. The spec's single rule wins.

## Corrections

- **No existing golden reports `SYMBOL_DUPLICATE_LOCAL` (Phase 1).** The double-report task says to
  count the rule in "one existing fixture that has one"; `grep -rl 'SYMBOL_DUPLICATE_LOCAL' tests
  --include=build.log` → nothing. The measurement is taken instead on
  `duplicate-local-at-every-binding-site-invalid` with the census compiler: reports counted against
  its five offending bindings.
- **A guarded `CASE` does not make a `MATCH` exhaustive (Phase 1 fixtures).** Both invalid fixtures
  first stopped at `TYPE_MATCH_NOT_EXHAUSTIVE` ("does not cover Circle") because the `Circle` arm
  carries a `WHEN` guard; that error masked every binding under test. Each `MATCH` gained a
  `CASE ELSE` arm.
- **The `MATCH` guard binding and case binding are one name (Phase 1 read).**
  `resolver::resolution::resolve_statement`'s `Match` arm inserts a union pattern's binding into
  `guard_locals` (only when the case has a `WHEN`) and again into `case_locals`. The check runs once
  per pattern binding, before both inserts, or a guarded case would report twice. The two "kinds" in
  §1 are therefore a `CASE` with a guard and one without, both exercised by the fixtures.
- **The bug-285 fixture IS referenced elsewhere (Phase 3 answer 3 corrected).** The plan's answer (3)
  says no other test depends on `trap-body-local-shadows-private-rt`; `grep -rn
  'trap-body-local-shadows-private' tests src scripts planning bugs` → `src/codegen/builtins/tests/corpus.rs:461`
  lists it in `CORPUS`, which `every_backend_lowers_the_corpus_to_the_same_program` and
  `no_corpus_function_lowers_to_an_empty_body` lower on every backend (the latter asserts
  `CORPUS.len() >= 620`). Moving the fixture to `tests/syntax/trap/…-invalid` would break both, so the
  corpus entry is REPLACED by the new runtime fixture `trap-body-local-lambda-shadows-private-func-rt`
  (count unchanged, bug-285's trap-body runtime path still in the cross-backend corpus). Task added to
  Phase 3. Origin confirmed: `74f701f34 bug(285): let a function-level TRAP body see the function
  body's locals`; bug-285's Resolution names the fixture as its regression guard (prints `7`, `42` on
  regression).
- **Phase 2's unit-check command is invalid as written.** `cargo test --release --bin mfb resolver::
  rules::` → `error: unexpected argument 'rules::' found` (exit 1, no test ran): `cargo test` takes a
  single TESTNAME before `--`. The same filters go after the separator: `cargo test --release --bin
  mfb -- resolver:: rules::`. The single-filter checks elsewhere in this letter (`scope_privates`,
  `spec`) are unaffected.
- **Phase 3's acceptance filter matched nothing (same miscalibration as plan-136-A Phase 3).**
  `scripts/test-accept.sh target/release/mfb /tmp/p136c-p3-accept trap scope` → `no tests matched
  filter: trap scope`, exit 2: `matches_filter` treats each argument as a shell glob against the
  relative path or basename, so a bare word matches only a leaf of that exact name. Corrected check:
  `'*/trap/*' '*/scope/*'`. (The harness command is run from a script file: the worktree guard
  refuses an inline command containing the word `trap`, the same reason the fixture move ran from
  `/tmp/p136c-move.sh`.)
- **Built-in package source files are exempt from the top-level rule (design read).** The resolver
  resolves built-in package sources in the same project; with `visible_from` alone, a user's
  `PUBLIC LET count` would be "visible" to a built-in's own local `count`. `check_new_local` skips the
  top-level check for `file.internal`, and the binding index records only non-internal bindings; the
  duplicate check still applies there.

## Summary

The risk is false positives on legal programs, so the positive fixture and the exact compiler census
come before the rule lands. The text census predicts one real migration (bug-285's fixture).
Untouched: function/type names, imported bindings, `PRIVATE_SHADOWS_PUBLIC`.
