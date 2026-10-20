# plan-176-Q: Collection code

Last updated: 2026-10-18
Effort: large (3h–1d)
Depends on: plan-176-P

Q splits the nine over-budget code files under `src/codegen/collection/` (in-place assign,
self-update dispatch, string self-update, collection compare, collection layout, list and map
mutation, search). Each `foo.rs` becomes `foo/mod.rs` plus parts grouped by responsibility. The
work is pure code motion: the `.ncode` the compiler emits stays byte-identical. The test code of
two of these files (`self_update.rs`, `builder_collection_layout.rs`) was already moved out by
plan-176-J. Q moves only the code, and it takes J's test files as they stand.

Single outcome: after Q lands, `cargo test --test file_size_budget` shows no row for any of the
nine Q paths or their parts, and `tests/guards/file_size_ceilings.txt` has no `ceiling` row for
any of them. The rule (owner, 2026-10-18): code ≤ 1000, inline test ≤ 250 and test-only file
≤ 1000 non-comment non-blank LOC, parts target ≤ 900; there are no exemptions (only `GENERATED`
files are skipped).

References:

- `planning/plan-176-A-file-size-guard.md`: §2 counting rules, §Decisions, and the
  `.ai/build-tooling.md` section "Splitting an over-budget file (plan-176)" that A writes.
- `planning/plan-176-J-backend-codegen-tests.md`: the `self_update/tests/` and
  `builder_collection_layout/tests/` test-only modules Q starts from.
- `.ai/collections.md`: List/Map/Set codegen. Its in-place mutation and self-update sections
  name these files, and §Phase tasks repoint them.
- `.ai/codegen-invariants.md` (record layout: `record_field_is_inlined`, `emit_build_inlined_record`).
- `.ai/testing-gates.md` § "Splits must sweep man AND spec citations", and the artifact-gate section.
- `.ai/resources-packages.md` (names `builder_inplace_assign.rs` and `builder_collection_layout.rs`).
- Spec topics whose citations move: `src/docs/spec/memory/{01_scalar-storage,03_heap-values,04_arenas,05_collections,09_closures,spec}.md`,
  `src/docs/spec/language/{04_types,12_collections,14_memory-semantics}.md`,
  `src/docs/spec/unicode/{01_tables-and-algorithms,02_strings-model}.md`,
  `src/docs/spec/stdlib/14_os.md`, `src/docs/spec/app/06_canvas.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-P archived | `ls planning/plan-176-P-* planning/completed/plan-176-P-*` → only `planning/completed/` matches | MET (2026-10-19) |
| plan-176-J's test moves for Q's files have landed (J precedes P) | `ls src/codegen/collection/assign/self_update/tests/mod.rs src/codegen/collection/layout/builder_collection_layout/tests/mod.rs` → both exist | MET (2026-10-19) |
| The guard exists | `cargo test --test file_size_budget` passes | MET (2026-10-19) |
| Q's files and every file Q's sweeps edit have no uncommitted changes | `git status --porcelain -- src/codegen/collection src/docs/spec .ai src/optimizer/opt1/lencache.rs src/codegen/error/emission/park_error_helper.rs src/codegen/builtins/tests src/codegen/builtins/collections tests/runtime tests/codegen tests/guards/file_size_ceilings.txt` → empty | MET (2026-10-19: 0 lines) |

Everything below assumes these rows hold.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify. A
> prerequisite recorded NOT MET may have landed since, and one recorded MET may
> have regressed.
>
> **If you stop, report the current status of *all* prerequisites**, not only
> the one that blocked you.

## 1. Goal

- Each of the nine files below is a directory module whose every file holds ≤ 1000 code LOC
  and ≤ 250 inline test LOC (target ≤ 900 code per part, A's convention). The largest part is
  `map_mutate/set_in_place.rs` at 869 code LOC, plus its `use` and `impl` wrapper lines.
- Native output is byte-identical: `sh scripts/artifact-gate.sh target/release/mfb all` passes
  with no golden touched.
- Every spec citation, `.ai/` reference, source comment, test doc and `include_str!` that named a
  moved file names the file that now holds the item.

### Non-goals (explicit constraints)

- No behavior change and no logic edit. The only non-motion edits are visibility widenings
  (`pub(super)`), `mod`/`use` lines, the `include_str!` path in
  `tests/runtime/rt_inplace_self_update.rs` (B's root), and comment/doc text.
- Every crate path that callers use today keeps resolving. Examples:
  `collection::search::builder_search::MidStringRegs`,
  `collection::assign::self_update::module_self_updates_with`, and `layout::record_field_is_inlined`
  through `layout/mod.rs`'s `pub(crate) use builder_collection_layout::*;`. Where an item moved
  into a part, this is done with re-exports.
- No `[[test]]` target renamed, no test fn renamed, and no test module path changed. J's test
  modules keep their paths because the host's module path (`…::self_update`,
  `…::builder_collection_layout`) does not change.
- The `.mfb` fixture sources under `tests/rt-behavior/` that mention a Q file name in a `'`
  comment are not edited. They are golden inputs, and the basename there names the module.

## 2. Current State

The starting state is the tree after plan-176-J. Sizes from `/tmp/loc/groups.json`, measured
by the planning census (`python3 -I /tmp/loc/census.py`; after A this is
`python3 -I tools/loc-census/census.py --file <path>`) on the working tree of `17a9b5799`:

| File | Kind | Code LOC | Test LOC (after J) | `#[test]` in file |
|---|---|---|---|---|
| `src/codegen/collection/assign/builder_inplace_assign.rs` | code | 1262 | 0 | 0 |
| `src/codegen/collection/assign/builder_inplace_setmap.rs` | code | 1027 | 0 | 0 |
| `src/codegen/collection/assign/self_update.rs` | code | 1059 (census row 1930, see below) | 2 (`#[cfg(test)] mod tests;`) | 0 (J moved the 9) |
| `src/codegen/collection/assign/string_self_update.rs` | code | 1805 | 133 inline (stays) | 2 |
| `src/codegen/collection/compare/builder_collection_compare.rs` | code | 1192 | 128 inline (stays) | 3 |
| `src/codegen/collection/layout/builder_collection_layout.rs` | code | 2592 | 2 (`#[cfg(test)] mod tests;`) | 0 (J moved the 19) |
| `src/codegen/collection/list/list_mutate.rs` | code | 4202 | 0 | 0 |
| `src/codegen/collection/map/map_mutate.rs` | code | 1293 | 0 | 0 |
| `src/codegen/collection/search/builder_search.rs` | code | 1264 | 0 | 0 |

`self_update.rs`: groups.json says 1930 code / 1214 test. The planning census closed the
`#[cfg(test)] const SELF_UPDATE_TABLE: &[…] = &[` span after one row. Measured with the extent
rule A's guard implements, `python3 -I /tmp/loc/span.py src/codegen/collection/assign/self_update.rs`
gives code 1059 (lines 1–1441) and test 2085. J moves all 2085 test LOC to `self_update/tests/`, so Q
starts from 1059 code.

Line numbers below are as of `17a9b5799`. J removes only test regions (self_update.rs from line
1442, builder_collection_layout.rs from line 3739), so every code range below is unchanged at Q's
start. Item names are authoritative if lines drift. Per-part LOC =
`python3 -I /tmp/loc/span.py <file> <a> <b>` summed over the part's ranges; after A this is
`tools/loc-census/census.py --file <file> --lines A B`.

### Verified properties

