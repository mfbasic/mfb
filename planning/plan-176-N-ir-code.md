# plan-176-N: IR code splits

Last updated: 2026-10-18
Effort: large (3h–1d)
Depends on: plan-176-M

N splits the five over-limit code files of `src/ir` into module directories by responsibility:
`src/ir/lower.rs` (5140 code LOC), `src/ir/shape.rs` (2933), `src/ir/binary.rs` (1448),
`src/ir/verify/mod.rs` (1203) and `src/ir/verify/resources.rs` (1153). Every split is pure code
motion: an item moves verbatim to a part file, gains at most a visibility widening, and the
module paths callers use stay the same through re-exports. Nothing the compiler emits changes.

The single outcome: after N lands, `cargo test --test file_size_budget` shows no row for any of
the five files or the files they become, and `tests/guards/file_size_ceilings.txt` has no
`ceiling` row for any of them. There are no exemptions (rule revised 2026-10-18).

References:

- `planning/plan-176-A-file-size-guard.md` — the rule, the guard, the ceiling ratchet, and
  §Decisions. Its "Splitting an over-budget file" conventions (written into
  `.ai/build-tooling.md` by A Phase 3) apply here.
- `.ai/testing-gates.md` § "Splits must sweep man AND spec citations" — `spec_citations_resolve`
  is file-level only.
- `.ai/codegen-invariants.md` (desugars, diagnostics; lines 64 and 87–88 name `ir/shape.rs` and
  `ir/binary.rs`), `.ai/resources-packages.md` (lines 28, 41, 215, 254, 439, 441 name the IR
  files), `.ai/collections.md` (lines 464, 608, 645), `.ai/compiler.md` (line 21),
  `.ai/build-tooling.md` (rustfmt/clippy policy).
- Spec topics whose citations move (`grep -rlE "\[\[src/ir/(lower|shape|binary|verify/mod|verify/resources)\.rs" src/docs/spec`
  → 38 files): `architecture/{02_frontend,03_packages,04_ir,05_binary-representation,08_artifacts,09_modules,12_monomorphization,19_internal-naming,20_ir-json-artifact,22_type-inference,23_escape-analysis,spec}.md`,
  `diagnostics/02_error-codes.md`,
  `language/{04_types,05_bindings-and-scope,06_functions,08_error-model,09_pattern-matching,10_control-flow,12_collections,13_modules-and-packages,14_memory-semantics,15_resource-management,16_threads,17_native-libraries}.md`,
  `package/{04_type-table,08_ir-section,09_resource-regions,10_native-bindings,12_verifier-rules,13_example-layout,14_compact-summary}.md`,
  `stdlib/{14_os,19_big}.md`, `threading/{01_source-model,02_isolation,03_package-requirements,06_thread-runtime-helpers}.md`.
- Split map: `/tmp/loc/maps/N.md` (measured on the working tree of `17a9b5799`); its content is
  restated below in decided form.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-M is archived | `ls planning/plan-176-M-* planning/completed/plan-176-M-*` → only the `planning/completed/` path | MET (2026-10-19) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | MET (2026-10-19) |
| N's files have no uncommitted changes | `git status --porcelain -- src/ir/lower.rs src/ir/shape.rs src/ir/binary.rs src/ir/verify/mod.rs src/ir/verify/resources.rs tests/guards/no_type_strings.rs tests/guards/no_operator_strings.rs .ai src/docs/spec` → empty | MET (2026-10-19: 0 lines) |

Everything below is written against the world where these hold. plan-176-G (an earlier letter)
has already moved the inline test modules this letter's files carried over the 250 inline
limit: `src/ir/shape.rs`'s `#[cfg(test)] mod tests` (1231 inline LOC, over the 1000 test-only
limit once moved, so G splits it into parts) is `#[cfg(test)] mod tests;` resolving to
`src/ir/shape/tests/`, and `src/ir/tests.rs` (6386, a test-only file over 1000) is
`src/ir/tests/`. G replaced only the trailing
test block, so every line range below 3761 in `shape.rs` is unchanged by G.

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

- Each of the five files becomes a directory module (`foo.rs` → `foo/mod.rs` + parts;
  `verify/mod.rs` gains one sibling part `verify/env.rs`), every resulting file ≤ 1000 code LOC
  and ≤ 250 inline test LOC.
- `cargo test --test file_size_budget 2>&1 | grep -E 'src/ir/(lower|shape|binary|verify)'` → no row,
  and `grep -E 'src/ir/(lower|shape|binary)\.rs|src/ir/verify/(mod|resources)\.rs' tests/guards/file_size_ceilings.txt`
  → no `ceiling` row.
- `sh scripts/artifact-gate.sh target/release/mfb all` → 0 diffs (byte-identical `.ast`/`.ir` and
  every native dump on every target).

### Non-goals (explicit constraints)

- No behavior change: no diagnostic, `.ir`, `.mfp` or native byte changes.
- No logic edit. The only edits besides motion are: visibility widenings, `use`/`mod`/re-export
  lines, three `super::` path rewrites per the hazards in §4, and comment/doc-link path fixes.
- No public or crate path a caller uses changes: everything `src/ir/mod.rs` re-exports from
  `lower` and `binary` stays reachable at the same path; `ir::shape::collect_diagnostics`,
  `ir::verify::resources::Unsendable`, `ir::binary::value_op_tests` keep their paths.
- No `[[test]]` target renamed; no test fn renamed or moved (inline test modules stay where
  they are: `binary`'s two test modules and `lower`'s two `#[cfg(test)]` wrapper fns stay in the
  new `mod.rs`).

## 2. Current State

| File | Kind | Code LOC | Test LOC | Inline `#[test]` fns |
|---|---|---|---|---|
| `src/ir/lower.rs` | code | 5140 | 57 | 0 |
| `src/ir/shape.rs` | code | 2933 | 1228 at planning; 23 after plan-176-G | 57 at planning; 0 after G |
| `src/ir/binary.rs` | code | 1448 | 117 | 4 |
| `src/ir/verify/mod.rs` | code | 1203 | 2 | 0 |
| `src/ir/verify/resources.rs` | code | 1153 | 0 | 0 |

LOC: `python3 -I /tmp/loc/census.py` on the working tree of `17a9b5799` (row values read from
`/tmp/loc/census2.json`); after plan-176-A the same numbers come from
`python3 -I tools/loc-census/census.py --file <path>`. Test fns:
`grep -cE '#\[(tokio::)?test\]' <file>` → 0, 57, 4, 0, 0. Unit tests that exercise this code:
`src/ir/verify/tests.rs` (492 `#[test]`, moved to `src/ir/verify/tests/` by G), the shape tests
(57, in `src/ir/shape/tests/` after G), `src/ir/tests.rs` and `src/ir/variant_corpus_tests.rs`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Spec citations of `src/ir/lower.rs` | 76 | `grep -rnoE "\[\[src/ir/lower\.rs(:[^]]+)?\]\]" src/docs/spec \| wc -l` |
| … of `src/ir/shape.rs` | 23 | same pattern, `shape` |
| … of `src/ir/binary.rs` | 40 | same pattern, `binary` |
| … of `src/ir/verify/mod.rs` | 10 (8 with a symbol, 2 bare) | same pattern, `verify/mod` |
| … of `src/ir/verify/resources.rs` | 27 | same pattern, `verify/resources` |
| `mfb man` citations of these files | 0 | same pattern over `src/docs/man` |
| Non-citation mentions of these paths in `src/docs` | 0 | `grep -rnE "(^\|[^a-z_/])(src/)?ir/lower\.rs" src/docs \| grep -v '\[\[src/ir/lower.rs'` (and per file) → empty |
| Guard rows naming these paths | 14 in `no_type_strings.rs` (12 `TYPE_KEYED_TABLES` rows, 1 `BOUNDARY_FILES` row, 1 closed-list entry) + 1 self-test literal; plan-176-E moves the closed list and the self-test to `tests/guards/no_type_strings/self_checks.rs` | `grep -n '"src/ir/\(lower\|shape\|binary\|verify/mod\)\.rs"' tests/guards/no_type_strings.rs` |
| `scripts/coverage-exceptions.txt` entries for these paths | 0 | `grep -n "ir/lower\|ir/shape\|ir/binary\|ir/verify" scripts/coverage-exceptions.txt` → only a comment naming `verify/link.rs` |
| `include_str!`/`include_bytes!`/`#[path]` in these files | 0 | `grep -nE "include_str\|include_bytes\|#\[path" <file>` |
| Host-`cfg` gating (`target_os`, `windows`, `unix`) in these files | 0 | `grep -cE "#\[cfg\((target_os\|windows\|unix\|target_family)" <file>` → 0 each |

### Verified properties

