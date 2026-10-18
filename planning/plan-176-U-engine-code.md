# plan-176-U: Engine code

Last updated: 2026-10-18
Effort: large (3h–1d)
Depends on: plan-176-T

U splits the nine over-budget code files of `src/codegen/engine/`: the statement lowering
(`control/builder_control.rs`), the module lowering (`builder/mod.rs`), the value lowering
(`value/builder_values.rs`), function and entry lowering (`function/function_lowering.rs`,
`function/entry.rs`), numeric operators (`operators/builder_numeric.rs`), two analyses
(`analysis/borrow_get.rs`, `analysis/module_analysis.rs`) and the conversions
(`convert/builder_conversions.rs`). Five of the phases are pure code motion: each file becomes a
directory module whose `mod.rs` keeps the surface. Three functions are over the 1000-LOC code
limit on their own: `lower_module_for_platform` (1801), `lower_ops_inner` (1313) and
`lower_value_inner` (1224). Each is decomposed by extract-method refactoring, in its own phase,
after all pure-motion phases. Every phase is gated by byte-identical codegen.

Single outcome: after U lands, `cargo test --test file_size_budget` prints no row for any file
under `src/codegen/engine/{analysis,builder,control,convert,function,operators,value}/`, and
`tests/guards/file_size_ceilings.txt` has no `ceiling <code|inline|test>` row for any of the nine
files. There are no exemptions (rule revised by the owner, 2026-10-18; only `GENERATED` files are
exempt automatically).

References:

- `planning/plan-176-A-file-size-guard.md` — the rule, §Decisions, the guard and the
  "Splitting an over-budget file (plan-176)" conventions in `.ai/build-tooling.md`.
- `.ai/compiler.md` and `.ai/codegen-invariants.md` — vreg allocation order, register
  lifetimes, desugars (the extract phases must not reorder a `temporary_vreg()` /
  `allocate_register()` / `allocate_stack_object()` / `label()` call).
- `.ai/collections.md` — element/field update lowering, borrow-get classification (§ cited at
  `.ai/collections.md` lines 34, 540, 556, 585, 592 as of 17a9b5799).
- `.ai/resources-packages.md` — STATE assignment, resource-typed binds, borrowed-return
  classification (lines 28, 45, 103, 113, 135, 178, 181, 384, 420).
- `.ai/testing-gates.md` — the artifact gate, what it does NOT cover (inline `TRAP`, whole-record
  STATE replace, canvas), and the per-phase vs. letter-end gate split.
- `.ai/canvas-threading.md` §2 (cites `function/entry.rs`).
- `.ai/build-tooling.md` — rustfmt policy (`rustfmt --edition 2021 <files>` for scoped
  formatting; `cargo fmt -- <files>` does not scope).
- Spec topics whose citations move (38 files, `grep -rlE "\[\[src/codegen/engine/(control/builder_control|builder/mod|value/builder_values|function/function_lowering|function/entry|operators/builder_numeric|analysis/borrow_get|analysis/module_analysis|convert/builder_conversions)\.rs" src/docs/spec | wc -l` → 38): app/01, app/02, app/05, architecture/06, 07, 17, 23, spec.md, language/04, 08, 10, 11, 13, 14, 15, 16, 18, linker/01, 02, 03, 05, 09, memory/02, 03, 04, 05, 06, 08, 09, stdlib/13, stdlib/23, threading/06, 07, 08, 09, tooling/01, 09, unicode/01.
- Split maps used to write this plan: `/tmp/loc/maps/U.md`, `/tmp/loc/maps/U-extract.md`
  (`lower_module_for_platform`), `/tmp/loc/maps/U-extract2.md` (`lower_ops_inner`,
  `lower_value_inner`).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-T complete and archived | `ls planning/plan-176-T-* planning/completed/plan-176-T-*` → only `planning/completed/…` | NOT MET (2026-10-14) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | NOT MET (2026-10-14; lands in plan-176-A) |
| U's files have no uncommitted changes | `bash -c 'S=$(grep -rlE "\[\[src/codegen/engine/(control/builder_control\|builder/mod\|value/builder_values\|function/function_lowering\|function/entry\|operators/builder_numeric\|analysis/borrow_get\|analysis/module_analysis\|convert/builder_conversions)\.rs" src/docs/spec); git status --porcelain -- src/codegen/engine $S .ai tests/guards/no_type_strings.rs scripts/spec-census.sh src/testutil.rs src/optimizer/opt1/strength.rs src/ir/fallible.rs src/codegen/memory src/codegen/runtime/thread/runtime_helpers.rs src/codegen/builtins/tests src/codegen/builtins/math/gen_fmod.rs src/codegen/builtins/collections/gen_flow.rs src/codegen/string/unicode_props.rs'` → empty (the `\|` inside the table are plain `\|` in the shell) | MET (2026-10-14, empty output) |

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

- Each of the nine files is replaced by a directory module (`builder/mod.rs` gains sibling
  files) whose every file is ≤ 900 code LOC and ≤ 250 inline test LOC by
  `python3 -I tools/loc-census/census.py --file <path>`.
- No function under `src/codegen/engine/` is over 1000 code LOC. The three oversize functions
  are decomposed exactly as §4.10–§4.12 specify; the largest function afterwards is
  `lower_bind_op` (490 LOC, guess from region counts; re-measured in Phase 5).
- `.ncode` (and every other artifact-gate dump kind) is byte-identical on every target before
  and after each phase.
- Every `[[…]]` spec citation, `.ai/` reference and code comment that names one of the nine
  old paths names the file that now holds the cited symbol.

### Non-goals (explicit constraints)

- No behavior change and no change to emitted code on any target.
- No logic edit except the three decided extractions (§4.10–§4.12). Pure-motion phases move
  text verbatim; the only other edits are `pub(super)` visibility, `use` lines, `mod`
  declarations, re-exports, and the two `include_str!` paths in §4.1.
- No crate path that callers use changes: every outside `use crate::codegen::engine::…`
  resolves unchanged through the existing parent globs (`control/mod.rs` `pub(crate) use
  builder_control::*;`, `value/mod.rs` `pub(crate) use builder_values::*;`, etc.) plus the
  re-exports in §4.
- No test fn renamed, no test module renamed, no `[[test]]` target renamed. The unit-test
  count of the `mfb` binary is unchanged by every phase.
- No guard weakened. U edits no guard: the overflow-label scan already covers `builder_*`
  directories since plan-176-O Phase 1 (§2).
- `bugs/*.md` that name the old paths are historical records and are not edited.

## 2. Current State

### Files (as of 17a9b5799)

LOC from `/tmp/loc/groups.json` (planning census `python3 -I /tmp/loc/census.py "$PWD"`; after A,
`python3 -I tools/loc-census/census.py --file <path>`). Inline test LOC from the U map.

| File | Kind | Code LOC | Inline test LOC | `#[test]` | Lines (`wc -l`) |
|---|---|---|---|---|---|
| `src/codegen/engine/control/builder_control.rs` | code | 4392 | 103 | 3 | 6058 |
| `src/codegen/engine/builder/mod.rs` | code | 3110 | 3 (a `#[cfg(test)]` statement) | 0 | 4974 |
| `src/codegen/engine/value/builder_values.rs` | code | 3053 | 111 | 6 | 4852 |
| `src/codegen/engine/function/function_lowering.rs` | code | 1707 | 0 | 0 | 2158 |
| `src/codegen/engine/operators/builder_numeric.rs` | code | 1582 | 0 | 0 | 1923 |
| `src/codegen/engine/analysis/borrow_get.rs` | code | 1373 | 522 at planning; moved to `borrow_get/tests/mod.rs` by plan-176-H | 27 | 2298 |
| `src/codegen/engine/convert/builder_conversions.rs` | code | 1207 | 0 | 0 | 1460 |
| `src/codegen/engine/analysis/module_analysis.rs` | code | 1122 | 0 | 0 | 1217 |
| `src/codegen/engine/function/entry.rs` | code | 1061 | 0 | 0 | 1430 |

State left by earlier letters: plan-176-H moves `borrow_get.rs`'s inline `mod tests` (lines
1704–2298) to the one test-only file `analysis/borrow_get/tests/mod.rs` (522 LOC, under the
1000 test-only limit, so not split) and leaves `#[cfg(test)] mod tests;` at the end of
`borrow_get.rs`; lines 1–1703 are unchanged by H. No earlier letter edits the other eight files.
All line ranges below are "as of 17a9b5799"; item and test NAMES are authoritative if lines drift.

### Measured populations

| What | Count | Command |
|---|---|---|
| Spec citations of the nine files | 116 (19 builder_control, 23 builder/mod.rs, 28 builder_values, 6 function_lowering, 18 entry, 9 builder_numeric, 4 borrow_get, 2 module_analysis, 8 builder_conversions) | `grep -rnoF "[[<path>" src/docs/spec \| wc -l` per path |
| — of which whole-file citations | 5 (4 × `builder/mod.rs`, 1 × `builder_values.rs`) | `grep -rnoE "\[\[src/codegen/engine/(builder/mod\|value/builder_values)\.rs\]\]" src/docs/spec` |
| Spec files holding those citations | 38 | see References |
| Overflow labels scanned by `no_overflow_label_returns_through_the_result_tag_register` | 42 in 747 scanned files; 10 of them in U files (3 `builder_numeric.rs`, 7 `builder_conversions.rs`) | python walk replicating `collect_builder_sources` (file name starts `builder_`/`func_`/`gen_`) over `src/codegen`; `grep -n 'abi::label(&.*overflow' src/codegen/engine/operators/builder_numeric.rs src/codegen/engine/convert/builder_conversions.rs` |
| `include_str!` in U files | 2, both in `builder_control.rs`'s inline test | `grep -n include_str src/codegen/engine/control/builder_control.rs` → 5937, 5938 |
| Non-test `#[cfg(...)]` gates in U files | 0 | `grep -n "#\[cfg(" <nine files> \| grep -v "cfg(test)"` → empty |
| Coverage-exception / `#[path]` entries for U files | 0 | `grep -nE "builder_control\|builder/mod\|builder_values\|function_lowering\|entry.rs\|builder_numeric\|borrow_get\|module_analysis\|builder_conversions" scripts/coverage-exceptions.txt` → empty; `grep -n '#\[path' <nine files>` → empty |

### Verified properties

- **The citation checker is file-level** (`src/docs/spec/mod.rs::spec_citations_resolve`,
  plan-176-A §2). Converting `foo.rs` → `foo/mod.rs` makes every `[[foo.rs:…]]` fail, which
  forces the sweep for eight of the nine files. `builder/mod.rs` keeps its path, so its 19
  symbol citations would pass silently after their symbols move: the sweep for that file is
  done by the explicit table in §4.2 and §4.12, not by the checker.
- **Glob-cycle imports compile without warnings** (`use super::*;` in a part, `use part::*;`
  in `mod.rs`): probe `/tmp/loc/probe2` printed `6` with no warning (brief, 2026-10-14). A
  child module's `use super::*;` also brings the parent's private `use` imports into scope, so
  each part sees the original header imports.
- **`ClosureHelperTables` holds exactly the eight frozen locals** of `lower_module_for_platform`:
  `src/codegen/memory/arena/closure_ops.rs` `pub(crate) struct ClosureHelperTables<'a>` has
  fields `function_symbols`, `functions`, `package_return_types`, `platform_imports`,
  `platform`, `build_mode`, `globals`, `string_symbols`, all `pub(crate)` (read 2026-10-14).
- **`VirtualRegister` is `Copy`** (`src/codegen/engine/operand/operand.rs`
  `#[derive(Clone, Copy, …)] struct VirtualRegister`), so `zero_slot` passes by value with no
  new mint.
- **The `transfer_temp_classification_tests` self-read survives the move.** It finds
  `fn transfer_temp_disposition(op: &NirOp) -> TransferTemps {` and ends at `"\n    }\n"`
  (4-space indent). `transfer_temp_disposition` stays in `builder_control/mod.rs` inside the
  `impl` block at the same indent (read 2026-10-14, `builder_control.rs` 5924–6058).
- **The overflow-label guard selects its corpus by file name.**
  `no_overflow_label_returns_through_the_result_tag_register` (`src/codegen/engine/tests/`,
  `collect_builder_sources`) scans only `.rs` files whose own name starts with `builder_`,
  `func_` or `gen_`. `builder_conversions/to_int.rs`, `builder_numeric/numeric_kernels.rs` etc.
  would drop out of it: 10 of its 42 overflow labels (count above). Its floor asserts
  (`checked_files > 20`, `overflow_labels >= 20`) would still pass, so the loss is silent.
  plan-176-O Phase 1 (the first letter to split a `builder_*` file) widens the scan to every `.rs`
  file below a `builder_*`/`func_*`/`gen_*` directory, so U's prerequisite on O covers it. U's
  Phase 1 proves the moved code is still scanned (Acceptance).
- **Stale references found while mapping** (fixed in U, §4.13):
  `.ai/resources-packages.md:135` names `try_inplace_state_collection_assign` — 0 hits in
  `src` (`grep -rn try_inplace_state_collection_assign src` → empty); the StateAssign arm calls
  `try_inplace_self_update` (`src/codegen/collection/assign/self_update.rs`) with an
  `InPlaceDest::StateField` site (`builder_control.rs` 4020–4029).
  `.ai/resources-packages.md:113` names `collect_value_used_locals` — removed by
  `f37874271` (bug-781) (`git log -S collect_value_used_locals --oneline`); the live
  `NirVisitor`-based collector in that file is `collect_value_local_reads`
  (`function_lowering.rs:280`).
  `tests/guards/no_type_strings.rs:380,403` doc comments say `function_lowering.rs`'s
  `union_extract_reads`; the field is in `borrow_get.rs` (`grep -rn union_extract_reads src` →
  `borrow_get.rs:1368,1467,1491,1500`), inside `collect_borrow_gets_by_name`.
  `src/codegen/engine/analysis/module_analysis.rs:83` says `emit_resource_state_init`,
  `builder_control.rs`; it is defined in `src/codegen/memory/value/builder_value_semantics.rs:41`.
  `src/codegen/engine/value/builder_values.rs:1770` cites `builder_control.rs:697-699`; the
  `resource_cleanup_symbol(type_)` reads are in the Bind arm (2792, 2985, 3299, 3346).
  `.ai/collections.md:540` cites `builder_control.rs ~592`; the `&&`-chain is in the Assign
  arm. `.ai/collections.md:585` cites `builder_values.rs:17` / `register_pending_temp:30`
  (line numbers that no longer hold).
- **Not stale** (checked): `.ai/canvas-threading.md:30` `ENTRY_STACK_SIZE` is defined in
  `src/codegen/error/constants/error_constants.rs` (`ENTRY_GLOBALS_OFFSET = ENTRY_STACK_SIZE`,
  line 231) and used by `lower_program_entry`; only the path changes.