- **Parent imports reach parts through `use super::*;`, with no unused-import warning.**
  Probe `/tmp/loc/probe_qr`: `mod.rs` does `use a::{fa, only_b_uses};`, sibling `b.rs` uses
  `only_b_uses` only through `use super::*;`, and mod.rs never calls it.
  `cargo build` printed no warning and the binary printed `7`. Glob cycles (`use super::*;` in
  parts, `use part::*;` in mod.rs) compile without warnings: `/tmp/loc/probe2` printed `6`.
- **A child module sees its parent's private items**, including methods and struct fields.
  So items that stay in `mod.rs` need no visibility change. An item moved into a part and used by
  `mod.rs` or a sibling needs `pub(super)` (Rust privacy rule; the lists below are the
  word-boundary xref of each part, `/tmp/pqwork/xref.py`).
- **J's tests use no item Q moves into a part.** For each of the 22 items going to
  `self_update/{global_capacity,scratch,record_temps}.rs`:
  `awk 'NR>=1442' src/codegen/collection/assign/self_update.rs | grep -cw <name>` → 0. For
  the layout tests (lines 3739–4375), the only `type_predicates` items they name that are private
  (`flatness_walk`, `is_resource_alias_payload`) appear in doc comments only
  (`sed -n '3739,4375p' … | grep -nw` → lines 224 and 485, both `///`). Every other name they
  use is `pub(crate)` and re-exported by the glob below.
- **One integration test reads a Q file's source.**
  `tests/runtime/rt_inplace_self_update.rs:696`
  `include_str!("../../src/codegen/collection/assign/self_update.rs")` parses
  `pub(crate) const SCRATCH_ARMS: &[&str] = &[`. Plan-176-B keeps that test, and
  its `include_str!` with the path `../../src/…/self_update.rs`, in the root
  `tests/runtime/rt_inplace_self_update.rs` (plan-176-B §4, cross-letter note; find it with
  `grep -n scratch_arms_match_the_compiler tests/runtime/rt_inplace_self_update.rs`). `SCRATCH_ARMS` goes to `self_update/scratch.rs`
  verbatim, so the opener text still matches. No other test or src file `include_str!`s a Q file:
  `grep -rn 'include_str!' src tests | grep -E 'builder_inplace|self_update|collection_compare|collection_layout|list_mutate|map_mutate|builder_search'`
  → only that line and J's `field_expect.tsv` include.
- **No guard allowlist names a Q file.**
  `grep -rnE "builder_inplace_assign|builder_inplace_setmap|self_update|builder_collection_compare|builder_collection_layout|list_mutate|map_mutate|builder_search" tests/guards scripts/coverage-exceptions.txt`
  hits only `tests/guards/inplace_self_update_census.rs` doc prose, which J repointed to
  `self_update/tests/…`, a path Q leaves unchanged. `architecture_guards.rs::code_above_tests`
  truncates at the first `#[cfg(test)]`. In every Q file that marker comes after all the code being
  moved (first `#[cfg(test)]`: self_update 1448, string_self_update 2245, compare 1359, layout 3739,
  the rest none; `grep -nF '#[cfg(test)]' <file> | head -1`). So the parts are scanned exactly as
  their lines are today.
- **No Q file is host-cfg-gated** (`grep -n '^#!\?\[cfg' <file>` → none outside test items), so
  the host `cargo check` covers them.
- **Moving Rust code cannot change emission through source locations** (A §2 Verified:
  no `module_path!`/`file!`/`line!`, no link-order registration).

## 3. Design Overview

