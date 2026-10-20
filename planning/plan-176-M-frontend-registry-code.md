# plan-176-M: Frontend and registry code

Last updated: 2026-10-18
Effort: large (3h–1d)
Depends on: plan-176-L

M splits the over-limit CODE files of the compiler front end — the lexer, the AST parser and
serializer, HIR, the resolver, the monomorphizer, the diagnostic rule table — and the builtin
registry (`src/codegen/registry/mod.rs`). Each `foo.rs` becomes `foo/mod.rs` plus part files;
the files that are already `mod.rs` gain sibling parts. Seven of the eight files are pure code
motion: items move between files, an `impl` is cut into one block per file, and private items
that now cross a file boundary get `pub(super)` (or `pub(in crate::<parent>)` where a method
moves one level deeper, §3). The eighth, `src/rules/table.rs`, is one `const RULES: &[Rule]`
data literal; it becomes three per-category `&[Rule]` slices under `src/rules/table/`, which one
accessor, `table::rules()`, walks in the original order (§4.8). The test modules of these files
were moved by letters G, H and K and stay declared in the new `mod.rs` files.

The single outcome: after M lands, `cargo test --test file_size_budget` shows no row for any of
M's files and `tests/guards/file_size_ceilings.txt` has no `ceiling` row for them.

References:

- `planning/plan-176-A-file-size-guard.md` — the rule, §Decisions, "Splitting an over-limit
  file" (`.ai/build-tooling.md`).