- **A parent `use` consumed only by a child through `use super::*;` does not warn.** Probe
  `/tmp/locN-probe` (parent `use std::collections::HashMap;` used only in `part.rs`):
  `cargo build` → no warning; `cargo run` printed `3`. So `mod.rs` keeps every original `use` line
  and each part file starts with `use super::*;`.
- **Glob-cycle imports compile without warnings** (`use super::*;` in parts, `use part::*;` in
  `mod.rs`): probe `/tmp/loc/probe2` printed `6` with no warning.
- **No name collision when `lower` parts are glob-imported.** Every item moving out of
  `lower.rs` (ranges in §4.1) was intersected with every name `src/ir/mod.rs` defines or
  imports (`lower.rs` starts `use super::*;`): intersection empty (python scan of both files,
  2026-10-14). A part item reaching `lower/mod.rs` through `use part::*;` therefore never
  ambiguates with the `use super::*;` glob of `ir`.
- **Moving Rust code cannot change compiler output through source locations**
  (plan-176-A §Verified properties: no `module_path!`/`file!`/`line!` in `src`, no link-order
  registration). Byte-identity is therefore the gate.
- **The artifact gate does not compare `build.log` or `.mfp`.** `scripts/artifact-gate.sh`
  compares only `ARTIFACT_HOST_KINDS="ast ir hex"` and the native kinds (`scripts/artifact-kinds.sh`).
  Diagnostics (shape, verify) are proven by the unit tests plus
  `scripts/test-accept.sh … '*-invalid'` (206 fixture dirs: `find tests -type d -name "*-invalid" | wc -l`),
  which compares `build.log`. The IR wire codec (`binary.rs`) is proven by `test-accept.sh` over
  the 18 fixtures that carry an `.mfp`/`.info` golden
  (`find tests -path "*golden/*.mfp" -o -path "*golden/*.info" | sed 's|/golden/.*||' | sort -u`).
- **`string_keyed_type_maps` is file-scoped and its rows must name a file that contains the
  identifier.** `curated_type_keyed_tables_all_exist` reads each row's file and fails if the
  identifier is absent from its test-free lines; `string_keyed_type_maps` only scans a file when
  `*file == rel`. Rows must follow the declarations they watch.
- **`BOUNDARY_FILES` and `boundary_list_is_closed` are exact-path and ordered.** `is_boundary_file`
  is `*p == rel`; `boundary_list_is_closed` `assert_eq!`s the full ordered vector.
- **No cfg-gated host code**, so no `cargo check --target x86_64-pc-windows-msvc` and no box-2223
  check is needed for N (table row above).

## 3. Design Overview

Conventions (plan-176-A, `.ai/build-tooling.md` "Splitting an over-budget file"):

- `foo.rs` → `foo/mod.rs` + parts. `mod.rs` keeps its original `use` lines, the types whose
  private fields are read across parts, the items outside callers name, and the inline test
  modules. Each part file starts `use super::*;`.
- A part item used from `mod.rs` or a sibling gets `pub(super)`. A part item that today is
  `pub(super)` (reaching the grandparent) or is re-exported to the grandparent is widened to
  `pub(in <grandparent path>)` — the exact reach it has today.
- Free functions: `mod.rs` adds a private `use <part>::*;` for every part that defines a
  `pub(super)` free item, so siblings reach it through their `use super::*;`. Methods split as
  several `impl X { … }` blocks and need no import.
- Line ranges are lines of the file **as of 17a9b5799**. Item NAMES are authoritative if lines
  drift; every range begins on the blank line after the previous item, so leading doc comments
  and attributes travel with their item.

**Gate class: byte-identity.** The work is provably-neutral code motion. Per phase: `cargo build
--release`, the scoped artifact gate for the selectors whose code moved, the moved code's unit
tests, the `build.log` and `.mfp` checks where §2 says the artifact gate is blind, `cargo check
--all-targets` (no new warnings), `spec_citations_resolve`, the guards, and the tighten step.
Letter end, once: `sh scripts/artifact-gate.sh target/release/mfb all`. A diff is never a
premise-falsified stop: objdump/diff ONE fixture, find the moved item that changed (a dropped
attribute, a changed path resolving to a different item), fix it, re-run.

Risk concentrates in `lower.rs` (Phase 5): 11 new files, 58 visibility edits, two `super::` path
rewrites, 76 citations, and the code that produces every `.ir`. It lands last. The verify
splits land first: two small files whose methods are already `pub(super)`.

Rejected alternatives:

- *Splitting `lower_expression_unwrapped` (741 LOC) or `lower_statement_inner` (503)* — not
  needed; no single item exceeds 900, and extract-method is reserved for functions over the
  1000 limit (letters S–V).
- *`pub(crate)` for the widened `verify::resources` methods* — wider than today's reach;
  `pub(in crate::ir::verify)` is exactly today's reach.
- *Leaving `TypeIndex` in a part* — its fields are watched by `TYPE_KEYED_TABLES`; keeping it in
  `lower/mod.rs` keeps one guard path for all five rows.
- *Naming the `binary` parts `link.rs`/`ops.rs`/`values.rs`* — `use super::*;` brings the
  `ir::link` module into scope; the `_codec` suffix avoids reading confusion.

## 4. Detailed Design

### 4.1 `src/ir/lower.rs` → `src/ir/lower/` (code 5140 + 57 test)

`src/ir/lower/` does not exist (`ls src/ir`); `src/ir/mod.rs:51` is `mod lower;` (no `#[path]`),
so `lower/mod.rs` resolves unchanged; sibling `src/ir/lower_link.rs` stays and
`super::lower_link` from `lower/mod.rs` still resolves. Sum of the ranges below = 5197 = 5140 + 57
(`/tmp/locNO/sumr.py src/ir/lower.rs …`).

| New file | LOC | Lines (as of 17a9b5799) | Items |
|---|---|---|---|
| `lower/mod.rs` | 533 (incl. 57 test) + mod/use lines | 1-513, 889-930, 1014-1019, 6638-6936 | imports, `LowerContext`, `Imported*`, test wrappers `lower_project_with_external_functions` / `lower_monomorphized_project`, `lower_augmented_project`, `LowerFacts` + impl, `lower_facts`; `RecoverTarget`, `CallParam`, `CallDefault`, `DefaultKind`; `CapturedLocal`; `TypeIndex` + both impls, `expanded_union_variants`, `write_ir` |
| `lower/decls.rs` | 363 | 514-888, 931-1013 | `lower_type`, `lower_binding(s)`, `lower_field`, `lower_variant`, `lower_enum_member`, `function_return_type`, `lower_function`, `lower_function_body`, `lower_param`, `lower_parameter_default`, `lower_link_defaults`, `statement_loc`, `default_kind`, `call_default`, `lower_call_default` |
| `lower/statement.rs` | 695 | 1020-1655, 3371-3607 | `lower_statement`, `lower_statement_inner`, `lower_statement_block`, `collection_iteration_type`, `strip_res`, `attributed_string_type`, `make_temp_local_name`, `numeric_constant_for_type`, `lower_match_case`, `match_case_binding`, `lower_match_expression`, `match_expression_type` |
| `lower/inline_trap.rs` | 792 | 1656-2448, 3083-3370 | `InlineTrapTarget`, `ops_read_local`, `lower_inline_trap`, `wrap_trap_slot_value`, `TrapHoist` + impl, `ConditionalStep`, `wrap_trap_hoists`, `is_conditional_root`, `TRAP_SCRUTINEE_MAX_DEPTH`, `is_short_circuit_operator`, `treeify_handler`, `distribute_continuation`, `treeify_statement`, `block_terminates`, `block_can_terminate`, `statement_can_terminate`, `statement_terminates` |
| `lower/trap_hoist.rs` | 522 | 2449-3082 | `hoist_trap_calls`, `scan_trap_operands`, `trap_hoist_kind`, `scan_trap_call`, `ir_call_arg_types`, `ir_operand_type`, `rewrite_trap_operands`, `rewrite_trap_call` |
| `lower/concat_fold.rs` | 303 | 3608-4021 | `ConcatReducer`, `hir_mentions`, `concat_reducer_shape`, `concat_reducers`, `ir_value_is_effect_free`, `hir_rename`, `FoldSite`, `take_concat_fold`, `rewrite_concat_folds` |
| `lower/typing.rs` | 520 | 4022-4725 | `function_returns`, `function_types`, `function_params`, `link_params`, `top_level_binding_type`, `lambda_param_type`, `declared_binding_types`, `native_type`, `infer_binding_types`, `expression_type`, `function_return_from_type`, `function_param_types_from_type`, `thread_resource_plane_target`, `qualified_imported_enum`, `canonical_import_type`, `generic_builtin_call_type`, `canonical_import_name`, `canonical_import_name_in` |
| `lower/call_args.rs` | 222 | 4726-4986 | `resolved_generic_argument_type`, `call_argument_expected_type`, `normalize_builtin_call_arguments`, `normalize_overloaded_builtin_call_arguments`, `normalize_local_call_arguments`, `lower_local_call_arguments`, `call_arg_value` |
| `lower/expression.rs` | 809 | 4987-6126 | `lower_expression`, `filter_predicate_arg_type`, `registry_record_constant`, `builtin_predicate_ref_type`, `lower_expression_with_expected`, `lower_expression_unwrapped` (741, the largest item) |
| `lower/construct.rs` | 267 | 6127-6450 | `error_loc_value`, `build_error_value`, `wrap_union_value`, `bare_variant`, `bare_payload_state`, `union_conversion`, `union_converter`, `lower_constructor_args`, `constructor_arg_value` |
| `lower/captures.rs` | 171 | 6451-6637 | `lambda_captured_names`, `captured_locals`, `collect_captured_locals`, `literal_expression_type` |