Conventions (A's build-tooling section), applied to every file:

- `foo.rs` → `foo/mod.rs` + parts. `mod.rs` keeps the original `use` block, the shared types and
  helpers, and the `mod <part>;` lines. Each part starts with `use super::*;` and reopens
  `impl CodeBuilder<'_> { … }` around its methods. Prune a `use` only when
  `cargo check --all-targets` reports it unused. If pruning removes a name a test file needs,
  that test file imports it explicitly.
- Moved text is byte-for-byte identical, including doc comments and attributes. The only changes
  are the visibility words listed per file.
- Each spec citation, `.ai/` line and source comment that names a moved file is repointed in the
  same commit as the move. `spec_citations_resolve` turns red on every `[[…/foo.rs:X]]` once
  `foo.rs` is gone, which forces the sweep.

Gate class: provably-neutral code motion, so **byte-identity is the gate**. The scoped
selectors that cover collection and string codegen are `collections` and `strings`
(`ls tests/byte-identity` lists both). Each phase runs both. The letter ends with one `all`
run. A diff means root-cause it: objdump ONE fixture, fix the motion error, re-run. A diff is
never a reason to stop.

Where risk concentrates: `builder_collection_layout` (30 citations; the type predicates exist both
as free fns and as `CodeBuilder` wrappers; `layout/mod.rs` glob re-export) and `list_mutate`
(9 parts, 11 citations). These go last.

Rejected alternatives:

- *Keeping `foo.rs` as the module root with `foo/` parts beside it.* Rejected because it leaves
  every `[[foo.rs:X]]` citation green while the symbol has moved away. The spec test is
  file-level only (A §2), and `foo/mod.rs` makes it fail loudly.
- *Splitting `map_mutate::lower_map_set_in_place` (869 LOC, one fn).* It fits the 1000 rule with
  128 lines of headroom in its own file (1000 − 869 − 3 wrapper lines). Decomposing it would be an extract-method refactor,
  which the owner reserved for functions over the limit (letters S–V).
- *Q's own test layout for `self_update` (map Q §self_update "Layout (tests)").* Superseded: J
  owns the test side and lands first. Q keeps J's `tests/` tree untouched.
- *Keeping `kind2_layout_tests` inline in `builder_collection_layout/mod.rs`.* Superseded for the
  same reason: J moved it, with the other two layout test modules, to `builder_collection_layout/tests/mod.rs`.

## 4. Detailed Design

### 4.1 `search/builder_search.rs` (1264)

- `search/builder_search/mod.rs` (588): lines 1–9 (`use`s) + impl 40–675
  (`emit_ascii_scalar_fastforward`, `lower_find`, `lower_list_find_item`,
  `lower_list_find_sublist`). Adds `mod mid;` and `pub(crate) use mid::{MidStringRegs, MidWindow};`.
  `string_self_update.rs` imports `collection::search::builder_search::MidStringRegs`.
- `…/builder_search/mid.rs` (676): 10–38 (`MidStringRegs`, `MidWindow`) + impl 677–1391
  (`lower_mid_string_window`, `lower_mid`, `lower_list_mid`).
- `pub(super)` edits: 0.
- Citations (4, `grep -rnoE "\[\[src/codegen/collection/search/builder_search.rs:[^]]+\]\]" src/docs/spec`):
  `unicode/02_strings-model.md` `lower_find` (:140) → `builder_search/mod.rs`; `lower_mid`
  (:94, :109) and `lower_mid_string_window` (:134) → `builder_search/mid.rs`.
- Other refs: none (§Phase 1 sweep command).

### 4.2 `map/map_mutate.rs` (1293)

- `map/map_mutate/mod.rs` (424): 1–353 (`use`s; the derive'd `InlineGrow` enum at 10–40;
  `is_map_or_set_type`, `map_or_set_may_tombstone`; first impl `tombstone_field_paths`,
  `collect_tombstone_field_paths`, `emit_tombstone_holder_settle`; second impl
  `emit_map_settle_value`, `emit_map_settle_slot`, `emit_map_retire`,
  `emit_inline_grow_extend_size`, `emit_inline_grow_split`, `emit_inline_grow_free_old`) +
  1323–1587 (`emit_map_reclaim_before_grow`, `emit_map_find_index`, `emit_map_mark_removed`,
  `emit_map_compact_from`, `lower_map_remove_key_in_place`). Adds `mod set_in_place;`.
  `InlineGrow` and `is_map_or_set_type` stay here, so the outside paths
  `collection::map::map_mutate::{InlineGrow, is_map_or_set_type}` are unchanged.
- `…/map_mutate/set_in_place.rs` (869 + its `use super::*;` and the 2-line impl wrapper):
  355–1321, `lower_map_set_in_place`.
- `pub(super)` edits: 0.
- Citations (6): `memory/05_collections.md` `emit_map_settle_value` (:327),
  `emit_tombstone_holder_settle` (:328), `lower_map_remove_key_in_place` (:795),
  `emit_map_reclaim_before_grow` (:1201) → `map_mutate/mod.rs`; `lower_map_set_in_place`
  (:285, :1007) → `map_mutate/set_in_place.rs`.
- Prose: `.ai/collections.md:532` names `src/target/shared/code/map_mutate.rs`, a path that does not
  exist (`ls src/target/shared/code/map_mutate.rs` → No such file). This is a pre-existing stale
  path, repointed to `src/codegen/collection/map/map_mutate/`. Also `src/codegen/builtins/collections/gen_mutate.rs:6` and
  `gen_map.rs:8` (`map_mutate.rs` → `map_mutate/`);
  `tests/runtime/rt_map_fifo_removal_flat.rs:5` (`map_mutate.rs:lower_map_remove_key_in_place` →
  `map_mutate/mod.rs:…`); `tests/runtime/rt_map_list_value_grow_cost.rs:5`
  (`map_mutate.rs:emit_map_reclaim_before_grow` → `map_mutate/mod.rs:…`). Locate with the grep, not
  the line, because B/C may have moved those test lines.

### 4.3 `compare/builder_collection_compare.rs` (1192 + 128 test)

- `compare/builder_collection_compare/mod.rs` (280 code + 128 test): 1–344 (`use`s,
  `ContentShape`, `ContentRoutine`, `ContentWalk`; impl `emit_compare_bytes_branch`,
  `emit_byte_compare_loop`, `emit_comparable_values_match_branch`,
  `emit_comparable_values_match_branch_from_slots`, `emit_compare_aligned_bytes_branch`) +
  1359–1508, the inline `#[cfg(test)] mod tests`, which stays with its 3 tests:
  `a_type_is_compared_by_one_routine_however_many_places_hold_it`,
  `an_entry_free_list_is_one_range_and_any_other_is_walked_by_entry`,
  `a_type_with_no_arm_is_an_error_not_a_byte_compare`. Adds `mod content; mod payload_match;`.
- `…/content.rs` (566): 345–981 (`emit_values_content_equal_branch`, `content_shape`,
  `content_variant_fields`, `emit_content_call`, `emit_content_call_shaped`,
  `emit_content_routine`, `emit_content_list`, `emit_content_bytes`, `emit_content_record`,
  `content_inline_words`, `content_is_words`, `emit_content_string`, `emit_content_union`).
- `…/payload_match.rs` (346): 983–1356 (`emit_collection_payload_match_branch`,
  `emit_collection_payload_matches_value_branch`, `emit_collection_payloads_match_branch`).
- `pub(super)` edits: 0. The test calls `emit_values_content_equal_branch`, which is already
  `pub(crate)`.
- Citations (2): `app/06_canvas.md:60` `emit_values_content_equal_branch` → `content.rs`;
  `language/04_types.md:148` `emit_collection_payload_match_branch` → `payload_match.rs`.
- Prose: `src/codegen/collection/layout/builder_collection_layout.rs:2282` doc
  (`builder_collection_compare.rs`'s → `builder_collection_compare/`'s);
  `src/codegen/builtins/tests/collection_compare.rs:3` (→ `collection/compare/builder_collection_compare/`).
  Not edited: the `'` comments in `tests/rt-behavior/collections/{collection-compare-payloads-rt,enum-elements-rt,comparable-records-rt}/src/main.mfb` (§1 Non-goals).

### 4.4 `assign/builder_inplace_setmap.rs` (1027)

- `assign/builder_inplace_setmap/mod.rs` (520): 1–603 (`use`s, `enum Payload`, shared emitters
  `emit_map_entry_payload` … `resolve_set_op`) + 1176–1199 (`emit_load_result_word`,
  `value_type_is_fixed`). Adds `mod set_ops; mod map_ops;`.
- `…/set_ops.rs` (233): 604–876 (`try_inplace_union_assign`, `try_inplace_intersection_assign`,
  `try_inplace_difference_assign`, `emit_claim_members`, `emit_compact_claimed`,
  `try_inplace_symmetric_difference_assign`).
- `…/map_ops.rs` (274): 878–1174 (`try_inplace_merge_assign`, `try_inplace_map_values_assign`).
- `pub(super)` edits: 0.
- Citations (2): `memory/05_collections.md` `try_inplace_difference_assign` (:800),
  `try_inplace_symmetric_difference_assign` (:801) → `set_ops.rs`.

### 4.5 `assign/builder_inplace_assign.rs` (1262)

- `assign/builder_inplace_assign/mod.rs` (814): 1–1073 (`use`s; first impl:
  `try_inplace_append_assign`, `try_inplace_set_add_assign`, `try_inplace_remove_key_assign`,
  `try_inplace_bulk_append_assign`, `try_inplace_set_assign`, `try_inplace_prepend_assign`,
  `try_inplace_concat_assign`, `lower_string_self_append_one`, `try_inplace_remove_at_assign`,
  `try_inplace_set_remove_assign`, `try_inplace_insert_assign`, `try_inplace_repeat_assign`) +
  1625–1674 (`self_slice_operands`). Adds `mod field;`.
- `…/field.rs` (448): 1074–1623 (the "plan-145-B: the field routes" section header,
  `field_slot_name`, the second impl: `forget_field_owner_constant`, `lower_field_append`,
  `lower_field_set_add`, `lower_field_set`, `lower_field_remove_key`, `lower_field_remove_at`,
  `lower_field_set_remove`, `lower_field_splice`).
- `pub(super)` edits: 8. These are the private methods in `field.rs` that `mod.rs` calls:
  `forget_field_owner_constant`, `lower_field_append`, `lower_field_set_add`, `lower_field_set`,
  `lower_field_remove_key`, `lower_field_remove_at`, `lower_field_set_remove`, `lower_field_splice`.
- Citations (3): `memory/05_collections.md` `try_inplace_repeat_assign` (:782, :1101) → `mod.rs`;
  `lower_field_set` (:860) → `field.rs`.
- Prose (`collection/assign/builder_inplace_assign.rs` → `collection/assign/builder_inplace_assign/`,
  or `…/field.rs` where a `lower_field_*` is named):
  `src/codegen/builtins/tests/{nir_fault,inplace_fields,inplace}.rs` doc lines;
  `tests/runtime/rt_field_map_fifo_removal_flat.rs:6` (`builder_inplace_assign.rs:lower_field_remove_key`
  → `builder_inplace_assign/field.rs:lower_field_remove_key`); `tests/runtime/rt_inplace_item_operand.rs:4`;
  `.ai/resources-packages.md:137`; `.ai/collections.md:9`; `.ai/collections.md:540`.
  The last one also carries a stale pointer, "wired into the `&&`-chain at `builder_control.rs`
  ~592". The dispatch is now `SELF_UPDATE_ARMS` reached through `try_inplace_self_update`
  (`grep -n 'try_inplace_self_update(' src/codegen/engine/control/builder_control.rs` → lines 189, 217).
  Rewrite the clause to "dispatched through `SELF_UPDATE_ARMS`
  (`collection/assign/self_update/mod.rs`) from `try_inplace_self_update` in `builder_control.rs`".
  That is done in Phase 3, after `self_update/mod.rs` exists. The `'` comment in
  `tests/rt-behavior/collections/inplace-decline-aliasing-rt/src/main.mfb` is not edited.

### 4.6 `assign/string_self_update.rs` (1805 + 133 test)

- `assign/string_self_update/mod.rs` (640 code + 133 test): 1–481 (`use`s,
  `STRING_SHADOW_ARMS`, `is_string_self_update`, `WindowKind`, `STRING_WINDOW_FNS`,
  `HOST_PATH_ARM_MEMBERS`, `GrowKind`, `STRING_GROW_FNS`, `GrowWrite`, `StringOut`,
  `RewriteKind`, `STRING_REWRITE_FNS`, `target_needs_string_scratch`, `string_shadow_captures`,
  `lambda_shares_resource_base`, `creator_body`, `ops_hold_string_self_update`, `StringShadow`,
  `StringRegrow`) + impl 482–870 (`resolve_string_self_update`, `string_shadow_exists`,
  `string_shadow_slot`, `publish_string_shadow`, `emit_string_regrow`, `emit_string_reserve`,
  `emit_string_set_len`) + 2245–2398, the inline `#[cfg(test)] mod tests`, which stays with its
  2 tests: `resolve_string_self_update_runs_every_gate`,
  `is_string_self_update_accepts_the_self_append_and_the_length_changing_arms`.
  Adds `mod grow; mod host_path; mod rewrite; mod window;`.
- `…/grow.rs` (588): 872–1541 (`try_inplace_string_grow_assign`, `emit_string_grow_measure`,
  `emit_grow_raise_here`, `string_pad_char_slot`, `emit_pad_to_width_measure`,
  `emit_string_grow_write`, `emit_string_move_right`, `emit_pad_copies`).
- `…/host_path.rs` (200): 1543–1794 (`emit_app_resource_prefix`, `emit_reject_dot_components`,
  `emit_reject_dot_component_check`).
- `…/rewrite.rs` (209): 1796–2037 (`emit_string_out_scratch`, `emit_string_out_publish`,
  `try_inplace_string_rewrite_assign`, `emit_ascii_scan_branch`, `emit_ascii_case_map_in_place`).
- `…/window.rs` (168): 2039–2242 (`try_inplace_string_window_assign`, `emit_string_window`,
  `emit_string_mid_window`, `try_inplace_string_identity_assign`).
- Every outside user (`StringOut`, `StringRegrow`, `HOST_PATH_ARM_MEMBERS`,
  `string_shadow_captures`, `lambda_shares_resource_base`, `is_string_self_update`,
  `target_needs_string_scratch`) names an item that stays in `mod.rs`. No re-exports.
- `pub(super)` edits: 2, in `host_path.rs`: `emit_app_resource_prefix` and
  `emit_reject_dot_components` (both called from `grow.rs`).
- Citations (10, `grep -rnoE "\[\[src/codegen/collection/assign/string_self_update.rs:[^]]+\]\]" src/docs/spec`):
  → `mod.rs`: `stdlib/14_os.md:299` `HOST_PATH_ARM_MEMBERS`, `memory/05_collections.md:735`
  `string_shadow_captures`, `unicode/02_strings-model.md:8` `resolve_string_self_update`,
  `memory/03_heap-values.md:45` `emit_string_set_len`;
  → `grow.rs`: `unicode/02_strings-model.md:80` `emit_pad_to_width_measure`;
  → `host_path.rs`: `stdlib/14_os.md:218` `emit_app_resource_prefix`;
  → `rewrite.rs`: `unicode/01_tables-and-algorithms.md:403` `try_inplace_string_rewrite_assign`;
  → `window.rs`: `try_inplace_string_window_assign` at `memory/05_collections.md:722`, `:805`
  and `unicode/02_strings-model.md:9`.
- Prose: `.ai/collections.md:100` (→ `collection/assign/string_self_update/`).

### 4.7 `assign/self_update.rs` (1059 code; tests already in `self_update/tests/` by J)

- `assign/self_update/mod.rs` (414): 1–615 (module `//!` doc, `use`s, `ArmId`,
  `SelfUpdateSite`, `FieldLevel`, `FieldSite` + impl, `FieldContainer`, `impl SelfUpdateSite`,
  `field_owner_is`, `field_place_is`, `ArmFn`, `FieldReach`, `SELF_UPDATE_ARMS`,
  `self_update_builtin`, `STRING_SELF_UPDATE_SPELLINGS`, impl `try_inplace_self_update`,
  `self_update_call_parts`, `is_self_update_call`, `returned_self_update_local`,
  `returned_field_self_update_local`, `is_global_self_update_call`) + J's
  `#[cfg(test)] mod tests;`, which resolves to the same `self_update/tests/mod.rs`. Adds
  `mod global_capacity; mod scratch; mod record_temps;` and
  `pub(crate) use global_capacity::add_global_string_capacities;`
  `pub(crate) use scratch::{module_self_updates_with, module_self_updates_with_replace, ops_hold_resource_path_self_update};`.
  The callers are `target/shared/lower.rs:114`, `target/shared/plan/symbols.rs:30,46`,
  `memory/data/data_objects.rs:713` and `string_self_update` (`grep -rn "assign::self_update::" src`).
  Every other outside import names an item that stays here.
- `…/self_update/global_capacity.rs` (62): 617–697 (`global_string_capacity_name`,
  `add_global_string_capacities`, impl `global_string_capacity`).
- `…/self_update/scratch.rs` (446): 699–1272 (`SCRATCH_ARMS`, `ops_hold_self_update`,
  `with_holds_field_self_update`, `same_field_owner`, `ops_create_scratch_closure`,
  `ops_hold_copying_sort`, `target_needs_self_update_scratch`, `module_self_updates_with`,
  `ops_hold_resource_path_self_update`, `module_self_updates_with_replace`, impl
  `function_needs_scratch` … `emit_publish_borrowed_scratch`). The line
  `pub(crate) const SCRATCH_ARMS: &[&str] = &[` moves verbatim (§2, test reads it).
- `…/self_update/record_temps.rs` (137): 1273–1440 (the "record-temp scratch (bug-766 B)" header,
  `self_update_with_items`, impl `fixed_record_size`, `install_scratch_copies`,
  `install_record_temps`, `emit_record_temp_region`, `emit_copy_record_words`).
- `pub(super)` edits: 2: `with_holds_field_self_update` (scratch.rs; used by `mod.rs`
  `returned_field_self_update_local`) and `emit_reserve_scratch_block` (scratch.rs; used by
  `record_temps.rs`).
- Citations (6): `try_inplace_self_update` at `memory/05_collections.md:668`, `:758`,
  `language/14_memory-semantics.md:24` and `language/12_collections.md:32` → `self_update/mod.rs`;
  `add_global_string_capacities` (`memory/05_collections.md:749`) → `global_capacity.rs`;
  `install_record_temps` (`memory/05_collections.md:658`) → `record_temps.rs`.
- Functional path: `tests/runtime/rt_inplace_self_update.rs` (B's root, holding
  `scratch_arms_match_the_compiler`; §2 grep),
  `include_str!("../../src/codegen/collection/assign/self_update.rs")` →
  `include_str!("../../src/codegen/collection/assign/self_update/scratch.rs")`. The doc comment
  above it (`SCRATCH_ARMS` (`src/codegen/collection/assign/self_update.rs`)) changes to
  `…/self_update/scratch.rs`.
- Prose: `.ai/collections.md:31` ("The seam is `src/codegen/collection/assign/self_update.rs`",
  listing `SELF_UPDATE_ARMS`) → `…/self_update/mod.rs`;
  `src/codegen/collection/assign/builder_inplace_sort.rs:73,102`
  (`self_update.rs:ops_hold_copying_sort` → `self_update/scratch.rs:ops_hold_copying_sort`);
  the `RETURN_NEVER` mention "in `self_update.rs`" in the `rt_inplace_self_update` tree (B moved
  it; `grep -rn 'RETURN_NEVER. in .self_update.rs' tests/runtime`) →
  J's `self_update/tests/` part holding `RETURN_NEVER`
  (`grep -rln RETURN_NEVER src/codegen/collection/assign/self_update/tests`);
  `tests/runtime/inplace_self_update/field_kinds_gen.py:18` docstring
  (`field_kind_class`, `self_update.rs` → J's part holding `field_kind_class`, same grep).
  These two follow whatever path J's comment repoints wrote, if J already repointed them.
- `.ai/collections.md:540`'s stale dispatch clause (§4.5) is fixed here.

### 4.8 `layout/builder_collection_layout.rs` (2592 code; J's `tests/mod.rs` stays)

- `layout/builder_collection_layout/mod.rs` (182 code + 2 test): 1–9 (`use`s) + 2643–2987 (free
  list-layout fns: `list_element_is_fixed_width`, `push_collection_data_pointer_into`,
  `push_collection_data_base_from_capacity`, `list_entry_stride`, `list_block_kind`,
  `kind2_payload_size`, `byte_list_entry_stride`, `refined_list_literal_type`,
  `emit_alloc_byte_list`, `byte_list_block_kind`) + J's
  `#[cfg(test)] mod tests;` (→ `builder_collection_layout/tests/mod.rs`).
  Adds `mod payload_copy; mod records; mod construct; mod access; mod type_predicates;` and
  `pub(crate) use type_predicates::*;`. The glob re-exports the 17 `pub(crate)` free items
  (`field_is_inlined_union_box`, `graph_drop_kind_names`, `is_resource_nominal`,
  `is_sendable_resource_nominal`, `record_field_is_inlined`, `record_field_is_pointer`,
  `recursive_transfer_types`, `thread_copy_symbol`, `type_components`, `type_contains_func`,
  `type_contains_resource`, `type_is_arena_transferable`, `type_is_memcpy_copyable`,
  `type_participates_in_cycle`, `type_reaches_cycle`, `union_is_data`, `RESOURCE_UNION_BOX_SIZE`).
  `layout/mod.rs`'s `pub(crate) use builder_collection_layout::*;` then carries them on, so
  `layout::X` and `layout::builder_collection_layout::X` both resolve as today. The glob is
  chosen over a 17-name list so that every current `pub(crate)` item in the moved range keeps its
  path with no list to keep in sync. `refined_list_literal_type` stays private in mod.rs; its
  users (`construct.rs`, the `kind2_layout_tests` code in `tests/mod.rs`) are descendants.
- `…/payload_copy.rs` (582): impl 11–812 (`inline_collection_payload_size`,
  `is_pointer_collection_payload_type`, `collection_payload_alignment`, the 126-LOC fn after it,
  `list_element_padding_alignment`, `emit_align_offset_slot`, `emit_align_offset_register`,
  `emit_copy_bytes`, `emit_block_copy_advance`, `emit_block_copy_backward`,
  `emit_flat_block_size`, `copy_flat_block`, `copy_collection_tight`,
  `copy_list_with_append_headroom`).
- `…/records.rs` (411): impl 866–1369 (`record_has_inline_data`,
  `emit_inlined_block_size_from_ptr_slot`, the `union_is_data` method, `is_enum_type`,
  `emit_data_union_size_to_slot`, `emit_wrap_record_in_union`, `emit_record_block_size_to_slot`,
  `emit_build_inlined_record`, `materialize_inline_value_in_arena`).
- `…/construct.rs` (767): impl 1371–2275 (`lower_len`, `lower_empty_collection`,
  `lower_list_literal`, `lower_set_literal`, `lower_map_literal`, `lower_collection_values`,
  `emit_write_collection_header`, `emit_write_collection_entry`, `emit_add_payload_length`,
  `emit_payload_length_to_stack`, `emit_copy_payload_to_collection`, `emit_reserve_map_buckets`).
- `…/access.rs` (258): impl 2277–2641 (`emit_element_value_offset`,
  `emit_collection_data_pointer_for`, `emit_collection_data_pointer`,
  `emit_load_collection_payload`, `…_with_alias_base`, `emit_load_map_payload_with_alias_base`,
  `emit_load_map_payload`, `emit_load_payload_with_stride`, `emit_copy_owned_string`,
  `emit_materialize_string_from_bytes`).
- `…/type_predicates.rs` (392): impl 813–865 (the 6 one-line `CodeBuilder` wrappers
  `record_field_is_pointer`, `type_is_memcpy_copyable`, `type_is_arena_transferable`,
  `is_resource_nominal`, `is_sendable_resource_nominal`, `record_field_is_inlined`, moved beside
  the free fns they delegate to) + 2988–3737 (section header "Record field-layout
  classification", free fns `record_field_is_pointer`, `named_field_is_pointer`,
  `collection_payload_types`, `is_resource_nominal`, `is_resource_alias_payload`,
  `is_sendable_resource_nominal`, `base_resource_type`, `Flatness`, `type_is_memcpy_copyable`,
  `type_is_arena_transferable`, `flatness_walk`, `flatness_of_model_type`,
  `record_field_is_inlined`, `RESOURCE_UNION_BOX_SIZE`, `field_is_inlined_union_box`,
  `union_is_data`, `type_components`, `type_participates_in_cycle`, `type_reaches_cycle`,
  `type_contains_resource`, `type_contains_func`, `graph_drop_kind_names`,
  `recursive_transfer_types`, `thread_copy_symbol`).
- `pub(super)` edits: 1: `base_resource_type` (type_predicates.rs; called by
  `inline_collection_payload_size` in payload_copy.rs).
- J's `builder_collection_layout/tests/mod.rs` keeps `use super::*;` (its three inline modules
  each `use super::*;` from it). `super` is still `builder_collection_layout`, and the glob
  re-export plus mod.rs's retained `use` block supply every name they use (§2 Verified; a
  child's `use super::*;` also sees the parent's imports, plan-176-U §2 probe2).