- `planning/plan-176-G-ir-monomorph-tests.md`, `planning/plan-176-H-codegen-builtins-engine-tests.md`,
  `planning/plan-176-K-frontend-tooling-tests.md` — where the test modules went (K also moves
  `src/rules/mod.rs`'s inline tests, six of which read the rule table, §4.8).
- `.ai/compiler.md`, `.ai/codegen-invariants.md` (names `src/ast/expr.rs`, `src/lexer.rs`,
  `src/monomorph/lower.rs`, `registry/mod.rs`), `.ai/resources-packages.md` (names
  `registry::build()` and `rules/table.rs:962`), `.ai/man-content.md` (names the registry
  vocabulary test), `.ai/testing-gates.md` (artifact gate), `.ai/build-tooling.md`,
  `.ai/specifications.md` (spec prose changes with the contract, §4.8).
- Spec topics whose citations move: `language/02_lexical-structure.md`,
  `language/03`, `04_types.md`, `05`, `06`, `08_error-model.md`, `11_operators.md`, `13`,
  `14_memory-semantics.md`, `15_resource-management.md`, `16_threads.md`, `17`,
  `18_builtin-functions.md`; `architecture/05`, `09_modules.md`, `12_monomorphization.md`,
  `19_internal-naming.md`, `21_type-name-encoding.md`, `22_type-inference.md`;
  `diagnostics/01_rule-codes.md`, `diagnostics/spec.md`; `memory/03_heap-values.md`;
  `stdlib/12_bits.md`, `15`, `17`, `18`; `tooling/02_source-selection.md`, `tooling/05_fmt.md`;
  and the topics that cite `src/codegen/registry/mod.rs`
  (`grep -rlE "\[\[src/codegen/registry/mod.rs" src/docs/spec`).
- Guard: `tests/guards/no_type_strings.rs` (`is_excluded_from_scan` and `BOUNDARY_FILES` stay in the root
  file; plan-176-E moves `boundary_list_is_closed` to `tests/guards/no_type_strings/self_checks.rs`).
- Open bugs this plan closes items of: `bugs/completed/bug-840-language-spec-stale-citations-and-keyword-example.md`
  (the five `table.rs` line cites), `bugs/bug-883-rule-codes-spec-drift-and-unemitted-rules.md`
  (the "All 252 `RULES` entries" count, on a line §4.8 rewrites anyway).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-L is complete and archived | `ls planning/plan-176-L-* planning/completed/plan-176-L-*` → only `planning/completed/…` | MET (2026-10-19) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | MET (2026-10-19): `ok. 35 passed` |
| M's files have no uncommitted changes | `git status --porcelain -- src/ast src/hir src/monomorph src/resolver src/rules src/lexer.rs src/codegen/registry tests/guards tools/loc-census src/docs/spec .ai/resources-packages.md src/manifest/config.rs` → empty | MET (2026-10-19) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify — a
> prerequisite recorded NOT MET may have landed since, and one recorded MET may
> have regressed.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you. The reader needs to know how far off the gate is,
> not just which row you hit first.

## 1. Goal

- Every M file and part is within the three limits of plan-176-A: ≤ 1000 code LOC, ≤ 250
  inline test LOC, and (for a test-only file) ≤ 1000 LOC
  (`python3 -I tools/loc-census/census.py --file <path>`). M creates no test-only file.
- The compiler's output is byte-identical: `sh scripts/artifact-gate.sh target/release/mfb all`
  reports no diff, and the diagnostics pinned by the `build.log` goldens are unchanged.
- The rule table is the same 270 rules in the same order (§4.8 order check).
- Every spec citation of a moved symbol names the file that defines it, and the stale/dangling
  citations listed in §2 name a real symbol.

### Non-goals (explicit constraints)

- No logic edit; no diagnostic text, rule code, AST/IR JSON shape or emitted byte changes. The
  one piece of new code is the rule-table accessor `table::rules()` and its `PARTS` slice
  (§4.8), which the rule's "no exemptions" decision forces.
- No test added, removed or renamed. The six rule-table tests change only the expression that
  yields the rules (§4.8); every assertion stays.
- Every `crate::…` path callers use is unchanged (`crate::lexer::lookup_keyword`,
  `crate::hir::HirFile`, `crate::codegen::registry::registry`, …) via glob re-exports.
- No visibility widened beyond what a move forces: `pub(super)` for a part item used by a
  sibling/`mod.rs`/tests, `pub(in crate::<parent>)` for the 12 methods listed in §3.

## 2. Current State

Counts: `python3 -I /tmp/loc/census.py "$PWD"` at `ce7a995ca` (fields `code`, `test` in
`/tmp/loc/census.json`); after plan-176-A, `python3 -I tools/loc-census/census.py --file <path>`.
Part LOC: `python3 -I /tmp/loc/rw-M/rsum.py <file> <part>=<ranges>…` at `ce7a995ca` (sums the
census counter's code lines over the listed line ranges; parts sum to the file total). Cites:
`grep -rnoE "\[\[<path>(:[^]]+)?\]\]" src/docs/spec | wc -l` at `ce7a995ca`.

| File | Code | Test (owner) | Parts after M | Cites |
|---|---|---|---|---|
| `src/ast/expr.rs` | 1083 | 155 (stays inline) | mod + 1 | 13 |
| `src/ast/serialize.rs` | 1570 | 51 (stays inline) | mod + 3 | 0 |
| `src/codegen/registry/mod.rs` | 3108 | 2695 (H) | mod + 8 | 35 |
| `src/hir/mod.rs` | 1002 | 192 (stays inline) | mod + 1 | 2 |
| `src/lexer.rs` | 1199 | 555 (K) | mod + 2 | 15 |
| `src/monomorph/lower.rs` | 2521 | 1292 (G) | mod + 4 | 33 |
| `src/resolver/resolution.rs` | 1496 | 737 (K) | mod + 3 | 27 |
| `src/rules/table.rs` | 1623 | 0 | mod + 3 | 30 |

After H, `src/codegen/registry/mod.rs` keeps `mod qualification_tests` (79 test LOC) and the
cfg(test) helper fns inside the unify range (`self_update_shaped`,
`is_string_self_update_type`, `strip_res`, `resolve_bound`, `occurs`, `types_coincide`,
`resolve_call`; 117 test LOC), plus `#[cfg(test)] mod tests;` and
`#[cfg(test)] #[path = "tests/raw_result_block_ownership.rs"] mod raw_result_block_ownership;`
(plan-176-H §4: `tests/mod.rs` + `selection_and_thread_probes.rs` + `rendering_and_gates.rs`,
and the single file `tests/raw_result_block_ownership.rs`). After G/K, `lower.rs`, `lexer.rs`, `resolution.rs` hold `#[cfg(test)] mod tests;`.
After K, `src/rules/mod.rs` holds `#[cfg(test)] mod tests;` (K's `src/rules/tests/`).

### Verified properties

- **Moving Rust code cannot change compiler output through source locations.**
  `grep -rnE 'module_path!|\bfile!\(|\bline!\(' src repository/src` → no hits; no link-order
  registration crates (plan-176-A). So byte-identity is the right gate, and a diff is a bug.
- **Glob re-exports keep paths; glob cycles compile clean** (probe `/tmp/loc/probe2` printed
  `6`, no warning). Inherent methods need no re-export: method lookup does not depend on the
  module path.
- **Visibility depth trap.** `src/ast/expr.rs`, `src/monomorph/lower.rs` and
  `src/resolver/resolution.rs` are child modules. Their `pub(super)` methods are visible to the
  parent (`ast`, `monomorph`, `resolver`). Moved one level deeper into `<child>/<part>.rs`,
  `pub(super)` would mean `<child>` and the parent's calls would fail to compile. Callers in the
  parent: `src/resolver/mod.rs:84,143` (`resolve_doc_blocks`), `:1328` (`check_new_local`);
  `src/monomorph/mod.rs` (`template_spellings`); `src/ast/*` (the 9 type-name methods).
- **`src/hir/mod.rs` is a `no_type_strings` boundary file**
  (`BOUNDARY_FILES` and `boundary_list_is_closed` name it). Every `ParameterType::parse` call
  stays in `hir/mod.rs`: `awk 'NR>=37&&NR<=607' src/hir/mod.rs | grep -n "ParameterType::parse"`
  matches only doc comments, and 37-607 is the range that moves to `nodes.rs`.
- **`no_type_strings` excludes the lexer by exact path.** `is_excluded_from_scan`:
  `rel.starts_with("src/ast/") || rel == "src/lexer.rs" || rel.starts_with("src/docs/")`
  (introduced by `9174857b8`, 2026-08-29, plan-111-A Phase 1:
  `git log -S 'rel == "src/lexer.rs"'`). After the move the lexer code is under `src/lexer/`,
  which `rel == "src/lexer.rs"` does not match, so the scan would newly flag the lexer's
  spellings. `src/ast/` subdirectories stay covered by its prefix rule.
- **The rule table is one literal with one reader.** `src/rules/table.rs` is
  `use super::{Rule, Severity};`, a blank line, and one item,
  `pub(super) const RULES: &[Rule] = &[ … ];` (lines 3-1721; 270 `Rule` literals:
  `grep -c '^    Rule {' src/rules/table.rs` → `270`). `Rule` (`src/rules/mod.rs`) derives
  nothing, so it is not `Copy`. Its only importer is `src/rules/mod.rs`
  (`mod table; use table::RULES;`; `grep -rn 'RULES' src | grep '\.rs:' | grep -v '^src/rules/'`
  → no hits). Seven uses: production `rule_for` (`RULES.iter().find(..)`, the only path from
  `show_diagnostic`, `show_general_diagnostic`, `is_error`, `code_and_name` to the table) and
  six tests — `dead_import_missing_package_name_is_gone` (`.iter().any`),
  `rule_names_are_unique` (`.iter().map`), `rule_codes_are_unique` (`for rule in RULES`),
  `every_rule_is_documented_in_the_spec` (`.iter().filter`),
  `code_and_name_resolves_representative_rules` and
  `is_error_partitions_the_whole_table_by_severity` (`for rule in RULES`). Lookup is by name and
  names are unique (`rule_names_are_unique`), so no output depends on table order; the split
  keeps the order anyway. Nothing renders the table: `mfb man` has no path to it
  (`grep -rn 'RULES\|table\.rs' src/docs/man` → no hits) and the spec's rule listing
  (`diagnostics/01_rule-codes.md`) is hand-written prose that
  `every_rule_is_documented_in_the_spec` checks against the table.
- **The code-range runs are clean seams.** `python3 -I /tmp/loc/rw-M/runs.py` lists the table's
  runs by code prefix: `2-200` 4-68, `1-100`/`1-101`/`1-102` 69-204, `2-201` 205-326, `2-203`
  327-1262 (143 rules, 858 code LOC), then `2-200-0011…0101`, `2-205`, `3-302`, `3-304`,
  `5-500`, `6-603`, `6-605`, `1-103`, `2-205`, `1-102-0010…0012`, `2-208` in 1263-1720. Lines
  327 and 1263 are `    Rule {` with no comment above them (`sed -n '326,327p;1262,1263p'`), so
  every reserved-row/dropped-rule comment stays with its rule.
- **A slice of slices plus an iterator accessor compiles clean and keeps order.** Probe
  `/tmp/loc/rw-M/probe/m2.rs` (edition 2021, the crate's): `table/mod.rs`-shaped module with
  `use super::super::{Rule, Severity};`, two parts each `use super::*;` +
  `pub(super) const RULES: &[Rule]`, `const PARTS: &[&[Rule]] = &[a::RULES, b::RULES];` and
  `pub(super) fn rules() -> impl Iterator<Item = &'static Rule> { PARTS.iter().flat_map(|part| part.iter()) }`.
  `rustc --edition 2021 -D warnings` → no warning (the parent's `Severity` import, used only
  by the parts through the glob, is not flagged); the binary printed `["1", "3"]` (part order);
  `rustc --test` → `1 passed`, with the test module reaching `table::rules()` through
  `use super::*;`.
- **Spec citations are checked by file only.** `spec_citations_resolve` (`src/docs/spec/mod.rs`)
  fails only when a cited *file* is missing ("Deliberately file-level only"), so a symbol that
  leaves a still-existing file is caught only by the §4 sweep check.
- **Stale or dangling citations** (pre-existing, fixed in M):
  - `[[src/lexer.rs:262]]` (`architecture/19_internal-naming.md:17`) and
    `[[src/lexer.rs:752]]` (`:50`): line 262 is `self.advance();` in `lex_all`, 752 is
    `value.push(self.peek());` in `lex_number`; the claim (the `internalize` rewrite) is at
    `src/lexer.rs:982` inside `lex_identifier_or_keyword` (`grep -n internalize src/lexer.rs`).
  - `[[src/rules/table.rs:312]]`, `[[…:318]]` (`language/08_error-model.md:147`, backing
    `TYPE_PROPAGATE_REQUIRES_TRAP` / `TYPE_FAIL_REQUIRES_ERROR`); `[[…:480]]`, `[[…:486]]`
    (`language/04_types.md:430`, backing "may not construct … update … with WITH"); `[[…:642]]`
    (`language/04_types.md:223`, backing `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION`). Lines
    312/318/480/486 are `severity: Severity::Error,` of other rules and 642 is `    Rule {`
    (`for l in 312 318 480 486 642; do sed -n ${l}p src/rules/table.rs; done`). The rules are
    at 383 (`TYPE_FAIL_REQUIRES_ERROR`), 389 (`TYPE_PROPAGATE_REQUIRES_TRAP`), 551
    (`TYPE_READ_ONLY_RECORD_UPDATE`), 557 (`TYPE_READ_ONLY_RECORD_CONSTRUCTOR`), 734
    (`TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION`) (`grep -n 'name: "…"' src/rules/table.rs`),
    all in the `2-203` run. This is bug-840 item 1.
  - `.ai/resources-packages.md:11` says the spec self-contradicts through §15.1 and rule
    `TYPE_RESOURCE_BORROW_INVALIDATE` (2-203-0086, `rules/table.rs:962`). `a6f4bf282` ("resources:
    remove "borrow"") renamed the rule to `TYPE_RESOURCE_INVALIDATE_NOT_OWNER` and removed the
    §15.1 vocabulary (`grep -n -i borrow src/docs/spec/language/15_resource-management.md` → no
    hits; `grep -n -A1 '"2-203-0086"' src/rules/table.rs` → line 1141, now a reserved row).
  - `diagnostics/01_rule-codes.md:222` says "All 252 `RULES` entries are listed"; the table
    holds 270 (`grep -c '^    Rule {' src/rules/table.rs`) and the page's table has 271 code
    rows including the synthetic `0-000-0000`
    (`grep -cE '^\| \`[0-9]-[0-9]{3}-[0-9]{4}\` \|' src/docs/spec/diagnostics/01_rule-codes.md`).
    This is bug-883 §A item 1.
  - `[[src/codegen/registry/mod.rs:call_return_type]]` (`stdlib/12_bits.md:21`): the registry
    defines `call_return_type_typed` (`src/codegen/registry/mod.rs:2595`);
    `call_return_type` is `src/codegen/builtins/mod.rs`, which delegates to it, and the same
    line already cites `[[src/codegen/builtins/mod.rs:resolve_call_return_type]]`.

## 3. Design Overview

Convention (plan-176-A): `mod.rs` keeps the module header, the core types/entry points and the
test declarations; parts by responsibility, each starting `use super::*;`; `mod.rs` declares
`mod <part>;` and, for free items, `pub(crate) use <part>::*;` (`pub use` for the `pub` lexer
API). Line ranges are source lines as of `ce7a995ca`; item NAMES are authoritative (the test
letters removed only trailing modules, below every listed range).

**Widenings to `pub(in crate::<parent>)` (12, forced by the depth trap, §2):**
`src/ast/expr/type_names.rs` — `normalize_qualified_builtin_type`,
`normalize_qualified_type_name`, `qualify_own_builtin_type`, `parse_qualified_name`,
`finish_qualified_name`, `parse_type_name`, `parse_thread_type_name`,
`parse_function_type_name`, `parse_type_base_name` → `pub(in crate::ast)`;
`src/monomorph/lower/types.rs` — `template_spellings` → `pub(in crate::monomorph)`;
`src/resolver/resolution/docs.rs` — `resolve_doc_blocks`, and
`src/resolver/resolution/body.rs` — `check_new_local` → `pub(in crate::resolver)`. These keep
exactly today's reach (the parent module); methods staying in `<child>/mod.rs` keep
`pub(super)`.

**Visibility rule:** the `pub(super)` items in §4 are the measured set; any further private item
or field the compiler reports as inaccessible across a part boundary gets `pub(super)`, nothing
wider.

Gate class: provably-neutral code motion in the compiler → byte-identity. Per phase:
`cargo build --release`, then `sh scripts/artifact-gate.sh target/release/mfb <builtin>` for the
builtins named in the phase (each regenerates that fixture set's `.ast`, `.ir` and every
target's native dumps and diffs them against the goldens; est. 1 min each), plus the moved
modules' unit tests. Diagnostics are pinned by `build.log` goldens the artifact gate does not
diff; phases that move diagnostic producers (the rule table, the resolver) run
`scripts/test-accept.sh` on the invalid fixtures. Once at the end:
`sh scripts/artifact-gate.sh target/release/mfb all`. A diff is root-caused (objdump / `diff`
ONE fixture's artifact), fixed, and the gate rerun; it never stops the plan.

Risk concentrates in the registry (eight parts, every builtin call resolves through it, and
the H-left cfg(test) helpers ride along into `unify.rs`) and the monomorphizer (one 2442-LOC
`impl` cut into five blocks). Both are the last two phases. The rule table is the one place M
writes new code; the order check (§4.8) proves the parts reproduce the literal line for line.

Rejected alternatives:
- *A `const fn` that concatenates the parts into one `[Rule; N]`* — needs `Rule: Copy` (a derive
  on a public type for a size rule) and a placeholder element to initialise the array; more new
  code than the accessor for no reader-visible gain (lookup is by name).
- *A `macro_rules!` per part spliced into one literal* (the `concat!` pattern of
  `/tmp/loc/probe_concat` works for `&str` only) — a macro cannot expand to a comma-separated
  run of array elements, so it needs a tt-munching chain; the rules would sit inside macro
  bodies rustfmt does not format and readers do not grep as data.
- *A `static RULES: OnceLock<Vec<Rule>>` built at first use* — a runtime allocation and an
  initialisation path where today there is a `const`.
- *Splitting `2-203` further (e.g. at the resource/thread rules) for more headroom* — 861 LOC
  is under the 900 part target; three parts by code range is the fewest that fit.
- *Leaving the twelve methods `pub(super)` and moving their callers instead* — moves code out
  of the parent modules, a second split nobody needs.
- *`pub(crate)` for the widened methods* — reaches further than today; `pub(in crate::<parent>)`
  is exactly today's reach.
- *Keeping `src/lexer.rs` and only adding parts under `src/lexer/`* — legal Rust, but defeats the
  citation forcing function (`[[src/lexer.rs:…]]` would keep passing for moved symbols).

## 4. Detailed Design

**Citation sweep check** (per directory, after its sweep):
`grep -rhoE '\[\[<new dir>/[^]]+\]\]' src/docs/spec | sort -u | sed 's/^\[\[//;s/\]\]$//' | while IFS=: read -r p s; do [ -n "$s" ] && [ -f "$p" ] && ! grep -qw -- "${s##*::}" "$p" && echo "MISSING $p:$s"; done`
→ no output; and `grep -rn '\[\[<old path>:' src/docs/spec` → 0 (for files that keep their
path, the list below is the sweep: `spec_citations_resolve` cannot see a symbol leave a file
that still exists).

### 4.1 `src/ast/expr.rs` → `src/ast/expr/` (1083)

All code is `impl<'a> FileParser<'a>` (28-1395), cut into two blocks.
- `mod.rs` (732): 1-50 (`use super::pipeline::…`, `use super::*`, `use crate::operators::…`,
  `MAX_EXPR_DEPTH`, `MAX_TYPE_DEPTH`, `MAX_PIPELINE_COPIED_NODES`); an impl block with 194-877
  (`enter_expr`, `leave_expr`, `note_expr_tree_depth`, `latch_hostile_expression`,
  `parse_expression`, `parse_pipeline`, `parse_or`, `parse_and`, `parse_not`,
  `parse_comparison`, `parse_concat`, `parse_addition`, `parse_multiplication`,
  `parse_power`, `parse_unary`, `parse_with_update`, `parse_member_access`,
  `parse_call_or_constructor`, `parse_argument_list`, `parse_constructor_argument_list`,
  `parse_primary`), 1228-1273 (`parse_lambda`), 1294-1395 (`parse_list_literal`,
  `parse_map_literal`, `parse_set_literal`); `mod type_names;`; the inline
  `#[cfg(test)] mod tests` (155 test LOC) stays inline.
- `type_names.rs` (352): an impl block with 51-193 (`normalize_qualified_builtin_type`,
  `normalize_qualified_type_name`, `package_exports_type`, `qualify_own_builtin_type`),
  878-1227 (`parse_qualified_name`, `finish_qualified_name`, `enter_type`, `leave_type`,
  `parse_type_name`, `parse_type_name_inner`, `parse_template_argument_list`,
  `parse_thread_type_name`, `parse_resource_plane_type`, `parse_optional_element_state`,
  `parse_function_type_name`, `parse_function_type_params`), 1274-1293
  (`parse_type_base_name`). (Named so it does not read as the sibling `src/ast/types.rs`.)

Nothing re-exported (inherent methods). 9 widenings (§3); new `pub(super)`: 0 (`enter_type`,
`leave_type`, `parse_type_name_inner`, `parse_template_argument_list`,
`parse_function_type_params`, … are used only inside `type_names.rs`:
`grep -rn 'parse_template_argument_list\|parse_function_type_params' src | grep '\.rs:'` → only
`src/ast/expr.rs` lines inside 878-1227; `MAX_TYPE_DEPTH` stays in `mod.rs`).
Citations (13 cites, 11 symbols): → `src/ast/expr/mod.rs` — `parse_argument_list`,
`parse_list_literal` (`language/02_lexical-structure.md`), `parse_member_access`,
`parse_power`, `parse_unary` (`language/11_operators.md`); → `src/ast/expr/type_names.rs` —
`finish_qualified_name`, `parse_optional_element_state`, `parse_thread_type_name`,
`parse_type_base_name`, `parse_type_name` (3), `parse_type_name_inner`.
Guards: `no_type_strings` `rel.starts_with("src/ast/")` covers the subdirectory;
`no_operator_strings`' `is_test_file` does not match `type_names.rs`.
Prose/comments: `.ai/codegen-invariants.md:85` "`src/ast/expr.rs`'s precedence ladder" →
`src/ast/expr/mod.rs`; `.ai/codegen-invariants.md:836` `qualify_own_builtin_type`,
`src/ast/expr.rs` → `src/ast/expr/type_names.rs`; `src/operators.rs:4,33` → `src/ast/expr/mod.rs`;
`src/types.rs:594` (grouped type names) → `ast/expr/type_names.rs`; the K-moved comment
"Expression-parser coverage (src/ast/expr.rs)" (`grep -rn 'src/ast/expr.rs' src/ast/tests`) →
`src/ast/expr/`.

### 4.2 `src/ast/serialize.rs` → `src/ast/serialize/` (1570)

- `mod.rs` (299): 1-264 (`use` lines, `impl AstProject::to_json`, `impl AstFile::to_json`,
  `ToJson` for `AstFile`, `Import`, `Item`, `TestingBlock`, `TestGroup`, `TestGroupMember`,
  `TestCase`, `DocBlock`), 1518-1537 (`visibility_name`, `visibility_prefix`), 1610-1646
  (`resource_json_suffix`, `exit_target_name`, `template_params_json`); `mod link; mod decls;
  mod body;`; the inline test module (51 test LOC) stays inline.
- `link.rs` (342; 265-625): `ToJson` for `ResourceDecl`, `FuncAlias`, `LinkBlock`,
  `CStructDecl`, `CStructField`, `BindIn`, `BindInField`, `LinkFunction`, `AbiSlot`,
  `ConstPin`; inherent `BindState::to_json`, `FreeSpec::to_json`, `AbiSpec::to_json`.
- `decls.rs` (337): 626-918 (`ToJson` for `TopLevelBinding`, `TypeDecl`, `TypeField`,
  `UnionVariant`, `EnumMember`, `Function`, `Trap`, `Param`; `param_state_json_suffix`),
  1538-1609 (`impl Function/TypeDecl/ResourceDecl { signature_line }`, which keep `pub`).
- `body.rs` (592; 919-1517): `ToJson` for `Statement`, `MatchCase`, `MatchPattern`,
  `Expression`; inherent `CallArg::to_json`, `ConstructorArg::to_json`,
  `RecordUpdate::to_json`.

No re-exports (trait impls and inherent methods); new `pub(super)`: 0 (the xref's 28 hits are
`to_json` methods called only in their own file; the helper fns in `mod.rs` are visible to the
children). Citations: 0. Comments: `src/operators.rs:24` and `src/operators/tests/mod.rs:210`
(the `binary` expression JSON) → `src/ast/serialize/body.rs`; the K-moved comment
"(src/ast/serialize.rs)" (`grep -rn 'src/ast/serialize.rs' src/ast/tests`) →
`src/ast/serialize/`.

### 4.3 `src/hir/mod.rs` + `src/hir/nodes.rs` (1002; siblings `build.rs`, `resource_params.rs`)

- `mod.rs` (622): 1-36 (module doc, `pub(crate) mod build; pub(crate) mod resource_params;`,
  `use` lines), `mod nodes; pub(crate) use nodes::*;`, 608-1282 (the AST→HIR elaboration:
  `parse_type`, `parse_optional_type`, `parse_optional_state`, `elaborate`,
  `elaborate_file`, `elaborate_file_structure`, `elaborate_item`, `elaborate_link_block`, …,
  `elaborate_expression`); the inline `mod tests` (192 test LOC) stays inline.
- `nodes.rs` (380; 37-607, the HIR node definitions): `HirProject`, `HirFile` +
  `import_bindings`, `HirItem`, `HirLinkBlock`, `HirLinkFunction`, `HirAbiSpec`,
  `HirAbiSlot`, `HirFreeSpec`, `HirCStructField`, `HirLinkParam`, `HirCStructDecl`,
  `HirTopLevelBinding`, `HirTypeDecl`, `HirUnionVariant`, `HirTypeField`, `HirFunction`,
  `InstantiationSite` + `describe`, `HirTrap`, `HirParam`, `HirCallArg`,
  `HirConstructorArg`, `HirRecordUpdate`, `HirStatement`, `HirMatchCase`,
  `HirMatchPattern`, `HirExpression`.

New `pub(super)`: 0 (all node types and fields are `pub(crate)`). The boundary guard entry
(`src/hir/mod.rs`) and its closed list are unchanged (§2). Citations (2,
`architecture/12_monomorphization.md`): `HirFile`, `InstantiationSite` →
`src/hir/nodes.rs`. `[[src/hir/]]` (`architecture/09_modules.md`) stays valid.

### 4.4 `src/lexer.rs` → `src/lexer/` (1199; K created `src/lexer/tests/`)

- `mod.rs` (540): 1-347 (`TokenKind`, `DocRaw`, `DocRawLine`, `Keyword`, `Token`,
  `lex_with`, `Lexer`, `OpenBracket`, `begins_statement_only`, an `impl Lexer` block with
  `lex_all`) and 932-1254 in the same block (`lex_identifier_or_keyword`,
  `try_capture_doc_block`, `lex_newline`, `next_line_statement_keyword`, `open_bracket`,
  `close_bracket`, `report_unclosed_brackets`, `lex_line_continuation`, `skip_line_comment`,
  `is_statement_start`, `is_line_start`, `push_simple`, `advance_line`, `report`, …);
  `mod literals; mod keywords; pub use keywords::*;`; `#[cfg(test)] mod tests;` (K).
- `literals.rs` (479): an `impl Lexer` block with 348-931 (`lex_string`,
  `recover_string_literal`, `lex_scalar`, `recover_scalar_literal`, `lex_unicode_escape`,
  `lex_number`, `exponent_follows`, `is_base_digit`, `underscore_is_line_continuation`,
  `scan_base_digits`, `lex_radix_number`).
- `keywords.rs` (180; 1255-1453): `lookup_keyword`, `is_end_line`, `is_doc_attr_rest`,
  `keyword`, `keyword_lexeme`.

`pub use keywords::*;` keeps `crate::lexer::{lookup_keyword, keyword_lexeme, is_end_line,
is_doc_attr_rest}` (`pub` today). pub(super) (4): `keyword` (keywords.rs, called by
`lex_identifier_or_keyword`); `lex_string`, `lex_scalar`, `lex_number` (literals.rs, called by
`lex_all`).

**Guard edit (same commit):** `tests/guards/no_type_strings.rs` `is_excluded_from_scan`:
`rel == "src/lexer.rs"` → `rel.starts_with("src/lexer/")`; its doc comment bullet
"`src/lexer.rs` — likewise" → "`src/lexer/` — likewise". (1) `9174857b8` (2026-08-29, plan-111-A
Phase 1). (2) Protects: no type spelling decides anything after the string domain; the lexer
(token producer) and the AST are that domain. (3) Dependents: none
(`grep -rn 'src/lexer' tests/guards` → this guard only). (4) Proof: the clause's intent is
"the lexer is the string domain"; the lexer is the same code at `src/lexer/`, and the prefix
covers only `src/lexer/*` (the three code parts and K's already-exempt `tests/`), so no file
outside the lexer becomes exempt. `.ai/codegen-invariants.md:54` "minus `src/ast`,
`src/lexer.rs`, `src/docs`" → `src/lexer/`.

Citations (15 cites, 12 symbols + 1 file cite): → `src/lexer/mod.rs` — `OpenBracket`,
`TokenKind`, `begins_statement_only`, `close_bracket`, `lex_newline`, `lex_with`,
`report_unclosed_brackets`; → `src/lexer/literals.rs` — `lex_number`, `lex_scalar`; →
`src/lexer/keywords.rs` — `lookup_keyword`; `[[src/lexer.rs]]`
(`architecture/09_modules.md:8`) → `[[src/lexer/]]` (directory cites are used in that table);
the two stale line cites `[[src/lexer.rs:262]]`, `[[src/lexer.rs:752]]`
(`architecture/19_internal-naming.md:17,50`) → `[[src/lexer/mod.rs:lex_identifier_or_keyword]]`
(§2).

### 4.5 `src/resolver/resolution.rs` → `src/resolver/resolution/` (1496)

`impl Resolver` (3-1742, 1489 code LOC) cut into four blocks.
- `mod.rs` (378): 1-13 (`use super::*`, `resolve`), 479-897 (`resolve_file`,
  `resolve_resource_decl`, `resolve_func_alias`, `resolve_link_block`, `resolve_binding`,
  `resolve_type_decl`, `resolve_member_field`, `resolve_function`); `mod docs; mod body;
  mod types; use types::peel_res;` (free fn); `#[cfg(test)] mod tests;` (K).
- `docs.rs` (422; 14-478): `resolve_doc_blocks`, `validate_doc_block`, `note_doc_target`,
  `check_doc_named`.
- `body.rs` (526; 898-1484): `resolve_block`, `resolve_statement`, `resolve_nested_block`,
  `resolve_match_pattern`, `resolve_expression`, `resolve_callable`,
  `resolve_parameter_default`, `report_default_names_parameter`, `check_new_local`,
  `resolve_identifier`.
- `types.rs` (170; 1485-1749): `resolve_type`, `resolve_leaf`, `report_unknown_type`,
  `resolve_package_qualified_name`, free fn `peel_res`.

2 widenings (§3); `resolve` stays in `mod.rs`. pub(super) (6): `resolve_block`,
`resolve_expression`, `resolve_parameter_default` (body); `resolve_type`,
`resolve_package_qualified_name`, `peel_res` (types).
Citations (27 cites, 12 symbols; `language/04, 05, 06, 08, 13, 17, 18`, `architecture/05, 21`,
`stdlib/15, 17, 18`): → `mod.rs` — `resolve_func_alias`, `resolve_function`,
`resolve_resource_decl`, `resolve_type_decl`; → `body.rs` — `check_new_local`,
`resolve_callable`, `resolve_expression`, `resolve_nested_block`,
`resolve_parameter_default`, `resolve_statement`; → `types.rs` —
`resolve_package_qualified_name`, `resolve_type` (10).
`tests/guards/no_type_strings.rs:422` (doc comment: `resolver/resolution.rs`'s `types` /
`resources` declaration indexes) → `resolver/resolution/docs.rs` (comment only; no allowlist
entry).

### 4.6 `src/monomorph/lower.rs` → `src/monomorph/lower/` (2521; G created `lower/tests/`)

`impl Monomorphizer` (92-3153, 2442 code LOC) cut into five blocks.
- `mod.rs` (452): 1-67 (`use super::*`, `use crate::operators…`, `strip_qualifier_prefixes`,
  `impl TypeKinds for Monomorphizer` with `is_enum`, `union_accepts`); an impl block with `new`
  (93-325), `run`, `into_project` (580-804), `report`, `charge_instantiation`,
  `report_instantiation_too_deep` (3085-3154); `mod resolve; mod instantiate; mod body;
  mod types; use types::expected_element_type;`; `#[cfg(test)] mod tests;` (G).
- `resolve.rs` (461): 326-579 (`collections_internal_callee`, `imported_overload_set`,
  `resolve_imported_overload`, `types_compatible`, `normalize_type`), 1334-1687
  (`resolve_overload`, `template_name`, `sole_file_sibling`, `resolve_function_reference`,
  `resolve_builtin_override_reference`, `resolve_general_builtin_override`,
  `single_signature_params`).
- `instantiate.rs` (490): 68-91 (free items `TemplateBindFailure`, `template_identity`, used only
  by `instantiate_function` and its helpers); 805-1333 (`lower_type`, `lower_binding`,
  `lower_function`, `lower_function_inner`, `arg_types_in_param_order`,
  `unique_concrete_symbol`, `instantiate_function`, `bind_template`,
  `report_binding_failure`, `most_specific`, `user_instantiation_site`), 1688-1748
  (`instantiate_type`, `lower_field`).
- `body.rs` (742; 1749-2592): `lower_statements`, `lower_statement` (324 LOC),
  `lower_expression` (378 LOC), `lower_constructor_arg`.
- `types.rs` (376): 2593-3084 (`leaf_symbol`, `concrete_type`, `template_view`,
  `template_spellings`, `union_pattern_type`, `collect_union_instantiations`,
  `record_fields`, `builtin_call_return_type`, `expression_type`), 3155-3165 (free fn
  `expected_element_type`).

No re-exports of methods; `monomorph/mod.rs` uses only `Monomorphizer::{new, run,
into_project, template_spellings}`. 1 widening (`template_spellings`, §3). pub(super) (23):
resolve — `collections_internal_callee`, `resolve_imported_overload`, `types_compatible`,
`normalize_type`, `resolve_overload`, `template_name`, `resolve_function_reference`,
`resolve_general_builtin_override`, `single_signature_params`; instantiate — `lower_type`,
`lower_binding`, `lower_function`, `arg_types_in_param_order`, `instantiate_function`,
`instantiate_type`; body — `lower_statements`, `lower_expression`; types — `concrete_type`,
`template_view`, `union_pattern_type`, `record_fields`, `expression_type`,
`expected_element_type`. `imported_overload_set`, `bind_template`, `report_binding_failure`,
`most_specific`, `TemplateBindFailure`, `template_identity` stay private (callers in the same
part: `grep -n 'imported_overload_set\|bind_template\|report_binding_failure\|most_specific\|TemplateBindFailure\|template_identity' src/monomorph/lower.rs`).
`Monomorphizer`'s fields are defined in `monomorph/mod.rs`, an ancestor, so they stay visible.
Citations (33 cites, 20 symbols; `architecture/12, 22`, `language/03, 04, 06, 18`): →
`mod.rs` — `Monomorphizer::new`, `into_project`; → `resolve.rs` —
`collections_internal_callee`, `imported_overload_set`, `normalize_type`,
`resolve_builtin_override_reference`, `resolve_function_reference`,
`resolve_general_builtin_override`, `resolve_imported_overload`, `resolve_overload`,
`single_signature_params`, `template_name`, `types_compatible`; → `instantiate.rs` —
`instantiate_function`, `instantiate_type`, `lower_type`, `user_instantiation_site`; →
`types.rs` — `collect_union_instantiations`, `expression_type`, `union_pattern_type`.
`.ai/codegen-invariants.md:808` "`lower_expression` in `src/monomorph/lower.rs`" →
`src/monomorph/lower/body.rs`. `no_type_strings` lists only `src/monomorph/mod.rs` (untouched).

### 4.7 `src/codegen/registry/mod.rs` + 8 parts (3108)

- `mod.rs` (417): 1-38 (`use` header); 105-145 (`AbiInline`, `AbiFunction`, `MfbFastPath`
  aliases); 1563-2169 (`ResolvedFunc`, `ResolvedType`, `Registry` + its whole impl:
  `enum_variant_advisory`, `resolve_type`, `augment_project`, `augment_hir_project`,
  `synthetic_files`, `is_member`, `is_builtin_type`, `qualify_value_type_references`,
  `qualified_builtin_type`, …; `registry()`, `BuiltinRecordLayout`,
  `builtin_record_layouts`); 2315-2361 (`build()`); the 8 `mod` lines and
  `pub(crate) use <part>::*;` for each; H's `mod qualification_tests` (inline, 79 test LOC),
  `#[cfg(test)] mod tests;` and the `#[path = "tests/raw_result_block_ownership.rs"]`
  declaration (the path is relative to `registry/`, unchanged).
- `descriptors.rs` (427): 39-104 (`AbiCtx`); 146-1128, the descriptor data model:
  `DefaultValue`, `ConsumingParameter`, `Parameter`, `Body` + impl, `HelperGate`,
  `RegistryHelper` + impl (`always`, `debug_split`, …), `Implementation`, `CallShape`,
  `Selection`, `RegistryFunction` + impl, `RecordProp`, `RegistryRecord` + `render`,
  `UnionVariant`, `RegistryUnion` + `render`, `EnumVariant`, `EnumAdvisory`, `RegistryEnum` +
  `render`, `SlotTransfer`, `SlotBackend`, `ResourceLiveSlot`, `RegistryResource`,
  `RegistryConstant`, `RegistryOverride`.
- `package.rs` (239; 1129-1562): `RegistryPackage` + its impl (`new`, `always_helper_bodies`,
  `get_mfb_for`, `add_record`, `add_union`, `add_enum`, `add_consuming_parameter`,
  `add_function`, `add_constant`, …).
- `qualify.rs` (103; 2170-2314): `qualify_type_leaves`, `qualify_type_leaves_for_source`,
  `qualify_type_leaves_inner`.
- `lookup.rs` (625): 2362-2649 (`RuntimeCall`, `runtime_specs`, `push_runtime_call`,
  `qualify_runtime_call`, `find_constant`, `is_package_constant`, `constant_type_name`,
  `constant_value`, `constant_components`, `errorcode_constant`, `runtime_error*`,
  `general_override_target`, `call_return_type_typed`, `alias_call_return_type`); 3531-4311
  (`resolved_return_type`, `rewrite_target*`, `rewrite_owner`, `abi_inline_lower`,
  `natively_called_helpers`, `abi_function_lower`, `is_abi_function_call`,
  `native_member_declares_error`, `mfb_fast_path`, `public_member_for_rewrite`,
  `native_bare_target`, `callback_member*`, `function_has_unary_callback`,
  `argument_types*`, `agreed_argument_type`, `resolved_parameter_type*`,
  `contains_unknown`, `expected_arguments`, `overloads_disagree_on_layout`,
  `call_param_name*`, `default_argument_padding`, `call_carries_its_fill_defaults`).
- `unify.rs` (474 code + 117 cfg(test); 2650-3530): `leaf_matches`, `union_has_member`,
  `value_type_accepts`, `registry_union_accepts`, `is_resource_type_name`, `unify`,
  `unify_with_kinds`, `VarPosition`, `Position` + impl, `converts_to_union_form`,
  `NothingBinding`, `unify_at`, `thread_slot_unifies`, `resource_base_eq`, `substitute`,
  `contains_var`, `resolve_call_typed`, `resolve_call_typed_with_kinds`, and the cfg(test)
  helpers H left in this range (`self_update_shaped`, `is_string_self_update_type`,
  `strip_res`, `resolve_bound`, `occurs`, `types_coincide`, `resolve_call`) — they travel
  with the code they sit between; `raw_result_block_ownership`'s `use super::runtime_specs;`
  still resolves through `mod.rs`'s glob.
- `late_pass.rs` (141; 4312-4519): `ProjectView` + impl (`of_hir`, `references_any`, …),
  `note_type`, `inject_late_pass`, `inject_late_pass_hir`, `late_pass_files`, `short_callee`.
- `ast_scan.rs` (356; 4520-4890): `item_types`, `params_types`, `group_types`, `stmt_types`,
  `expr_types`, `item_callees`, `group_callees`, `stmt_callees`, `expr_callees`.
- `hir_scan.rs` (326; 4891-5225): `hir_item_types`, `hir_params_types`, `hir_stmt_types`,
  `hir_expr_types`, `hir_item_callees`, `hir_stmt_callees`, `hir_expr_callees`.

Every `crate::codegen::registry::X` stays valid through the 8 globs (including
`call_carries_its_fill_defaults`, called as `crate::codegen::registry::…` from
`src/codegen/engine/value/builder_values.rs`). pub(super) (21):
`RegistryRecord::render`, `RegistryUnion::render`, `RegistryEnum::render`,
`RegistryPackage::always_helper_bodies`, `qualify_type_leaves`,
`qualify_type_leaves_for_source`, `qualify_type_leaves_inner`, `leaf_matches`, `unify`,
`unify_with_kinds`, `converts_to_union_form`, `substitute`, `resolved_return_type`,
`note_type`, `short_callee`, `item_types`, `group_types`, `item_callees`, `group_callees`,
`hir_item_types`, `hir_item_callees`; plus private descriptor fields read by `Registry`'s impl
or by `package.rs` (visibility rule; most are already `pub(crate)`).
Citations (35; 33 symbol cites over 25 symbols, the test-fn cite and the file cite in
`architecture/09_modules.md`, which stays valid): stay in `mod.rs` — `augment_project`,
`is_builtin_type`, `is_member`; → `descriptors.rs` — `EnumAdvisory`, `HelperGate`,
`debug_split`; → `unify.rs` — `VarPosition`, `leaf_matches`, `resolve_call` (cfg(test)
helper), `resource_base_eq`, `unify_at`, `unify_with_kinds`, `union_has_member`,
`value_type_accepts`; → `lookup.rs` — `abi_inline_lower`, `constant_value`,
`default_argument_padding`, `find_constant`, `general_override_target`,
`native_bare_target`, `resolved_parameter_type`; → `late_pass.rs` — `late_pass_files`,
`references_any`; → `qualify.rs` — `qualify_type_leaves_inner`. The test-fn cite
`range_and_index_parameters_use_the_documented_vocabulary` (`language/18`) is repointed by H.
Dangling `[[src/codegen/registry/mod.rs:call_return_type]]` (`stdlib/12_bits.md:21`) →
`[[src/codegen/registry/lookup.rs:call_return_type_typed]]` (§2).
Prose/comments: `.ai/codegen-invariants.md:836` `general_override_target`
(`src/codegen/registry/mod.rs`) → `src/codegen/registry/lookup.rs`;
`src/cli/man.rs:666` (`RegistryPackage::unqualified_global`) → `src/codegen/registry/package.rs`;
`src/codegen/builtins/vector/mod.rs:549` (`leaf_matches`) → `codegen/registry/unify.rs`;
the G-moved comment citing `src/codegen/registry/mod.rs:1372`
(`grep -rn 'registry/mod.rs:1372' src/ir`; `get_mfb_for`) →
`` `RegistryPackage::get_mfb_for` in `src/codegen/registry/package.rs` ``;
`src/codegen/memory/value/builder_value_semantics.rs:1711` ("`registry/mod.rs` states the two
never interchange", the comment at old 3176 inside `unify_at`) → `registry/unify.rs`
(`unify_at`); `tests/guards/no_type_strings.rs:929` (`RegistryConstant::type_name`,
`RegistryOverride::arg_type`) → `registry/descriptors.rs` (comment only).
`.ai/resources-packages.md:372` (`build()` in `registry/mod.rs`) stays valid.

### 4.8 `src/rules/table.rs` → `src/rules/table/` (1623)

Three category parts, cut at the two clean seams of §2 (lines 327 and 1263), plus a `mod.rs`
that orders them. Part LOC = sum of the old range + 3 header/footer code lines
(`python3 -I /tmp/loc/rw-M/cnt.py`: 4-326 → 312, 327-1262 → 858, 1263-1720 → 450).
- `front_end.rs` (315; old 4-326, 52 rules: `grep -c '^    Rule {'` over the range):
  `PROJECT_JSON_MISSING` (2-200-0001) … `SYMBOL_SHADOWS_TOP_LEVEL_BINDING` (2-201-0022) — the
  first `project.json` rules (2-200-0001…0010), source/lex/parse (1-100, 1-101, 1-102) and
  import/symbol (2-201).
- `type_check.rs` (861; old 327-1262, 143 rules): `TYPE_BINARY_OPERATOR_MISMATCH` (2-203-0001)
  … `TYPE_INSTANTIATION_BUDGET_EXCEEDED` (2-203-0135) — every `2-203` rule. (Not `types.rs`,
  which would read as `crate::types`.)
- `tooling.rs` (453; old 1263-1720, 75 rules): `PROJECT_ENTRY_INVALID` (2-200-0011) …
  `TESTING_EXPECT_TYPE_MISMATCH` (2-208-0008) — the later `project.json` rules, `BUILD_FAILED`,
  `FMT_CHECK_FAILED`, package/native/verify/target/link/lockfile/registry rules (2-205-0022/0023,
  3-302, 3-304, 5-500, 6-603, 6-605), doc-comment rules (1-103, 2-205-0001…0021), TESTING
  parse rules (1-102-0010…0012) and `2-208`.

Each part is the old file's shape: `use super::*;`, a blank line,
`pub(super) const RULES: &[Rule] = &[`, the old lines verbatim (every comment stays with its
rule), `];`.

`table/mod.rs` (8 code LOC):

```rust
//! The diagnostic rule table: one `&[Rule]` per category, walked in a fixed order.
use super::{Rule, Severity};

mod front_end;
mod tooling;
mod type_check;

/// The parts in table order. Lookup is by name, so the order is the listing order only.
const PARTS: &[&[Rule]] = &[front_end::RULES, type_check::RULES, tooling::RULES];

/// Every rule, in table order.
pub(super) fn rules() -> impl Iterator<Item = &'static Rule> {
    PARTS.iter().flat_map(|part| part.iter())
}
```

`src/rules/mod.rs` keeps `mod table;` and drops `use table::RULES;`. The seven uses read
`table::rules()` (the test module reaches `table` through `use super::*;`, §2 probe):
- `rule_for`: `RULES.iter().find(|rule| rule.name == rule_name)` →
  `table::rules().find(|rule| rule.name == rule_name)`.
- In K's `src/rules/tests/` (named by test, since K owns the file layout):
  `dead_import_missing_package_name_is_gone` `!RULES.iter().any(..)` → `!table::rules().any(..)`;
  `rule_names_are_unique` `RULES.iter().map(..)` → `table::rules().map(..)`;
  `rule_codes_are_unique`, `code_and_name_resolves_representative_rules`,
  `is_error_partitions_the_whole_table_by_severity` `for rule in RULES` →
  `for rule in table::rules()`; `every_rule_is_documented_in_the_spec`
  `RULES.iter().filter(..)` → `table::rules().filter(..)`.
  Test-edit justification: (1) the tests were written with the table (`9cbb67c7c` split
  `rules.rs` into `rules/`; bug-40, bug-216, plan-46, bug-883 added the drift guards). (2)
  They protect: every rule is defined once, uniquely coded, documented, and resolvable. (3)
  Dependents: none outside `src/rules/` (`grep -rn 'RULES' src | grep '\.rs:'`). (4) The
  edit is forced, not a re-baseline: `RULES` as one slice no longer exists; each test keeps
  every assertion over the same 270 rules in the same order (the order check below).
- Comments/messages naming the single slice: the `code_and_name` doc ("not defined in
  `RULES`") → "not in the rule table"; the `rule_for` comment ("absent from `RULES`") → "absent
  from the rule table"; the `debug_assert!` text
  `"diagnostic rule `{rule_name}` is not defined in RULES (src/rules/table.rs)"` →
  `"… is not defined in RULES (src/rules/table/)"` — the `RULES` word stays, so
  `unknown_rule_name_trips_the_debug_assert`'s `#[should_panic(expected = "not defined in RULES")]`
  is unchanged, and each part's const is still named `RULES`; the
  `every_rule_is_documented_in_the_spec` doc ("Every rule in `RULES`") → "Every rule in the
  table". `src/manifest/config.rs:165` "The `src/rules/table.rs` rule name." → "The rule name
  (`src/rules/table/`)."

**Order check (before the commit):**
`diff <(git show HEAD:src/rules/table.rs | sed -n '4,1720p') <(for p in front_end type_check tooling; do sed '1,3d;$d' src/rules/table/$p.rs; done)`
→ no output (the three bodies concatenated are the old literal, line for line); and
`grep -c '^    Rule {' src/rules/table/*.rs` → `front_end.rs:52`, `tooling.rs:75`,
`type_check.rs:143`.

**Spec citations (30 cites):**
- `[[src/rules/table.rs:RULES]]` (13: `diagnostics/01_rule-codes.md` ×10, `diagnostics/spec.md`
  ×1, `language/04_types.md:221`, `tooling/02_source-selection.md:190`) →
  `[[src/rules/table/mod.rs:rules]]`.
- Rule-name cites (12) → the part defining the rule: `type_check.rs` — `TYPE_USE_AFTER_MOVE`
  (`language/14`), `TYPE_RESOURCE_REQUIRES_RES` (`language/15`, `16`), `TYPE_STATE_MISMATCH` ×2,
  `TYPE_RESOURCE_RETURN_ORDER`, `TYPE_THREAD_RESOURCE_PLANE_REQUIRED` (`language/15`, `16`),
  `TYPE_THREAD_WORKER_NOT_TRANSFERABLE` (`language/16`), `TYPE_MIXED_RESOURCE_UNION`
  (`memory/03`); `front_end.rs` — `MFB_PARSE_PARAMETER_RES` (`language/15`); `tooling.rs` —
  `FMT_CHECK_FAILED` (`tooling/05`).
- Stale line cites (5, §2) → rule-name cites in `type_check.rs`:
  `language/08_error-model.md:147` `[[src/rules/table.rs:312]] [[src/rules/table.rs:318]]` →
  `[[src/rules/table/type_check.rs:TYPE_PROPAGATE_REQUIRES_TRAP]] [[src/rules/table/type_check.rs:TYPE_FAIL_REQUIRES_ERROR]]`;
  `language/04_types.md:430` `:480` `:486` → `TYPE_READ_ONLY_RECORD_CONSTRUCTOR`,
  `TYPE_READ_ONLY_RECORD_UPDATE`; `language/04_types.md:223` `:642` →
  `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION`. Record the fix in bug-840 (item 1) in the same
  commit.

**Spec prose (the registry's shape is stated in `diagnostics/01_rule-codes.md`; it changes with
the code, `.ai/specifications.md`):** `:3` "drawn from the `RULES` registry" → "drawn from the
rule registry"; `:11-12` "the registry `RULES` is a flat `&[Rule]` slice" → "the registry is
three `&[Rule]` slices, one per category file under `src/rules/table/`, which
`table::rules()` walks in a fixed order; lookup is by name"; `:55` "as it actually appears in
`RULES`" → "in the registry"; `:62` and `:223`, `:616` "not in/a member of `RULES`" → "not in
the registry"; `:222` "All 252 `RULES` entries are listed" → "All 270 registry entries are
listed" (the measured count, `cat src/rules/table/*.rs | grep -c '^    Rule {'` → `270`;
bug-883 §A item 1 — record it there in the same commit).

**Other prose:** `.ai/resources-packages.md:11` — the closing sentence "The spec
self-contradicts: … — reading it back uncritically is how the drift recurs." → "`a6f4bf282`
removed the borrow half (§15.1's vocabulary and the rule `TYPE_RESOURCE_BORROW_INVALIDATE`,
now the reserved `TYPE_RESOURCE_INVALIDATE_NOT_OWNER`, 2-203-0086,
`src/rules/table/type_check.rs`); reading borrow vocabulary back in is how the drift recurs."

## Phases

> **NOTE — keep the checkboxes current as you go. This is not bookkeeping; it is
> the only way anyone can see where the work actually is.**
>
> - Tick `- [x]` **in the same commit as the work it describes** — never in a
>   batch at the end of a phase, never "I'll update the plan after". If it is a
>   separate step it will be skipped, and the list becomes a lie.
> - Use `- [~]` for partially done and say in one line what remains.
> - Mark a task moot with `- [x] ~~text~~ — moot: <evidence>` rather than
>   deleting it. A vanished task is indistinguishable from a forgotten one.
> - Fill the phase's `Commit:` line with the hash(es) the moment it lands.
> - Add any task you discover you needed; an unlisted task that got done is a
>   task the next reader will do twice.
>
> **An unticked box means NOT DONE.** Someone reading this file — including the
> person who has to decide whether to stop — is entitled to trust that.

Common gate:

- **G0 (once, before the first edit):** `cargo check --all-targets 2>&1 | grep -c '^warning' > /tmp/m_check.txt`,
  `cargo clippy --all-targets 2>&1 | grep -c '^warning' > /tmp/m_clippy.txt`,
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$' > /tmp/m_tests.txt` (est. 8 min cold).
- **G1:** the three commands → equal to the G0 files.
- **G2 (byte identity):** `cargo build --release` then `sh scripts/artifact-gate.sh target/release/mfb <builtin>`
  for each builtin the phase names → no diff (est. 3 min build + 1 min per builtin).
- **G3:** `cargo test -p mfb --bin mfb spec_citations_resolve` → pass; the §4 citation sweep
  check for the phase's files → no output.
- **G4:** `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings` → pass.
- **G5:** `cargo test --test file_size_budget 2>&1 | grep -E '^  (NEW|GREW)'` → no output; every
  `TIGHTEN`/`STALE` row names this phase's files; `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`,
  rerun → `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`; commit the ceiling file with the phase.
- **G6:** `cargo fmt --check` → no output.

### Phase 1 — AST and HIR

- [x] Split `src/hir/mod.rs` → `mod.rs` + `nodes.rs` per §4.3; repoint 2 citations.
      Done: `codesplit.py` → nodes 28 items, none left unlisted, 0 `pub(super)`; `grep -c ParameterType::parse
      src/hir/nodes.rs` → 0; `motion.py` → only the wiring; `citesweep.py` → 2.
- [x] Split `src/ast/serialize.rs` per §4.2; repoint the 3 comments.
      Done: `codesplit.py` → link 13, decls 12, body 7 items, none left unlisted, 0 `pub(super)`, no re-export;
      `motion.py` → only the `mod` lines; `src/operators.rs:24`, `src/operators/tests/mod.rs:210` →
      `serialize/body.rs`, `src/ast/tests/mod.rs:838` → `serialize/`, and `.ai/codegen-invariants.md:87`'s
      `ast/serialize.rs` (not in §4.2's list) → `ast/serialize/`.
- [x] Split `src/ast/expr.rs` per §4.1 (9 widenings); repoint 13 citations,
      `.ai/codegen-invariants.md:85,836` (the `ast/expr` half), the comments in
      `src/operators.rs`, `src/types.rs`, `src/ast/tests/`.
      Done: `items.py split` (method-level) → type_names 17 methods; the 9 `pub(super)` there are exactly §3's
      list and became `pub(in crate::ast)` (16 E0624s before, 0 after); `motion.py` → only the wiring and the
      9 visibility edits; `citesweep.py` → 13; `.ai/codegen-invariants.md:85,836`, `src/operators.rs:4,33`,
      `src/types.rs` (the grouped-type comment is at :652, not :594) and `src/ast/tests/mod.rs:150` repointed.
- [x] G5.
      Done: → 3 STALE `code` rows (expr 1083, serialize 1570, hir/mod 1002); tighten drops them; re-run → 0.

Acceptance: parsing, AST JSON and HIR are byte-identical.
  Check: G1; `cargo test -p mfb --bin mfb ast:: hir::` → 0 failed; G2 for `general`,
  `strings`, `collections` (their `.ast`/`.ir` goldens exercise the serializer and the
  expression and type-name parser); `cargo test --test no_type_strings` → passes with
  `src/hir/mod.rs` still the boundary; G3; G4; G6 (est. 15 min).
  Verified 2026-10-19: G1 → check 0, clippy 37065, tests 5017 (= G0, `target/p176/m0/`); `ast:: hir::` →
  `ok. 241 passed`; G2 (`bash`, Corrections) general, strings, collections → 7 goldens, 0 diffs each; G3 →
  `spec_citations_resolve` ok + `citedef.py` → 18 cites, 0 not defined where cited; G4 → 8, 8, 7 + 1 ignored
  (`no_type_strings` with `src/hir/mod.rs` still the boundary); census → hir 624/192 + 381, expr 732/155 + 355,
  serialize 302/51 + 343/338/593; G6 → exit 0.
Commit: 5a5cd0cd7

### Phase 2 — Lexer and the rules table

- [x] Split `src/lexer.rs` per §4.4; repoint 15 citations including the file cite and the two
      stale line cites.
      Done: `items.py split` → literals 11 methods, keywords 6 items (incl. `is_identifier_continue`, which
      §4.4 did not list; it sits in 1255-1453 and is called from `mod.rs`); `pub(super)` from the compiler's
      errors → §4.4's 4 + `is_identifier_continue`; `motion.py` → only the wiring and the 5; `citesweep.py` → 12,
      plus `[[src/lexer.rs]]` → `[[src/lexer/]]` and the two line cites → `lex_identifier_or_keyword` (it holds
      the `internalize` call, `grep -n internalize src/lexer/*.rs`).
- [x] `tests/guards/no_type_strings.rs` `is_excluded_from_scan` + its doc bullet (§4.4,
      justification there); `.ai/codegen-invariants.md:54`.
      Done: both edited. §2 predicted the scan would flag the moved lexer; it did not (`no_type_strings` → `7
      passed` before the edit too), so the edit keeps the exemption's intent rather than fixing a failure.
- [x] Before the table edit: `target/release/mfb spec diagnostics rule-codes > /tmp/m_spec_before.txt`
      and `target/release/mfb man > /tmp/m_man_before.txt` (release build of the phase's
      starting commit).
      Done: from the release binary of `5a5cd0cd7`, into `target/p176/m_{spec,man}_before.txt` (1269 / 94 lines).
- [x] Split `src/rules/table.rs` → `src/rules/table/{mod,front_end,type_check,tooling}.rs` per
      §4.8; run the order check → no output.
      Done: `m2-table.py` writes each part as `use super::*;`, blank, `pub(super) const RULES: &[Rule] = &[`, the
      old lines, `];`, and `mod.rs` exactly as §4.8; order check → no output; `grep -c '^    Rule {'` → 52 / 143 / 75.
- [x] `src/rules/mod.rs`: drop `use table::RULES;`, `rule_for` → `table::rules()`, the
      comments and the `debug_assert!` path (§4.8); the six tests in K's `src/rules/tests/`
      → `table::rules()`; `src/manifest/config.rs:165`.
      Done: as §4.8, every assertion and message unchanged (the `debug_assert!` keeps the word `RULES`).
- [x] Repoint the 30 `src/rules/table.rs` citations (13 `RULES`, 12 rule-name, 5 stale line)
      and the `diagnostics/01_rule-codes.md` prose (§4.8); record bug-840 item 1 and bug-883
      §A item 1 in their bug files.
      Done: `m2cites.py` → 13 `RULES` + 12 rule-name + 5 line = 30; `wordcheck.py` (the §4 sweep check) → 0
      missing; the six prose sites rewritten (`mfb spec` diff below). bug-840: items 2 (already fixed by bug-945)
      and 3 (the `next` field, a parse error, reproduced) were also open; item 3 fixed (`child AS Node` →
      `error[2-203-0065 …]` with the release `mfb`), so bug-840 is Fixed and moved to `bugs/completed/`. bug-883
      A1 recorded; its A2–A5 and Part B stay open there.
- [x] `.ai/resources-packages.md:11` (§4.8).
      Done: the claims re-checked first (`grep -i borrow 15_resource-management.md` → 0; 2-203-0086 is the reserved
      `TYPE_RESOURCE_INVALIDATE_NOT_OWNER`).
- [x] G5.
      Done: → 2 STALE `code` rows (lexer 1199, rules/table 1623); tighten drops them; re-run → 0.

Acceptance: tokens are byte-identical, the lexer is still outside the type-string scan, and
every diagnostic resolves to the same rule.
  Check: G1; `cargo test -p mfb --bin mfb lexer:: rules::` → 0 failed (the 20 `rules::` tests
  include the six table tests); G2 for `general`, `strings`, `money` (literal, escape and number
  lexing); every diagnostic still renders the same code/severity/message — the invalid fixtures'
  `build.log` goldens: `bash scripts/test-accept.sh target/release/mfb /tmp/m_accept_p2 '*invalid*'`
  → no golden diff (est. 12 min); `target/release/mfb man | diff /tmp/m_man_before.txt -` → no
  output; `target/release/mfb spec diagnostics rule-codes | diff /tmp/m_spec_before.txt -` →
  only the §4.8 prose lines and lines containing `src/rules/table`;
  `grep -rnE '\[\[src/(lexer\.rs|rules/table\.rs)' src/docs/spec` → 0;
  `python3 -I tools/loc-census/census.py --file src/rules/table/type_check.rs` → `861 0`;
  `cargo test --test file_size_budget 2>&1 | grep 'src/rules/table'` → no row; G3; G4; G6
  (est. 25 min).
  Verified 2026-10-19: G1 → 0 / 37065 / 5017; `lexer:: rules::` → `ok. 55 passed` (`rules::` alone → 20); G2 general,
  strings, money → 0 diffs; `test-accept.sh … '*invalid*'` → `acceptance tests passed (611 test(s) ran)`, no
  file under `tests/` changed; `mfb man` → identical; `mfb spec diagnostics rule-codes` → only the six §4.8
  prose lines; `[[src/lexer.rs` / `[[src/rules/table.rs` cites → 0; `type_check.rs` → `861 0`; `file_size_budget`
  rows for `src/rules/table` → 0; G3 → ok + `wordcheck.py` 0 missing; G4 → 8, 8, 7 + 1 ignored; census → lexer
  543/2 + 482 + 181, table 8 + 316/861/453; G6 → exit 0.
Commit: a21321f5d

### Phase 3 — Resolver

- [x] Split `src/resolver/resolution.rs` per §4.5 (2 widenings); repoint 27 citations; the
      `no_type_strings.rs:422` comment.
      Done: `items.py split` (method-level) → docs 4, body 10, types 4 methods + `peel_res`; `resolve_doc_blocks`,
      `check_new_local` → `pub(in crate::resolver)`; `pub(super)` from the compiler's errors → exactly §4.5's 6;
      `motion.py` → only the wiring, the two new impl blocks and the 8 visibility edits; `citesweep.py` → 27,
      `wordcheck.py` → 0 missing; the guard comment (now :425) → `resolver/resolution/docs.rs`.
- [x] G5.
      Done: → 1 STALE `code` row (resolution 1496); tighten drops it; re-run → 0.

Acceptance: name resolution and its diagnostics are byte-identical.
  Check: G1; `cargo test -p mfb --bin mfb resolver::` → 0 failed; G2 for `general`,
  `collections`; resolver diagnostics are pinned by the `build.log` goldens of the invalid
  fixtures, which the artifact gate does not diff:
  `bash scripts/test-accept.sh target/release/mfb /tmp/m_accept_p3 '*invalid*'` → no golden
  diff (est. 12 min; the invalid fixtures are the only goldens that pin resolver diagnostics);
  G3; G4; G6.
  Verified 2026-10-19: G1 → 0 / 37065 / 5017; `resolver::` → `ok. 105 passed`; G2 general, collections → 0 diffs;
  `test-accept.sh … '*invalid*'` → `acceptance tests passed (611 test(s) ran)`, no golden changed; G3 → ok +
  `wordcheck.py` 0 missing; G4 → 8, 8, 7 + 1 ignored; census → 383/2, 425, 529, 172; G6 → exit 0.
Commit: 860760b44

### Phase 4 — Monomorphizer

- [x] Split `src/monomorph/lower.rs` per §4.6 (five impl blocks, 1 widening, 23
      `pub(super)`); repoint 33 citations and `.ai/codegen-invariants.md:808`.
      Done: `items.py split` (method-level) → resolve 12, instantiate 13 methods + 2 items, body 4, types 9 + 1;
      `template_spellings` → `pub(in crate::monomorph)`; `pub(super)` from the compiler's errors → 22 (§4.6's 23
      minus `arg_types_in_param_order`, whose only callers are in `instantiate.rs`); `motion.py` → only the
      wiring, four new impl blocks and the visibility edits; `citesweep.py` → 33, `wordcheck.py` → 0 missing;
      `.ai/codegen-invariants.md:808` → `lower/body.rs`, and its :467 historical mention gains "(now
      `monomorph/lower/`)".
- [x] G5.
      Done: → 1 STALE `code` row (lower 2521); tighten drops it; re-run → 0.

Acceptance: instantiation and overload selection are byte-identical.
  Check: G1; `cargo test -p mfb --bin mfb monomorph::` → 0 failed; G2 for `collections`,
  `general`, `thread` (generic collections, overloads and the thread plane drive the moved
  resolve/instantiate code); G3; G4; G6 (est. 15 min).
  Verified 2026-10-19: G1 → 0 / 37065 / 5017; `monomorph::` → `ok. 71 passed`; G2 collections, general, thread
  → 0 diffs; G3 → ok + `wordcheck.py` 0 missing; G4 → 8, 8, 7 + 1 ignored; census → 457/2, 464, 493, 745, 381;
  G6 → exit 0.
Commit: —

### Phase 5 — Builtin registry (largest blast radius)

- [ ] Split `src/codegen/registry/mod.rs` into `mod.rs` + the 8 parts per §4.7 (the H-left
      cfg(test) helpers go to `unify.rs`).
- [ ] Repoint the 27 moved-symbol citations and the dangling `call_return_type` cite.
- [ ] Prose/comments per §4.7 (`.ai/codegen-invariants.md:836` registry half,
      `src/cli/man.rs:666`, `src/codegen/builtins/vector/mod.rs:549`, the `src/ir` test comment,
      `builder_value_semantics.rs:1711`, `no_type_strings.rs:929`).
- [ ] G5.

Acceptance: every builtin call resolves and lowers byte-identically.
  Check: G1; `cargo test -p mfb --bin mfb codegen::registry` → 0 failed (H's 62 tests, the
  `qualification_tests` and the unify helpers); G2 for `collections`, `general`, `thread`,
  `crypto`, `vector`; G3; G4; G6 (est. 18 min).
Commit: —

## Validation Plan

- Tests: no test added or removed (G1 per phase). Unit tests of every moved module run in
  their phase. The six rule-table tests change only how they obtain the rules (§4.8).
- Coverage check: pure motion inside `src/` — the moved lines keep their coverage counts; the
  denominators move between files. The new `table::rules()` runs on every diagnostic and in
  every rule-table test. No coverage exception names an M file
  (`grep -nE 'src/(ast|hir|lexer|monomorph|resolver|rules|codegen/registry)' scripts/coverage-exceptions.txt`
  → 0).
- Runtime proof / final gate (once, after Phase 5): `cargo build --release` and
  `sh scripts/artifact-gate.sh target/release/mfb all` → no diff (est. 15–20 min; code motion
  in the front end and the registry can change emission for any fixture on any target, and
  only the full gate covers every fixture on every target). Diagnostics from the lexer,
  parser, resolver and the rule table are pinned by `build.log` goldens the artifact gate
  does not diff: `bash scripts/test-accept.sh target/release/mfb /tmp/m_accept '*invalid*'` →
  no golden diff (est. 12 min). Then G1, G3, G4 and
  `cargo test --test file_size_budget` → banner 0 rows (est. 8 min). A diff is root-caused on
  one fixture and fixed; the gate is rerun. The full workspace suite runs once, in
  plan-176-V.
- Doc sync: spec citations listed in §4 (path-only changes plus the stale/dangling repairs);
  `diagnostics/01_rule-codes.md` prose on the registry's shape and count (§4.8);
  `.ai/codegen-invariants.md:54,85,808,836`; `.ai/resources-packages.md:11`. The only spec
  contract that changes is the registry's described shape (one slice → three parts walked in
  order); no diagnostic changes.

## Decisions

- Rule revised — owner, 2026-10-18. Three kinds, three limits, all non-comment non-blank LOC
  (counting rules unchanged from plan-176-A §2); scope `.rs`/`.mfb` under `src/` `packages/`
  `examples/` `repository/` `tests/`:

  | Kind | Applies to | Limit | Over the limit → |
  |---|---|---|---|
  | `code` | code LOC of a file that is not a test-only file | 1000 | split the file into modules |
  | `inline` | inline test LOC (`#[cfg(test)]` items / MFBASIC `TESTING … END TESTING`) in a file that is not test-only | 250 | move the tests into their own test-only file(s) |
  | `test` | total LOC of a test-only file | 1000 | split the test-only file into modules |

  Ceiling rows are `ceiling <code|inline|test> <n> <path>`; parts target ≤ 900 of their kind.
  Inline modules within 250 stay inline: `ast/expr` (155), `ast/serialize` (51), `hir` (192),
  `registry::qualification_tests` (79); `unify.rs` carries 117 cfg(test) LOC.
- No exemptions; the guard warns (banner), never fails; only `GENERATED`-banner files are
  skipped; a new module is always an acceptable split — owner, 2026-10-18 / 2026-10-14.
- A single indivisible unit over its limit gets a real split — owner, 2026-10-18. M's one such
  unit is `RULES`: three per-category `&[Rule]` parts walked by `table::rules()` in the old
  order (§4.8), the fewest parts that keep each ≤ 900.
- A single function over the code limit is decomposed by extract-method gated by byte identity
  — owner, 2026-10-14. No M function is over 1000 (largest: `lower_expression` 378), so M does
  no extraction.
- Guards that classify by file name are updated in the same commit as the move, with the
  4-point justification — author, 2026-10-14 (`no_type_strings` lexer clause, §4.4).
- Pre-existing defects found by the map are fixed here: the two lexer line cites, five rules
  line cites (bug-840 item 1), `call_return_type`, the stale `.ai/resources-packages.md:11`
  borrow sentence, the "All 252" count (bug-883 §A item 1) — author, 2026-10-18.
- Byte identity is the gate for compiler code motion; the full artifact gate runs once at the
  end — brief, 2026-10-14.
- Glob-cycle imports compile without warnings — verified (`/tmp/loc/probe2`); the rule-table
  accessor shape compiles clean and keeps order — verified (`/tmp/loc/rw-M/probe/m2.rs`).

## Corrections

- 2026-10-18 — Rule revised by the owner: test-only files limit 1000 (was 250), inline tests
  250, no exemptions. Dropped 0 files (M has no test-only file; every inline block it keeps is
  ≤ 250). `src/rules/table.rs` is no longer exempt: it is split into three category parts plus
  an ordering `mod.rs` (§4.8), its `exempt` row and `EXEMPT` entry are gone, and its 30 spec
  citations, the `diagnostics/01_rule-codes.md` shape prose and the `.ai/resources-packages.md`
  cite move with it. Counts and line ranges re-taken at `ce7a995ca`: `expr.rs` 1083 (new
  `parse_template_argument_list`, `parse_function_type_params` → `type_names.rs`),
  `lower.rs` 2521 (new `union_accepts` → `mod.rs`, `imported_overload_set` → `resolve.rs`,
  `TemplateBindFailure`, `template_identity`, `bind_template`, `report_binding_failure`,
  `most_specific` → `instantiate.rs`), registry 3108 (`call_carries_its_fill_defaults` →
  `lookup.rs`), `resolution.rs` 1496 (`report_result_not_user_visible` was deleted upstream;
  the dangling `resolve_type_name` cite is already gone). The registry's H test declarations follow H's revised layout
  (`#[path = "tests/raw_result_block_ownership.rs"]`).

- 2026-10-19 — `scripts/artifact-gate.sh` is a bash script (process substitution at line 191); `sh
  scripts/artifact-gate.sh` fails on macOS with `syntax error near unexpected token '<'` before checking anything.
  Every G2 and final-gate run uses `bash scripts/artifact-gate.sh`. At the starting commit `211227cb9` it reported
  0 diffs for general, strings, collections, money, thread, crypto and vector.

## Summary

M is code motion through the compiler front end and the registry, proven by byte-identical
artifacts, plus one real data split: the rule table becomes three category slices that
`table::rules()` walks in the original order, proven line for line by the order check and by
the invalid-fixture `build.log` goldens. The real risks are the visibility depth trap (12
methods that must become `pub(in crate::<parent>)`, not `pub(super)`), the lexer's exact-path
guard exemption, and the registry split that every builtin call passes through — which is why
it lands last, behind the full artifact gate. Stale citations get real symbols on the way.