- **Moving Rust code cannot change emitted code through source locations**
  (plan-176-A §2: no `module_path!`/`file!`/`line!` in `src`, no link-order registration).
  Byte-identity is therefore the gate for the pure-motion phases, and for the extract phases
  because they keep every lowering call in its original order (§4.10–§4.12).
- **What the artifact gate does not cover** (`.ai/testing-gates.md`): no byte-identity fixture
  has an inline `TRAP`, and no golden covers a whole-record STATE replace (only
  `tests/runtime/rt_res_state_inplace_mutation.rs`). Canvas codegen is covered only by
  `tests/syntax/app/app-mouse-surface` (full gate, not a scoped one). The Phase 5 and Phase 7
  checks add the runtime targets that do cover those paths.

## 3. Design Overview

**Conventions** (plan-176-A, `.ai/build-tooling.md` "Splitting an over-budget file"):
`<stem>.rs` → `<stem>/mod.rs` + parts. The parent `mod.rs` keeps `pub(crate) mod <stem>;
pub(crate) use <stem>::*;` unchanged. Each part starts with `use super::*;` plus the `use`
lines the moved text needs that the parent does not import; unused imports are trimmed per file
(`unused_imports`). Methods of `CodeBuilder` move inside `impl CodeBuilder<'_> { … }` in the
part; inherent methods need no re-export. A private method called from another part becomes
`pub(super)`. Free items re-export from `mod.rs` so outside paths do not change. ≤ 900 code LOC
per part. Pure motion: text is moved verbatim (rustfmt may re-wrap; no token changes).