Surface and visibility:

- Everything `src/ir/mod.rs:89-96` re-exports from `lower` stays in `mod.rs`.
- `mod.rs` declares the ten parts and adds `use call_args::*; use captures::*; use concat_fold::*;
  use construct::*; use decls::*; use expression::*; use inline_trap::*; use statement::*;
  use trap_hoist::*; use typing::*;`.
- `mod.rs` adds re-exports for items outside `lower` names as `lower::X` (`grep -rnE "lower::" src/ir`:
  `fallible.rs:58`, `resource_hoist.rs:31`, `shape.rs` ×14, `resource_escape.rs:468`, the G-moved
  `named_argument_normalization.rs` `use super::super::super::lower::{default_kind, DefaultKind}`):
  `pub(super) use decls::{default_kind, function_return_type};`
  `pub(super) use statement::{collection_iteration_type, make_temp_local_name, match_case_binding, match_expression_type};`
  `pub(super) use typing::{expression_type, lambda_param_type};`
  `pub(crate) use captures::lambda_captured_names;`
- 8 items widen `pub(super)` → `pub(in crate::ir)` (re-exported to `ir`): `function_return_type`,
  `default_kind`, `collection_iteration_type`, `make_temp_local_name`, `match_case_binding`,
  `match_expression_type`, `lambda_param_type`, `expression_type`.
- 50 private items become `pub(super)` (used from another part): decls 8 (`lower_type`,
  `lower_bindings`, `lower_field`, `lower_function`, `lower_link_defaults`, `statement_loc`,
  `call_default`, `lower_call_default`); statement 3 (`lower_statement`, `lower_statement_block`,
  `attributed_string_type`); inline_trap 6 (`InlineTrapTarget`, `lower_inline_trap`, `TrapHoist`
  + its 6 fields, `ConditionalStep` + its 3 fields, `is_conditional_root`,
  `TRAP_SCRUTINEE_MAX_DEPTH`); trap_hoist 3 (`hoist_trap_calls`, `trap_hoist_kind`,
  `ir_call_arg_types`); concat_fold 2 (`concat_reducers`, `rewrite_concat_folds`); typing 12
  (`function_returns`, `function_types`, `function_params`, `link_params`,
  `declared_binding_types`, `infer_binding_types`, `function_param_types_from_type`,
  `thread_resource_plane_target`, `qualified_imported_enum`, `canonical_import_type`,
  `canonical_import_name`, `canonical_import_name_in`); call_args 4
  (`call_argument_expected_type`, `normalize_builtin_call_arguments`, `lower_local_call_arguments`,
  `call_arg_value`); expression 4 (`lower_expression`, `filter_predicate_arg_type`,
  `registry_record_constant`, `lower_expression_with_expected`); construct 6
  (`build_error_value`, `wrap_union_value`, `bare_variant`, `union_conversion`,
  `lower_constructor_args`, `constructor_arg_value`); captures 2 (`captured_locals`,
  `literal_expression_type`). `is_short_circuit_operator`, `top_level_binding_type` and
  `ConcatReducer` are already `pub(super)`.
- The two `#[cfg(test)]` wrapper fns stay in `mod.rs` (57 test LOC).

Hazards:

- Required path rewrites (`grep -n "super::value::VALUE_VISIT\|super::resource_hoist" src/ir/lower.rs`):
  old 661 (→ `decls.rs`, in `lower_function`) `super::resource_hoist::hoist_resource_temporaries`
  → `crate::ir::resource_hoist::hoist_resource_temporaries`; old 2437 (→ `inline_trap.rs`)
  `super::value::VALUE_VISIT_MAX_DEPTH` → `crate::ir::value::VALUE_VISIT_MAX_DEPTH`, and the doc
  link at old 2434 likewise.
- Stale in-code line reference (a pre-existing defect, fixed here): the `TypeIndex` doc comment
  at old 6843 cites `` (`ir/verify/mod.rs:1002`) `` for the verifier's preference of the annotated
  `MemberAccess` type. Line 1002 is inside `TypeEnv::build`
  (`awk 'NR<=1002 && /fn [a-z_]+/ {l=NR": "$0}; NR==1002{print l}' src/ir/verify/mod.rs` →
  `900: pub(super) fn build`); the preference is in `infer_type_depth`'s `MemberAccess` arm
  (`grep -n "Prefer the annotated member type" src/ir/verify/mod.rs`). Rewrite to
  `` (`ir/verify/env.rs:infer_type_depth`) ``.
- Spec citations: 76, all rewritten (the path vanishes). → `lower/mod.rs`: `LowerContext`,
  `recover_targets` (field), `RecoverTarget`, `CapturedLocal`, `lower_augmented_project`,
  `lower_facts`, `lower_project_with_external_functions` ×2, `write_ir`, and the bare
  `[[src/ir/lower.rs]]` (`threading/06_thread-runtime-helpers.md:80`) → `[[src/ir/lower/mod.rs]]`.
  → `decls.rs`: `default_kind`, `function_return_type`, `lower_call_default`, `lower_function`,
  `lower_link_defaults`, `lower_param`, `lower_parameter_default`. → `statement.rs`:
  `lower_statement` ×7, `lower_statement_inner`, `collection_iteration_type` ×2,
  `lower_match_case` ×2, `lower_match_expression`. → `inline_trap.rs`: `InlineTrapTarget`,
  `lower_inline_trap` ×3, `wrap_trap_hoists` ×2, `wrap_trap_slot_value`, `treeify_handler`,
  `distribute_continuation`. → `trap_hoist.rs`: `trap_hoist_kind`. → `typing.rs`:
  `expression_type` ×5, `function_types` ×3, `lambda_param_type` ×2, `top_level_binding_type` ×2.
  → `call_args.rs`: `call_argument_expected_type`, `normalize_local_call_arguments`.
  → `expression.rs`: `lower_expression_with_expected` ×10, `builtin_predicate_ref_type`.
  → `construct.rs`: `build_error_value`, `error_loc_value`, `wrap_union_value`,
  `union_conversion` ×3, `union_converter`, `lower_constructor_args`. → `captures.rs`:
  `lambda_captured_names`, `captured_locals`, `literal_expression_type`.
- Guard `tests/guards/no_type_strings.rs`: `TYPE_KEYED_TABLES` rows `("src/ir/lower.rs", records|enums|variants|variant_unions|variant_fields)`
  (the `TypeIndex` fields, `grep -nE "\b(records|enums|variants|variant_unions|variant_fields)\s*:\s*HashMap<" src/ir/lower.rs`
  → 6646-6651, all in `mod.rs`) → `src/ir/lower/mod.rs`. The self-test in
  `tests/guards/no_type_strings/self_checks.rs` (`scanners_fire_on_their_own_needles`, moved there by plan-176-E) `string_keyed_type_maps` cases, `string_keyed_type_maps("src/ir/lower.rs", "    binding_types: HashMap<String, X>,")`,
  → `"src/ir/lower/mod.rs"`. Justification (AGENTS.md test rule): (1) written in `9174857b8`
  (plan-111-A Phase 1, the no-type-strings ratchet), rows extended in `4e67183c5` (plan-111-B);
  (2) protects: an identifier NOT on the curated list is not scanned even inside a listed file;
  (3) dependents: only this guard (`grep -rn "string_keyed_type_maps" tests src` → this file);
  (4) proof: after the move no row names `src/ir/lower.rs`, so the assertion would pass through
  the file mismatch instead of the identifier mismatch it was written to pin — the listed file is
  now `src/ir/lower/mod.rs`, the same code at a new path. Prose lines 374, 398, 972, 993 →
  `ir/lower/` wording (`ir/lower.rs:TypeIndex::new` at 993 → `ir/lower/mod.rs:TypeIndex::new`).
