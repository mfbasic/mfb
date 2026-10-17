# plan-176-M: Frontend and registry code

Last updated: 2026-10-14
Effort: large (3h–1d)
Depends on: plan-176-L

M splits the over-budget CODE files of the compiler front end — the lexer, the AST parser and
serializer, HIR, the resolver, the monomorphizer — and the builtin registry
(`src/codegen/registry/mod.rs`). Each `foo.rs` becomes `foo/mod.rs` plus part files; the files
that are already `mod.rs` gain sibling parts. The work is pure code motion: items move between
files, an `impl` is cut into one block per file, and private items that now cross a file
boundary get `pub(super)` (or `pub(in crate::<parent>)` where a method moves one level deeper,
§3). One file, `src/rules/table.rs`, is a single data literal with no code-motion split; it
gets an `exempt` row. The test modules of these files were moved by letters G, H and K and
stay declared in the new `mod.rs` files.

The single outcome: after M lands, `cargo test --test file_size_budget` shows no row for any of
M's files and `tests/guards/file_size_ceilings.txt` has no `ceiling` row for them; its only
M row is `exempt code src/rules/table.rs # …`.

References:

- `planning/plan-176-A-file-size-guard.md` — the rule, §Decisions, "Splitting an over-budget
  file" (`.ai/build-tooling.md`).
- `planning/plan-176-G-ir-monomorph-tests.md`, `planning/plan-176-H-codegen-builtins-engine-tests.md`,
  `planning/plan-176-K-frontend-tooling-tests.md` — where the test modules went.
- `.ai/compiler.md`, `.ai/codegen-invariants.md` (names `src/ast/expr.rs`, `src/lexer.rs`,
  `src/monomorph/lower.rs`, `registry/mod.rs`), `.ai/resources-packages.md` (names
  `registry::build()`), `.ai/man-content.md` (names the registry vocabulary test),
  `.ai/testing-gates.md` (artifact gate), `.ai/build-tooling.md`.
- Spec topics whose citations move: `language/02_lexical-structure.md`,
  `language/03`, `04_types.md`, `05`, `06`, `08_error-model.md`, `11_operators.md`, `13`,
  `17`, `18_builtin-functions.md`; `architecture/05`, `09_modules.md`,
  `12_monomorphization.md`, `19_internal-naming.md`, `21_type-name-encoding.md`,
  `22_type-inference.md`; `stdlib/12_bits.md`, `15`, `17`, `18`; and the 18 topics that cite
  `src/codegen/registry/mod.rs` (`grep -rlE "\[\[src/codegen/registry/mod.rs" src/docs/spec`).
- Guard: `tests/guards/no_type_strings.rs` (`is_excluded_from_scan`, and `BOUNDARY_FILES` in
  `tests/guards/no_type_strings/allowlists.rs` after plan-176-E).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-L is complete and archived | `ls planning/plan-176-L-* planning/completed/plan-176-L-*` → only `planning/completed/…` | NOT MET (2026-10-14) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | NOT MET (2026-10-14) |
| M's files have no uncommitted changes | `git status --porcelain -- src/ast src/hir src/monomorph src/resolver src/rules src/lexer.rs src/codegen/registry tests/guards tools/loc-census` → empty | NOT MET (2026-10-14): ` M src/codegen/registry/mod.rs` |

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

- Every M file and part is ≤ 1000 code LOC and ≤ 250 test LOC
  (`python3 -I tools/loc-census/census.py --file <path>`), except `src/rules/table.rs`, which
  carries an `exempt code` row with its reason in the ceiling file and in
  `tools/loc-census/census.py`'s `EXEMPT` table.
- The compiler's output is byte-identical: `sh scripts/artifact-gate.sh target/release/mfb all`
  reports no diff.
- Every spec citation of a moved symbol names the file that defines it, and the stale/dangling
  citations listed in §2 name a real symbol.

### Non-goals (explicit constraints)

- No logic edit; no diagnostic text, rule code, AST/IR JSON shape or emitted byte changes.
- No test added, removed or renamed.
- Every `crate::…` path callers use is unchanged (`crate::lexer::lookup_keyword`,
  `crate::hir::HirFile`, `crate::codegen::registry::registry`, …) via glob re-exports.