**Gate class: byte-identity.** Every phase: `cargo build --release`, then the scoped
`sh scripts/artifact-gate.sh target/release/mfb <builtin>` for the built-ins whose lowering
moved (named per phase, ~1 min each), the moved modules' unit tests, `cargo check
--all-targets` with no new warning, `spec_citations_resolve`, the three name-based guards, and
the file-size tighten. Letter end, once: `sh scripts/artifact-gate.sh target/release/mfb all`
(15–20 min) — engine code is reached by every fixture on every target, and only the full gate
covers all targets and the canvas fixture. A diff is never a stop: localize it on ONE fixture
(`mfb build -ncode` on HEAD vs. the phase's tree, diff the two `.ncode`), fix the motion or
extraction, re-run.

**Risk concentrates** in the three extract phases (5–7), which change function boundaries in
the statement, value and module lowerings — every program passes through all three. Phase 7
(`lower_module_for_platform`) is last: it re-plumbs 30+ locals of the module orchestrator into
two structs and ten functions. The pure-motion phases carry low risk: the danger there is a
missed citation or a private item left unreachable, both caught by `cargo check` and the checker.

**Intermediate NEW rows.** The pure-motion phases put each oversize function, whole, into its
own part file (`builder_control/statement_dispatch.rs`, `builder_values/lower_inner.rs`,
`builder/lower_module.rs`). Those three files are over 1000 until their extract phase, so
between Phase 3/4 and Phase 5/6/7 the banner shows a NEW row for each (the guard warns and never
fails, plan-176-A §Decisions). Each phase's acceptance names exactly the rows expected at that
point. No ceiling row is added by hand for them: they are gone by the end of U.

**Overflow-label guard.** plan-176-O Phase 1 makes `collect_builder_sources` scan every `.rs` file
below a `builder_*`/`func_*`/`gen_*` directory, with the AGENTS.md justification recorded there.
U changes no guard. Phase 1's Acceptance shows the 7 `builder_conversions` labels are still
counted after the move.

**Rejected alternatives.**
- *Exempt the three oversize functions* — the owner decided single functions over the limit are
  decomposed by extract-method refactoring (plan-176-A §Decisions). The U map's exemption notes
  for them are superseded by §4.10–§4.12.
- *Keep the oversize function in the old file's `mod.rs` during pure motion* — the path changes
  anyway (`foo.rs` → `foo/mod.rs`), so it still produces a NEW row, and it would make the
  extract phases move the function twice.
- *Field parameters for the extracted arm helpers* (`name: &str`, `args: &[NirValue]`) — needs
  `&String`/`&Vec` (clippy `ptr_arg`; `grep -rn "fn .*: &String" src/codegen` → empty) or body
  edits (`name.clone()` into `String` fields, `retain(|_, (l, _)| l != name)`). The let-else
  re-destructure of the scrutinee keeps every body verbatim (precedent:
  `src/codegen/engine/analysis/borrow_get.rs:151`, `:1002`).
- *A new struct for `lower_module_for_platform`'s frozen tables* — `ClosureHelperTables` already
  is that struct, so it is reused.
- *`builder/mod.rs` parts declared `pub(crate) mod`* — about ten outside files glob
  `engine::builder::*` and `runtime::thread` already has a `runtime_helpers` module; private
  `mod` plus explicit `pub(crate) use` lists avoid a name collision.
- *Re-pointing `bugs/*.md`* — bug documents record the tree of their time.

## 4. Detailed Design

LOC figures: `python3 -I /tmp/loc/span.py <file> <a> <b>` over the stated ranges (sums over
several ranges by the U map's wrapper). "+hdr" = the part's own `use` lines and `impl
CodeBuilder<'_> { }` wrapper, a few LOC (guess). Re-measure every new file with
`python3 -I tools/loc-census/census.py --file <path>` before committing.

### 4.1 `control/builder_control.rs` → `control/builder_control/`

| New file | Code LOC | Contents (lines as of 17a9b5799) |
|---|---|---|
| `builder_control/mod.rs` | 234 + 103 test | 1–134 (uses, `TransferTemps`, `FieldStoreKind`, `SpilledValue`, `FieldNested`, `FieldReplace`, `FieldOverwrite`, impl opener); 320–378 (`transfer_temp_disposition`, `forget_pending_temps_to`, `lower_ops`); 5769 (impl closer); 5770–5923 (`string_self_append_operands_of`, `nir_value_reads_local`, `nir_op_context`, `nir_value_context`, `is_inline_trap_delivery_slot`); 5924–6058 `#[cfg(test)] mod transfer_temp_classification_tests` (stays with `transfer_temp_disposition`: it reads its own source); part `mod` declarations |
| `builder_control/store_global.rs` | 146 | 135–319 `lower_store_global` |
| `builder_control/field_with.rs` | 704 | 379–1180 (`try_inplace_scalar_fields`, `emit_field_stores`, `copy_into_record_temp`, `lower_self_update_item`, `spill_scalar_record_literal`, `lower_with_update_in_record_temp`, `replace_field_admitted`, `nested_scalar_with`); 2507–2668 (`record_collection_last_inlined`, `field_self_update_site`, `peel_field_path`) |
| `builder_control/element_update.rs` | 563 | 1181–1607 (`element_replace_admitted`, `emit_element_replace_or_write_back`, `emit_into_result_is_owner`, `element_into_call`, `later_value_misses_destination`, `element_write_back_item`, `element_nested_write_back_value`, `bind_hidden_value`); 2111–2386 (`append_headroom_bind`, `install_borrow_gets`, `apply_rebound`, `lower_element_update`, `try_single_expression_element_update`) |
| `builder_control/field_replace.rs` | 416 | 1608–2110 (`emit_field_replace_or_rebuild`, `emit_field_owner_install`, `spill_field_overwrite`, `record_field_is_inlined_fixed`, `emit_field_record_block`, `emit_field_owner_block`, `try_inplace_mixed_with`, `with_target_is_owner`) |
| `builder_control/value_effects.rs` | 91 | 2387–2506 (`field_arm_may_raise`, `nir_value_is_effect_free`, `nir_value_cannot_fail`, `nir_comparison_promotion_cannot_fail`, `nir_call_is_infallible_builtin`) |
| `builder_control/statement_dispatch.rs` | 1313 + hdr (Phase 3); ≈ 432 after Phase 5 (guess) | 2669–4605 `lower_ops_inner` (doc comment at 2669, `fn` at 2670) |
| `builder_control/loops.rs` | 431 | 4606–5040 (`resolve_len_list`, `emit_branch_unless`, `fusable_comparison_tail`, `lower_loop_condition`, `lower_loop_body`, `resolve_for_local`, `recognize_provable_index`, `is_provable_index_access`, `numeric_value_as`, `lower_numeric_for`); 5179–5292 (`closures_created_in`, `begin_loop_promotion`, `end_loop_promotion`, `update_promoted_float`) |
| `builder_control/string_shadow.rs` | 92 | 5041–5178 (`string_capacity_slot_for`, `string_shadow_working_copy`, `reset_shared_string_shadow`, `reset_string_capacity_shadow`, `prescan_string_self_appends`) |
| `builder_control/for_each.rs` | 402 | 5293–5768 `lower_for_each` |
| `builder_control/bind_op.rs` | ≈ 495 (guess; Phase 5) | `lower_bind_op` (§4.10) |
| `builder_control/assign_op.rs` | ≈ 415 (guess; Phase 5) | `lower_assign_op`, `lower_state_assign_op` (§4.10) |

Surface: no new re-export (free `pub(crate)` fns and `TransferTemps` stay in `mod.rs`).
Visibility, 22 private methods → `pub(super)`: `lower_store_global`, `emit_field_stores`,
`copy_into_record_temp`, `replace_field_admitted`, `element_replace_admitted`,
`emit_element_replace_or_write_back`, `emit_into_result_is_owner`, `element_into_call`,
`emit_field_replace_or_rebuild`, `spill_field_overwrite`, `emit_field_record_block`,
`emit_field_owner_block`, `try_inplace_mixed_with`, `append_headroom_bind`, `apply_rebound`,
`lower_element_update`, `try_single_expression_element_update`, `field_arm_may_raise`,
`nir_call_is_infallible_builtin`, `lower_ops_inner`, `lower_loop_condition`, `lower_loop_body`.

Hazards:
- Inline test includes: `include_str!("builder_control.rs")` → `include_str!("mod.rs")`;
  `include_str!("../../../target/shared/nir/mod.rs")` →
  `include_str!("../../../../target/shared/nir/mod.rs")`. Module name
  `transfer_temp_classification_tests` does not collide with a `tests` directory.
- Spec citations (19), Phase 3 destinations: `element_into_call` (memory/06_native-calling-convention:142),
  `emit_element_replace_or_write_back`, `element_replace_admitted`, `lower_element_update`
  (memory/05_collections 941/940/926) → `element_update.rs`; `record_field_is_inlined_fixed`,
  `emit_field_replace_or_rebuild`, `try_inplace_mixed_with` (05: 886/881/846) →
  `field_replace.rs`; `replace_field_admitted`, `nested_scalar_with`, `try_inplace_scalar_fields`,
  `field_self_update_site`, `lower_with_update_in_record_temp` (05: 880/838/830/823/654) →
  `field_with.rs`; `lower_for_each` ×2 (05:745, language/10_control-flow:43) → `for_each.rs`;
  `lower_numeric_for` ×2 (10:39, 10:30), `numeric_value_as` (10:22) → `loops.rs`;
  `lower_ops_inner` (language/15_resource-management:155), `live_resource_aliases`
  (memory/04_arenas:423) → `statement_dispatch.rs` in Phase 3, then `bind_op.rs` in Phase 5
  (§4.10).
- `.ai` and code-comment repoints: §4.13.

### 4.2 `builder/mod.rs` → `builder/mod.rs` + four siblings

Siblings `code_impl.rs` (392) and `builder_emit_helpers.rs` (619) are untouched.

| File | Code LOC | Contents (lines as of 17a9b5799) |
|---|---|---|
| `builder/mod.rs` | 538 + ≈ 8 decl lines | 1–1510: imports, `mod code_impl` / `mod builder_emit_helpers`, `EmitCtx`, `HelperBody`, `HelperResult`, `struct CodeBuilder` + impl, state types `ElementUpdates` … `TypeModel` |
| `builder/lower_module.rs` | 1864 (Phase 4) = 1813 [1511–3975] + 51 [4909–4974]; ≈ 460 after Phase 7 (guess) | `lower_module_for_platform`, `lower_module_mir_for_platform`, private `variant_demand` (its sole caller is `lower_module_for_platform`). Carries the 3-LOC `#[cfg(test)]` statement at 2115–2117 (inside the fn) |
| `builder/runtime_helpers.rs` | 573 | 3976–4713: `lower_runtime_helper` (with its `#[allow(clippy::too_many_arguments)]`), `is_tls_server_symbol`, `emit_alloc`, `emit_arena_free`, `internal_branch`, `external_branch`, `lower_map_build_buckets_helper`, `lower_map_bucket_put_helper`, `lower_string_concat_helper`, `lower_map_probe_helper` |
| `builder/link_resolution.rs` | 66 | 4714–4828: `resolve_closer_symbol`, `resolve_link_libraries`, `bind_deferred_relocation_libraries` |
| `builder/error_messages.rs` | 72 | 4829–4908: `native_link_error_messages`, `standard_error_messages`, `standard_error_message_symbol` |
| `builder/module_layout.rs` | ≈ 560 (guess; Phase 7) | `ModuleLayout`, `module_data_objects`, `plan_arena_layout`, `emit_program_entries` (§4.12) |
| `builder/module_helpers.rs` | ≈ 500 (guess; Phase 7) | `CoreHelperFlags`, `lower_graph_and_closure_helpers`, `lower_core_runtime_helpers`, `lower_runtime_helpers_and_threads` (§4.12) |
| `builder/module_finish.rs` | ≈ 600 (guess; Phase 7) | `add_companion_runtime_symbols`, `lower_relocation_gated_helpers`, `lower_canvas_and_link_support`, `add_relocation_gated_data_objects` (§4.12) |

Surface: parts are declared PRIVATE in `builder/mod.rs` (`mod lower_module; mod runtime_helpers;
mod link_resolution; mod error_messages;`, and in Phase 7 `mod module_layout; mod module_helpers;
mod module_finish;`) with explicit re-exports:

```rust
pub(crate) use lower_module::{lower_module_for_platform, lower_module_mir_for_platform};
pub(crate) use runtime_helpers::{lower_runtime_helper, is_tls_server_symbol, emit_alloc, emit_arena_free,
    internal_branch, external_branch, lower_map_build_buckets_helper, lower_map_bucket_put_helper,
    lower_string_concat_helper, lower_map_probe_helper};
pub(crate) use link_resolution::{resolve_closer_symbol, resolve_link_libraries, bind_deferred_relocation_libraries};
pub(crate) use error_messages::{native_link_error_messages, standard_error_messages, standard_error_message_symbol};
```

Visibility changes: 0 (all moved fns are already `pub(crate)`; all 113 `CodeBuilder` fields are
`pub(crate)`; `variant_demand` moves with its caller).

Hazards:
- `builder/mod.rs` keeps its path, so the checker does NOT force this sweep. Phase 4 repoints
  by this table (23 citations, listed by `grep -rnoE "\[\[src/codegen/engine/builder/mod.rs[^]]*\]\]" src/docs/spec`):
  → `lower_module.rs`: `lower_module_for_platform` ×10 (architecture/07_flows:63,
  architecture/spec:64, linker/01_pipeline:76, linker/02_object-plan:33,
  linker/03_import-selection:41, memory/08_program-startup:89, threading/06_thread-runtime-helpers:49,108,
  threading/09_os-integration:107, tooling/09_debug-report:45), `lower_module_mir_for_platform`
  (architecture/17_mir-instruction-set:241), `lower_module` (app/05_presentation-mode:73,
  written `[[…/lower_module.rs:lower_module_for_platform]]`), `arena_global_slots`
  (language/16_threads:31), `referenced_unicode_tables` (unicode/01_tables-and-algorithms:252);
  → `runtime_helpers.rs`: `lower_map_probe_helper` (memory/05_collections:229);
  → `link_resolution.rs`: `bind_deferred_relocation_libraries` (app/02_linux-runtime:403);
  stay `mod.rs`: `ClosureEnvWord` (memory/09_closures:187), `OwnedValueCleanup`
  (memory/04_arenas:401), `StatementTemps` (memory/04_arenas:481).
  Whole-file `[[src/codegen/engine/builder/mod.rs]]`: threading/09_os-integration:179 and
  app/05_presentation-mode:117 describe code inside `lower_module_for_platform` →
  `[[src/codegen/engine/builder/lower_module.rs]]`; linker/01_pipeline:18 and
  architecture/06_native:368 stay. Phase 7 repoints again (§4.12).
- `.ai/testing-gates.md:136` (`standard_error_messages()` at `builder/mod.rs`) →
  `builder/error_messages.rs`.
- Guards: `tests/guards/no_type_strings.rs:428–436` (+ self-test strings, in `tests/guards/no_type_strings/self_checks.rs` after plan-176-E: `grep -n TypeModel` there) name
  `TypeModel` fields declared in `builder/mod.rs`; `TypeModel` stays there — no change.
  `architecture_guards.rs:257`, `no_operator_strings.rs:129,625` name `code_impl.rs` (untouched).

### 4.3 `value/builder_values.rs` → `value/builder_values/`

| New file | Code LOC | Contents (lines as of 17a9b5799) |
|---|---|---|
| `builder_values/mod.rs` | 68 + 4 `mod` lines; 63 test | 1–91 (uses 2–18; `impl<'a> CodeBuilder<'a> { inline_abi_ctx }` 20–49; `flatten_concat_spine` 50–70 — stays here: `operand_snapshot.rs:100` calls `super::builder_values::flatten_concat_spine`; `EscapingValue` 71–90); 4692–4703 `value_loc`; 4705–4781 `#[cfg(test)] mod concat_spine_tests` |
| `builder_values/pending_temps.rs` | 628 + hdr | 93–1186: `lower_value` entry and `register_pending_temp` … `drop_interior_temps_before_branch` (pending / union-box / resource / statement / fresh-string temps; claim, drop, free) |
| `builder_values/ownership.rs` | 492 + hdr; 48 test | 1187–2125: `owned_param_bind_moves` … `record_closure_env_layout` (`lower_value_owned`, owning-copy and string-provenance classification, runtime-result ownership predicates, `value_aliases_live_resource`, `target_returns_borrowed_resource`, `needs_graph_copy`, `owns_graph`, `lower_value_stored*`, shallow temps, `release_moved_source`, `closure_env_word`, `record_closure_env_layout`); 4783–4852 `#[cfg(test)] mod borrowed_resource_tests` (tests `value_aliases_live_resource`) |
| `builder_values/lower_inner.rs` | 1224 + hdr (Phase 3); ≈ 505 after Phase 6 (guess) | 2126–3699 `lower_value_inner` (fn 2126–3679; 3680–3699 is the next fn's doc comment, which moves with `lower_checked_value` to `inline_calls.rs`). Needs `use super::flatten_concat_spine;` (Binary arm, 3618) |
| `builder_values/inline_calls.rs` | 643 + hdr | 3700–4690: `lower_checked_value`, `lower_inline_conversion_raw`, `try_abi_inline_lower`, `lower_abi_inline_args`, `try_mfb_fast_path`, `lower_inline_builtin_raw`, `lower_inline_infallible_raw`, `claim_raw_member_result_for_unboxed_trap`, `lower_infallible_member`, `net_*_form` predicates, `lower_runtime_helper_call`, `lower_runtime_helper_call_copied` |
| `builder_values/lower_call.rs` | ≈ 395 (guess; Phase 6) | `lower_call_value`, `lower_call_result_value` (§4.11) |
| `builder_values/lower_construct.rs` | ≈ 355 (guess; Phase 6) | `lower_closure_value`, `lower_union_wrap_value` (§4.11) |

The doc comment at 3680–3699 belongs to the item after it; move it with that item.

Surface: no new re-export (`value/mod.rs` keeps `pub(crate) use builder_values::*;`; free items
stay in `mod.rs`). Visibility, 9 private methods → `pub(super)`:
`register_raw_member_result_temp` (pending_temps → inline_calls), `call_returns_fresh_string`
(ownership → pending_temps), `lower_value_inner` (lower_inner → pending_temps),
`lower_checked_value`, `lower_inline_conversion_raw`, `try_abi_inline_lower`, `try_mfb_fast_path`,
`lower_inline_builtin_raw`, `lower_inline_infallible_raw` (inline_calls → lower_inner).
Tests: both inline test modules are < 250 inline test LOC and stay inline; 6 `#[test]` before = 6 after
(4 in `mod.rs`, 2 in `ownership.rs`).

Hazards — spec citations (28 = 27 symbol + 1 whole-file), Phase 3 destinations:
→ `pending_temps.rs`: `lower_value` ×6, `adopt_statement_temp` ×2, `emit_statement_temp_drop`,
`register_union_box_temp`, `register_resource_temp`, `pending_temp_is_freeable`;
→ `ownership.rs`: `needs_graph_copy` ×2, `owns_graph`, `value_is_runtime_managed`,
`runtime_call_result_is_copied_at_call_site` ×2, `value_aliases_live_resource`,
`target_returns_borrowed_resource`, `closure_env_word`, `record_closure_env_layout`;
→ `lower_inner.rs`: `NirValue::FunctionRef` (memory/09_closures:22; that arm stays in the
remainder after Phase 6);
→ `inline_calls.rs`: `lower_abi_inline_args`, `net_write_payload_form`, `lower_runtime_helper_call` ×2;
whole-file `[[src/codegen/engine/value/builder_values.rs]]` (threading/06_thread-runtime-helpers:80,
worker-direction split in `lower_runtime_helper_call`) → `[[src/codegen/engine/value/builder_values/inline_calls.rs]]`.

### 4.4 `function/function_lowering.rs` → `function/function_lowering/`

| New file | Code LOC | Contents |
|---|---|---|
| `function_lowering/mod.rs` | 468 + ≈ 10 decl lines | 1–18 (uses); 990–1597 (`OwnedVariant` struct + impl — the `#[allow(clippy::too_many_arguments)]` at 991 moves with it — and `lower_function`, 433) |
| `function_lowering/local_analysis.rs` | 146 + hdr | 19–276 (`expanded_nir_union_variants`, `collect_address_taken_locals`, `collect_reassigned_locals`, `function_returns_param_borrow`, `param_borrow_returns`, `function_returns_fresh_string`) |
| `function_lowering/trap_analysis.rs` | 225 + hdr | 295–588 (`is_trap_discard_conversion`, `trap_discard_error_results`, `unboxed_trap_result_candidates`, `trap_delivery_moves`) |
| `function_lowering/promotion_analysis.rs` | 376 + hdr | 277–294 (`collect_value_local_reads`, used only by `scan_loop_locals`); 589–989 (`promotable_vector_locals`, `is_vector_native_producing`, `collect_vector_native_bindings`, `collect_assigned_locals`, `ops_write_local`, `ops_write_state`, `mark_vector_escaping_value/ops`, `scan_loop_locals`) |
| `function_lowering/helper_lowering.rs` | 492 + hdr | 1598–2158 (`lower_builtin_function_wrapper`, `lower_abi_function_helper`, `lower_thread_copy_function`, `helper_code_builder`, `finish_helper_function`) |

Surface: `mod.rs` declares `mod local_analysis; mod trap_analysis; mod promotion_analysis; mod
helper_lowering;` and `pub(crate) use local_analysis::*; pub(crate) use promotion_analysis::*;
pub(crate) use helper_lowering::*;` plus `use trap_analysis::{trap_discard_error_results,
unboxed_trap_result_candidates, trap_delivery_moves};`. Those 3 private fns → `pub(super)`
(called only from `lower_function`, 1418–1421). `OwnedVariant` stays in `mod.rs`, so
`crate::codegen::engine::function::function_lowering::OwnedVariant` (used in `builder/mod.rs`)
still resolves.

Hazards: citations 6 — `lower_function` ×3 (memory/06:49, language/16_threads:84,
threading/07_control-block:59) and `OwnedVariant` (memory/06:74) → `mod.rs`;
`param_borrow_returns` (memory/06:109) → `local_analysis.rs`; `unboxed_trap_result_candidates`
(memory/03_heap-values:170) → `trap_analysis.rs`. `.ai` repoints: §4.13.

### 4.5 `function/entry.rs` → `function/entry/`

| New file | Code LOC | Contents |
|---|---|---|
| `entry/mod.rs` | 543 + decl lines | 1–791 (uses, `lower_program_entry` 532) |
| `entry/process_setup.rs` | 142 + hdr | 792–996 (`DEV_NULL_PATH_SYMBOL` with the hwcap doc block 793–802 attached, `dev_null_path_data_object`, aliases `AddressOfDevNull`/`CallLibc`, `emit_reserve_standard_fds`, `emit_riscv_hwcap_probe`, `emit_default_arena_start_time`) |
| `entry/entry_args.rs` | 160 + hdr | 997–1170 `emit_entry_args_list_materialization` |
| `entry/cleanup_audit.rs` | 216 + hdr | 1171–1430 (`emit_cleanup_failure_audit_report`, `emit_write_string_object`, `emit_write_integer_to_stderr`, `emit_write_integer_to_stderr_with_labels`) |

Surface: `pub(crate) use process_setup::*;` (`src/target/linux_gtk/bootstrap.rs:6` and
`src/target/macos_aarch64/app/bootstrap.rs:5` import `function::entry::{emit_reserve_standard_fds,
DEV_NULL_PATH_SYMBOL}`; `builder/mod.rs:1681` calls `function::entry::dev_null_path_data_object`).
5 private → `pub(super)`: `emit_riscv_hwcap_probe`, `emit_entry_args_list_materialization`,
`emit_cleanup_failure_audit_report`, `emit_write_string_object`, `emit_write_integer_to_stderr`
(all called from `lower_program_entry`), imported by `use` in `entry/mod.rs`.

Hazards: citations 18 — `lower_program_entry` ×14 → `entry/mod.rs` (tooling/01:474; app/05:78;
memory/08_program-startup:14,28,43,115,174,211,217,227; language/08:252; language/13:65;
linker/05:66; linker/09:80); `emit_reserve_standard_fds` ×3 → `process_setup.rs` (app/01:26,
app/02:68, memory/08:139); `emit_entry_args_list_materialization` → `entry_args.rs`
(memory/08:221).

### 4.6 `operators/builder_numeric.rs` → `operators/builder_numeric/`

One `impl CodeBuilder<'_>` at 18–1923; the largest method `lower_comparison_binary` is < 346.

| New file | Code LOC | Contents |
|---|---|---|
| `builder_numeric/mod.rs` | ≈ 5 | doc 1–7 + `mod` declarations |
| `builder_numeric/operator_lowering.rs` | 417 + hdr | 19–442 (boolean / short-circuit / xor, `lower_arithmetic_binary`, `lower_float_arithmetic_dnative`, `lower_numeric_unary_negation`); 823–903 (`integer_sub_elidable`, `integer_add_elidable`, `guard_lower_bound`) |
| `builder_numeric/comparison.rs` | 346 + hdr | 443–822 (`lower_comparison_binary`, `lower_numeric_comparison_binary`) |
| `builder_numeric/binary_emit.rs` | 314 + hdr | 904–1305 (`emit_integer_binary`, `emit_integer_binary_checked`, `emit_fixed_binary`, `emit_float_binary`, `float_is_dnative`, `float_value_as_gpr`, `materialize_float`, `store_value_at`, `operand_as_double`) |
| `builder_numeric/numeric_kernels.rs` | 495 + hdr | 1306–1923 (overflow / checked / division checks, `emit_min_i64_negation_check`, `emit_integer_pow`, Fixed mul/div/q32/pow, abs/neg, `load_numeric_as_double/fixed`, `emit_f64_const`, `emit_float_pow`) |

Surface: no re-export (inherent methods). 1 private → `pub(super)`: `emit_min_i64_negation_check`
(called at 397/403/412, in `operator_lowering.rs`).

Hazards: citations 9 — `emit_fixed_binary` (language/11_operators:44), `emit_integer_binary`
(stdlib/13_money:88) → `binary_emit.rs`; `emit_float_fmod` (11:44) →
`[[src/codegen/builtins/math/gen_fmod.rs:emit_float_fmod]]` (its definition, line 20; the
builder only calls it; `gen_fmod.rs` is in no plan-176 letter: `grep -c gen_fmod /tmp/loc/groups.json` → 0);
`emit_integer_pow`, `emit_fixed_pow`, `emit_float_pow` (11:56–58), `load_numeric_as_double`
(language/04_types:136) → `numeric_kernels.rs`; `lower_numeric_unary_negation` (11:59) →
`operator_lowering.rs`; `lower_numeric_comparison_binary` (04:107) → `comparison.rs`.

### 4.7 `analysis/borrow_get.rs` → `analysis/borrow_get/`

| New file | Code LOC | Contents |
|---|---|---|
| `borrow_get/mod.rs` | 535 | 1–260 (uses, `ContainerRef` + impl, `ElementBinding`, `ElementSite`, `ElementBinding` impl, `element_update_with`, `BorrowGets`, `Rebound`, `ReboundRole`, `BorrowGets` impl, `copied_item`); 878–1339 (`collect_borrow_gets` and the bug-784 rename machinery it drives: `rebound_names`, `RenameState` + impl, `rename_ops`, `rename_value`, `pair_ops`, `for_each_bind`); `mod predicates; mod by_name;`; the `#[cfg(test)] mod tests;` line plan-176-H left |
| `borrow_get/predicates.rs` | 403 + hdr | 261–719 (`op_key`, `builtin`, `get_call`, `pure_index`, `same_index`, `index_locals`, `write_back_of`, `is_with_over`, `for_each_block`, `Reads`, `reads_in`, `writes_any`, `local_reads_are_lookups`, `container_untouched`, `single_expression_update`, `is_element_get`, `children_mut`, `bind_single_expression_element`) |
| `borrow_get/by_name.rs` | 435 + hdr | 720–877 (`SiteCheck`, `SiteTally`, `impl SiteCheck` — used only by `collect_borrow_gets_by_name`); 1340–1703 `collect_borrow_gets_by_name` (304) |
| `borrow_get/tests/mod.rs` | — | plan-176-H's test-only file, unchanged; `mod tests;` in `borrow_get/mod.rs` resolves to the same file |

(`sum.py borrow_get.rs 1-260 878-1339` = 535; `261-719` = 403; `720-877 1340-1703` = 435.)

Surface: `mod.rs` has `use predicates::*; use by_name::collect_borrow_gets_by_name;` (private) and
`pub(crate) use predicates::{pure_index, same_index, single_expression_update,
bind_single_expression_element};` (`builder_control.rs:2314` imports
`borrow_get::{bind_single_expression_element, single_expression_update, …}`; the other external
names — `BorrowGets`, `ContainerRef`, `ElementBinding`, `Rebound`, `ReboundRole`,
`collect_borrow_gets`, `copied_item`, `element_update_with` — stay in `mod.rs`;
`grep -rn "borrow_get::" src`). The tests reach moved private fns through `use super::*` and the
private glob. Visibility edits, 15 → `pub(super)`: `op_key`, `builtin` (called in `mod.rs` at
176/252), `get_call`, `index_locals`, `write_back_of`, `is_with_over`, `for_each_block`,
`reads_in`, `writes_any`, `container_untouched`, `Reads` and its 3 fields `all`, `member`,
`with_target` (read at 1587/1593 in `by_name`), `collect_borrow_gets_by_name` (called from
`mod.rs` at 901/921).

Hazards: citations 4 — `collect_borrow_gets` ×3 (memory/05_collections:613,621,646),
`ElementBinding` (05:918) → `borrow_get/mod.rs`. `.ai/collections.md:592` →
`borrow_get/mod.rs`. `no_type_strings.rs:380,403` doc comments: §4.13.

### 4.8 `analysis/module_analysis.rs` → `analysis/module_analysis/`

| New file | Code LOC | Contents |
|---|---|---|
| `module_analysis/mod.rs` | 611 | 1–357 (uses, empty-string constant, type-name / call / migrated / canvas / cleanup-failure predicates, `CANVAS_SCENE_USERS`, `module_uses_any_call`, `module_field_types`); 589–834 (`ops_use_call`, `value_uses_call`, `collect_function_value_refs`, `ops_use_type_name`, `value_uses_type_name`); 1131–1217 (`value_may_return_invalid_format`); `mod float_errors; mod unicode_tables;` |
| `module_analysis/float_errors.rs` | 218 + hdr | 358–588 (`module_may_emit_float_numeric_error`, `ops_may_emit_float_arithmetic_error`, `value_may_emit_float_arithmetic_error`) |
| `module_analysis/unicode_tables.rs` | 293 + hdr | 835–1130 (`module_uses_unicode_runtime_tables`, `ops_use_unicode_runtime_tables`, `value_uses_unicode_runtime_tables`) |

Surface: `mod.rs` `pub(crate) use float_errors::*; pub(crate) use unicode_tables::*;`
(`analysis/mod.rs` keeps `pub(crate) use module_analysis::*;`). Visibility changes 0.

Hazards: citations 2 — `collect_function_value_refs` (memory/09_closures:38) → `mod.rs`;
`module_uses_unicode_runtime_tables` (unicode/01:251) → `unicode_tables.rs`. Line 83's stale
comment: §4.13.

### 4.9 `convert/builder_conversions.rs` → `convert/builder_conversions/`

One `impl CodeBuilder<'_>` at 15–1460.

| New file | Code LOC | Contents |
|---|---|---|
| `builder_conversions/mod.rs` | 518 | 1–15, 860–1460 (`lower_to_float`, `lower_to_fixed`, `lower_to_money`, `lower_is_numeric`, `lower_integer_parity_predicate`, `lower_numeric_filter_predicate`, `lower_empty_filter_predicate`, `emit_integer_to_fixed_value`, `emit_float_bits_to_fixed_value`, `emit_parse_decimal_string_to_double`, `emit_float_exponent_range_guard`, `emit_double_overflow_check`); `mod to_int; mod to_byte_scalar;` |
| `builder_conversions/to_int.rs` | 441 + hdr | 16–537 (`lower_to_int`, `emit_fixed_to_int_value`, `emit_money_to_int_value`, `emit_float_to_int_value`, `emit_string_to_int_sign_prologue`, `emit_int_parse_cutoff_guard`, `emit_int_parse_sign_epilogue`, `emit_string_to_int_value`, `emit_string_to_int_value_base`) |
| `builder_conversions/to_byte_scalar.rs` | 248 + hdr | 538–859 (`lower_to_byte`, `lower_to_scalar`, `emit_continuation_byte_check`, `emit_string_to_scalar_value`, `emit_scalar_to_string_value`) |

Surface: inherent methods, no re-export; visibility changes 0 (`emit_continuation_byte_check`
stays with its caller).

Hazards: citations 8 — `lower_to_int` ×2 (language/18_builtin-functions:70,162),
`emit_money_to_int_value` (stdlib/13_money:89) → `to_int.rs`; `lower_to_byte` (13_money:90) →
`to_byte_scalar.rs`; `emit_integer_to_fixed_value`, `emit_float_bits_to_fixed_value`
(language/04_types:123,124), `lower_to_money`, `lower_to_fixed` (13_money:79,80) → `mod.rs`.

### 4.10 Extract-method: `lower_ops_inner` (Phase 5)

Source after Phase 3: `builder_control/statement_dispatch.rs`, `fn lower_ops_inner(&mut self,
ops: &[NirOp], cleanup_scope_start: usize) -> Result<(), String>`, 1313 LOC (lines 2670–4605 of
`builder_control.rs` as of 17a9b5799; `span.py … 2670 4605`).

**Structure.**

| Region | Lines | LOC | Where |
|---|---|---|---|
| Prologue: `zero_slot` mint (2671), `for op in ops`, pre-op bound clear (2681–2692), `temp_watermark`, `enclosing_op_key`, `expansion::enter`, `enclosing_temp_exempt` | 2670–2734 | 44 | outer `for` body |
| Arm `Bind` | 2735–3519 (body 2741–3518) | 488 (body 481) | closure |
| Arm `StoreGlobal` | 3520–3545 | 16 | closure |
| Arm `Assign` | 3546–4000 (body 3547–3999) | 310 (body 308) | closure |
| Arm `StateAssign` | 4001–4166 (body 4002–4165) | 90 (body 88) | closure |
| Arms `Eval`, `Return`, `ExitLoop`, `ContinueLoop`, `ExitProgram`, `Fail`, `If` (30), `Match` (89), `While` (38), `For`, `DoUntil` (24), `ForEach`, `Trap` (37) | 4167–4513 | 294 | closure |
| Closure tail `Ok(())`, `expansion::exit`, restores, `map_err` + `?`, graph-drop hook, post-op bound invalidation, Assign float, temp retire, `reset_temporary_registers`; after the loop: scope cleanup drain + reclaims | 4514–4605 | 71 | outer |

The `match op` (2734–4514) is inside the immediately-invoked closure
`let result = (|| -> Result<(), String> { match op { … } Ok(()) })();`. Its result is
post-processed at 4517–4520 however the closure exited.

**Control-flow audit.** Inside the closure, `return X` and `?` leave the closure only; the code
after it (4517–4568) always runs. The `match` is a statement followed by `Ok(())` (4515).
- Bind (2741–3518): `return` at 2748 (`self.lower_element_update(..)`), 2826
  (`self.lower_unboxed_trap_bind(..)`), 3113 (`return Ok(())`, promote-vector path). `?` at
  2765, 3090, 3095, 3144 (`r?`), 3147, 3158 (`r?`), 3160, 3166, 3177, 3198, 3205, 3240, 3242,
  3269, 3271, 3353, 3429, 3517. No `break`/`continue`/labels. Falls through at 3518.
- Assign (3547–3999): `return Ok(())` at 3562, 3568, 3588; `return self.lower_element_update(..)`
  3565. `?` at 3567, 3580, 3587, 3593, 3661, 3665, 3668, 3677, 3683, 3702, 3712, 3754, 3790,
  3832, 3889, 3932, 3952, 3966. Falls through at 3999.
- StateAssign (4002–4165): `return Ok(())` at 4008, 4029, 4040. `?` at 4006, 4028, 4039, 4048,
  4099, 4102, 4110, 4115, 4163. Falls through at 4165.
- None of the three reads `temp_watermark` or `cleanup_scope_start` (only `Return` 4190, `Fail`
  4229 and the epilogue 4573 do). `zero_slot` is read only in Bind (3042–3043).

Exit mapping: each helper returns `Result<(), String>`. Every `return X` in a moved body stays
`return X` (now returning from the helper, same type); every `?` stays `?`; each helper body
gets a trailing `Ok(())` (the fall-through value). The arm in the closure becomes
`NirOp::Bind { .. } => self.lower_bind_op(op, zero_slot)?,` (and likewise for the other two).
`?` inside the closure returns `Err(e)` from the closure exactly as the inline `?`/`return Err`
did; `Ok(())` falls to the closure's `Ok(())`. The error is still wrapped by
`map_err(… nir_op_context(op))` because the call is still inside the closure.

**Shared state.** `op: &NirOp` (borrows `ops`, not `self`; Bind reads `op_key(op)` 2744, 2903
and `append_headroom_bind(op, ..)` 3151; Assign reads `op_key(op)` 3560 and
`try_single_expression_element_update(op, value)` 3567) — passed as `op: &NirOp`, also the
destructure source. `zero_slot: VirtualRegister` (Copy, minted once at 2671) — passed by value
to `lower_bind_op`; the body text `&zero_slot` is unchanged; not re-minted. Arm fields are
re-destructured from `op` by let-else:

```rust
let NirOp::Bind { mutable, name, type_, value } = op else {
    unreachable!("lower_bind_op is called from the NirOp::Bind arm only")
};
```

No arm-local is read after its arm. Fewer than 6 crossing locals, so no struct.

**Emission order.** The call sits at the exact position of the inline body, inside the same
closure, after the same prologue and before the same epilogue. Bodies are verbatim, so every
vreg/label/stack-slot counter advances identically. No clone is introduced. The only new code
is the never-taken `unreachable!` branch, which emits nothing into the program.

**New methods.**

| Method | Signature | File | LOC (guess) |
|---|---|---|---|
| `lower_bind_op` | `pub(super) fn lower_bind_op(&mut self, op: &NirOp, zero_slot: VirtualRegister) -> Result<(), String>`; let-else `NirOp::Bind { mutable, name, type_, value }` | `builder_control/bind_op.rs` | 490 |
| `lower_assign_op` | `pub(super) fn lower_assign_op(&mut self, op: &NirOp) -> Result<(), String>`; let-else `NirOp::Assign { name, value }` | `builder_control/assign_op.rs` | 315 |
| `lower_state_assign_op` | `pub(super) fn lower_state_assign_op(&mut self, op: &NirOp) -> Result<(), String>`; let-else `NirOp::StateAssign { resource, value }` | `builder_control/assign_op.rs` | 95 |
| `lower_ops_inner` (remainder) | unchanged signature, `pub(super)` | `builder_control/statement_dispatch.rs` | 428 |

The arm form is `=> self.lower_<x>_op(op, …)?,`. Each new method gets a one-line doc comment
naming the arm it lowers and that it runs inside `lower_ops_inner`'s per-op closure (so its `?`
is attributed to the op by `nir_op_context`). `builder_control/mod.rs` adds `mod bind_op; mod
assign_op;`. The §4.1 `pub(super)` list already covers every private callee of the moved bodies
(`lower_element_update`, `apply_rebound`, `append_headroom_bind`, `copy_into_record_temp`,
`try_single_expression_element_update`, `try_inplace_mixed_with`; the others are already
`pub(crate)`). Free fns `is_inline_trap_delivery_slot` (Bind 3183) and
`string_self_append_operands_of` (Assign 3607) stay in `mod.rs` and reach the parts via
`use super::*;`.

**Borrow checker.** Inside the closure the arm bodies borrowed `self` through the closure's
unique capture; as `&mut self` methods they are less constrained. `op` borrows `ops`, not
`self`. Bind 3321–3343 (`self.active_cleanups.iter_mut().rev().find_map(..)` holding
`&mut owner.state_type`) and 3415–3424 keep their exact shape inside one method. The closure now
captures only `self`, `op`, `zero_slot`, `temp_watermark`. rustfmt reflows the dedented bodies
(12 fewer leading spaces), which only lowers LOC.

**Repoints this phase makes** (on top of Phase 3's):
`language/15_resource-management.md:155` `[[…/statement_dispatch.rs:lower_ops_inner]]` (alias
classification of a bind) → `[[src/codegen/engine/control/builder_control/bind_op.rs:lower_bind_op]]`;
`memory/04_arenas.md:423` `[[…/statement_dispatch.rs:live_resource_aliases]]` (Bind 3310) →
`[[src/codegen/engine/control/builder_control/bind_op.rs:live_resource_aliases]]`;
`.ai` and comments per §4.13 rows marked "Phase 5".

### 4.11 Extract-method: `lower_value_inner` (Phase 6)

Source after Phase 3: `builder_values/lower_inner.rs`, `fn lower_value_inner(&mut self, value:
&NirValue) -> Result<ValueResult, String>`, 1224 LOC (lines 2126–3679 of `builder_values.rs` as
of 17a9b5799).

**Structure.** Not a closure: a prologue, then `match value { … }` (2140–3678), which is the
function's tail expression.

| Region | Lines | LOC |
|---|---|---|
| Prologue: `scratch9_reg`, `scratch10_reg` minted (2127–2128), refs `scratch9`/`scratch10`, static-string early return (2131–2139) | 2126–2139 | 14 |
| `Const` | 2141–2159 | 15 |
| `Local` | 2160–2230 | 54 |
| `LocalRef` | 2231–2256 | 20 |
| `Global` | 2257–2286 | 27 |
| `FunctionRef` | 2287–2326 | 33 |
| `Closure` (capture-less guard arm, recursion) | 2327–2339 | 10 |
| **`Closure` (capturing)** | 2340–2619 (body 2345–2618) | **246** |
| `Capture` | 2620–2637 | 14 |
| **`Call`** | 2638–2833 (body 2639–2832) | **139** |
| **`CallResult`** | 2834–3147 (body 2835–3146) | **242** |
| `RuntimeCall` | 3148–3197 | 35 |
| `Constructor` | 3198–3313 | 54 |
| **`UnionWrap`** | 3314–3429 (body 3319–3428) | **101** |
| `UnionExtract`, `ResultIsOk`, `ResultValue`, `ResultError`, `Checked`, `WithUpdate`, `MemberAccess`, `Binary`, `Unary`, literals | 3430–3679 | 219 |

**Control-flow audit.** `return X` leaves the function and an arm's tail expression is the
function's result, so for an arm moved into a helper with the same return type both map 1:1. No
`break`/`continue`/labels in the four arms (the `.map(|variant| …)` at 3372 and `.or_else`
closures are value closures with no `return`).
- Closure (capturing): `return Err(..)` at 2387, 2529, 2554; `?` at 2423, 2469, 2476, 2498,
  2501, 2577, 2589; tail `Ok(ValueResult{..})` 2613. Uses no `scratch*`. The inner
  `let value = self.lower_value_owned(capture)?` (2469) shadows locally; the outer `value` is not
  read in this arm.
- Call: `return` at 2647, 2653, 2683, 2709, 2734, 2738, 2741, 2744, 2747, 2756, 2761, 2768,
  2776, 2779, 2782, 2785, 2788, 2791, 2794, 2806, 2809, 2814, 2817; `?` at 2646, 2652, 2700,
  2766, 2767; tail `result` 2832. Reads the outer `value` at 2829 (`handover::call_key(value)`).
  Uses no `scratch*`.
- CallResult: `return` at 2894, 2938, 2954, 2984, 3003, 3011, 3025 (`Err`), 3096; `?` at 2867,
  2889, 2915, 2936, 3047, 3060 (`call?`), 3084, 3117, 3140; tail `Ok(..)` 3142. Reads the outer
  `value` at 3057, 3091, 3126. Uses `scratch9` (2909–2913, 2928, 3111–3115, 3132).
- UnionWrap: the pattern rebinds `value` to the inner value (3317); the arm never reads the
  outer one. `return Ok(..)` 3359; `?` at 3320, 3341, 3350, 3358, 3396; tail `Ok(..)` 3423.
  Uses `scratch9` (3418, 3420) and `scratch10` (3419, 3420).

Exit mapping: helpers return `Result<ValueResult, String>`; bodies move verbatim; the arm
becomes an arm expression (`NirValue::Call { .. } => self.lower_call_value(value),`), which is
the function tail exactly as before. Nothing follows the `match`.

The capture-less guard arm (2327–2339) stays BEFORE the capturing arm: the remainder keeps
`NirValue::Closure { name, type_, captures } if captures.is_empty() => { … }` and then
`NirValue::Closure { .. } => self.lower_closure_value(value),`. The helper's let-else pattern
`NirValue::Closure { name, type_, captures }` (no guard) always matches.

**Shared state.** `value: &NirValue` (caller-owned) — destructure source, and read by Call and
CallResult. `scratch9: &VirtualRegister` (CallResult, UnionWrap) and `scratch10:
&VirtualRegister` (UnionWrap) — passed as references; body text unchanged; not re-minted. Nothing
is mutated across the cut. No struct.

**Emission order.** Both scratch vregs are still minted first (2127–2128) on every call, before
the static-string check. The remainder keeps the recursive
`self.lower_value_inner(&NirValue::FunctionRef{..})` (2335), which mints its own two scratch
vregs as before. No clone, no reordering, no new allocation.

**New methods.**

| Method | Signature | File | LOC (guess) |
|---|---|---|---|
| `lower_closure_value` | `pub(super) fn lower_closure_value(&mut self, value: &NirValue) -> Result<ValueResult, String>`; let-else `NirValue::Closure { name, type_, captures }` | `builder_values/lower_construct.rs` | 246 |
| `lower_union_wrap_value` | `pub(super) fn lower_union_wrap_value(&mut self, value: &NirValue, scratch9: &VirtualRegister, scratch10: &VirtualRegister) -> Result<ValueResult, String>`; let-else `NirValue::UnionWrap { union_type, member_type, value }` (shadows the parameter, as the arm did; do not rename) | `builder_values/lower_construct.rs` | 105 |
| `lower_call_value` | `pub(super) fn lower_call_value(&mut self, value: &NirValue) -> Result<ValueResult, String>`; let-else `NirValue::Call { target, args, loc }` | `builder_values/lower_call.rs` | 143 |
| `lower_call_result_value` | `pub(super) fn lower_call_result_value(&mut self, value: &NirValue, scratch9: &VirtualRegister) -> Result<ValueResult, String>`; let-else `NirValue::CallResult { target, args, .. }` | `builder_values/lower_call.rs` | 248 |
| `lower_value_inner` (remainder) | unchanged signature, `pub(super)` | `builder_values/lower_inner.rs` | 500 |

Remainder arms: `NirValue::Closure { .. } => self.lower_closure_value(value),`,
`NirValue::Call { .. } => self.lower_call_value(value),`,
`NirValue::CallResult { .. } => self.lower_call_result_value(value, scratch9),`,
`NirValue::UnionWrap { .. } => self.lower_union_wrap_value(value, scratch9, scratch10),`.
These are the four largest arms; extracting only three would leave ≈ 600, so `UnionWrap` is
included for margin. `Constructor` (54) and `MemberAccess` (51) stay. `builder_values/mod.rs`
adds `mod lower_call; mod lower_construct;`. The moved bodies call `try_abi_inline_lower`,
`try_mfb_fast_path` (Call) and `lower_inline_conversion_raw`, `lower_inline_builtin_raw`,
`lower_inline_infallible_raw` (CallResult), all already `pub(super)` from §4.3; every other
callee is already `pub(crate)`. Free items come from the original header via `use super::*;`.

**Borrow checker.** `scratch9`/`scratch10` borrow locals of `lower_value_inner`, not `self`.
`value` is independent of `self`. The Closure arm's `string_shadow_captures(self.functions, name)`
/ `lambda_shares_resource_base(self.functions, name)` read a `&'a` field by copy. CallResult's
`self.locals.get(target).cloned()` / `self.globals.get(target).cloned()` already clone before the
`&mut self` calls.

**Repoints this phase makes:** `memory/09_closures.md:22` `NirValue::FunctionRef` stays at
`lower_inner.rs` (that arm does not move); `.ai/codegen-invariants.md:658` (`RuntimeCall` arm)
stays at `lower_inner.rs`; `.ai/collections.md:556` → `inline_calls.rs` (`try_mfb_fast_path`) and
`lower_call.rs` (`lower_call_value`, the consult site); code comments per §4.13 rows marked
"Phase 6". `memory/09_closures.md` has no citation of the capturing `Closure` arm
(`grep -n "builder_values.rs:" src/docs/spec/memory/09_closures.md`), so none moves to
`lower_construct.rs`.

### 4.12 Extract-method: `lower_module_for_platform` (Phase 7)

Source after Phase 4: `builder/lower_module.rs`, the free fn `lower_module_for_platform`, 1801
LOC (lines 1511–3956 of `builder/mod.rs` as of 17a9b5799; `span.py … 1511 3956`). No `self`, no
immediately-invoked closure, no labelled block.

**Structure.**

| Region | Lines | LOC | What |
|---|---|---|---|
| P prologue | 1511–1645 | 104 | sig, `reset_vreg_ids`, target check (`return Err` 1521), `set_backend`, two resets, `link_libraries` (`?` 1546), `function_symbols`, `globals`, `functions`, `platform_imports`, `imports`, `package_return_types`, `package_global_count`, `type_model` (`?` 1611), `module_resource_slots` + `globals.insert`, `module_teardown_symbol`, `string_symbols` |
| B data objects | 1646–1915 | 188 | string data objects … crypto cert set; only `data_objects.push/extend` |
| P2 | 1916–1928 | 7 | `close_op_symbols`, `code_functions`, `runtime_symbols` |
| C arena layout | 1929–2231 (+2241–2243) | 166 | `skip_entry_arena_destroy` … closure descriptors; `register_signal_handlers` |
| D entry | 2232–2400 | 116 | `debug_features`, program entry / app entry |
| E functions | 2401–2604 | 169 | callback refs, constructor census, base lowering, owned variants, destination variants, builtin wrappers |
| F graph/closure | 2605–2787 | 158 | recursive copy shims, closure env ops, copy/drop walkers, closure copy/drop |
| G core runtime | 2788–2967 | 136 | arena helpers, stdout/stdin/input/fs/process helpers, shutdown, debug, signal, audio callbacks |
| H companion symbols | 2968–3194 | 167 | pushes onto `runtime_symbols` only |
| I runtime helpers + threads | 3195–3399 | 172 | module drain, `lower_runtime_helper` loop, rng, utf8, tls trampolines, sort, thread trampoline/finish, block release, thread value fns |
| J relocation-gated helpers | 3400–3697 | 257 | map hash/remove, `construct.T`, drops, fail_status, restamp, park, concat, float/int formatters, toString |
| K canvas + LINK | 3698–3767 | 58 | pathJoin, graphics trampoline, vulkan builder, LINK support + data merge |
| L relocation-gated data | 3768–3913 | 81 | math const pool, empty string, unicode tables, error strings, status prefixes, powers of ten |
| Z epilogue | 3914–3956 | 22 | route through MIR, bind relocation libraries (`?` 3940), plan, `validate()?`, `Ok(plan)` |

**Control-flow audit (every exit).**
- 1521 `return Err(..)`, `?` 1546, 1611 — P, stay in the orchestrator.
- B: no `return`/`?`/`break`/`continue` (closures inside iterator adaptors only). Infallible fn
  returning the `Vec`.
- C: no exits. Local `const WINSOCK_FAMILIES` (2012) moves with it. The `#[cfg(test)]` statement
  2115–2117 (thread-local `RESERVED_INPUT_STATE_OFFSET.set`) moves verbatim. Block-expr
  2201–2213 and the `if/else` 2214–2231 are plain expressions. Infallible fn returning a struct.
- D: `ok_or_else(..)??` 2283–2288, `?` 2320, `?` 2398. All propagate `String`; nothing in the
  orchestrator between D and E runs on the error path.
- E: `return Err` 2482, 2500 (inside `while let`), 2545, 2559 (inside `for`); `continue` 2479
  (stays inside the `while let`); `?` 2450, 2526, 2585, 2603. The `crate::trace::timed_item`
  closure (2428–2450) wraps `lower_function`; its result is `?`-ed outside the closure — moves as
  is.
- F: `return Err` 2633 (top level), 2680 (match arm inside `for`); `?` 2657, 2697, 2733, 2746,
  2770, 2779, 2785. Closure `references` (2703–2710) is non-capturing and used only at 2715 and
  2774, both inside F.
- G: `?` 2796, 2805, 2830, 2856–2863, 2871, 2877, 2904, 2918, 2936, 2948, 2954, 2957, 2967.
- H: no exits. Infallible.
- I: `?` 3210, 3256 (outside the `timed_item` closure), 3331, 3338, 3357–3373, 3397. The `use`
  item 3352–3356 moves with the block.
- J: `return Err` 3561 (inside `if let` inside `if`); `?` 3475, 3496, 3516, 3538, 3590, 3612,
  3695. The `as fn() -> CodeFunction` coercion at 3421 moves verbatim.
- K: `return Err` 3754 (match arm inside `for`); `?` 3714, 3741.
- L: `?` 3898 (`ok_or_else(..)?`).
- Z: `?` 3940, `?` 3954, tail `Ok(plan)` — stays in the orchestrator.

Every extracted region either falls through or exits only with `Err(String)`; no region has an
early `Ok` exit that skips later shared code. `?` on `Result<_, String>` inside a fn returning
`Result<_, String>` is identity.

**Shared state.** Frozen after line 1645 (`function_symbols` last mutated 1554, `globals` 1634):
`function_symbols`, `functions`, `package_return_types`, `platform_imports`, `globals`,
`string_symbols`, `platform`, `module.build_mode` — exactly the eight fields of
`ClosureHelperTables<'a>` (`src/codegen/memory/arena/closure_ops.rs`). The literal at 2663–2672
is hoisted to directly after 1645, named `tables`, and `&tables` is passed everywhere; F's two
`&closure_tables` uses become `tables`. Inside helpers, the repeated 8-argument tail
`&function_symbols, &functions, &package_return_types, &platform_imports, platform,
module.build_mode, &globals, &string_symbols` becomes `tables.function_symbols, tables.functions,
tables.package_return_types, tables.platform_imports, tables.platform, tables.build_mode,
tables.globals, tables.string_symbols` (same values).

| Local | Type | Born | Used in | Form |
|---|---|---|---|---|
| `link_libraries` | `link_locator::LinkLibraries` | P | K | `&LinkLibraries` |
| `imports` | `Vec<CodeImport>` | P | Z (moved) | stays in orchestrator |
| `package_global_count` | `usize` | P | C | by value |
| `type_model` | `TypeModel` | P | E F I J K | `&TypeModel`; every `type_model.clone()` stays |
| `module_resource_slots` | `usize` | P | C | by value |
| `module_teardown_symbol` | `Option<&'static str>` | P | D I K | stored into `ModuleLayout` by C |
| `string_symbols` | `HashMap<String,String>` | P | B, tables | `&HashMap` for B |
| `data_objects` | `Vec<CodeDataObject>` | B (returned) | C D F I K L Z | orchestrator owns; `&mut Vec<CodeDataObject>` |
| `close_op_symbols` | `HashSet<String>` | P2 | K | `&HashSet<String>` |
| `code_functions` | `Vec<CodeFunction>` | P2 | all | orchestrator owns; `&mut Vec<CodeFunction>` (L: `&[CodeFunction]`) |
| `runtime_symbols` | `Vec<String>` | P2 | C (mut) G (read) H (mut) I (mut) | `&mut Vec<String>` / `&[String]` |
| C outputs | `ModuleLayout` | C | D G I K | `&ModuleLayout` |
| `constructor_types` | `Vec<(ParameterType, usize)>` | E | J | returned by E; `&[(ParameterType, usize)]` to J |
| `closure_env_layouts` | `Vec<ClosureEnvLayout>` | E | F (consumed by `for layout in closure_env_layouts`) | returned by E; BY VALUE to F |
| `debug_arena`, `uses_stdout_buffer`, `uses_stdin` | `bool` | G | I | returned by G as `CoreHelperFlags` (Copy) |
| `perf_arena` | `bool` | G | — | local to G |
| `callback_referenced_functions`, `synthesized_constructors` | `HashSet<..>` | E | — | local to E |
| `recursive_copy_types`, `copy_kind_count`, `typed_kinds`, `references`, `drop_walkers_referenced` | | F | — | local to F |
| `debug_features` | `Vec<&'static dyn DebugFeature>` | 2239 | D only | computed at the top of D (`active_features` is a pure filter over a static table) |
| `uses_thread_start`, `uses_block_release` | `bool` | I | — | local to I |

```rust
pub(super) struct ModuleLayout {          // module_layout.rs; built once by C, passed &
    pub(super) module_teardown_symbol: Option<&'static str>,
    pub(super) skip_entry_arena_destroy: bool,
    pub(super) global_initializer_symbol: Option<String>,
    pub(super) globals_base: usize,
    pub(super) link_count: usize,
    pub(super) uses_term: bool,
    pub(super) uses_net: bool,
    pub(super) uses_rng: bool,
    pub(super) uses_mouse: bool,
    pub(super) term_state_offset: Option<usize>,
    pub(super) presentation_mode_offset: Option<usize>,
    pub(super) mouse_state_offset: Option<usize>,
    pub(super) input_state_offset: Option<usize>,
    pub(super) initial_mode: PresentationMode,
    pub(super) seed_presentation_mode_offset: Option<usize>,
    pub(super) arena_global_slots: usize,
    pub(super) link_init_symbol: Option<&'static str>,
    pub(super) closure_init_symbol: Option<&'static str>,
    pub(super) register_signal_handlers: bool,
}

#[derive(Clone, Copy)]                    // module_helpers.rs
pub(super) struct CoreHelperFlags { pub(super) debug_arena: bool, pub(super) uses_stdout_buffer: bool, pub(super) uses_stdin: bool }
```

Locals of C that stay local: `link_slot_count`, `free_count`, `term_state_slots`,
`presentation_mode_slots`, `mouse_state_slots`, `input_state_slots`, `uses_app`,
`uses_input_buffer`, `closure_descriptor_func_symbols`, `WINSOCK_FAMILIES`. Consumers read
`layout.x`; `global_initializer_symbol.as_deref()` → `layout.global_initializer_symbol.as_deref()`.

No vreg, label or stack slot is allocated in the orchestrator itself: every one is allocated
inside the callee lowerings (`lower_function`, `lower_*_helper`, `platform.emit_*`,
`lower_closure_descriptor_initializer`), whose call ORDER is unchanged, against the module-wide
counter reset at 1520.

**Emission order.** The orchestrator calls the regions in source order P, B, P2, C, D, E, F, G,
H, I, J, K, L, Z, each verbatim. So the sequence of every lowering call (and `reset_vreg_ids`'
counter, the `thread::value_functions` / `module_owner` thread-local requests, the
`crate::trace` spans/counts) and the push order into `data_objects`, `code_functions`,
`runtime_symbols` are identical. Three statements move, all pure: (1) the `ClosureHelperTables`
literal (2663) → after 1645 (a struct of references); (2) `debug_features` (2239–2240) → top of
D; (3) `register_signal_handlers` (2241–2243) → tail of C (no call between 2231 and 2241). The
`timed_item` closures in E and I capture `tables`/`layout` by shared reference instead of the
individual locals — same values.

**New fns and files.**

`builder/lower_module.rs` (≈ 460, guess): the orchestrator, `lower_module_mir_for_platform` (12),
`variant_demand` (51), and private `lower_module_functions`.

```rust
pub(crate) fn lower_module_for_platform(module, native_plan, packages, platform) -> Result<NativeCodePlan, String> {
    /* P verbatim 1511–1645 */
    let tables = ClosureHelperTables { function_symbols: &function_symbols, functions: &functions,
        package_return_types: &package_return_types, platform_imports: &platform_imports, platform,
        build_mode: module.build_mode, globals: &globals, string_symbols: &string_symbols };
    let mut data_objects = module_layout::module_data_objects(module, native_plan, platform, &string_symbols);
    /* P2 verbatim 1916–1928 */
    let layout = module_layout::plan_arena_layout(module, platform, &function_symbols, package_global_count,
        module_resource_slots, module_teardown_symbol, &mut runtime_symbols, &mut data_objects, &mut code_functions);
    module_layout::emit_program_entries(module, platform, &platform_imports, &layout, &mut code_functions, &mut data_objects)?;
    let (constructor_types, closure_env_layouts) = lower_module_functions(module, &tables, &type_model, &mut code_functions)?;
    module_helpers::lower_graph_and_closure_helpers(&tables, &type_model, closure_env_layouts, &mut code_functions, &mut data_objects)?;
    let flags = module_helpers::lower_core_runtime_helpers(module, platform, &platform_imports, &layout, &runtime_symbols, &mut code_functions)?;
    module_finish::add_companion_runtime_symbols(module, &mut runtime_symbols);
    module_helpers::lower_runtime_helpers_and_threads(module, &tables, &type_model, &layout, flags, &mut runtime_symbols, &mut code_functions, &mut data_objects)?;
    module_finish::lower_relocation_gated_helpers(&tables, &type_model, &constructor_types, &mut code_functions)?;
    module_finish::lower_canvas_and_link_support(module, platform, &platform_imports, &layout, &type_model, &link_libraries, &close_op_symbols, &mut code_functions, &mut data_objects)?;
    module_finish::add_relocation_gated_data_objects(module, &code_functions, &mut data_objects)?;
    /* Z verbatim 3914–3956 */
}
```

Orchestrator ≈ 190 LOC (guess, from region counts).

| Fn | Signature | Body | File | LOC (guess) |
|---|---|---|---|---|
| `lower_module_functions` | `fn lower_module_functions(module: &NirModule, tables: &ClosureHelperTables<'_>, type_model: &TypeModel, code_functions: &mut Vec<CodeFunction>) -> Result<(Vec<(ParameterType, usize)>, Vec<ClosureEnvLayout>), String>` | E 2401–2604 with the tables rename, `functions.get` → `tables.functions.get`, `function_symbols.values()` → `tables.function_symbols.values()`, `&type_model` → `type_model`, `variant_demand(module, tables.functions, type_model, &callback_referenced_functions)`; ends `Ok((constructor_types, closure_env_layouts))` | `lower_module.rs` (private) | 178 |
| `module_data_objects` | `pub(super) fn module_data_objects(module: &NirModule, native_plan: &NativePlan, platform: &dyn CodegenPlatform, string_symbols: &HashMap<String, String>) -> Vec<CodeDataObject>` | B 1646–1915, tail `data_objects` | `module_layout.rs` | 192 |
| `plan_arena_layout` | `pub(super) fn plan_arena_layout(module: &NirModule, platform: &dyn CodegenPlatform, function_symbols: &HashMap<String, String>, package_global_count: usize, module_resource_slots: usize, module_teardown_symbol: Option<&'static str>, runtime_symbols: &mut Vec<String>, data_objects: &mut Vec<CodeDataObject>, code_functions: &mut Vec<CodeFunction>) -> ModuleLayout` with `#[allow(clippy::too_many_arguments)]` (9 args; the same attribute `lower_runtime_helper` carries) | C 1929–2231, then 2241–2243, then the struct literal | `module_layout.rs` | 195 |
| `emit_program_entries` | `pub(super) fn emit_program_entries(module: &NirModule, platform: &dyn CodegenPlatform, platform_imports: &HashMap<String, String>, layout: &ModuleLayout, code_functions: &mut Vec<CodeFunction>, data_objects: &mut Vec<CodeDataObject>) -> Result<(), String>` | 2239–2240, then 2244–2400 with `layout.` prefixes; `Ok(())` | `module_layout.rs` | 122 |
| `lower_graph_and_closure_helpers` | `pub(super) fn lower_graph_and_closure_helpers(tables: &ClosureHelperTables<'_>, type_model: &TypeModel, closure_env_layouts: Vec<ClosureEnvLayout>, code_functions: &mut Vec<CodeFunction>, data_objects: &mut Vec<CodeDataObject>) -> Result<(), String>` | F 2605–2787 minus the 2663–2672 literal; `&closure_tables` → `tables` | `module_helpers.rs` | 158 |
| `lower_core_runtime_helpers` | `pub(super) fn lower_core_runtime_helpers(module: &NirModule, platform: &dyn CodegenPlatform, platform_imports: &HashMap<String, String>, layout: &ModuleLayout, runtime_symbols: &[String], code_functions: &mut Vec<CodeFunction>) -> Result<CoreHelperFlags, String>` | G 2788–2967; `&runtime_symbols` → `runtime_symbols`; `uses_term`, `skip_entry_arena_destroy`, `input_state_offset`, `register_signal_handlers` → `layout.` | `module_helpers.rs` | 150 |
| `lower_runtime_helpers_and_threads` | `pub(super) fn lower_runtime_helpers_and_threads(module: &NirModule, tables: &ClosureHelperTables<'_>, type_model: &TypeModel, layout: &ModuleLayout, flags: CoreHelperFlags, runtime_symbols: &mut Vec<String>, code_functions: &mut Vec<CodeFunction>, data_objects: &mut Vec<CodeDataObject>) -> Result<(), String>` with `#[allow(clippy::too_many_arguments)]` | I 3195–3399 | `module_helpers.rs` | 182 |
| `add_companion_runtime_symbols` | `pub(super) fn add_companion_runtime_symbols(module: &NirModule, runtime_symbols: &mut Vec<String>)` | H 2968–3194 (reads only `module.build_mode` at 3184) | `module_finish.rs` | 170 |
| `lower_relocation_gated_helpers` | `pub(super) fn lower_relocation_gated_helpers(tables: &ClosureHelperTables<'_>, type_model: &TypeModel, constructor_types: &[(ParameterType, usize)], code_functions: &mut Vec<CodeFunction>) -> Result<(), String>` | J 3400–3697 (`for (type_, arity) in constructor_types`) | `module_finish.rs` | 262 |
| `lower_canvas_and_link_support` | `pub(super) fn lower_canvas_and_link_support(module: &NirModule, platform: &dyn CodegenPlatform, platform_imports: &HashMap<String, String>, layout: &ModuleLayout, type_model: &TypeModel, link_libraries: &link_locator::LinkLibraries, close_op_symbols: &HashSet<String>, code_functions: &mut Vec<CodeFunction>, data_objects: &mut Vec<CodeDataObject>) -> Result<(), String>` with `#[allow(clippy::too_many_arguments)]` | K 3698–3767; `link_count`, `globals_base`, `link_init_symbol`, `global_initializer_symbol`, `module_teardown_symbol` → `layout.`; `&link_libraries` → `link_libraries`; `&close_op_symbols` → `close_op_symbols` | `module_finish.rs` | 72 |
| `add_relocation_gated_data_objects` | `pub(super) fn add_relocation_gated_data_objects(module: &NirModule, code_functions: &[CodeFunction], data_objects: &mut Vec<CodeDataObject>) -> Result<(), String>` | L 3768–3913 | `module_finish.rs` | 86 |

Each new file starts with `use super::*;` (which brings `builder/mod.rs`'s private imports) plus
any `use` the moved text needs. Result: largest fn `lower_relocation_gated_helpers` ≈ 262;
orchestrator ≈ 190; largest file `module_finish.rs` ≈ 600 (all guesses from region counts;
re-measured in the phase).

**Borrow checker (each resolved).**
1. `tables` holds `&` to six maps for the rest of the orchestrator; none is mutated after 1645.
   It is constructed AFTER `globals.insert` (1634) and `string_symbols` (1645).
2. `ClosureHelperTables<'a>.platform: &'a dyn CodegenPlatform` — the param coerces as the
   existing 2663 literal does.
3. E: the `timed_item` closure captures `tables` (shared) and `&mut closure_env_layouts` (local
   to E) — same shape as today.
4. F: `for layout in closure_env_layouts` consumes it → the parameter is BY VALUE.
5. G reads `runtime_symbols` while pushing `code_functions` — pass `&runtime_symbols` as
   `&[String]` (all callees take `&[String]`: `program_uses_grapheme_clusters`,
   `AudioBackend::callback_functions`); never `&Vec<String>` (clippy `ptr_arg`).
6. I: `for symbol in &*runtime_symbols` borrows it shared while pushing `code_functions`; the
   mutable pushes at 3217 happen in the drain block BEFORE the loop, so the `&mut Vec<String>`
   param reborrows sequentially.
7. J: `fail_status_callers: Vec<&str>` borrows `*code_functions`; its last use is 3559, before
   the push at 3567 — NLL accepts it with `code_functions: &mut Vec<CodeFunction>`.
8. L: `referenced_unicode_tables` / `referenced_error_strings` / `referenced_status_prefixes`
   borrow `&str` from `code_functions` while `data_objects` is pushed — separate params.
9. `layout.global_initializer_symbol.as_deref()` borrows `layout` inside the
   `ProgramEntrySpec` / `ArenaInitSymbols` literals — `&ModuleLayout` outlives them.

**Repoints this phase makes** (from the Phase 4 `lower_module.rs` citations; measured with
`grep -rn -B3 "builder/lower_module.rs:\(lower_module_for_platform\|arena_global_slots\|referenced_unicode_tables\)\]\]" src/docs/spec`):
- `language/16_threads.md:31` `arena_global_slots` → `[[src/codegen/engine/builder/module_layout.rs:arena_global_slots]]`.
- `unicode/01_tables-and-algorithms.md:252` `referenced_unicode_tables` → `[[src/codegen/engine/builder/module_finish.rs:referenced_unicode_tables]]`.
- `memory/08_program-startup.md:89` (region offsets), `threading/09_os-integration.md:107`
  (`skip_entry_arena_destroy` deferral), `threading/06_thread-runtime-helpers.md:108`
  (per-arena regions), `tooling/09_debug-report.md:45` (no signal handlers for app builds →
  `register_signal_handlers`), `app/05_presentation-mode.md:73` (`setMode` → `initial_mode`) →
  `[[src/codegen/engine/builder/module_layout.rs:plan_arena_layout]]`.
- `threading/06_thread-runtime-helpers.md:49` (trampoline/table/block-arena emission gates) →
  `[[src/codegen/engine/builder/module_helpers.rs:lower_runtime_helpers_and_threads]]`.
- Whole-file `threading/09_os-integration.md:179` (input-buffer region) →
  `[[src/codegen/engine/builder/module_layout.rs]]`; `app/05_presentation-mode.md:117` →
  `[[src/codegen/engine/builder/module_layout.rs]]` (the presentation-mode slot is laid out in C).
- `architecture/07_flows:63`, `architecture/spec:64`, `linker/01_pipeline:76`,
  `linker/02_object-plan:33`, `linker/03_import-selection:41`,
  `architecture/17_mir-instruction-set:241` stay at `lower_module.rs` (orchestrator: `imports`,
  plan assembly, MIR routing).
- Prose mentions of the fn name (`src/testutil.rs:220`, `src/codegen/builtins/tests/validation.rs:4`,
  `src/codegen/engine/mir/mir.rs:727`) still name a live fn — no change.

### 4.13 `.ai`, script and code-comment sweep

Every mention of a removed path is repointed to the file that holds the named symbol after the
phase. Found by `grep -rnE "(builder_control|builder_values|function_lowering|builder_numeric|borrow_get|module_analysis|builder_conversions)\.rs|function/entry\.rs|\bentry\.rs\b|builder/mod\.rs" src .ai scripts tests/guards AGENTS.md` (paths as of 17a9b5799; plan-176-H/J/K may have moved
some of the test files named here — the comment TEXT is authoritative). Stale-before-U items are
marked **pre-existing** and each is its own task (AGENTS.md: never leave a bug).

| Where | Text | New text names | Phase |
|---|---|---|---|
| `.ai/collections.md:592` | `analysis/borrow_get.rs` (`collect_borrow_gets`) | `analysis/borrow_get/mod.rs` | 1 |
| `tests/guards/no_type_strings.rs:380,403` (doc comments) | `function_lowering.rs`'s `union_extract_reads` | `borrow_get/by_name.rs`'s `union_extract_reads` — **pre-existing** | 1 |
| `src/codegen/engine/analysis/module_analysis.rs:83` (now `module_analysis/mod.rs`) | (`emit_resource_state_init`, `builder_control.rs`) | `memory/value/builder_value_semantics.rs` — **pre-existing** | 1 |
| `src/codegen/builtins/tests/general_declared_errors.rs:6` | `builder_conversions.rs` | `builder_conversions/` | 1 |
| `src/ir/fallible.rs:387` | `builder_numeric.rs:lower_numeric_unary_negation` | `builder_numeric/operator_lowering.rs:lower_numeric_unary_negation` | 2 |
| `src/optimizer/opt1/strength.rs:10` | `raise_error_bare("ErrOverflow")` (`builder_numeric.rs`) | `builder_numeric/numeric_kernels.rs` (the overflow check, `builder_numeric.rs:1304–1309`) | 2 |
| `src/codegen/builtins/math/gen_fmod.rs:2` | `builder_numeric.rs` | `builder_numeric/binary_emit.rs` (`emit_float_binary`, the `fmod` call site) | 2 |
| `.ai/codegen-invariants.md:98` | `function_lowering.rs` (`owned_value_slots` splice) | `function_lowering/mod.rs` | 2 |
| `.ai/codegen-invariants.md:532` | `function_lowering.rs:unboxed_trap_result_candidates` | `function_lowering/trap_analysis.rs:unboxed_trap_result_candidates` | 2 |
| `.ai/resources-packages.md:113` | `function_lowering.rs` `collect_address_taken_locals`/`collect_value_used_locals` | `function_lowering/local_analysis.rs` `collect_address_taken_locals` / `function_lowering/promotion_analysis.rs` `collect_value_local_reads` — **pre-existing** (`collect_value_used_locals` removed in `f37874271`) | 2 |
| `.ai/canvas-threading.md:30` | `src/codegen/engine/function/entry.rs` | `src/codegen/engine/function/entry/mod.rs` (`lower_program_entry`) | 2 |
| `src/testutil.rs:241` | `engine/function/entry.rs` (args-capture path) | `engine/function/entry/` (`mod.rs` + `entry_args.rs`) | 2 |
| `src/codegen/runtime/thread/runtime_helpers.rs:1077` | `entry_stack_misaligned_on_entry` (`entry.rs`) | (`function/entry/mod.rs`) | 2 |
| `src/codegen/builtins/tests/entry.rs:3` | `engine/function/entry.rs` | `engine/function/entry/` | 2 |
| `.ai/collections.md:585` | `builder_values.rs:17` → `register_pending_temp:30` | `builder_values/pending_temps.rs` (`lower_value` → `register_pending_temp`) — **pre-existing** stale line numbers | 3 |
| `.ai/resources-packages.md:103` | `lower_value_owned` (`builder_values.rs`) | `builder_values/ownership.rs` | 3 |
| `.ai/resources-packages.md:178` | `value_aliases_live_resource` (`builder_values.rs`) | `builder_values/ownership.rs` | 3 |
| `.ai/resources-packages.md:181,384,420` | `builder_values::lower_runtime_helper_call`, `builder_values.rs`'s `runtime_target = match target` | `builder_values/inline_calls.rs` | 3 |
| `.ai/codegen-invariants.md:658` | `src/codegen/engine/value/builder_values.rs` (`NirValue::RuntimeCall` arm) | `src/codegen/engine/value/builder_values/lower_inner.rs` | 3 |
| `.ai/collections.md:556` | `#collections_<name>$<types>` prefix gates in `builder_values.rs` | `builder_values/lower_inner.rs` in Phase 3; `builder_values/inline_calls.rs` (`try_mfb_fast_path`) and `builder_values/lower_call.rs` (`lower_call_value`) in Phase 6 | 3, 6 |
| `src/codegen/string/unicode_props.rs:355` | `builder_values.rs::register_pending_temp` | `builder_values/pending_temps.rs::register_pending_temp` | 3 |
| `src/codegen/builtins/tests/corpus.rs:8` | `engine/value/builder_values.rs` | `engine/value/builder_values/` | 3 |
| `src/codegen/builtins/tests/nir_fault.rs:429` | `builder_values.rs` alone has eight `ok_or_else` closures on `helper_args.first()` | `builder_values/inline_calls.rs` (`lower_runtime_helper_call*`, lines 4266, 4460, 4474, 4495, 4509) | 3 |
| `src/codegen/builtins/collections/gen_flow.rs:96` | (bug-822, `builder_values.rs`) | (bug-822, `builder_values/`) | 3 |
| `scripts/spec-census.sh:110` (comment example) | `[[src/codegen/engine/value/builder_values.rs:NirValue::FunctionRef]]` | `[[src/codegen/engine/value/builder_values/lower_inner.rs:NirValue::FunctionRef]]` | 3 |
| `src/codegen/memory/value/builder_value_semantics.rs:1265` | `nir_call_is_infallible_builtin` (`engine/control/builder_control.rs`) | `engine/control/builder_control/value_effects.rs` | 3 |
| `.ai/collections.md:34` | `NirOp::StoreGlobal` lowerings in `engine/control/builder_control.rs` | `builder_control/store_global.rs` and `builder_control/statement_dispatch.rs` (Phase 3); `store_global.rs` and `builder_control/assign_op.rs` (Phase 5) | 3, 5 |
| `.ai/collections.md:540` | `&&`-chain at `builder_control.rs` ~592 | `builder_control/statement_dispatch.rs` (`lower_ops_inner`, Assign arm) in Phase 3; `builder_control/assign_op.rs` (`lower_assign_op`) in Phase 5 — **pre-existing** stale line | 3, 5 |
| `.ai/resources-packages.md:28,45` | StateAssign at `builder_control.rs` | `statement_dispatch.rs` (Phase 3); `builder_control/assign_op.rs` (`lower_state_assign_op`) (Phase 5) | 3, 5 |
| `.ai/resources-packages.md:135` | `try_inplace_state_collection_assign` (`builder_control.rs`) | `try_inplace_self_update` (`collection/assign/self_update.rs`), called from `lower_state_assign_op` (`builder_control/assign_op.rs`) with an `InPlaceDest::StateField` site — **pre-existing** (0 hits in `src`) | 5 |
| `.ai/resources-packages.md:178` (resource-typed bind gate) | `builder_control.rs` | `builder_control/bind_op.rs` (`lower_bind_op`) | 5 |
| `.ai/resources-packages.md:75` | scope end in `lower_ops_inner` | unchanged (`lower_ops_inner` keeps the scope end) | — |
| `src/codegen/builtins/tests/nir_fault.rs:391` | `engine/control/builder_control.rs`'s assignment path | `statement_dispatch.rs` (Phase 3); `engine/control/builder_control/assign_op.rs` (Phase 5) | 3, 5 |
| `src/codegen/builtins/tests/vector_promotion.rs:9` | `lower_ops_inner`'s `if promote_vector` arm | `lower_bind_op`'s `if promote_vector` path | 5 |
| `src/codegen/engine/value/builder_values.rs:1565,1578` (now `builder_values/ownership.rs`) | `owns_freeable_value` in `lower_ops_inner` | `owns_freeable_value` in `lower_bind_op` | 5 |
| `src/codegen/engine/value/builder_values.rs:1770` (now `builder_values/ownership.rs`) | (`builder_control.rs:697-699`: `resource_cleanup_symbol(type_)` …) | (`lower_bind_op`, `builder_control/bind_op.rs`: `resource_cleanup_symbol(type_)` …) — **pre-existing** stale line numbers | 5 |
| `src/codegen/memory/arena/builder_arena_transfer.rs:123,540,549` | `builder_values.rs`, `NirValue::CallResult` / the two `builder_values.rs` paths | `builder_values/lower_call.rs` (`lower_call_result_value`) | 6 (Phase 3 writes `builder_values/lower_inner.rs`) |
| `src/codegen/engine/builder/builder_emit_helpers.rs:652` | `builder_values.rs`'s inline-TRAP machinery (the `Some` caller of `emit_call`, `builder_values.rs:3058`) | `builder_values/lower_call.rs`'s `lower_call_result_value` (Phase 6; Phase 3 writes `builder_values/lower_inner.rs`) | 3, 6 |
| `.ai/testing-gates.md:136` | `standard_error_messages()` (`src/codegen/engine/builder/mod.rs`) | `src/codegen/engine/builder/error_messages.rs` | 4 |

Module-name mentions without `.rs` (`builder_control`'s …, `builder_values` picks …) stay valid:
the module names do not change.

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

**Common per-phase check sequence** (each phase's Acceptance names which built-ins and filters):

1. Before editing: `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` → record N;
   `cargo check --all-targets 2>&1 | grep -c '^warning'` → record W.
2. After editing: `rustfmt --edition 2021 <each new/changed .rs file>`; `git diff --stat` shows only this phase's files.
3. `cargo check --all-targets 2>&1 | grep -c '^warning'` → W (no new warning) (est. 2 min).
4. `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` → N (est. 2 min).
5. `cargo test -p mfb --bin mfb spec_citations_resolve` → passes (est. 2 min).
6. `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings` → pass (est. 2 min).
7. `cargo build --release` (est. 5 min, guess), then each named
   `sh scripts/artifact-gate.sh target/release/mfb <builtin>` → 0 diffs (~1 min each). Do not run
   two gates at once (`pgrep -f artifact-gate` empty first; `.ai/testing-gates.md`).
8. `cargo test --test file_size_budget 2>&1 | grep -E 'src/codegen/engine/(analysis|builder|control|convert|function|operators|value)/'`
   → only the rows the phase names; then `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`
   and commit `tests/guards/file_size_ceilings.txt` with the phase (est. 1 min).
9. Every new file: `python3 -I tools/loc-census/census.py --file <path>` → code ≤ 900.

### Phase 1 — Analysis and conversions (pure motion)

Splits the three smallest-surface files. The moved `builder_conversions` code stays inside the
overflow-label scan through the directory rule from plan-176-O Phase 1.

- [ ] `analysis/borrow_get.rs` → `analysis/borrow_get/{mod,predicates,by_name}.rs` per §4.7
      (`git mv` to `borrow_get/mod.rs` first so history follows); keep `#[cfg(test)] mod tests;`;
      the 15 `pub(super)` edits and the re-exports of §4.7.
- [ ] Repoint the 4 `borrow_get.rs` spec citations and `.ai/collections.md:592` (§4.7, §4.13).
- [ ] `tests/guards/no_type_strings.rs:380,403` doc comments → `borrow_get/by_name.rs`'s
      `union_extract_reads` (**pre-existing**; evidence `grep -rn union_extract_reads src`).
      Doc comments only; no allowlist entry changes.
- [ ] `analysis/module_analysis.rs` → `analysis/module_analysis/{mod,float_errors,unicode_tables}.rs`
      per §4.8; repoint its 2 citations.
- [ ] `module_analysis/mod.rs` comment (old line 83) → `emit_resource_state_init`
      (`memory/value/builder_value_semantics.rs`) (**pre-existing**; evidence
      `grep -rn "fn emit_resource_state_init" src`).
- [ ] `convert/builder_conversions.rs` → `convert/builder_conversions/{mod,to_int,to_byte_scalar}.rs`
      per §4.9; repoint its 8 citations; `general_declared_errors.rs:6` comment (§4.13).
Acceptance: the three modules compile from their new layout with no outside path changed, the
code they emit is byte-identical, and the overflow-label guard still scans the moved code.
  Check: the common sequence with built-ins `collections`, `strings`, `math`, `money`, `general`
  (borrow-get, unicode tables, float-error analysis, every `to*` conversion); plus
  `cargo test -p mfb --bin mfb codegen::engine::analysis` → the 27 borrow-get tests pass, and
  `cargo test -p mfb --bin mfb no_overflow_label_returns_through_the_result_tag_register` → passes,
  and `grep -rn 'abi::label(&.*overflow' src/codegen/engine/convert` → 7 hits, all under
  `builder_conversions/` (the scan's directory rule picks up every one of them);
  step 8 rows: none for these three files after tighten (their `ceiling` rows were removed as
  missing paths) (est. 20 min total).
Commit: —

### Phase 2 — Operators and function lowering (pure motion)

- [ ] `operators/builder_numeric.rs` → `operators/builder_numeric/{mod,operator_lowering,comparison,binary_emit,numeric_kernels}.rs`
      per §4.6; `emit_min_i64_negation_check` → `pub(super)`.
- [ ] Repoint its 9 citations (`emit_float_fmod` → `gen_fmod.rs:emit_float_fmod`) and the
      `fallible.rs:387`, `strength.rs:10`, `gen_fmod.rs:2` comments (§4.13).
- [ ] `function/function_lowering.rs` → `function/function_lowering/{mod,local_analysis,trap_analysis,promotion_analysis,helper_lowering}.rs`
      per §4.4; repoint its 6 citations and `.ai/codegen-invariants.md:98,532`.
- [ ] `.ai/resources-packages.md:113` → `local_analysis.rs` `collect_address_taken_locals` /
      `promotion_analysis.rs` `collect_value_local_reads` (**pre-existing**; evidence
      `git log -S collect_value_used_locals --oneline | head -1` → `f37874271`).
- [ ] `function/entry.rs` → `function/entry/{mod,process_setup,entry_args,cleanup_audit}.rs` per
      §4.5; repoint its 18 citations, `.ai/canvas-threading.md:30`, and the `testutil.rs:241`,
      `runtime_helpers.rs:1077`, `builtins/tests/entry.rs:3` comments.

Acceptance: byte-identical emission for numeric operators, function bodies, helper wrappers and
the program entry on every target the scoped gates build.
  Check: the common sequence with built-ins `math`, `money`, `bits`, `general`, `thread`, `os`,
  `process` (operators, `lower_function`, thread copy functions, entry args / std-fd reservation
  / riscv hwcap probe); plus `cargo test -p mfb --bin mfb builtins_codegen::entry` → passes
  (the entry tests in `src/codegen/builtins/tests/entry.rs`); step 8 rows: none for these three
  files (est. 25 min).
Commit: —

### Phase 3 — Statement and value lowering (pure motion)

- [ ] `value/builder_values.rs` → `value/builder_values/{mod,pending_temps,ownership,lower_inner,inline_calls}.rs`
      per §4.3; the 9 `pub(super)` edits; `use super::flatten_concat_spine;` in `lower_inner.rs`;
      the 3680–3699 doc comment goes with `lower_checked_value`.
- [ ] Repoint the 28 `builder_values.rs` citations (§4.3) and the Phase-3 rows of §4.13
      (`.ai/collections.md:556,585` — 585 is **pre-existing**; `.ai/resources-packages.md:103,178,181,384,420`;
      `.ai/codegen-invariants.md:658`; `unicode_props.rs:355`, `corpus.rs:8`, `nir_fault.rs:429`,
      `gen_flow.rs:96`, `builder_arena_transfer.rs:123,540,549` and `builder_emit_helpers.rs:652`
      → `lower_inner.rs`; `scripts/spec-census.sh:110`).
- [ ] `control/builder_control.rs` → `control/builder_control/{mod,store_global,field_with,element_update,field_replace,value_effects,statement_dispatch,loops,string_shadow,for_each}.rs`
      per §4.1; the 22 `pub(super)` edits; the two `include_str!` paths in
      `transfer_temp_classification_tests`.
- [ ] Repoint the 19 `builder_control.rs` citations (§4.1) and the Phase-3 rows of §4.13
      (`.ai/collections.md:34,540` — 540 is **pre-existing**; `.ai/resources-packages.md:28,45`;
      `builder_value_semantics.rs:1265`; `nir_fault.rs:391`).

Acceptance: statement and value lowering emit byte-identical code; the 9 inline tests of the two
files pass in their new homes.
  Check: the common sequence with built-ins `collections`, `strings`, `general`, `json`,
  `thread`, `resource-xfer-slots`, `vector` (element/field updates, for-each, string shadows,
  pending temps, ownership copies, inline/ABI calls, vector promotion); plus
  `cargo test -p mfb --bin mfb transfer_temp_classification_tests` → 3 pass,
  `cargo test -p mfb --bin mfb concat_spine_tests` → 4 pass,
  `cargo test -p mfb --bin mfb borrowed_resource_tests` → 2 pass; step 8 rows after tighten: exactly
  `NEW code <n> > 1000 src/codegen/engine/control/builder_control/statement_dispatch.rs` and
  `NEW code <n> > 1000 src/codegen/engine/value/builder_values/lower_inner.rs`
  (removed in Phases 5 and 6) (est. 30 min).
Commit: —

### Phase 4 — Module lowering (pure motion)

- [ ] `builder/mod.rs`: move 1511–3975 and 4909–4974 to `builder/lower_module.rs`, 3976–4713 to
      `builder/runtime_helpers.rs`, 4714–4828 to `builder/link_resolution.rs`, 4829–4908 to
      `builder/error_messages.rs` (§4.2); private `mod` declarations and the four explicit
      `pub(crate) use` lists.
- [ ] Repoint the `builder/mod.rs` citations by the §4.2 table (16 symbol citations move, 3 stay;
      2 whole-file citations move, 2 stay) and `.ai/testing-gates.md:136`. The checker does not
      force this one: after editing,
      `grep -rnoE "\[\[src/codegen/engine/builder/mod.rs[^]]*\]\]" src/docs/spec` → exactly the 5
      that stay (`ClosureEnvWord`, `OwnedValueCleanup`, `StatementTemps`, linker/01:18,
      architecture/06:368).

Acceptance: the module plan (every data object, function and import, in order) is byte-identical.
  Check: the common sequence with built-ins `general`, `thread`, `term`, `tls`, `audio`,
  `link-const-pins`, `collections` (data objects, runtime helpers, LINK, map helpers, error
  strings); step 8 rows after tighten: the two Phase-3 NEW rows plus
  `NEW code <n> > 1000 src/codegen/engine/builder/lower_module.rs` (removed in Phase 7); no row
  for `builder/mod.rs` (est. 25 min).
Commit: —

### Phase 5 — Extract `lower_ops_inner` arms

- [ ] `builder_control/bind_op.rs`: `lower_bind_op` = the Bind arm body (2741–3518) verbatim
      behind the let-else, trailing `Ok(())` (§4.10).
- [ ] `builder_control/assign_op.rs`: `lower_assign_op` (Assign 3547–3999) and
      `lower_state_assign_op` (StateAssign 4002–4165), same form.
- [ ] `statement_dispatch.rs`: the three arms become `=> self.lower_<x>_op(op, …)?,`;
      `builder_control/mod.rs` gains `mod bind_op; mod assign_op;`.
- [ ] Repoint `language/15_resource-management.md:155` and `memory/04_arenas.md:423` to
      `bind_op.rs` (§4.10) and the Phase-5 rows of §4.13 (`.ai/collections.md:34,540`;
      `.ai/resources-packages.md:28,45,178`; `nir_fault.rs:391`; `vector_promotion.rs:9`;
      `ownership.rs` comments at old 1565, 1578).
- [ ] `.ai/resources-packages.md:135` → `try_inplace_self_update` (**pre-existing**; evidence
      `grep -rn try_inplace_state_collection_assign src` → empty).
- [ ] `ownership.rs` comment at old 1770 → `lower_bind_op` (**pre-existing** stale line numbers).
- [ ] `cargo clippy --all-targets 2>&1 | grep -c '^warning'` → the count recorded before the phase.

Acceptance: every statement lowers to the same code; the paths the artifact gate does not cover
(STATE whole-record replace, inline TRAP binds) still behave.
  Check: the common sequence with built-ins `collections`, `strings`, `general`, `json`,
  `resource-xfer-slots`, `vector`; plus
  `cargo test --test rt_res_state_inplace_mutation --test rt_inline_trap_union_bind --test rt_inline_trap_borrowed_resource`
  → pass (StateAssign and the Bind arm's unboxed-trap path, which `.ai/testing-gates.md` says no
  golden covers; est. 4 min);
  `python3 -I tools/loc-census/census.py --file src/codegen/engine/control/builder_control/statement_dispatch.rs`
  → code ≤ 900 and no function in `bind_op.rs`/`assign_op.rs`/`statement_dispatch.rs` > 1000;
  step 8 rows after tighten: no `statement_dispatch.rs` row; the `lower_inner.rs` and
  `lower_module.rs` NEW rows remain (est. 30 min).
Commit: —

### Phase 6 — Extract `lower_value_inner` arms

- [ ] `builder_values/lower_construct.rs`: `lower_closure_value` (capturing Closure 2345–2618)
      and `lower_union_wrap_value` (UnionWrap 3319–3428) (§4.11).
- [ ] `builder_values/lower_call.rs`: `lower_call_value` (Call 2639–2832) and
      `lower_call_result_value` (CallResult 2835–3146).
- [ ] `lower_inner.rs`: the four remainder arms of §4.11 (capture-less guard arm kept first);
      `builder_values/mod.rs` gains `mod lower_call; mod lower_construct;`.
- [ ] Repoint the Phase-6 rows of §4.13 (`.ai/collections.md:556`;
      `builder_arena_transfer.rs:123,540,549`; `builder_emit_helpers.rs:652`).
- [ ] `cargo clippy --all-targets 2>&1 | grep -c '^warning'` → the count recorded before the phase.

Acceptance: every value expression lowers to the same code.
  Check: the common sequence with built-ins `general`, `collections`, `strings`, `json`,
  `thread`, `tcp`, `vector` (closures, direct and FUNC-value calls, trapped call results, union
  wraps, ABI-inline and fast-path calls); plus
  `cargo test --test rt_inline_trap_union_bind --test rt_inline_trap_nested_call --test rt_callback_failure_reaches_trap`
  → pass (CallResult under inline TRAP, uncovered by goldens; est. 4 min); step 8 rows after
  tighten: only the `lower_module.rs` NEW row (est. 30 min).
Commit: —

### Phase 7 — Extract `lower_module_for_platform` regions (largest blast radius last)

- [ ] `lower_module.rs`: hoist the `ClosureHelperTables` literal to after the `string_symbols`
      statement (old 1645) as `tables`; replace regions B–L with the calls of §4.12 in source
      order; add private `lower_module_functions` (E).
- [ ] `builder/module_layout.rs`: `ModuleLayout`, `module_data_objects` (B), `plan_arena_layout`
      (C + `register_signal_handlers`, with the `#[cfg(test)]` statement), `emit_program_entries`
      (`debug_features` + D).
- [ ] `builder/module_helpers.rs`: `CoreHelperFlags`, `lower_graph_and_closure_helpers` (F),
      `lower_core_runtime_helpers` (G), `lower_runtime_helpers_and_threads` (I).
- [ ] `builder/module_finish.rs`: `add_companion_runtime_symbols` (H),
      `lower_relocation_gated_helpers` (J), `lower_canvas_and_link_support` (K),
      `add_relocation_gated_data_objects` (L).
- [ ] `builder/mod.rs`: `mod module_layout; mod module_helpers; mod module_finish;` (private).
- [ ] Repoint the §4.12 citations (9 move, 6 stay at `lower_module.rs`).
- [ ] `cargo clippy --all-targets 2>&1 | grep -c '^warning'` → the count recorded before the phase.

Acceptance: the whole module plan — data objects, functions, runtime symbols, imports, in order —
is byte-identical, including the canvas/app path.
  Check: the common sequence with built-ins `general`, `thread`, `term`, `tls`, `audio`, `io`,
  `link-const-pins`, `collections`, `strings`. The canvas region (K) is covered only by
  `tests/syntax/app/app-mouse-surface`, which the letter-end full gate builds; step 8 after
  tighten: NO row for any `src/codegen/engine/{analysis,builder,control,convert,function,operators,value}/`
  path, and `grep -cE 'engine/(control/builder_control|builder/mod|value/builder_values|function/function_lowering|function/entry|operators/builder_numeric|analysis/borrow_get|analysis/module_analysis|convert/builder_conversions)' tests/guards/file_size_ceilings.txt`
  → 0 (est. 30 min).
Commit: —

## Validation Plan

- Tests: no test is added or removed. The moved inline tests (`transfer_temp_classification_tests`
  3, `concat_spine_tests` 4, `borrowed_resource_tests` 2, borrow-get 27) run in each phase that
  moves them; the `mfb` binary's test count is equal before and after every phase.
- Coverage check: engine code is reached by every byte-identity fixture; the scoped gates name
  the built-ins whose lowering each phase moved. The paths goldens do not cover
  (`.ai/testing-gates.md`: inline `TRAP`, whole-record STATE replace, canvas) are covered by the
  runtime targets in Phases 5–6 and by `tests/syntax/app/app-mouse-surface` in the full gate.
- Runtime proof: the runtime targets listed in Phases 5 and 6 build and run real programs through
  the extracted Bind / StateAssign / CallResult paths.
- Doc sync: no `mfb spec` or `mfb man` contract changes; only citations move (38 spec files,
  `spec_citations_resolve`), plus `.ai/collections.md`, `.ai/resources-packages.md`,
  `.ai/codegen-invariants.md`, `.ai/testing-gates.md`, `.ai/canvas-threading.md`.
- Final gate (run ONCE, after Phase 7): `cargo build --release` then
  `sh scripts/artifact-gate.sh target/release/mfb all` → 0 diffs (15–20 min; engine code is on
  every program's path on every target, and only the full gate builds all targets plus the canvas
  fixture `tests/syntax/app/app-mouse-surface` — no scoped selector reaches it). Then
  `cargo test --test file_size_budget 2>&1 | grep -c 'src/codegen/engine/'` → 0 rows for U's
  directories. The full workspace suite runs once, in plan-176-V's Validation Plan.

## Decisions

- Rule revised — owner, 2026-10-18 (replaces the 2026-10-14 "code ≤ 1000, test ≤ 250" rule).
  Three kinds, non-comment non-blank LOC, scope `.rs`/`.mfb` under `src/`, `packages/`,
  `examples/`, `repository/`, `tests/`: `code` ≤ 1000 (code LOC of a file that is not test-only;
  over → split into modules); `inline` ≤ 250 (`#[cfg(test)]` / `TESTING … END TESTING` LOC inside
  a file that is not test-only; over → moved to a test-only file, one `tests/mod.rs` unless
  > 1000); `test` ≤ 1000 (total LOC of a test-only file; over → split). Parts target ≤ 900. No
  exemptions: only `GENERATED` files are exempt automatically. Ceiling rows are
  `ceiling <code|inline|test> <n> <path>`; the guard warns with a banner and never fails; a new
  module is always an acceptable split. U's two remaining inline test modules (103 and 111
  inline test LOC by the U map) are under 250 and stay inline; `borrow_get`'s 522 moved in H.
- A single function over the code limit is decomposed by extract-method refactoring, gated by
  byte-identical `.ncode` — owner, 2026-10-14. In U: `lower_ops_inner` (§4.10),
  `lower_value_inner` (§4.11), `lower_module_for_platform` (§4.12). Nothing in U is exempt.
- Each extract-method refactor is its own phase, after all pure-motion phases, gated by the
  scoped artifact gate plus the letter-end full gate — author, 2026-10-14.
- Arm extraction uses whole-arm helpers that take the scrutinee and re-destructure it with
  let-else + `unreachable!` — author (§3 rejected alternatives).
- `lower_module_for_platform`'s frozen tables travel as the existing `ClosureHelperTables`; the
  arena-layout outputs as `ModuleLayout`; G's flags as `CoreHelperFlags` — author (§4.12).
- `builder/` parts are private modules with explicit `pub(crate) use` lists — author (§4.2).
- The overflow-label guard's directory rule is owned by plan-176-O Phase 1, the first letter that
  splits a `builder_*` file — author (§3).
- `emit_float_fmod`'s citation points at its definition in `gen_fmod.rs` — author (§4.6).
- Pre-existing stale references found in U's files are fixed in U (§4.13, **pre-existing** rows)
  — owner rule (AGENTS.md "never leave a bug"), applied by the author.

## Corrections

- 2026-10-18: Rule revised by the owner (code ≤ 1000, inline ≤ 250, test-only file ≤ 1000, no exemptions); file list and split design unchanged; rule text (intro, Goal, Decisions) restated, `borrow_get`'s H-moved tests named as the single `borrow_get/tests/mod.rs`.

## Summary

The risk is in Phases 5–7. They change function boundaries in the three lowerings every
program passes through. They are safe because each moves verbatim bodies to the exact call
position, adds no vreg/label/slot allocation, keeps every lowering call in source order, and is
proven by byte-identical artifacts plus runtime tests for the paths goldens miss. Phases 1–4 are
pure motion. Their hazards are bookkeeping: 116 spec citations, about 40 prose references, and
one guard whose corpus is chosen by file name. Untouched: every emitted byte, every public crate
path, every test name, the remaining engine files (`code_impl.rs`, `builder_emit_helpers.rs`,
`builder_exits.rs`, `regalloc/`, `mir/`, …), and the bug documents.