- `.ai` prose: `.ai/collections.md:464` "a post-pass over the ops `lower_statement` produces
  (`src/ir/lower.rs`)" → "(`rewrite_concat_folds`, `src/ir/lower/concat_fold.rs`) over the ops
  `lower_statement` (`src/ir/lower/statement.rs`) produces"; `.ai/resources-packages.md:28`
  `ir/lower.rs::match_case_binding` → `ir/lower/statement.rs::match_case_binding`;
  `.ai/resources-packages.md:439` "`src/ir/lower.rs` by calling `builtins::default_argument_padding`"
  → "`lower_expression_unwrapped` (`src/ir/lower/expression.rs`) by calling
  `registry::default_argument_padding`" (stale symbol: the call is
  `crate::codegen::registry::default_argument_padding`, `grep -n default_argument_padding src/ir/lower.rs` → 5421);
  `:441` "lower.rs builds" → "`lower_expression_unwrapped` builds".
- Stale `.ai` symbol (pre-existing, fixed here): `.ai/resources-packages.md:254` says a
  type-overloaded source member "dispatches via `source_implementation_name(name, arg_types)` in
  its own `.or_else` in `ir/lower.rs`". `source_implementation_name` no longer exists
  (`grep -rn "source_implementation_name(" src` → none; deleted by `4ff9f1d99`, "audio: migrate
  onto the clean-room registry"). Today the dispatch is
  `crate::codegen::registry::rewrite_target(&canonical_callee, &arg_types)` in
  `lower_expression_unwrapped` (old 5636-5637). Rewrite the sentence to name
  `registry::rewrite_target` in `src/ir/lower/expression.rs`.
- No `scripts/coverage-exceptions.txt` entry; no respawn test; no `module_path!`.

### 4.2 `src/ir/shape.rs` → `src/ir/shape/` (code 2933)

`src/ir/shape/` exists after G holding only `tests/`. The 2462-LOC `impl<'a> Walker<'a>`
(746-3760) splits into one `impl<'a> Walker<'a> { … }` block per part (pure motion). Each part =
`use super::*;` + `impl<'a> Walker<'a> {` + the method range + `}`.

| New file | LOC | Lines | Items |
|---|---|---|---|
| `shape/mod.rs` | 492 + 5 `mod` lines + 23 test | 1-745, and G's `#[cfg(test)] mod tests;` | imports, `collect_diagnostics`, `check_project`, `CalleeParams`, `Walker` struct (fields stay here), `DeclaredFunction`, `ShapeParam`, `export_in_executable_diagnostics`, `export_names_non_exported_type_diagnostics`, `TypeShape`, `ShapeKinds` + `TypeKinds` impl, `Flow`, the free helpers (`strip_res` … `source_order`) |
| `shape/declarations.rs` | 595 | 747-1461 | `new`, `walk_project`, `check_imported_packages`, `validate_package_type`, `walk_file`, `check_stray_state`, `check_stray_param_state`, `walk_function`, `walk_link` |
| `shape/statements.rs` | 475 | 1462-2046 | `walk_block`, `walk_block_flow`, `walk_statements_flow`, `statement_flow`, `block_flow`, `match_covered`, `walk_statement`, `walk_value`, `check_foreign_record_field`, `walk_handler`, `walk_match_case` |
| `shape/expressions.rs` | 592 | 2047-2768 | `walk_expression`, `callee_params`, `canonical_callee`, `thread_start_entry_valid`, `report_thread_entry`, `check_builtin_call`, `type_of`, `check_expect_call`, `check_trap_guardable` |
| `shape/compat.rs` | 196 | 2769-3025 | `is_comparable`, `is_comparable_seen`, `is_resource_type`, `compatible`, `expression_compatible` |
| `shape/calls.rs` | 602 | 3026-3759 | `check_builtin_arity`, `check_call_shape`, `emit_call_typed_unknown`, `check_initializer_known`, `checker_types_unknown`, `constructor_typed`, `declared_record_constructible`, `with_update_typed`, `type_visible`, `checker_binds_pattern`, `check_config_call_literals`, `report_unknown_name`, `report_duplicate_name`, `bind`, `emit` |

(`span.py src/ir/shape.rs a b` per range; range sums include the 4 `#[cfg(test)]` attribute lines
span.py's whole-file mode counts as test, so they read at most 2 high.)

Surface and visibility: outside users call only `crate::ir::shape::collect_diagnostics`
(`src/testutil.rs` ×3) and `ir/mod.rs` uses `check_project`/`export_*` — all stay in `mod.rs`; no
re-exports. 30 `Walker` methods become `pub(super)` (visible to all of `shape`, including
`shape::tests`): declarations 4 (`new`, `walk_project`, `validate_package_type`,
`check_stray_param_state`); statements 4 (`walk_block`, `walk_statements_flow`,
`check_foreign_record_field`, `walk_handler`); expressions 8 (`walk_expression`, `callee_params`,
`canonical_callee`, `thread_start_entry_valid`, `report_thread_entry`, `check_builtin_call`,
`check_expect_call`, `type_of`); compat 4 (`is_comparable`, `is_resource_type`, `compatible`,
`expression_compatible`); calls 10 (`check_builtin_arity`, `check_call_shape`,
`emit_call_typed_unknown`, `check_initializer_known`, `checker_types_unknown`,
`declared_record_constructible`, `checker_binds_pattern`, `check_config_call_literals`, `bind`,
`emit`). Walker fields, free helpers and types stay in `mod.rs`. The `#[cfg(test)] bound_types`
field stays in `mod.rs`; its initializer (old 982) moves with `new` to `declarations.rs` and its
push (old 3735) with `bind` to `calls.rs`, unchanged.

Hazards:

- Required path rewrites, all in `declarations.rs` (`awk 'NR>745 && NR<3761 && /super::/' src/ir/shape.rs`):
  old 756 `super::lower_link::native_resource_types` → `super::super::lower_link::native_resource_types`;
  old 848, 862, 863, 864 `super::ImportedTypeKind::…` → `super::super::ImportedTypeKind::…`.
  (`lower::` inside the impl resolves through `mod.rs`'s `use super::lower::{self, …}` via the glob.)
- Spec citations: 23, all rewritten. → `shape/mod.rs`: `collect_diagnostics`,
  `export_names_non_exported_type_diagnostics`, `is_builtin_package_type` ×2, `is_config_literal`,
  bare `[[src/ir/shape.rs]]` ×2 (`architecture/02_frontend.md:263`, `architecture/09_modules.md:13`)
  → `[[src/ir/shape/mod.rs]]`. → `declarations.rs`: `check_stray_state`, `walk_link`.
  → `expressions.rs`: `check_builtin_call` ×2, `thread_start_entry_valid` ×4, `type_of`.
  → `compat.rs`: `compatible` ×3, `expression_compatible`. → `calls.rs`: `check_call_shape` ×2,
  `type_visible`.
- Guard `tests/guards/no_type_strings.rs`: rows 453-454 `("src/ir/shape.rs", "resource_types"|"types")`
  watch the `Walker` fields (old 126, 134) → `src/ir/shape/mod.rs`; ADD
  `("src/ir/shape/declarations.rs", "resource_types")` for the typed local at old 756 in `new`
  (`grep -nE "\b(resource_types|types)\s*:\s*(HashMap|HashSet)<" src/ir/shape.rs` → 126, 134, 756),
  so the scanned population is unchanged. Justification: (1) rows written in `9174857b8`
  (plan-111-A Phase 1); (2) protects: the shape checker's type registries stay keyed by
  `ParameterType`, not `String`; (3) dependents: `curated_type_keyed_tables_all_exist` and
  `string_keyed_type_maps` in the same file; (4) proof: the same declarations at new paths — the
  scan is file-scoped, so a row left on the vanished path would fail
  `curated_type_keyed_tables_all_exist` and an unlisted `declarations.rs` would silently drop the
  local from the scan. Line 101 prose `src/ir/shape.rs:158` `bound_types` →
  `src/ir/shape/mod.rs` `bound_types` (symbol, not line). Prose 378, 400, 423, 967 → `ir/shape/`.
- `.ai` prose: `.ai/codegen-invariants.md:64` `ir/shape.rs`'s `bound_types` → `ir/shape/mod.rs`'s;
  `.ai/resources-packages.md:41` `ir/shape.rs` `compatible` → `ir/shape/compat.rs` `compatible`,
  `check_call_shape` → in `ir/shape/calls.rs`, `ir/shape.rs check_builtin_call` →
  `ir/shape/expressions.rs check_builtin_call`; `.ai/compiler.md:21` `src/ir/shape.rs` →
  `src/ir/shape/`; `.ai/collections.md:645` "`ir/shape.rs`'s `validate_package_type` and its
  sibling `is_comparable_seen`" → "`ir/shape/`'s `validate_package_type` (`declarations.rs`) and
  `is_comparable_seen` (`compat.rs`)". `scripts/coverage-report.py:18` uses `ir/shape.rs` only as a
  suffix-matching example in a docstring; it stays (it illustrates matching, names no live file
  contract).

### 4.3 `src/ir/binary.rs` → `src/ir/binary/` (code 1448 + 117 test)

| New file | LOC | Lines | Items |
|---|---|---|---|
| `binary/mod.rs` | 481 + 3 `mod` + 2 `use` lines; 117 test | 1-316, 501-551, 794-993, 1659-1908 | magic/version consts, `put_*`, `MAX_DECODE_DEPTH`, `IrReader` + impl, `encode_binary_repr`, `decode_binary_repr`, `encode_project`; `decode_project`; `decode_vec`, binding/field/variant/type/param/function codecs, `decode_resource_owners`; merge banner comment, `verify_package`, `verify_ops`; `mod value_op_tests` (1768-1857) and `mod link_expr_op_tests` (1859-1908) unchanged |
| `binary/link_codec.rs` | 332 | 317-500, 552-793 | `encode_link_state_trailer`, `decode_link_state_trailer`, `encode_link_function`, `encode_opt_link_expr`, `encode_link_expr`, `MAX_CSTRUCTS`, `MAX_CSTRUCT_FIELDS`, `MAX_LINK_BUFFERS`, `decode_vec_capped`, `decode_cstructs`, `decode_link_function`, `decode_opt_link_expr`, `decode_link_expr`, `decode_link_expr_body` |
| `binary/op_codec.rs` | 312 | 994-1317 | `encode_op`, `decode_op`, `decode_op_body`, `decode_loop_kind`, match case/pattern codecs |
| `binary/value_codec.rs` | 323 | 1318-1658 | `encode_value`, `put_loc`, `get_loc`, `decode_value`, `decode_value_body` |

Surface and visibility: `BINARY_REPR_MAGIC`, `BINARY_REPR_VERSION`, `encode_binary_repr`,
`decode_binary_repr`, `verify_package` (`src/ir/mod.rs:71`, `variant_corpus_tests.rs:649,862-863`)
stay in `mod.rs`. 13 items become `pub(super)`: link_codec 7 (`encode_link_state_trailer`,
`decode_link_state_trailer`, `encode_link_function`, `decode_cstructs`, `decode_link_function`,
`encode_link_expr`, `decode_link_expr`); op_codec 2 (`encode_op`, `decode_op`); value_codec 4
(`encode_value`, `decode_value`, `put_loc`, `get_loc`). `mod.rs` adds the private
`use link_codec::{decode_cstructs, decode_link_function, decode_link_state_trailer, encode_link_function, encode_link_state_trailer};`
`use op_codec::{decode_op, encode_op};` `use value_codec::{decode_value, encode_value, get_loc, put_loc};`
(op_codec reaches value_codec's four through its `use super::*;`). Test modules stay in `mod.rs`,
so `binary::value_op_tests::…` (cited in prose in the G-moved verify tests) is unchanged;
`value_op_tests` gets `decode_value` via `mod.rs`'s import; `link_expr_op_tests` gets one added
line `use super::link_codec::{decode_link_expr, encode_link_expr};`. `IrReader` and its private
methods stay in `mod.rs`. `use crate::operators::{BinaryOp, UnaryOp};` stays in `mod.rs`.

Hazards:

- Relative paths: every `super::` use (old 122, 1689-1755) is in a range that stays in `mod.rs`
  (`awk '/super::/' src/ir/binary.rs`) → no rewrite.
- Spec citations: 40, all rewritten. → `binary/mod.rs`: `BINARY_REPR_VERSION` ×5,
  `decode_binary_repr` ×3, `decode_project` ×3, `encode_project` ×4, `encode_function` ×3,
  `decode_function`, `decode_resource_owners` ×2, `encode_binding`, `encode_type`,
  `verify_package`. → `link_codec.rs`: `MAX_LINK_BUFFERS`, `decode_cstructs`,
  `decode_link_function` ×2, `decode_link_state_trailer`, `encode_link_expr` ×2,
  `encode_link_function` ×2, `encode_link_state_trailer` ×2. → `op_codec.rs`: `encode_op` ×2.
  → `value_codec.rs`: `encode_value` ×2, `decode_value_body`.
- Guard (the gate fails without it): `tests/guards/no_type_strings.rs` `BOUNDARY_FILES` row
  `("src/ir/binary.rs", "IR wire/JSON decode reads types as text")` and the `"src/ir/binary.rs"`
  entry of `boundary_list_is_closed` (in `tests/guards/no_type_strings/self_checks.rs` after plan-176-E) → replaced, in the same position in both lists, by four rows
  `src/ir/binary/mod.rs`, `src/ir/binary/link_codec.rs`, `src/ir/binary/op_codec.rs`,
  `src/ir/binary/value_codec.rs` (same reason text). `ParameterType::parse(` sites land in all
  four (`grep -nE "ParameterType::parse\(" src/ir/binary.rs`: mod 506, 828, 849, 867, 912, 959;
  link_codec 607, 623, 642, 643, 663, 676; op_codec 1165, 1228, 1240; value_codec 1530-1649).
  Justification: (1) written in `9174857b8` (plan-111-A Phase 1); (2) protects: only the named
  wire-decode boundary may parse type spellings; (3) dependents: `is_boundary_file`,
  `boundary_list_is_closed`, the `BOUNDARY_FILES` justification test (`boundary_files_exist_and_are_justified`, in `tests/guards/no_type_strings/self_checks.rs`); (4) proof:
  the exempt file was exempt as a whole, and the four parts are exactly its code — the
  boundary group (#2) is unchanged and no other production file becomes exempt. The assertion
  message ("closed at five GROUPS") stays true. Prose 411, 424 (`ir/binary.rs`'s `seen_types`,
  in `verify_package`) → `ir/binary/mod.rs`'s; 1078 → `ir/binary/`; `no_operator_strings.rs:79`
  `src/ir/binary.rs`'s decode tests → `src/ir/binary/mod.rs`'s.
- `.ai` prose: `.ai/resources-packages.md:215`, `.ai/codegen-invariants.md:87-88` `ir/binary.rs`
  → `ir/binary/`.

### 4.4 `src/ir/verify/mod.rs` → `+ src/ir/verify/env.rs` (code 1203 + 2 test)

| File | LOC | Lines | Items |
|---|---|---|---|
| `verify/mod.rs` | 848 code + 2 test + 1 `mod env;` | 1-877, 1335-1839 | module docs, imports, `Diagnostic`, `ImportedResource`, consts, `is_comparable_defaultable_primitive`, `collect_diagnostics`, `imported_field_pairs`, `collect_diagnostics_with`, `check`, `collect_source_diagnostics`, `MAX_DEPTH`, `StateTypeFault`, `RecordInfo`, `UnionInfo`, `FnSig`, `TypeEnv` struct; `derived_binary_type`, `derived_unary_type`, `usable_type`, free helpers `numeric_literal_is_zero` … `walk_captures`, the `mod calls; … mod values;` list, `#[cfg(test)] mod tests;` |
| `verify/env.rs` | 355 + 1 | `use super::*;` + 878-1334 | `POISONING_RULES` (doc comment from 878) and the whole `impl TypeEnv` block 895-1334 (`build`, `emit`, `closure_slot_count`, `check_closure_capture_arity`, `record_fields`, `collect_record_fields`, `union_variants`, `collect_union_variants`, `infer_type`, `infer_type_depth`, `field_type`) |

(`span.py … 1 877` = 489, `1335 1839` = 361, `881 894` = 13, `895 1334` = 342.) The `TypeEnv`
struct stays in `mod.rs`, so its 37 private fields stay visible to `env.rs` exactly as they are
to the eight sibling `impl TypeEnv` files (`calls.rs`, `link.rs`, `ops.rs`, … all
`use super::*; impl TypeEnv {`). Visibility: 0 edits — every moved method is already
`pub(super)`, and from `verify::env` that reaches `verify` and its descendants, the same set as
today. `POISONING_RULES` is used only inside the moved block (`grep -rn POISONING_RULES src/ir` →
old 1101). `mod env;` is added next to `mod calls;`.

Hazards:

- Spec citations: 10. `collect_diagnostics_with` ×3, `check` ×2, `collect_diagnostics`,
  `TypeEnv`, and the 2 bare `[[src/ir/verify/mod.rs]]` (`language/06_functions.md:95`,
  `language/12_collections.md:9`) stay. `[[src/ir/verify/mod.rs:infer_type]]`
  (`architecture/22_type-inference.md:16`) → `[[src/ir/verify/env.rs:infer_type]]`.
  `spec_citations_resolve` does not flag this one (the file survives); it is repointed by hand.
- Guard `tests/guards/no_type_strings.rs` rows `("src/ir/verify/mod.rs", records|unions|resource_closers|resource_sendable|field_types|record_field_lists|enums|type_decl_info|private_fields|variants)`
  match the `TypeEnv`/`UnionInfo` field declarations, which stay → keep. `build()` moving to
  `env.rs` carries typed locals `enums`, `field_types`, `record_field_lists`, `private_fields`,
  `type_decl_info` (`grep -nE "let (mut )?(enums|field_types|record_field_lists|private_fields|type_decl_info):" src/ir/verify/mod.rs`
  → 908, 909, 911, 913, 915) → ADD 5 rows `("src/ir/verify/env.rs", …)` for those names, so the
  file-scoped scan watches the same declarations. Justification: (1) rows written in `9c3c2f704`
  (plan-111-B Phase 1/2, "type ir::verify's rules, widen the map census"); (2) protects: the
  verifier's type tables stay keyed by `ParameterType`; (3) dependents: same as §4.2;
  (4) proof: the same five declarations at a new path; no row is removed.
- Relative paths in 878-1334: none (`sed -n 878,1334p src/ir/verify/mod.rs | grep "super::\|self::"` → none).
- `#[cfg(test)] mod tests;` stays in `mod.rs`; the G-moved verify tests call `TypeEnv::build` /
  `infer_type`, still `pub(super)` from `verify::env` and visible to `verify::tests`.

### 4.5 `src/ir/verify/resources.rs` → `src/ir/verify/resources/` (code 1153)

| New file | LOC | Lines | Items |
|---|---|---|---|
| `resources/mod.rs` | 577 + 3 `mod` lines | 1-821, 1718-1772 | `use super::*;`, `Plane`, `Unsendable` + impl, `impl TypeEnv {` with `check_resource_moves`, `loop_carried_moves`, `scan_nested_loop`, `leaves_scanned_loop`, `value_type_poisoned`, `is_resource_sendable`, `is_resource_type`; `check_collection_res_axis`, `res_axis_slot`, the impl's closing `}` |
| `resources/sendability.rs` | 266 + 3 | 822-1233 | `thread_unsendable_cause`, `resource_sendability_cause`, `record_fields_unsendable_cause`, `require_thread_sendable`, `belongs_on_the_resource_plane`, `emit_thread_worker_not_transferable`, `emit_thread_resource_plane_required`, `check_thread_sendability`, `check_thread_boundary_sendability` |
| `resources/flow.rs` | 172 + 3 | 1234-1383, 1629-1717 | `check_inline_trap_scrutinee`, `checked_value_can_raise`, `block_always_returns`, `match_covers_all` |
| `resources/defaultability.rs` | 138 + 3 | 1384-1628 | `is_defaultable`, `state_type_fault`, `state_host_is_data`, `check_state_clauses` |

Each part = `use super::*;` + `impl TypeEnv {` … `}`.

Surface and visibility: `Plane` and `Unsendable` stay in `resources/mod.rs`
(`super::resources::Unsendable` in the G-moved verify tests is unchanged). 10 moved methods are
`pub(super)` today and are called across `verify` (`ops.rs`, `calls.rs`, `link.rs`, `types.rs`,
`values.rs`, `verify/mod.rs`, tests); one level deeper `pub(super)` would shrink to `resources`,
so they widen to `pub(in crate::ir::verify)` — exactly today's reach: sendability
`thread_unsendable_cause`, `check_thread_sendability`, `check_thread_boundary_sendability`; flow
`check_inline_trap_scrutinee`, `block_always_returns`, `match_covers_all`; defaultability
`is_defaultable`, `state_type_fault`, `state_host_is_data`, `check_state_clauses`. The private
moved methods (`resource_sendability_cause`, `record_fields_unsendable_cause`,
`require_thread_sendable`, `belongs_on_the_resource_plane`, `emit_thread_worker_not_transferable`,
`emit_thread_resource_plane_required`, `checked_value_can_raise`) are called only inside their own
new file and stay private. Children call the parent-private `is_resource_sendable` and the
`pub(super)` `is_resource_type` — visible to descendants. `verify/mod.rs`'s `mod resources;` is
unchanged.

Hazards:

- Required path rewrites, all in `flow.rs` (`grep -n "super::super::\|super::fold_match" src/ir/verify/resources.rs`):
  old 1666 `super::super::IrMatchPattern::Else` → `super::super::super::IrMatchPattern::Else`;
  old 1698 `super::super::IrMatchCase` → `super::super::super::IrMatchCase`; old 1712
  `super::fold_match_coverage` → `super::super::fold_match_coverage`.
- Spec citations: 27, all rewritten. → `resources/mod.rs`: `check_resource_moves` ×11,
  `loop_carried_moves`, `leaves_scanned_loop`. → `sendability.rs`: `thread_unsendable_cause`,
  `require_thread_sendable`, `check_thread_sendability`, `check_thread_boundary_sendability`.
  → `defaultability.rs`: `is_defaultable` ×4, `state_type_fault`, `state_host_is_data`,
  `check_state_clauses`. → `flow.rs`: `check_inline_trap_scrutinee`, `block_always_returns`.
- Stale spec citation (pre-existing, fixed here): `language/16_threads.md:34`
  `[[src/ir/verify/resources.rs:is_thread_sendable]]` names a fn that no longer exists
  (`grep -n "fn is_thread_sendable" src/ir/verify/resources.rs` → none; plan-114-A replaced it with
  `thread_unsendable_cause`, and the name survives only in the doc comment at old 836) →
  `[[src/ir/verify/resources/sendability.rs:thread_unsendable_cause]]`.
- `.ai/collections.md:608` `src/ir/verify/resources.rs:is_defaultable` →
  `src/ir/verify/resources/defaultability.rs:is_defaultable`. `src/types.rs:913,2576` name the
  module path `ir::verify::resources` in prose — still valid.
- No guard, script, `Cargo.toml` or coverage-exception reference
  (`grep -rn "verify/resources" tests/guards scripts .github Cargo.toml` → none). No inline tests.

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

Before Phase 1, once: record the baselines the phases compare against.

- [x] `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` → write the number
      into this task line as the N baseline (est. 4 min, first test build). Baseline: 5017 (`ir::verify` 493).
- [x] `cp target/release/mfb /tmp/mfb-before-N` after `cargo build --release` at the pre-N
      commit (used only for diagnosing a diff; est. 4 min). Done: `target/p176/mfb-before-N` (release of
      `000497dc9`; `/tmp` is wiped on restart here).

### Phase 1 — `verify/mod.rs` → `verify/env.rs`

The smallest split: one part, zero visibility edits.

- [x] Create `src/ir/verify/env.rs` (`use super::*;` + old 878-1334: `POISONING_RULES` and the
      whole `impl TypeEnv` block); delete those lines from `verify/mod.rs`; add `mod env;` beside
      `mod calls;` (§4.4).
      Done: `items.py split` → env 2 items; `mod env;` placed after `mod compat;` (alphabetical); 0 visibility
      edits; `motion.py` → only `mod env;`.
- [x] `src/docs/spec/architecture/22_type-inference.md:16`: `[[src/ir/verify/mod.rs:infer_type]]`
      → `[[src/ir/verify/env.rs:infer_type]]`.
- [x] `tests/guards/no_type_strings.rs`: add the 5 `("src/ir/verify/env.rs", …)` rows of §4.4
      after the `("src/ir/verify/mod.rs", "variants")` row, with the §4.4 justification in the
      commit message.
- [x] Tighten: `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`; commit
      `tests/guards/file_size_ceilings.txt`.

Acceptance: `verify` behaves identically and the file is within budget.
  Check: `cargo build --release && cargo test -p mfb --bin mfb ir::verify` → all pass, same count
  as before the phase (`cargo test -p mfb --bin mfb ir::verify -- --list 2>/dev/null | grep -c ': test$'`
  before == after) (est. 6 min). `bash scripts/test-accept.sh target/release/mfb /tmp/accept-N '*-invalid'`
  → 0 failures (est. 5 min, guess; the verifier's diagnostics reach only `build.log`, which the
  artifact gate does not compare). `sh scripts/artifact-gate.sh target/release/mfb general` →
  0 diffs (est. 1 min). `cargo check --all-targets 2>&1 | grep -c '^warning'` → same as before
  (est. 2 min). `cargo test -p mfb --bin mfb spec_citations_resolve` → pass (est. 1 min).
  `cargo test --test no_type_strings` → pass (est. 1 min). Before the tighten,
  `cargo test --test file_size_budget 2>&1 | grep 'src/ir/verify'` → exactly one row,
  `TIGHTEN … src/ir/verify/mod.rs`; after it, `grep 'src/ir/verify/mod.rs' tests/guards/file_size_ceilings.txt`
  → no `ceiling code` row (est. 2 min).
  Verified 2026-10-19: `ir::verify` → `ok. 493 passed` (= before); `test-accept.sh … '*-invalid'` →
  `acceptance tests passed (208 test(s) ran)`; `bash scripts/artifact-gate.sh … general` (bash, as M recorded) →
  0 diffs; check warnings 0; `spec_citations_resolve` ok; `no_type_strings` → 7 + 1 ignored; budget before the
  tighten → one row `TIGHTEN code 849 (ceiling 1203) src/ir/verify/mod.rs`; after → no row; census → mod 849/2,
  env 356.
Commit: f719a4af1

### Phase 2 — `verify/resources.rs` → `verify/resources/`

- [x] `git mv src/ir/verify/resources.rs src/ir/verify/resources/mod.rs`; create
      `sendability.rs`, `flow.rs`, `defaultability.rs` with the §4.5 ranges, each `use super::*;` +
      `impl TypeEnv { … }`; add the three `mod` lines to `resources/mod.rs`.
      Done: `items.py split` (method-level) → sendability 9, flow 4, defaultability 4 methods; `motion.py` →
      only the wiring, three new impl blocks, the 10 widenings and the 3 path rewrites.
- [x] Widen the 10 methods listed in §4.5 to `pub(in crate::ir::verify)`.
- [x] `flow.rs`: the three `super::` rewrites of §4.5.
- [x] Spec: rewrite all 27 `[[src/ir/verify/resources.rs:…]]` citations per §4.5, including the
      stale `is_thread_sendable` → `[[src/ir/verify/resources/sendability.rs:thread_unsendable_cause]]`
      (`language/16_threads.md:34`).
      Done: `citesweep.py` → 26 + the stale one by hand; `citedef.py src/ir/verify/` → 89 cites, 0 not defined.
- [x] `.ai/collections.md:608` → `src/ir/verify/resources/defaultability.rs:is_defaultable`.
      Also the same path in `tests/runtime/inplace_self_update/field_kinds_gen.py`'s `defaultable` docstring
      (§4.5 missed it; the acceptance grep covers `tests`); the generator's output is unchanged
      (`inplace_self_update_census` → 3 passed).
- [x] Tighten and commit the ceiling file.

Acceptance: the resource and sendability rules behave identically; no citation of the old path
remains.
  Check: `grep -rn "src/ir/verify/resources\.rs" src/docs .ai tests scripts` → none (est. 5 s).
  `cargo build --release && cargo test -p mfb --bin mfb ir::verify` → pass, count before == after
  (est. 6 min). `bash scripts/test-accept.sh target/release/mfb /tmp/accept-N '*-invalid'` → 0
  failures (est. 5 min, guess). `sh scripts/artifact-gate.sh target/release/mfb thread` → 0 diffs
  (the sendability rules gate every thread program; est. 1 min). `cargo check --all-targets` →
  no new warning; `spec_citations_resolve` → pass. Before the tighten
  `cargo test --test file_size_budget 2>&1 | grep 'src/ir/verify/resources'` → exactly one row,
  `STALE … src/ir/verify/resources.rs`, and no `NEW` row for a `resources/` part; after it, none
  (est. 2 min).
  Verified 2026-10-19: old-path grep → 0; `ir::verify` → `ok. 493 passed`; `test-accept.sh … '*-invalid'` → 208
  passed; G2 thread → 0 diffs; check warnings 0; `spec_citations_resolve` ok; budget before the tighten → one row
  `STALE code (ceiling 1153) src/ir/verify/resources.rs`, no `NEW`; after → none; census → 580, 269, 175, 144.
Commit: 18c724935

### Phase 3 — `binary.rs` → `binary/`

- [x] `git mv src/ir/binary.rs src/ir/binary/mod.rs`; create `link_codec.rs`, `op_codec.rs`,
      `value_codec.rs` with the §4.3 ranges; the 13 `pub(super)` edits; the three private `use`
      lines in `mod.rs`; the one added `use` line in `link_expr_op_tests`.
      Done: `codesplit.py` → link_codec 14, op_codec 8, value_codec 5 items, none left unlisted; `pub(super)` from
      the compiler's errors → exactly §4.3's 13; `motion.py` → only the wiring, the imports and the 13.
- [x] `tests/guards/no_type_strings.rs`: replace the `src/ir/binary.rs` row in `BOUNDARY_FILES`,
      and in `tests/guards/no_type_strings/self_checks.rs`: `boundary_list_is_closed`, with the four `src/ir/binary/…` rows, same position, same
      reason (§4.3 justification in the commit message); repoint prose 411, 424, 1078;
      `tests/guards/no_operator_strings.rs:79` prose.
      Done: before the edit `no_type_strings` → 2 FAILED (`no_type_strings`, `boundary_files_exist_and_are_justified`);
      after → 7 passed + 1 ignored; prose (now 414, 427, 1086) and `no_operator_strings.rs:79` repointed.
- [x] Spec: rewrite all 40 `[[src/ir/binary.rs:…]]` citations per §4.3.
- [x] `.ai/resources-packages.md:215`, `.ai/codegen-invariants.md:87-88` → `ir/binary/`.
      Done: those 3 plus 9 code comments §4.3 did not list (`validation.rs`, `link_thunk.rs`, `ir/link.rs`,
      `ir/package.rs`, `ir/types.rs`, `manifest/package.rs`, `operators.rs` ×2, `rt_imported_resource_scope_drop.rs`)
      → `ir/binary/`; `git grep 'ir/binary\.rs'` → 0 outside the ceiling file. `citesweep.py` → 40, `citedef.py` →
      0 not defined.
- [x] Tighten and commit the ceiling file.

Acceptance: the IR wire codec writes and reads the same bytes.
  Check: `cargo build --release && cargo test -p mfb --bin mfb -- ir::binary ir::variant_corpus_tests`
  → pass, count before == after (est. 6 min). `bash scripts/test-accept.sh target/release/mfb /tmp/accept-N 'package-*' 'native-library-*' 'native-resource-*' 'roundtrip-package'`
  → 0 failures; these are the 18 fixtures whose `.mfp`/`.info` goldens carry the encoded IR
  (est. 3 min, guess). `sh scripts/artifact-gate.sh target/release/mfb thread` → 0 diffs (its
  worker package is decoded from `packages/thread_cover_worker.mfp`; est. 1 min).
  `cargo test --test no_type_strings --test no_operator_strings` → pass (est. 1 min).
  `cargo check --all-targets` → no new warning; `spec_citations_resolve` → pass.
  `cargo test --test file_size_budget 2>&1 | grep 'src/ir/binary'` → one `STALE` row for
  `src/ir/binary.rs` before the tighten, none after (est. 2 min).
  Verified 2026-10-19: `ir::binary ir::variant_corpus_tests` → `ok. 38 passed`, whole list 5017 (= baseline);
  `test-accept.sh … 'package-*' 'native-library-*' 'native-resource-*' 'roundtrip-package'` → `acceptance tests
  passed (42 test(s) ran)`; G2 thread → 0 diffs; guards → 8 / 7 + 1 ignored; check warnings 0;
  `spec_citations_resolve` ok; budget → one `STALE code (ceiling 1448) src/ir/binary.rs` before, none after; census
  → 490/118, 333, 313, 324.
Commit: 332846edd

### Phase 4 — `shape.rs` → `shape/`

- [x] `git mv src/ir/shape.rs src/ir/shape/mod.rs` (G's `#[cfg(test)] mod tests;` keeps resolving
      to `src/ir/shape/tests/`); create `declarations.rs`, `statements.rs`, `expressions.rs`,
      `compat.rs`, `calls.rs` per §4.2, each `use super::*;` + `impl<'a> Walker<'a> { … }`.
      Done: `items.py split` (method-level) → 9, 11, 9, 5, 15 methods; the emptied `impl Walker` shell removed from
      `mod.rs`; `motion.py` → only the wiring, five impl blocks, the 30 `pub(super)` and the path rewrites.
- [x] The 30 `pub(super)` method edits of §4.2; the four `super::` rewrites in `declarations.rs`.
- [x] `tests/guards/no_type_strings.rs`: rows 453-454 → `src/ir/shape/mod.rs`; add
      `("src/ir/shape/declarations.rs", "resource_types")`; line 101 prose →
      `src/ir/shape/mod.rs` `bound_types`; prose 378, 400, 423, 967 → `ir/shape/`.
      Done: before the edit `curated_type_keyed_tables_all_exist` FAILED; after → 7 passed + 1 ignored (rows now
      468-470, prose 104, 393, 415, 438, 987).
- [x] Spec: rewrite all 23 `[[src/ir/shape.rs…]]` citations per §4.2.
- [x] `.ai/codegen-invariants.md:64`, `.ai/resources-packages.md:41`, `.ai/compiler.md:21`,
      `.ai/collections.md:645` per §4.2.
      Done: as §4.2, plus `src/codegen/builtins/tests/literal_coercion.rs:17` (unlisted) → `ir/shape/`; citations:
      `citesweep.py` → 21 + the two bare cites by hand; `citedef.py` → 0 not defined; `git grep 'ir/shape\.rs'` → 0
      outside the ceiling file and the `coverage-report.py` docstring §4.2 keeps.
- [x] Tighten and commit the ceiling file.

Acceptance: the shape checker reports the same diagnostics.
  Check: `cargo build --release && cargo test -p mfb --bin mfb ir::shape` → 57 pass (the G-moved
  tests; count before == after) (est. 6 min). `bash scripts/test-accept.sh target/release/mfb /tmp/accept-N '*-invalid'`
  → 0 failures (est. 5 min, guess). `sh scripts/artifact-gate.sh target/release/mfb general` → 0
  diffs (est. 1 min). `cargo test --test no_type_strings` → pass. `cargo check --all-targets` →
  no new warning; `spec_citations_resolve` → pass. `cargo test --test file_size_budget 2>&1 | grep 'src/ir/shape'`
  → one `STALE` code row for `src/ir/shape.rs` before the tighten (G already removed its inline
  row), none after (est. 2 min).
  Verified 2026-10-19: `ir::shape` → `ok. 58 passed` (58 in the G0 list too; the plan's 57 was the shape-tests
  count alone); `test-accept.sh … '*-invalid'` → 208 passed; G2 general → 0 diffs; `no_type_strings` → 7 + 1
  ignored; check warnings 0; `spec_citations_resolve` ok; budget → one `STALE code (ceiling 2950) src/ir/shape.rs`
  before, none after; census → 495/4, 597/2, 482, 600, 199, 600/2.
Commit: —

### Phase 5 — `lower.rs` → `lower/` (largest blast radius)

- [ ] `git mv src/ir/lower.rs src/ir/lower/mod.rs`; create `decls.rs`, `statement.rs`,
      `inline_trap.rs`, `trap_hoist.rs`, `concat_fold.rs`, `typing.rs`, `call_args.rs`,
      `expression.rs`, `construct.rs`, `captures.rs` per §4.1.
- [ ] `mod.rs`: the ten `mod` lines, the ten private `use <part>::*;` lines, the four re-export
      lines; widen the 8 items to `pub(in crate::ir)`; the 50 `pub(super)` edits.
- [ ] The two path rewrites (`hoist_resource_temporaries` in `decls.rs`,
      `VALUE_VISIT_MAX_DEPTH` + its doc link in `inline_trap.rs`).
- [ ] `TypeIndex` doc comment: `` `ir/verify/mod.rs:1002` `` → `` `ir/verify/env.rs:infer_type_depth` ``.
- [ ] `tests/guards/no_type_strings.rs`: the 5 `TYPE_KEYED_TABLES` rows → `src/ir/lower/mod.rs`;
      in `tests/guards/no_type_strings/self_checks.rs` the self-test literal → `"src/ir/lower/mod.rs"` (§4.1 justification in the commit message);
      prose 374, 398, 972, 993.
- [ ] Spec: rewrite all 76 `[[src/ir/lower.rs…]]` citations per §4.1.
- [ ] `.ai/collections.md:464`, `.ai/resources-packages.md:28, 254, 439, 441` per §4.1 (254 also
      fixes the stale `source_implementation_name`; 439 the stale `builtins::` path).
- [ ] Tighten and commit the ceiling file.

Acceptance: lowering produces byte-identical IR for every builtin family.
  Check: `cargo build --release && cargo test -p mfb --bin mfb ir::` → pass (est. 7 min).
  `sh scripts/artifact-gate.sh target/release/mfb general`, then `collections`, then `strings`
  → 0 diffs each (`.ast`/`.ir` plus 5 native targets; est. 1 min each).
  `bash scripts/test-accept.sh target/release/mfb /tmp/accept-N '*-invalid'` → 0 failures
  (lowering-time diagnostics; est. 5 min, guess). `cargo test --test no_type_strings` → pass.
  `cargo check --all-targets` → no new warning; `spec_citations_resolve` → pass.
  `grep -rn "src/ir/lower\.rs\|ir/lower\.rs" src/docs .ai tests/guards` → none (est. 5 s).
  `cargo test --test file_size_budget 2>&1 | grep 'src/ir/lower'` → one `STALE` code row for
  `src/ir/lower.rs` (its 57 inline test LOC are under 250, so it has no inline ceiling) before the
  tighten, none after (est. 2 min).
Commit: —

## Validation Plan

- Tests: no test is added or removed. Whole-binary count:
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` → equals the N baseline
  recorded before Phase 1 (est. 4 min).
- Coverage check: every moved line stays under `src/ir`, inside the coverage denominator
  (`scripts/coverage-common.sh` `IGNORE` excludes only `tests/` dirs and the listed patterns); no
  coverage exception exists for these files, so none is repointed.
- Runtime proof: byte-identity. Letter end, once:
  `sh scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)` (est. 15–20 min; needed
  because lowering feeds every fixture on every target, and only the full sweep covers every
  fixture's `.ir` and native dumps). And `bash scripts/test-accept.sh target/release/mfb /tmp/accept-N '*-invalid' 'package-*' 'native-library-*' 'native-resource-*' 'roundtrip-package'`
  → 0 failures (est. 8 min, guess; `build.log` and `.mfp` are outside the artifact gate).
- Guards: `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings`
  → pass (est. 2 min).
- Budget: `cargo test --test file_size_budget 2>&1 | grep -E 'src/ir/(lower|shape|binary|verify)'`
  → no row; `grep -E 'src/ir/(lower|shape|binary)|src/ir/verify/(mod|resources)' tests/guards/file_size_ceilings.txt`
  → no `ceiling` row.
- Doc sync: `spec_citations_resolve` → pass; `grep -rnoE "\[\[src/ir/(lower|shape|binary|verify/resources)\.rs" src/docs/spec`
  → none. No `mfb man` page cites these files (0, §2). No compiler contract changes, so no spec
  prose changes beyond citations and the stale-symbol fixes.
- Full workspace suite: not run here; it runs once in plan-176-V's Validation Plan.

## Decisions

- Rule revised — owner, 2026-10-18 (replaces the 2026-10-14 "code ≤ 1000, test ≤ 250" rule):
  three kinds, non-comment non-blank LOC, scope `.rs`/`.mfb` under `src/`, `packages/`,
  `examples/`, `repository/`, `tests/`: `code` (code LOC of a non-test-only file) ≤ 1000 → split
  into modules; `inline` (`#[cfg(test)]` / `TESTING … END TESTING` LOC inside a non-test-only
  file) ≤ 250 → move the tests to a test-only file; `test` (whole test-only file) ≤ 1000 → split
  into modules. Parts target ≤ 900. No exemptions except `GENERATED` files; ceiling rows are
  `ceiling <code|inline|test> <n> <path>`; the guard warns with a banner, never fails; a new
  module is always an acceptable split. (G moved `shape.rs`'s inline tests; N's remaining
  inline tests are each < 250.)
- A single function over the code limit is decomposed by extract-method only in letters S–V —
  owner, 2026-10-14. No N item exceeds 900 LOC, so N does no extract-method.
- Guard rows that name a moved file are updated in the same commit as the move, each with the
  AGENTS.md 4-point justification; no production file becomes newly exempt — author, 2026-10-14.
- Pre-existing defects found by the map are fixed here: the stale `is_thread_sendable` spec
  citation, the stale `ir/verify/mod.rs:1002` line reference in `TypeIndex`'s doc, and the stale
  `source_implementation_name` / `builtins::default_argument_padding` names in
  `.ai/resources-packages.md` — author, 2026-10-14 (AGENTS.md "never leave a bug").
- `verify::resources` methods widen to `pub(in crate::ir::verify)`, not `pub(crate)` — exactly
  today's reach — author.
- `lower` parts are glob-imported into `lower/mod.rs` (`use <part>::*;`), verified collision-free
  (§2) — author.

## Corrections

- 2026-10-18: Rule revised by the owner (code ≤ 1000, inline ≤ 250, test-only file ≤ 1000, no exemptions); file list and split design unchanged; rule text (intro, Goal, Decisions) restated, `test` ceiling/row wording for inline blocks now says `inline`, and the G-moved test directories are noted as staying split (each over 1000).

## Summary

The engineering risk is in `lower.rs`: it produces every `.ir`, and its split touches 58
visibilities, two relative paths and 76 citations. Byte-identity over all fixtures, plus the
`build.log` and `.mfp` checks the artifact gate cannot see, contains it. `ir/mod.rs`'s surface,
every test, and every emitted byte stay unchanged.