- No visibility widened beyond what a move forces: `pub(super)` for a part item used by a
  sibling/`mod.rs`/tests, `pub(in crate::<parent>)` for the 12 methods listed in §3.
- `src/rules/table.rs` is not edited (its citations resolve by text in that file).

## 2. Current State

Counts: `python3 -I /tmp/loc/span.py <file>` on the working tree of `17a9b5799`
(`(code, test)`); after plan-176-A, `python3 -I tools/loc-census/census.py --file <path>`.
Part LOC: the split map's `xref.py` sum of code lines over the listed ranges (parts sum to the
file total). Cites: `grep -rnoE "\[\[<path>(:[^]]+)?\]\]" src/docs/spec | wc -l`.

| File | Code | Test (owner) | Parts after M | Cites |
|---|---|---|---|---|
| `src/ast/expr.rs` | 1047 | 155 (stays inline) | mod + 1 | 12 |
| `src/ast/serialize.rs` | 1570 | 51 (stays inline) | mod + 3 | 0 |
| `src/codegen/registry/mod.rs` | 3093 | 2695 (H) | mod + 8 | 34 |
| `src/hir/mod.rs` | 1002 | 192 (stays inline) | mod + 1 | 2 |
| `src/lexer.rs` | 1199 | 555 (K) | mod + 2 | 15 |
| `src/monomorph/lower.rs` | 2378 | 1282 (G) | mod + 4 | 31 |
| `src/resolver/resolution.rs` | 1514 | 737 (K) | mod + 3 | 27 |
| `src/rules/table.rs` | 1617 | 0 | exempt | 30 |

After H, `src/codegen/registry/mod.rs` keeps `mod qualification_tests` (79 test LOC) and the
cfg(test) helper fns inside the unify range (`self_update_shaped`,
`is_string_self_update_type`, `strip_res`, `resolve_bound`, `occurs`, `types_coincide`,
`resolve_call`; 117 test LOC), plus `#[cfg(test)] mod tests;` and
`#[cfg(test)] #[path = "tests/raw_result_block_ownership/mod.rs"] mod raw_result_block_ownership;`
(H map). After G/K, `lower.rs`, `lexer.rs`, `resolution.rs` hold `#[cfg(test)] mod tests;`.

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
- **`src/rules/table.rs` has no code-motion split.** It is `use super::{Rule, Severity};` plus
  one item, `pub(super) const RULES: &[Rule] = &[ … ];` (lines 3-1712, 1616 LOC, 269 `Rule`
  literals). A `&[T]` const cannot be assembled from several files without new code: a
  `const fn` concatenation needs `Rule: Copy` (not derived) and a placeholder element; a
  slice-of-slices changes the seven `RULES` uses in `src/rules/mod.rs` (427, 467, 519, 534,
  564, 705, 716). It is data, not a function, so the extract-method decision does not apply.