- Citations (30, `grep -rnoE "\[\[src/codegen/collection/layout/builder_collection_layout.rs:[^]]+\]\]" src/docs/spec | wc -l` → 30):
  → `payload_copy.rs`: `collection_payload_alignment` (memory/05_collections.md:166, :188;
  memory/01_scalar-storage.md:17), `copy_collection_tight` (05_collections.md:6, :550, :565, :1206;
  memory/spec.md:14), `copy_flat_block` (05_collections.md:413), `copy_list_with_append_headroom`
  (:406), `is_pointer_collection_payload_type` (:346);
  → `records.rs`: `emit_inlined_block_size_from_ptr_slot` (05_collections.md:414;
  04_arenas.md:364; 03_heap-values.md:37), `emit_record_block_size_to_slot` (03_heap-values.md:133);
  → `construct.rs`: `emit_payload_length_to_stack` (05_collections.md:357);
  → `mod.rs`: `list_element_is_fixed_width` (05_collections.md:89, :468);
  → `type_predicates.rs`: `type_is_memcpy_copyable` (05_collections.md:357; 04_arenas.md:589;
  03_heap-values.md:158), `record_field_is_pointer` (05_collections.md:357; 09_closures.md:154;
  03_heap-values.md:86), `record_field_is_inlined` (memory/spec.md:13; 03_heap-values.md:73),
  `field_is_inlined_union_box` (03_heap-values.md:83), `flatness_walk` (app/06_canvas.md:1632),
  `type_contains_func` (04_arenas.md:373; 09_closures.md:154).
  The three names that are both a method and a free fn now live together in
  `type_predicates.rs`, so each citation is unambiguous. (Spec paths are under `src/docs/spec/memory/`
  unless shown otherwise.)