- **Stale or dangling citations** (pre-existing, fixed in M):
  - `[[src/lexer.rs:262]]` (`architecture/19_internal-naming.md:17`) and
    `[[src/lexer.rs:752]]` (`:50`): line 262 is `self.advance();` in `lex_all`, 752 is
    `value.push(self.peek());` in `lex_number`; the claim (the `internalize` rewrite) is at
    `src/lexer.rs:982` inside `lex_identifier_or_keyword` (`grep -n internalize src/lexer.rs`).
  - `[[src/rules/table.rs:312]]`, `[[…:318]]` (`language/08_error-model.md:150`, backing
    `TYPE_PROPAGATE_REQUIRES_TRAP` / `TYPE_FAIL_REQUIRES_ERROR`); `[[…:480]]`, `[[…:486]]`
    (`language/04_types.md:413`, backing "may not construct … update … with WITH"); `[[…:642]]`
    (`language/04_types.md:212`, backing `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION`). Lines
    312/318/480/486 are `severity: Severity::Error,` of other rules and 642 is the "map value
    type" message (`for l in 312 318 480 486 642; do sed -n ${l}p src/rules/table.rs; done`).
    The rules are at 383 (`TYPE_FAIL_REQUIRES_ERROR`), 389 (`TYPE_PROPAGATE_REQUIRES_TRAP`), 551
    (`TYPE_READ_ONLY_RECORD_UPDATE`), 557 (`TYPE_READ_ONLY_RECORD_CONSTRUCTOR`), 729
    (`TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION`) (`grep -n 'name: "…"' src/rules/table.rs`).
  - `[[src/resolver/resolution.rs:resolve_type_name]]` (`language/08_error-model.md:138`): no
    such fn (`grep -rn "fn resolve_type_name" src` → only a test fn in
    `src/codegen/builtins/general/mod.rs:978`). The claim ("`Unknown` and any active template
    parameter are always resolved") is `resolve_leaf`:
    `src/resolver/resolution.rs:1625` `if type_.is_named("Unknown") || self.active_template_params.contains(name)`.
  - `[[src/codegen/registry/mod.rs:call_return_type]]` (`stdlib/12_bits.md:21`): the registry
    defines `call_return_type_typed` (`src/codegen/registry/mod.rs:2595`);
    `call_return_type` is `src/codegen/builtins/mod.rs:667`, which delegates to it, and the same
    line already cites `[[src/codegen/builtins/mod.rs:resolve_call_return_type]]`.

## 3. Design Overview

Convention (plan-176-A): `mod.rs` keeps the module header, the core types/entry points and the
test declarations; parts by responsibility, each starting `use super::*;`; `mod.rs` declares
`mod <part>;` and, for free items, `pub(crate) use <part>::*;` (`pub use` for the `pub` lexer
API). Line ranges are source lines as of `17a9b5799`; item NAMES are authoritative (the test
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
modules' unit tests. Once at the end: `sh scripts/artifact-gate.sh target/release/mfb all`.
A diff is root-caused (objdump / `diff` ONE fixture's artifact), fixed, and the gate rerun; it
never stops the plan.

Risk concentrates in the registry (eight parts, every builtin call resolves through it, and
the H-left cfg(test) helpers ride along into `unify.rs`) and the monomorphizer (one 2314-LOC
`impl` cut into five blocks). Both are the last two phases.

Rejected alternatives:
- *Splitting `RULES` by a `const fn` concatenation or a slice of slices* — new code to satisfy a
  size rule, against "no logic edit"; the owner's exception for single over-limit units covers
  it (`exempt`).
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

### 4.1 `src/ast/expr.rs` → `src/ast/expr/` (1047)

All code is `impl<'a> FileParser<'a>` (28-1348, 1041 LOC), cut into two blocks.
- `mod.rs` (731): 1-27 (`use super::pipeline::…`, `use super::*`, `use crate::operators::…`,
  `MAX_EXPR_DEPTH`, `MAX_TYPE_DEPTH`, `MAX_PIPELINE_COPIED_NODES`); an impl block with 194-877
  (`enter_expr`, `leave_expr`, `note_expr_tree_depth`, `latch_hostile_expression`,
  `parse_expression`, `parse_pipeline`, `parse_or`, `parse_and`, `parse_not`,
  `parse_comparison`, `parse_concat`, `parse_addition`, `parse_multiplication`,
  `parse_power`, `parse_unary`, `parse_with_update`, `parse_member_access`,
  `parse_call_or_constructor`, `parse_argument_list`, `parse_constructor_argument_list`,
  `parse_primary`), 1182-1227 (`parse_lambda`), 1248-1348 (`parse_list_literal`,
  `parse_map_literal`, `parse_set_literal`); `mod type_names;`; the inline
  `#[cfg(test)] mod tests` (155 test LOC) stays inline.
- `type_names.rs` (316): an impl block with 51-193 (`normalize_qualified_builtin_type`,
  `normalize_qualified_type_name`, `package_exports_type`, `qualify_own_builtin_type`),
  878-1181 (`parse_qualified_name`, `finish_qualified_name`, `enter_type`, `leave_type`,
  `parse_type_name`, `parse_type_name_inner`, `parse_thread_type_name`,
  `parse_resource_plane_type`, `parse_optional_element_state`, `parse_function_type_name`),
  1228-1247 (`parse_type_base_name`). (Named so it does not read as the sibling
  `src/ast/types.rs`.)

Nothing re-exported (inherent methods). 9 widenings (§3); new `pub(super)`: 0 (`enter_type`,
`leave_type`, `parse_type_name_inner`, … are used only inside `type_names.rs`;
`MAX_TYPE_DEPTH` stays in `mod.rs`).
Citations (12 cites, 10 symbols): → `src/ast/expr/mod.rs` — `parse_argument_list`,
`parse_list_literal` (`language/02_lexical-structure.md`), `parse_member_access`,
`parse_power`, `parse_unary` (`language/11_operators.md`); → `src/ast/expr/type_names.rs` —
`finish_qualified_name`, `parse_optional_element_state`, `parse_thread_type_name`,
`parse_type_base_name`, `parse_type_name` (`architecture/21_type-name-encoding.md`).
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

- `mod.rs` (543): 1-347 (`TokenKind`, `DocRaw`, `DocRawLine`, `Keyword`, `Token`,
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
- `keywords.rs` (177; 1255-1449): `lookup_keyword`, `is_end_line`, `is_doc_attr_rest`,
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

### 4.5 `src/resolver/resolution.rs` → `src/resolver/resolution/` (1514)

`impl Resolver` (3-1768) cut into four blocks.
- `mod.rs` (378): 1-13 (`use super::*`, `resolve`), 479-897 (`resolve_file`,
  `resolve_resource_decl`, `resolve_func_alias`, `resolve_link_block`, `resolve_binding`,
  `resolve_type_decl`, `resolve_member_field`, `resolve_function`); `mod docs; mod body;
  mod types; use types::peel_res;` (free fn); `#[cfg(test)] mod tests;` (K).
- `docs.rs` (422; 14-478): `resolve_doc_blocks`, `validate_doc_block`, `note_doc_target`,
  `check_doc_named`.
- `body.rs` (531; 898-1491): `resolve_block`, `resolve_statement`, `resolve_nested_block`,
  `resolve_match_pattern`, `resolve_expression`, `resolve_callable`,
  `resolve_parameter_default`, `report_default_names_parameter`, `check_new_local`,
  `resolve_identifier`.
- `types.rs` (183; 1492-1775): `resolve_type`, `resolve_leaf`, `report_unknown_type`,
  `report_result_not_user_visible`, `resolve_package_qualified_name`, free fn `peel_res`.

2 widenings (§3); `resolve` stays in `mod.rs`. pub(super) (6): `resolve_block`,
`resolve_expression`, `resolve_parameter_default` (body); `resolve_type`,
`resolve_package_qualified_name`, `peel_res` (types).
Citations (27 cites, 14 symbols; `language/04, 05, 06, 08, 13, 17, 18`, `architecture/05, 21`,
`stdlib/15, 17, 18`): → `mod.rs` — `resolve_func_alias`, `resolve_function`,
`resolve_resource_decl`, `resolve_type_decl`; → `body.rs` — `check_new_local`,
`resolve_callable`, `resolve_expression`, `resolve_match_pattern`, `resolve_nested_block`,
`resolve_parameter_default`, `resolve_statement`; → `types.rs` —
`resolve_package_qualified_name`, `resolve_type`. Dangling
`[[src/resolver/resolution.rs:resolve_type_name]]` (`language/08_error-model.md:138`) →
`[[src/resolver/resolution/types.rs:resolve_leaf]]` (§2).
`tests/guards/no_type_strings.rs:422` (doc comment: `resolver/resolution.rs`'s `types` /
`resources` declaration indexes) → `resolver/resolution/docs.rs` (comment only; no allowlist
entry).

### 4.6 `src/monomorph/lower.rs` → `src/monomorph/lower/` (2378; G created `lower/tests/`)

`impl Monomorphizer` (68-2963, 2314 LOC) cut into five blocks.
- `mod.rs` (439): 1-67 (`use super::*`, `use crate::operators…`, `strip_qualifier_prefixes`,
  `impl TypeKinds for Monomorphizer`); an impl block with `new` (69-290), `run`,
  `into_project` (530-749), `report`, `charge_instantiation`,
  `report_instantiation_too_deep` (2894-2963); `mod resolve; mod instantiate; mod body;
  mod types; use types::expected_element_type;`; `#[cfg(test)] mod tests;` (G).
- `resolve.rs` (451): 291-529 (`collections_internal_callee`, `resolve_imported_overload`,
  `types_compatible`, `normalize_type`), 1145-1498 (`resolve_overload`, `template_name`,
  `sole_file_sibling`, `resolve_function_reference`, `resolve_builtin_override_reference`,
  `resolve_general_builtin_override`, `single_signature_params`).
- `instantiate.rs` (368): 750-1144 (`lower_type`, `lower_binding`, `lower_function`,
  `lower_function_inner`, `arg_types_in_param_order`, `unique_concrete_symbol`,
  `instantiate_function`, `user_instantiation_site`), 1499-1559 (`instantiate_type`,
  `lower_field`).
- `body.rs` (745; 1560-2406): `lower_statements`, `lower_statement` (324 LOC),
  `lower_expression` (381 LOC), `lower_constructor_arg`.
- `types.rs` (375): 2407-2893 (`leaf_symbol`, `concrete_type`, `template_view`,
  `template_spellings`, `union_pattern_type`, `collect_union_instantiations`,
  `record_fields`, `builtin_call_return_type`, `expression_type`), 2964-2974 (free fn
  `expected_element_type`).

No re-exports of methods; `monomorph/mod.rs` uses only `Monomorphizer::{new, run,
into_project, template_spellings}`. 1 widening (`template_spellings`, §3). pub(super) (23):
resolve — `collections_internal_callee`, `resolve_imported_overload`, `types_compatible`,
`normalize_type`, `resolve_overload`, `template_name`, `resolve_function_reference`,
`resolve_general_builtin_override`, `single_signature_params`; instantiate — `lower_type`,
`lower_binding`, `lower_function`, `arg_types_in_param_order`, `instantiate_function`,
`instantiate_type`; body — `lower_statements`, `lower_expression`; types — `concrete_type`,
`template_view`, `union_pattern_type`, `record_fields`, `expression_type`,
`expected_element_type`. `Monomorphizer`'s fields are defined in `monomorph/mod.rs`, an
ancestor, so they stay visible.
Citations (31 cites, 19 symbols; `architecture/12, 22`, `language/03, 04, 06, 18`): →
`mod.rs` — `Monomorphizer::new`, `into_project`; → `resolve.rs` —
`collections_internal_callee`, `normalize_type`, `resolve_builtin_override_reference`,
`resolve_function_reference`, `resolve_general_builtin_override`,
`resolve_imported_overload`, `resolve_overload`, `single_signature_params`, `template_name`,
`types_compatible`; → `instantiate.rs` — `instantiate_function`, `instantiate_type`,
`lower_type`, `user_instantiation_site`; → `types.rs` — `collect_union_instantiations`,
`expression_type`, `union_pattern_type`. `.ai/codegen-invariants.md:808` "`lower_expression`
in `src/monomorph/lower.rs`" → `src/monomorph/lower/body.rs`. `no_type_strings` lists only
`src/monomorph/mod.rs` (untouched).

### 4.7 `src/codegen/registry/mod.rs` + 8 parts (3093)

- `mod.rs` (417): 1-38 (`use` header); 105-145 (`AbiInline`, `AbiFunction`, `MfbFastPath`
  aliases); 1563-2169 (`ResolvedFunc`, `ResolvedType`, `Registry` + its whole impl:
  `enum_variant_advisory`, `resolve_type`, `augment_project`, `augment_hir_project`,
  `synthetic_files`, `is_member`, `is_builtin_type`, `qualify_value_type_references`,
  `qualified_builtin_type`, …; `registry()`, `BuiltinRecordLayout`,
  `builtin_record_layouts`); 2315-2361 (`build()`); the 8 `mod` lines and
  `pub(crate) use <part>::*;` for each; H's `mod qualification_tests` (inline, 79 test LOC),
  `#[cfg(test)] mod tests;` and the `#[path = "tests/raw_result_block_ownership/mod.rs"]`
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
- `lookup.rs` (609): 2362-2649 (`RuntimeCall`, `runtime_specs`, `push_runtime_call`,
  `qualify_runtime_call`, `find_constant`, `is_package_constant`, `constant_type_name`,
  `constant_value`, `constant_components`, `errorcode_constant`, `runtime_error*`,
  `general_override_target`, `call_return_type_typed`, `alias_call_return_type`); 3531-4292
  (`resolved_return_type`, `rewrite_target*`, `rewrite_owner`, `abi_inline_lower`,
  `natively_called_helpers`, `abi_function_lower`, `is_abi_function_call`,
  `native_member_declares_error`, `mfb_fast_path`, `public_member_for_rewrite`,
  `native_bare_target`, `callback_member*`, `function_has_unary_callback`,
  `argument_types*`, `agreed_argument_type`, `resolved_parameter_type*`,
  `contains_unknown`, `expected_arguments`, `overloads_disagree_on_layout`,
  `call_param_name*`, `default_argument_padding`).
- `unify.rs` (474 code + 117 cfg(test); 2650-3530): `leaf_matches`, `union_has_member`,
  `value_type_accepts`, `registry_union_accepts`, `is_resource_type_name`, `unify`,
  `unify_with_kinds`, `VarPosition`, `Position` + impl, `converts_to_union_form`,
  `NothingBinding`, `unify_at`, `thread_slot_unifies`, `resource_base_eq`, `substitute`,
  `contains_var`, `resolve_call_typed`, `resolve_call_typed_with_kinds`, and the cfg(test)
  helpers H left in this range (`self_update_shaped`, `is_string_self_update_type`,
  `strip_res`, `resolve_bound`, `occurs`, `types_coincide`, `resolve_call`) — they travel
  with the code they sit between; `raw_result_block_ownership`'s `use super::runtime_specs;`
  still resolves through `mod.rs`'s glob.
- `late_pass.rs` (142; 4293-4501): `ProjectView` + impl (`of_hir`, `references_any`, …),
  `note_type`, `inject_late_pass`, `inject_late_pass_hir`, `late_pass_files`, `short_callee`.
- `ast_scan.rs` (356; 4502-4872): `item_types`, `params_types`, `group_types`, `stmt_types`,
  `expr_types`, `item_callees`, `group_callees`, `stmt_callees`, `expr_callees`.
- `hir_scan.rs` (326; 4873-5207): `hir_item_types`, `hir_params_types`, `hir_stmt_types`,
  `hir_expr_types`, `hir_item_callees`, `hir_stmt_callees`, `hir_expr_callees`.

Every `crate::codegen::registry::X` stays valid through the 8 globs. pub(super) (21):
`RegistryRecord::render`, `RegistryUnion::render`, `RegistryEnum::render`,
`RegistryPackage::always_helper_bodies`, `qualify_type_leaves`,
`qualify_type_leaves_for_source`, `qualify_type_leaves_inner`, `leaf_matches`, `unify`,
`unify_with_kinds`, `converts_to_union_form`, `substitute`, `resolved_return_type`,
`note_type`, `short_callee`, `item_types`, `group_types`, `item_callees`, `group_callees`,
`hir_item_types`, `hir_item_callees`; plus private descriptor fields read by `Registry`'s impl
or by `package.rs` (visibility rule; most are already `pub(crate)`).
Citations (34; 33 symbol cites over 26 symbols + the file cite in
`architecture/09_modules.md`, which stays valid): stay in `mod.rs` — `augment_project`,
`is_builtin_type`, `is_member`; → `descriptors.rs` — `EnumAdvisory`, `HelperGate`,
`debug_split`; → `unify.rs` — `VarPosition`, `leaf_matches`, `resolve_call` (cfg(test)
helper), `resource_base_eq`, `unify_at`, `unify_with_kinds`, `union_has_member`,
`value_type_accepts`; → `lookup.rs` — `abi_inline_lower`, `constant_value`,
`default_argument_padding`, `find_constant`, `general_override_target`,
`native_bare_target`, `resolved_parameter_type`; → `late_pass.rs` — `late_pass_files`,
`references_any`; → `qualify.rs` — `qualify_type_leaves_inner`. The test-fn cite
`range_and_index_parameters_use_the_documented_vocabulary` (`language/18`) was repointed by H.
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

### 4.8 `src/rules/table.rs` — exempt (1617)

Not split (§2). Edits in the same commit:
- `tests/guards/file_size_ceilings.txt`: the `ceiling code 1617 src/rules/table.rs` row is
  replaced by
  `exempt code src/rules/table.rs # one const RULES: &[Rule] data literal (269 Rule entries); Rule is not Copy, so the slice cannot be assembled from several files without new code`.
- `tools/loc-census/census.py` `EXEMPT` table: the same path, kind `code` and reason.
- Stale line cites (§2): `language/08_error-model.md:150` `[[src/rules/table.rs:312]] [[src/rules/table.rs:318]]`
  → `[[src/rules/table.rs:TYPE_PROPAGATE_REQUIRES_TRAP]] [[src/rules/table.rs:TYPE_FAIL_REQUIRES_ERROR]]`;
  `language/04_types.md:413` `:480` `:486` → `TYPE_READ_ONLY_RECORD_CONSTRUCTOR`,
  `TYPE_READ_ONLY_RECORD_UPDATE`; `language/04_types.md:212` `:642` →
  `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION`.

The runtime message in `src/rules/mod.rs:436` naming `src/rules/table.rs` and the comment in
`src/manifest/config.rs:165` stay valid.

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

- [ ] Split `src/hir/mod.rs` → `mod.rs` + `nodes.rs` per §4.3; repoint 2 citations.
- [ ] Split `src/ast/serialize.rs` per §4.2; repoint the 3 comments.
- [ ] Split `src/ast/expr.rs` per §4.1 (9 widenings); repoint 12 citations,
      `.ai/codegen-invariants.md:85,836` (the `ast/expr` half), the comments in
      `src/operators.rs`, `src/types.rs`, `src/ast/tests/`.
- [ ] G5.

Acceptance: parsing, AST JSON and HIR are byte-identical.
  Check: G1; `cargo test -p mfb --bin mfb ast:: hir::` → 0 failed; G2 for `general`,
  `strings`, `collections` (their `.ast`/`.ir` goldens exercise the serializer and the
  expression and type-name parser); `cargo test --test no_type_strings` → passes with
  `src/hir/mod.rs` still the boundary; G3; G4; G6 (est. 15 min).
Commit: —

### Phase 2 — Lexer and the rules table

- [ ] Split `src/lexer.rs` per §4.4; repoint 15 citations including the file cite and the two
      stale line cites.
- [ ] `tests/guards/no_type_strings.rs` `is_excluded_from_scan` + its doc bullet (§4.4,
      justification there); `.ai/codegen-invariants.md:54`.
- [ ] `src/rules/table.rs` exemption: ceiling file row and `tools/loc-census/census.py`
      `EXEMPT` entry (§4.8).
- [ ] The five stale `[[src/rules/table.rs:<line>]]` cites → rule-name cites (§4.8).
- [ ] G5.

Acceptance: tokens are byte-identical, the lexer is still outside the type-string scan, and the
rules table is exempt by hand-reviewed reason.
  Check: G1; `cargo test -p mfb --bin mfb lexer:: rules::` → 0 failed; G2 for `general`,
  `strings`, `money` (literal, escape and number lexing); `grep -rnE '\[\[src/(lexer\.rs|rules/table\.rs:[0-9])' src/docs/spec`
  → 0; `python3 -I tools/loc-census/census.py --seed | grep 'src/rules/table.rs'` → the
  `exempt code` row; `cargo test --test file_size_budget 2>&1 | grep 'src/rules/table.rs'` → no
  row; G3; G4; G6 (est. 12 min).
Commit: —

### Phase 3 — Resolver

- [ ] Split `src/resolver/resolution.rs` per §4.5 (2 widenings); repoint 27 citations and the
      dangling `resolve_type_name` cite; the `no_type_strings.rs:422` comment.
- [ ] G5.

Acceptance: name resolution and its diagnostics are byte-identical.
  Check: G1; `cargo test -p mfb --bin mfb resolver::` → 0 failed; G2 for `general`,
  `collections`; resolver diagnostics are pinned by the `build.log` goldens of the invalid
  fixtures, which the artifact gate does not diff:
  `bash scripts/test-accept.sh target/release/mfb /tmp/m_accept_p3 '*invalid*'` → no golden
  diff (est. 12 min; the invalid fixtures are the only goldens that pin resolver diagnostics);
  G3; G4; G6.
Commit: —

### Phase 4 — Monomorphizer

- [ ] Split `src/monomorph/lower.rs` per §4.6 (five impl blocks, 1 widening, 23
      `pub(super)`); repoint 31 citations and `.ai/codegen-invariants.md:808`.
- [ ] G5.

Acceptance: instantiation and overload selection are byte-identical.
  Check: G1; `cargo test -p mfb --bin mfb monomorph::` → 0 failed; G2 for `collections`,
  `general`, `thread` (generic collections, overloads and the thread plane drive the moved
  resolve/instantiate code); G3; G4; G6 (est. 15 min).
Commit: —

### Phase 5 — Builtin registry (largest blast radius)

- [ ] Split `src/codegen/registry/mod.rs` into `mod.rs` + the 8 parts per §4.7 (the H-left
      cfg(test) helpers go to `unify.rs`).
- [ ] Repoint the 30 moved-symbol citations and the dangling `call_return_type` cite.
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
  their phase.
- Coverage check: pure motion inside `src/` — the moved lines keep their coverage counts; the
  denominators move between files. No coverage exception names an M file
  (`grep -nE 'src/(ast|hir|lexer|monomorph|resolver|rules|codegen/registry)' scripts/coverage-exceptions.txt`
  → 0).
- Runtime proof / final gate (once, after Phase 5): `cargo build --release` and
  `sh scripts/artifact-gate.sh target/release/mfb all` → no diff (est. 15–20 min; code motion
  in the front end and the registry can change emission for any fixture on any target, and
  only the full gate covers every fixture on every target). Diagnostics from the lexer,
  parser and resolver are pinned by `build.log` goldens the artifact gate does not diff:
  `bash scripts/test-accept.sh target/release/mfb /tmp/m_accept '*invalid*'` → no golden diff
  (est. 12 min). Then G1, G3, G4 and
  `cargo test --test file_size_budget` → banner 0 rows (est. 8 min). A diff is root-caused on
  one fixture and fixed; the gate is rerun. The full workspace suite runs once, in
  plan-176-V.
- Doc sync: spec citations listed in §4 (path-only changes plus the 8 stale/dangling
  repairs); `.ai/codegen-invariants.md:54,85,808,836`. No observable contract changes.

## Decisions

- Rule: code ≤ 1000, test ≤ 250 non-comment non-blank LOC; scope `.rs`/`.mfb` under `src/`
  `packages/` `examples/` `repository/` `tests/` — owner, 2026-10-14.
- Inline test code counts against the 250 test budget — owner, 2026-10-14. Inline modules
  under 250 stay inline: `ast/expr` (155), `ast/serialize` (51), `hir` (192),
  `registry::qualification_tests` (79); `unify.rs` carries 117 cfg(test) LOC.
- Generated files exempt; the guard warns, never fails; a new module is always an acceptable
  split — owner, 2026-10-14.
- A single function over the code limit is decomposed by extract-method gated by byte identity
  — owner, 2026-10-14. No M function is over 1000 (largest: `lower_expression` 381), so M does
  no extraction.
- `src/rules/table.rs` is exempt (code kind): one `const RULES: &[Rule]` data literal; `Rule`
  is not `Copy`; no concatenation without new code. It is data, not a function — author,
  2026-10-14 (brief).
- Guards that classify by file name are updated in the same commit as the move, with the
  4-point justification — author, 2026-10-14 (`no_type_strings` lexer clause, §4.4).
- Pre-existing defects found by the map are fixed here: the two lexer line cites, five rules
  line cites, `resolve_type_name`, `call_return_type` — author, 2026-10-14.
- Byte identity is the gate for compiler code motion; the full artifact gate runs once at the
  end — brief, 2026-10-14.
- Glob-cycle imports compile without warnings — verified (`/tmp/loc/probe2`).

## Corrections

## Summary

M is code motion through the compiler front end and the registry, proven by byte-identical
artifacts. The real risks are the visibility depth trap (12 methods that must become
`pub(in crate::<parent>)`, not `pub(super)`), the lexer's exact-path guard exemption, and the
registry split that every builtin call passes through — which is why it lands last, behind the
full artifact gate. `src/rules/table.rs` stays one file with a reviewed `exempt` row, and eight
stale citations get real symbols on the way.