- Prose:
  - `.ai/codegen-invariants.md:116`: "`record_field_is_inlined` in
    `src/codegen/collection/layout/builder_collection_layout.rs`; the authoritative call-site
    construction is `emit_build_inlined_record` in the same file" → "`record_field_is_inlined` in
    `…/builder_collection_layout/type_predicates.rs`; the authoritative call-site construction is
    `emit_build_inlined_record` in `…/builder_collection_layout/records.rs`".
  - `.ai/collections.md`, each repointed to the part holding the item named in that sentence:
    :185 (`type_participates_in_cycle` → `type_predicates.rs`), :247
    (`emit_inlined_block_size_from_ptr_slot` → `records.rs`), :274 (`list_element_is_fixed_width` →
    `mod.rs`), :546 (`copy_collection_tight` → `payload_copy.rs`), :628 (`union_is_data` and
    `inline_collection_payload_size` → "`type_predicates.rs` and `payload_copy.rs`").
  - `.ai/resources-packages.md:519` ("both in `builder_collection_layout.rs`", naming
    `is_resource_nominal`/`is_sendable_resource_nominal`) → `builder_collection_layout/type_predicates.rs`.
  - `src/optimizer/opt1/lencache.rs:9` (`lower_len` … `builder_collection_layout.rs`) →
    `builder_collection_layout/construct.rs`.
  - `src/codegen/error/emission/park_error_helper.rs:379`
    (`builder_collection_layout.rs::emit_flat_block_size`) → `builder_collection_layout/payload_copy.rs::emit_flat_block_size`.
  - `tests/codegen/codegen_inplace_remove_at_insert.rs:438` → `builder_collection_layout/`.
  - `src/codegen/collection/list/list_mutate.rs` comments at lines 76, 303 and 3398
    (`builder_collection_layout.rs`) → `builder_collection_layout/`. These are edited in this phase,
    before list_mutate moves.
  - The moved comment at old line 454 (in `copy_flat_block`, now in `payload_copy.rs`) reads
    "Byte-identical; clears `builder_collection_layout.rs:332`". It cites a line number, which is
    stale on the current file (line 332 is inside `emit_block_copy_backward`'s loop). Rewrite it to
    "Byte-identical; clears the plan-71-C Family-1a finding at this call".
  - Not edited: the `'` comment in `tests/rt-behavior/collections/enum-elements-rt/src/main.mfb`.

### 4.9 `list/list_mutate.rs` (4202)

One `impl CodeBuilder<'_>` today (31–5089); every part reopens it. The largest fn is
`lower_inline_list_bulk_append_in_place` (538).

- `list/list_mutate/mod.rs` (409): 1–30 (`use`s, `SpliceAt`, `PackedLength`) + 3587–4084 (shared
  grow/repack helpers: `emit_list_packed_length`, `emit_list_insert_pack_a`,
  `emit_bulk_append_extra`, `emit_list_reclaim_before_grow`, `emit_repack_list_data`). Adds
  `mod insert; mod append; mod inline_append; mod inline_bulk_append; mod bulk_append; mod splice; mod set; mod remove;`.
  `SpliceAt` stays here, so `collection::list::list_mutate::SpliceAt` is unchanged.
- `…/insert.rs` (383): 32–478 (`lower_list_insert_collection`) + 2875–2928
  (`lower_list_prepend_in_place`, `lower_list_insert_in_place`).
- `…/append.rs` (396): 480–935 (`lower_list_append_in_place`).
- `…/inline_append.rs` (514): 937–1524 (`lower_inline_list_append_in_place`).
- `…/inline_bulk_append.rs` (538): 1526–2117 (`lower_inline_list_bulk_append_in_place`).
- `…/bulk_append.rs` (640): 2119–2873 (`lower_list_self_slice_append_in_place`,
  `emit_disjoint_copy_word_tail`, `lower_list_bulk_append_in_place`).
- `…/splice.rs` (529): 2930–3585 (`lower_list_splice_in_place`).
- `…/set.rs` (387): 4086–4565 (`lower_list_set_in_place`, `lower_list_set_in_place_with`).
- `…/remove.rs` (406): 4567–5088 (`lower_list_remove_at`, `lower_list_remove_at_in_place`).
- `pub(super)` edits: 0. The private helpers `emit_list_packed_length`, `emit_list_insert_pack_a`
  and `emit_bulk_append_extra` stay in mod.rs. `emit_disjoint_copy_word_tail` is used only in
  bulk_append.rs. `PackedLength`'s private fields are visible to the child parts.
- Citations (11, all in `memory/05_collections.md`): → `mod.rs`: `emit_list_reclaim_before_grow`
  (:813, :1164), `emit_repack_list_data` (:1163); → `insert.rs`: `lower_list_insert_collection`
  (:1026, :1171); → `append.rs`: `lower_list_append_in_place` (:949); → `bulk_append.rs`:
  `lower_list_self_slice_append_in_place` (:776); → `set.rs`: `lower_list_set_in_place` (:963),
  `lower_list_set_in_place_with` (:1114); → `remove.rs`: `lower_list_remove_at` (:1057),
  `lower_list_remove_at_in_place` (:787).
- Prose: `.ai/collections.md:9` (`collection/list/list_mutate.rs` → `collection/list/list_mutate/append.rs`,
  where `lower_list_append_in_place` lives); `src/codegen/builtins/collections/gen_mutate.rs:6`
  (`list_mutate.rs` → `list_mutate/`); the `src/codegen/builtins/tests` corpus doc line naming
  `collection/list/list_mutate.rs` (`grep -rn 'list_mutate.rs' src/codegen/builtins/tests`; H may
  have moved it) → `collection/list/list_mutate/`. Not edited: the `'` comment in
  `tests/rt-behavior/arena/member-iterable-mutate/src/main.mfb`.

### 4.10 Measurements after the split

For each new file, `python3 -I tools/loc-census/census.py --file <new path>` prints `code test`,
with code ≤ 900 and inline test ≤ 250. The LOC in §4.1–4.9 are the planning `span.py` sums. A part
adds 2–12 lines (`use super::*;`, the impl wrapper, `mod` lines). The closest to 900 is
`set_in_place.rs` (869 + 3).

## Phases

> **NOTE: keep the checkboxes current as you go. This is not bookkeeping; it is
> the only way anyone can see where the work actually is.**
>
> - Tick `- [x]` **in the same commit as the work it describes**: never in a
>   batch at the end of a phase, never "I'll update the plan after". If it is a
>   separate step it will be skipped, and the list becomes a lie.
> - Use `- [~]` for partially done and say in one line what remains.
> - Mark a task moot with `- [x] ~~text~~ — moot: <evidence>` rather than
>   deleting it. A vanished task is indistinguishable from a forgotten one.
> - Fill the phase's `Commit:` line with the hash(es) the moment it lands.
> - Add any task you discover you needed; an unlisted task that got done is a
>   task the next reader will do twice.
>
> **An unticked box means NOT DONE.** Someone reading this file, including the
> person who has to decide whether to stop, is entitled to trust that.

Every phase uses the same gate set, referred to below as **the Q gate**:

1. `cargo build --release` (est. 4 min), then `sh scripts/artifact-gate.sh target/release/mfb collections`
   and `sh scripts/artifact-gate.sh target/release/mfb strings` → both pass, `git status --porcelain tests/`
   shows no golden change (est. 1 min each).
2. `cargo test -p mfb --bin mfb codegen::collection` → all pass, and the whole-binary count
   `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` equals the count taken
   before the phase (est. 3 min).
3. `cargo check --all-targets 2>&1 | grep -c '^warning'` equals the count before the phase (est. 2 min).
4. `cargo test -p mfb --bin mfb spec_citations_resolve` → passes (est. 1 min).
5. `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings --test inplace_self_update_census` → pass (est. 1 min).
6. The phase's sweep grep (given per phase) → no hit outside the listed not-edited `.mfb` fixtures,
   `bugs/` and `planning/`.
7. `cargo test --test file_size_budget 2>&1 | grep -E '<phase paths>'` → only `STALE` rows (the old
   `foo.rs` ceiling entries) and no `NEW`/`GREW`. Then `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`
   removes them; commit `tests/guards/file_size_ceilings.txt` with the phase (est. 1 min).

### Phase 1: search, map mutate, compare

- [x] `src/codegen/collection/search/builder_search.rs` → `builder_search/{mod,mid}.rs` per §4.1,
      with the `pub(crate) use mid::{MidStringRegs, MidWindow};` re-export.
      Done: the re-export is `pub(crate) use mid::MidStringRegs;` only — `MidWindow` has no user outside `mid.rs`
      (`grep -rn MidWindow src`), and re-exporting it warns `unused import`. 0 visibility edits.
- [x] `src/codegen/collection/map/map_mutate.rs` → `map_mutate/{mod,set_in_place}.rs` per §4.2.
- [x] `src/codegen/collection/compare/builder_collection_compare.rs` → `builder_collection_compare/{mod,content,payload_match}.rs` per §4.3.
      Done: `items.py split` → 13, 3 methods (the comparable-match helpers added since planning stay in `mod.rs`);
      0 visibility edits; `motion.py` → only wiring and impl wrappers for all three.
- [x] Repoint the 4 + 6 + 2 spec citations listed in §4.1–4.3.
      Done: `citesweep.py` → 4 + 6 + 4 (two compare cites added since); `citedef.py src/codegen/collection/` → 0 not
      defined.
- [x] Prose sweep per §4.2–4.3, including the pre-existing stale `.ai/collections.md:532`
      `src/target/shared/code/map_mutate.rs` (evidence: `ls` → No such file).
      Done: the seven sites of §4.2–4.3 (`.ai/collections.md` now :533).

Acceptance: the three modules are split with byte-identical codegen, and no reference names the
old files.
  Check: the Q gate. Sweep grep:
  `grep -rnE "builder_search\.rs|map_mutate\.rs|builder_collection_compare\.rs" src tests scripts .ai AGENTS.md tools`
  → only the three `tests/rt-behavior/collections/*/src/main.mfb` comments of §4.3. File-size grep
  paths: `builder_search|map_mutate|builder_collection_compare` (est. 15 min total).
  Verified 2026-10-19: G2 (`bash`) collections, strings → 0 diffs, `git status --porcelain tests/` → only the ceiling
  file and the two repointed test docs; `codegen::collection` → `ok. 58 passed` (= G0's 58), list 5018; check
  warnings 0; `spec_citations_resolve` ok; guards → 9, 3, 8, 7 + 1 ignored; sweep → only the three fixture comments;
  budget → three STALE rows (1264, 1293, 1246) before, none after; census → builder_search 590/679, map_mutate
  425/872, compare 336/128 + 569/349.
Commit: 60ff846dc

### Phase 2: in-place set/map and in-place assign

- [x] `src/codegen/collection/assign/builder_inplace_setmap.rs` → `builder_inplace_setmap/{mod,set_ops,map_ops}.rs` per §4.4.
      Done: `items.py split` → 6, 2 methods; 0 visibility edits.
- [x] `src/codegen/collection/assign/builder_inplace_assign.rs` → `builder_inplace_assign/{mod,field}.rs`
      per §4.5, with the 8 `pub(super)` edits.
      Done: `field_slot_name` + the second impl's 8 methods; exactly the 8 `pub(super)`; the emptied second
      `impl CodeBuilder` shell removed from `mod.rs`; `motion.py` → wiring and the 8 only.
- [x] Repoint the 2 + 3 spec citations.
      Done: `citesweep.py` → 2 + 3.
- [x] Prose sweep per §4.5, except the `.ai/collections.md:540` dispatch clause (Phase 4).
      Done: the 8 sites (`.ai/collections.md` :9 and :541 — its path now `builder_inplace_assign/`, its clause left for
      Phase 4).

Acceptance: both modules are split with byte-identical codegen.
  Check: the Q gate. Sweep grep:
  `grep -rnE "builder_inplace_setmap\.rs|builder_inplace_assign\.rs" src tests scripts .ai AGENTS.md tools`
  → only `tests/rt-behavior/collections/inplace-decline-aliasing-rt/src/main.mfb`. File-size grep
  paths: `builder_inplace_setmap|builder_inplace_assign` (est. 15 min).
  Verified 2026-10-19: G2 collections, strings → 0 diffs; `codegen::collection` → 58 passed, list 5018; check warnings
  0; `spec_citations_resolve` ok; guards → 9, 3, 8, 7 + 1 ignored; sweep → only the fixture; budget → two STALE rows
  (1027, 1262) before, none after; census → setmap 522/236/277, assign 815/453.
Commit: —

### Phase 3: string self-update

- [ ] `src/codegen/collection/assign/string_self_update.rs` → `string_self_update/{mod,grow,host_path,rewrite,window}.rs`
      per §4.6, with the 2 `pub(super)` edits. The inline `mod tests` stays in mod.rs.
- [ ] Repoint the 10 spec citations.
- [ ] `.ai/collections.md:100` → `collection/assign/string_self_update/`.

Acceptance: the module is split with byte-identical codegen, and its 2 inline tests still pass at
the same paths.
  Check: the Q gate, plus `cargo test -p mfb --bin mfb string_self_update::tests` → 2 passed.
  Sweep grep: `grep -rn "string_self_update\.rs" src tests scripts .ai AGENTS.md tools` → no hit.
  File-size grep path: `string_self_update` (est. 15 min).
Commit: —

### Phase 4: self-update dispatch

- [ ] `src/codegen/collection/assign/self_update.rs` → `self_update/{mod,global_capacity,scratch,record_temps}.rs`
      per §4.7, with the two `pub(crate) use` re-export lines and the 2 `pub(super)` edits. J's
      `#[cfg(test)] mod tests;` line stays at the end of mod.rs.
- [ ] `tests/runtime/rt_inplace_self_update.rs` (plan-176-B's root, holding
      `scratch_arms_match_the_compiler`; §2 grep): in the same commit as the split, repoint
      `include_str!("../../src/codegen/collection/assign/self_update.rs")` →
      `include_str!("../../src/codegen/collection/assign/self_update/scratch.rs")`, the part
      holding `pub(crate) const SCRATCH_ARMS: &[&str] = &[`, and repoint the doc comment
      above the test the same way (§4.7). The test asserts that the names it parses from that
      declaration equal the harness's own `SCRATCH_ARMS`, and that the parsed list is non-empty.
      So a pass shows every arm is still found in the new file.
      Check: `cargo test --test rt_inplace_self_update scratch_arms_match_the_compiler` → 1 passed.
- [ ] Repoint the 6 spec citations.
- [ ] Prose sweep per §4.7, including `.ai/collections.md:540`'s stale `builder_control.rs ~592`
      clause (§4.5; evidence `grep -n 'try_inplace_self_update(' src/codegen/engine/control/builder_control.rs`).

Acceptance: the module is split with byte-identical codegen. J's 9 self-update tests and the
source-reading harness test pass.
  Check: the Q gate, plus `cargo test -p mfb --bin mfb self_update::tests` → 9 passed, and
  `cargo test --test rt_inplace_self_update scratch_arms_match_the_compiler` → 1 passed (est. 3 min).
  Sweep grep: `grep -rnE "[^_]self_update\.rs" src tests scripts .ai AGENTS.md tools` → no hit.
  File-size grep path: `assign/self_update` (est. 20 min).
Commit: —

### Phase 5: collection layout

- [ ] `src/codegen/collection/layout/builder_collection_layout.rs` → `builder_collection_layout/{mod,payload_copy,records,construct,access,type_predicates}.rs`
      per §4.8, with `pub(crate) use type_predicates::*;` and the 1 `pub(super)` edit. J's
      `#[cfg(test)] mod tests;` declaration stays in mod.rs.
- [ ] Repoint the 30 spec citations.
- [ ] Prose sweep per §4.8: `.ai/codegen-invariants.md:116` reworded, the 5 `.ai/collections.md` lines,
      `.ai/resources-packages.md:519`, `lencache.rs:9`, `park_error_helper.rs:379`,
      `codegen_inplace_remove_at_insert.rs:438`, the three `list_mutate.rs` comments, and the stale
      line-number comment in `payload_copy.rs`.

Acceptance: the module is split with byte-identical codegen. Its 19 J-moved tests pass at the
same paths.
  Check: the Q gate, plus `cargo test -p mfb --bin mfb builder_collection_layout::` → 19 passed.
  Sweep grep: `grep -rn "builder_collection_layout\.rs" src tests scripts .ai AGENTS.md tools` →
  only `tests/rt-behavior/collections/enum-elements-rt/src/main.mfb`. File-size grep path:
  `builder_collection_layout` (est. 20 min).
Commit: —

### Phase 6: list mutate

- [ ] `src/codegen/collection/list/list_mutate.rs` → `list_mutate/{mod,insert,append,inline_append,inline_bulk_append,bulk_append,splice,set,remove}.rs` per §4.9.
- [ ] Repoint the 11 spec citations.
- [ ] Prose sweep per §4.9.

Acceptance: the module is split with byte-identical codegen.
  Check: the Q gate. Sweep grep: `grep -rn "list_mutate\.rs" src tests scripts .ai AGENTS.md tools` →
  only `tests/rt-behavior/arena/member-iterable-mutate/src/main.mfb`. File-size grep path:
  `list_mutate` (est. 20 min).
Commit: —

### Phase 7: letter gate

- [ ] `sh scripts/artifact-gate.sh target/release/mfb all` on the Phase 6 binary (est. 15–20 min).
      This is justified once per letter: code motion in codegen can change emission for any
      fixture on any target, and only `all` covers every fixture's native goldens on all five targets.
- [ ] `cargo test --test rt_self_update_cost --test rt_map_fifo_removal_flat --test rt_inplace_self_update`
      (the runtime suites that exercise the moved in-place paths; est. 10 min).
- [ ] `cargo test --test file_size_budget 2>&1 | grep -E 'src/codegen/collection/'` → no row, and
      `grep -E 'src/codegen/collection/' tests/guards/file_size_ceilings.txt` → no `ceiling` line.

Acceptance: the whole letter is byte-identical on every target and the budget is clean.
  Check: the three commands above. They print a pass, all tests passed, and no output, respectively.
Commit: —

## Validation Plan

- Byte-identity: the scoped `collections` + `strings` artifact gates per phase, and `all` once in Phase 7.
- Unit tests: `cargo test -p mfb --bin mfb codegen::collection` per phase. The whole-binary test
  count is unchanged per phase.
- Integration: `scratch_arms_match_the_compiler` (the only source-reading test) in Phase 4, and the
  runtime self-update/map suites in Phase 7.
- Guards: architecture_guards, no_type_strings, no_operator_strings, inplace_self_update_census per phase.
- Doc sync: `spec_citations_resolve` per phase. No `mfb man` topic changes (no contract changes).
  No spec prose changes beyond citation paths.
- Full workspace suite: not in Q. It runs once, in plan-176-V's Validation Plan.

## Decisions

- Rule revised — owner, 2026-10-18: three kinds, non-comment non-blank LOC, scope `.rs`/`.mfb`
  under `src/ packages/ examples/ repository/ tests/`: `code` (a file that is not test-only)
  ≤ 1000 → split into modules; `inline` (`#[cfg(test)]` / `TESTING … END TESTING` LOC in such a
  file) ≤ 250 → moved to a test-only file; `test` (a test-only file's total) ≤ 1000 → split into
  modules. Parts target ≤ 900. Ceiling rows are `ceiling <code|inline|test> <n> <path>`. No
  exemptions. (Q's two remaining inline modules are 133 and 128.)
- Generated (`GENERATED` banner) files are skipped. The guard warns with a banner and never fails. A new module is
  always an acceptable split. Owner, 2026-10-14.
- A single function over the code limit is decomposed by extract-method refactoring only in
  letters S–V. Q has no function over 1000 (largest: `lower_map_set_in_place`, 869), so Q does no
  extraction. Owner, 2026-10-14.
- Q adds no exemption (there are none, owner 2026-10-18). `SELF_UPDATE_TABLE` (880 LOC, the
  largest test-side unit of a Q file) fits the 1000 test-only limit in J's `self_update/tests/`.
  Author.
- J owns the test side of `self_update.rs` and `builder_collection_layout.rs`, and Q takes J's
  layout unchanged. Author.
- `builder_collection_layout/mod.rs` re-exports `type_predicates` by glob, not by list (§4.8). Author.
- Prose mentions of moved files are repointed in Rust comments, tests, `.ai/`, scripts and tool
  docstrings. They are not edited inside `tests/rt-behavior` `.mfb` fixture sources (golden inputs).
  Author.
- Pre-existing stale references found while sweeping are fixed here: `.ai/collections.md:532`
  (non-existent `src/target/shared/code/map_mutate.rs`), `.ai/collections.md:540`
  (`builder_control.rs ~592`), and the `builder_collection_layout.rs:332` line citation in a
  `copy_flat_block` comment. Author, per AGENTS.md "never leave a bug".

## Corrections

- 2026-10-19 — Every `sh scripts/artifact-gate.sh` runs as `bash` (the script needs bash).

- 2026-10-18: Rule revised by the owner (code ≤ 1000, inline ≤ 250, test-only file ≤ 1000, no exemptions); file list and split design unchanged; rule text, Decisions, and references to J's layout test module (now one `builder_collection_layout/tests/mod.rs`), J's `self_update/tests/` parts and B's `rt_inplace_self_update` part reworded to stay true; `SELF_UPDATE_TABLE` is no longer an exemption.

## Summary

Q is nine pure moves in one subsystem, gated by byte-identity on the `collections` and `strings`
selectors per phase and `all` once. The real hazards are reference hazards, not logic:
74 spec citations, one source-reading integration test (`SCRATCH_ARMS`), the
`layout::*` re-export chain that the collection lowerings import through, and 13 `.ai/`
lines (two of them already stale). The riskiest files go last.
