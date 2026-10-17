# plan-176-J: Backend and remaining codegen test code

Last updated: 2026-10-14
Effort: large (3h–1d)
Depends on: plan-176-I

J moves the over-budget test code in `src/arch`, `src/os`, `src/binary_repr` and the remaining
`src/codegen` files (io/input, resource, string/format, collection, link, runtime/canvas) into
topic modules, so that no new file is over 250 test LOC. 35 files are in scope: 24 whole test
files and 11 code files whose inline `#[cfg(test)]` code is over 250. No test is added, removed
or renamed; only module paths gain a topic segment (`arch::x86_64::encode::tests::mov_reg_reg`
→ `arch::x86_64::encode::tests::integer_alu::mov_reg_reg`). Host code does not move; the code
splits of `v128.rs`, `self_update.rs`, `builder_collection_layout.rs`, `link_thunk.rs` and
`vulkan.rs` are letters O, Q and V. Four test-side units that are one indivisible item over
250 become exempt files.

The single outcome: after J lands, `cargo test --test file_size_budget` shows no row for any of
J's 35 files or the files J creates, and `tests/guards/file_size_ceilings.txt` has no `ceiling
test` row for them — only the four `exempt` rows J adds (§4.0).

References:

- `planning/plan-176-A-file-size-guard.md` — the rule, §Decisions, counting rules (the
  `#[cfg(test)]` extent rule that makes `self_update.rs` 2085 test LOC), and "Splitting an
  over-budget file" in `.ai/build-tooling.md`.
- `planning/plan-176-H-codegen-builtins-engine-tests.md` §4.1 — the `/tests/` component rule in
  `tests/guards/architecture_guards.rs` that J's arch/os topic files rely on.
- `.ai/testing-gates.md` § "Splits must sweep man AND spec citations", and the coverage section.
- `.ai/arch-abi.md` — names `src/arch/x86_64/select.rs`, `src/arch/riscv64/select.rs`,
  `src/os/windows/link/mod.rs` (host paths J keeps) and `signed_build_emits_both_mfbnote_and_mfbsign_disjoint`
  by fn name (stable).
- `.ai/collections.md` — the self-update seam; its `SELF_UPDATE_TABLE` bullet is repointed (§4.31).
- `.ai/codegen-invariants.md` — names `float_parse_ref.rs` (repointed, §4.27).
- `.ai/net-tls.md` — cites `codegen::resource::tests::every_builtin_close_refuses_an_already_closed_handle`
  (repointed, §4.24).
- `.ai/canvas-threading.md` — names `the_pipeline_specializes_every_region_base` /
  `the_shared_buffer_holds_every_region` by fn name (stable).
- `.ai/resources-packages.md` — names `every_known_ctype_lowers`, `every_builtin_resource_has_a_close_op`
  by fn name (stable).
- Spec topics whose citations move: `src/docs/spec/architecture/14_aarch64-instruction-set.md`
  (2), `src/docs/spec/architecture/15_x86_64-instruction-set.md` (2),
  `src/docs/spec/linker/04_symbols-and-relocations.md` (1), `src/docs/spec/linker/07_linux-aarch64.md` (1).
- `scripts/coverage-exceptions.txt` — three entries whose prose names moved test paths (§4.14,
  §4.18, §4.20).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-I is complete and archived | `ls planning/plan-176-I-* planning/completed/plan-176-I-*` → only `planning/completed/…` | NOT MET (2026-10-14) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | NOT MET (2026-10-14) |
| The architecture guards treat a `tests` directory component as test code (plan-176-H §4.1) | `grep -rn 'contains("/tests/")' tests/guards/architecture_guards.rs` → a hit in each of `is_physical_scan_exempt`, `no_hand_picked_vreg` and `emission_lines` | NOT MET (2026-10-14) |
| J's files and the docs/scripts J edits have no uncommitted changes | `git status --porcelain -- src/arch src/os src/binary_repr src/codegen/io/input src/codegen/resource/tests src/codegen/string/format src/codegen/collection/assign/self_update.rs src/codegen/collection/layout/builder_collection_layout.rs src/codegen/link/thunk/link_thunk.rs src/codegen/runtime/canvas/vulkan.rs src/target/win_x86_64/app/mod.rs src/docs/spec/architecture src/docs/spec/linker .ai/collections.md .ai/codegen-invariants.md .ai/net-tls.md scripts/coverage-exceptions.txt tests/guards/inplace_self_update_census.rs tests/runtime/rt_inplace_self_update.rs tests/runtime/inplace_self_update` → empty | MET (2026-10-14) |

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

- Every file in §2's table is ≤ 250 test LOC, and every file J creates is ≤ 250 test LOC
  except the four exempt files of §4.0
  (`python3 -I tools/loc-census/census.py --file <path>` → second field ≤ 250).
- `cargo test --test file_size_budget` prints no row naming a J path; the ceiling file has no
  `ceiling test` row for one and exactly four new `exempt` rows.
- The unit-test binary lists the same number of tests before and after each phase.

### Non-goals (explicit constraints)

- No behavior change. No test body edit beyond moving it (dedented), a leading `use super::*;`,
  the visibility edits listed in §4 (19 in total), and the `include_str!` path repoints of §4
  (5, each the same file at its new relative path).
- No `#[test]` fn renamed, added or removed. No `[[test]]` target renamed (J touches none).
- No host code moves; `vulkan.rs`'s `#[cfg(test)] const SPIRV_MAGIC` and `appimage/mod.rs`'s two
  `#[cfg(test)] const RUNTIME_*_LEN` stay in their hosts. `src/codegen/io/input/corpus.rs` is not
  split (exempt, owner decision).
- Every reference implementation keeps its module path and its `pub(crate)` surface
  (`float_parse_ref::{scan, Scanned, lemire, BiasedFp, DECLINED, parse, …}`,
  `float_format_sci_ref::{sci_digits, Fraction}`, `model::{decode, Outcome, Text, Token, CLUSTER, EVENTS, FINAL}`,
  `corpus::{cases, Case}`) through `mod.rs` re-exports.
- No guard weakens; no production file becomes exempt.

## 2. Current State

LOC: `/tmp/loc/groups.json` (planning census on the working tree of `17a9b5799`); after
plan-176-A, `python3 -I tools/loc-census/census.py --file <path>`. Re-measured now with
`python3 -I /tmp/loc/span.py <file>`: every row agrees except
`src/codegen/collection/assign/self_update.rs`, which groups.json lists as 1214 but which is
(1059 code, 2085 test) under plan-176-A's extent rule (`span.py` → `(1059, 2085)`): every
top-level item from line 1442 to 3829 is `#[cfg(test)]`. Tests: `grep -cE '#\[(tokio::)?test\]' <file>`.

| File | Kind | Test LOC | Tests |
|---|---|---|---|
| `src/arch/aarch64/decode/tests.rs` | test file | 742 | 11 |
| `src/arch/aarch64/encode/tests.rs` | test file | 1251 | 41 |
| `src/arch/riscv64/encode/tests.rs` | test file | 1166 | 41 |
| `src/arch/x86_64/encode/tests.rs` | test file | 3185 | 130 |
| `src/binary_repr/tests/cross_package_tests.rs` | test file | 303 | 7 |
| `src/binary_repr/tests/fixtures.rs` | test file | 251 | 0 |
| `src/binary_repr/tests/gap_tests.rs` | test file | 282 | 10 |
| `src/binary_repr/tests/reader_gap_tests.rs` | test file | 303 | 12 |
| `src/binary_repr/tests/reader_tests.rs` | test file | 727 | 40 |
| `src/binary_repr/tests/sections_tests.rs` | test file | 651 | 18 |
| `src/binary_repr/tests/writer_tests.rs` | test file | 375 | 20 |
| `src/binary_repr/tests/writer_walker_tests.rs` | test file | 584 | 7 |
| `src/codegen/io/input/corpus.rs` | test file | 903 | 0 |
| `src/codegen/io/input/encode_tests.rs` | test file | 436 | 4 |
| `src/codegen/io/input/model.rs` | test file | 418 | 0 |
| `src/codegen/io/input/reader_tests.rs` | test file | 284 | 11 |
| `src/codegen/io/input/tests.rs` | test file | 386 | 7 |
| `src/codegen/resource/tests/mod.rs` | test file | 325 | 8 |
| `src/codegen/string/format/float_format_sci_ref.rs` | test file | 320 | 0 |
| `src/codegen/string/format/float_parse_ref.rs` | test file | 763 | 10 |
| `src/os/linux/appimage/squashfs/tests.rs` | test file | 726 | 30 |
| `src/os/linux/link/tests.rs` | test file | 1796 | 51 |
| `src/os/macos/link/tests.rs` | test file | 1656 | 45 |
| `src/os/windows/link/spike.rs` | test file | 327 | 2 |
| `src/arch/riscv64/select.rs` | inline | 629 | 39 |
| `src/arch/riscv64/v128.rs` | inline | 913 | 36 |
| `src/arch/x86_64/select.rs` | inline | 279 | 13 |
| `src/codegen/collection/assign/self_update.rs` | inline | 2085 | 9 |
| `src/codegen/collection/layout/builder_collection_layout.rs` | inline | 432 | 19 |
| `src/codegen/link/thunk/link_thunk.rs` | inline | 551 | 7 |
| `src/codegen/runtime/canvas/vulkan.rs` | inline | 748 | 8 |
| `src/os/linux/appimage/mod.rs` | inline | 254 | 19 |
| `src/os/linux/object.rs` | inline | 259 | 16 |
| `src/os/macos/object.rs` | inline | 420 | 17 |
| `src/os/windows/link/mod.rs` | inline | 686 | 23 |

Totals: 35 files, 25,416 test LOC (groups.json's 24,545 with `self_update.rs` at 2085), 711
tests (sum of the per-file greps).

### Verified properties

- **Topic layouts compile warning-free.** `/tmp/hij_probe`: a non-`mod.rs` host `src/host.rs`
  with `#[cfg(test)] mod tests;` loads `src/host/tests/mod.rs` (`use super::*;` + `mod` lines),
  whose topics see the host's private items and private `use` imports; `cargo test` → no
  warning. A data-only part under `use super::*;` → `warning: unused import`, so a part that
  names nothing from its parent carries no `use` line. `/tmp/loc/probe2`: glob cycles
  (`use part::*;` + `use super::*;`) → no warning. A `foo.rs` host and a `foo/` directory
  coexist (the same probe), which is the layout J uses for every non-`mod.rs` host.
- **`include_str!` resolves relative to the file that contains the macro.** Five moved tests
  include files by relative path; each is repointed by the extra directory depth (§4).
- **What the architecture guards scan does not grow.** After plan-176-H, any path with a
  `/tests/` component is exempt from `is_physical_scan_exempt`, `no_hand_picked_vreg` and
  `emission_lines`. Every J file that lands outside a `tests/` directory
  (`float_parse_ref/{mod,scan,lemire,exact}.rs`, `float_format_sci_ref/{mod,exact}.rs`,
  `io/input/model/{mod,sequences}.rs`, `windows/link/spike/{mod,image}.rs`) holds lines the
  guards already scanned in the parent file: `spike.rs` contains neither `#[cfg(test)]` nor
  `mod tests` (`grep -c "#\[cfg(test)\]\|mod tests" src/os/windows/link/spike.rs` → 0) so it is
  scanned whole today; `model.rs` has no inline test module; the two `_ref` files' non-test
  lines sit above their test module, and their new `mod.rs` keeps `#[cfg(test)] mod tests;` as
  its LAST item so `code_above_tests` still sees every part declaration and item. The
  `no_type_strings` / `no_operator_strings` guards scan the same lines for the same reason
  (`is_test_file` keys on `/tests/` and test file names).
- **The self-update census leaves the host's scanned region unchanged.** Its first
  `#[cfg(test)]` (line 1442) stays the cut point: the host keeps `#[cfg(test)] mod tests;` there.
  The census goes under `self_update/tests/census/` so it stays exempt from the string guards
  (`rel.contains("/tests/")`) once its per-item `#[cfg(test)]` no longer sits in a scanned file.
- **No census item is used outside `self_update.rs`.** `grep -rn "SELF_UPDATE_TABLE\|FIELD_KIND_TABLE\|ENABLED_SITES\|FIELD_NEVER\b\|RETURN_NEVER\|field_kind_class" src tests`
  outside the file → only prose/doc mentions (`registry/mod.rs:3324` doc, `tests/runtime/rt_inplace_self_update.rs`
  defines its own `ENABLED_SITES` and names `RETURN_NEVER` in a comment,
  `field_kinds_gen.py` and `inplace_self_update_census.rs` prose). The external
  `include_str!("../../src/codegen/collection/assign/self_update.rs")` in
  `tests/runtime/rt_inplace_self_update.rs::scratch_arms_match_the_compiler` reads
  `SCRATCH_ARMS` (host code at line 706), which does not move.
- **Only macOS- and unix-gated code moves, and this host compiles both.**
  `grep -n "cfg(target_os\|cfg(unix\|cfg(windows"` over the 35 files: `os/macos/link/tests.rs`
  (`#[cfg(target_os = "macos")]` ×8), `squashfs/tests.rs` (`#[cfg(unix)]` ×1), `appimage/mod.rs`
  (`#[cfg(unix)]` in tests; its `#[cfg(windows)]` at 291 is host code that stays). No
  Windows- or Linux-only line moves, so no cross-target check is needed.
- **Moving an inline test module out of a code file removes covered lines from its coverage
  denominator** (`.ai/testing-gates.md`). Per-file coverage is not a plan-176 gate
  (§Validation Plan "Coverage check"), so J runs no coverage measurement.

## 3. Design Overview

The H convention: a whole test file `x.rs` becomes `x/mod.rs` + topics; an inline module
becomes `#[cfg(test)] mod tests;` → `<host>/tests/mod.rs` + topics. `mod.rs` keeps the `use`
lines, shared helpers and one `mod` per topic; each topic starts with `use super::*;`; a helper
with one user moves with it. A group of topics sharing private fixtures becomes a nested
directory module (`tests/debug_symbols/mod.rs` + topics), one extra level, no `#[path]`.
Reference implementations (cfg(test) modules that are not suites) split into part files whose
cross-file items become `pub(super)` and whose public surface is re-exported from `mod.rs`.
Non-`mod.rs` hosts stay `foo.rs`; their tests go to `foo/tests/`. When O/Q/V later turn a host
into `foo/mod.rs`, its `#[cfg(test)] mod tests;` resolves to the same `foo/tests/` directory.

Gate class: test letter (same G0–G5 as H).

Risk concentrates in:
- **`self_update.rs`** — 1873 LOC of census data plus the test module move into a nested
  `tests/census/` tree; one visibility edit; one `include_str!` two levels deeper; four prose
  references in other files. Last phase.
- **`x86_64/encode/tests.rs`** — 130 tests into 18 topics; the spec cites line 1 of the file.
- **Reference-implementation splits** (`float_parse_ref`, `float_format_sci_ref`, `model`) —
  their `pub(crate)` surface must keep resolving at the old paths.

Rejected alternatives:
- *Section fns for `io/input/corpus.rs`'s `cases()`* — wrapping its ten sections in new fns is
  new code, not motion; the owner exempted the file unsplit.
- *Concatenating `SELF_UPDATE_TABLE` or the aarch64 decode `table()` `vec![…]` from parts* — a
  logic edit; each is one data item and exempt.
- *Splitting `layout_tables_are_self_consistent` into several tests* — renames/creates tests;
  it is one `#[test]` and exempt.
- *`builder_collection_layout/kind2_layout_tests.rs` beside the host (the map's first layout)* —
  outside a `tests/` directory, so coverage counts it and the guards scan it. J uses one
  `#[cfg(test)] mod tests;` → `builder_collection_layout/tests/mod.rs` declaring the three
  existing modules; their paths gain a `tests::` segment and nothing references them by path
  (`grep -rn "kind2_layout_tests\|flatness_split_tests\|res_field_record_layout_tests" src/docs .ai scripts tests/guards`
  → none).
- *Moving squashfs's golden builders into `golden_and_external.rs`* — they are shared helpers of
  `mod.rs`'s builder family; keeping them in `mod.rs` needs no visibility edit.

## 4. Detailed Design

Line ranges are source lines as of `17a9b5799`; item and test NAMES are authoritative if lines
drift. LOC = `span.py` count of the moved items + 1 for `use super::*;`. Shared items stay in
`mod.rs`. Visibility edits: none unless stated. Hazard commands per file:
`grep -rnoE "\[\[<path>:[^]]+\]\]" src/docs/spec`; `grep -rn "<path>" tests/guards scripts .ai AGENTS.md src/docs .github Cargo.toml`;
`grep -n "<path>" scripts/coverage-exceptions.txt`. "Hazards: none" means all returned nothing
to repoint.

### 4.0 Exemptions J adds

Each gets an `exempt test <path> # <reason>` row in `tests/guards/file_size_ceilings.txt` and
the same entry in `tools/loc-census/census.py`'s `EXEMPT` table, in the commit that creates
(or, for `corpus.rs`, keeps) the file:

| File | Test LOC | Reason (row text) |
|---|---|---|
| `src/arch/aarch64/decode/tests/table.rs` | 466 | one `fn table()` returning a single `vec![…]` literal of encoder forms (465 LOC); a literal cannot be split without a logic change |
| `src/codegen/io/input/corpus.rs` | 903 | one `fn cases()` (810 LOC), a flat push sequence of the input-decoder corpus; splitting needs new section fns (owner: exempt, unsplit) |
| `src/codegen/collection/assign/self_update/tests/census/self_update_table.rs` | 881 | one `const SELF_UPDATE_TABLE: &[…]` census literal (880 LOC); a const slice cannot be split without a logic change |
| `src/codegen/runtime/canvas/vulkan/tests/layout_tables.rs` | 486 | one `#[test] layout_tables_are_self_consistent` (485 LOC of hand-transcribed Vulkan C layouts); splitting creates tests |

### 4.1 `src/arch/aarch64/decode/tests.rs` → `decode/tests/` (742, 11 tests)

- `mod.rs` (32): `use super::{decode, Flow}` + crate imports, `mod table; use table::table;`,
  3 topic `mod` lines; shared `PC`, `round_trip`, `op`, `Case`.
- `table.rs` (466, exempt) [src 44-509]: `table` → `pub(super) fn table` (1 edit).
- `encoder_forms.rs` (48, 2) [src 511-520, 754-792]: `every_encoded_form_decodes_to_its_text`,
  `the_table_covers_every_form_the_encoder_emits`.
- `flow.rs` (125, 6) [src 522-575, 625-709]: `branches_decode_their_targets`,
  `an_adrp_and_its_add_carry_the_address_they_form`, `every_way_out_of_a_function_has_its_flow`,
  `an_adr_and_a_literal_load_carry_the_address_they_name`, `a_mov_of_an_immediate_carries_its_signed_value`,
  `a_word_outside_the_decoded_extensions_does_not_decode`.
- `lldb.rs` (79, 3) [src 577-623, 711-752]: `decodes_words_as_lldb_does`, `dyld_entry_code_decodes`,
  `decodes_a_corpus_as_lldb_does`; helper `lldb_rows`.
Hazards: `include_str!("lldb_words.txt")` → `include_str!("../lldb_words.txt")` (lldb.rs);
`include_str!("../encode/emitter.rs")` → `include_str!("../../encode/emitter.rs")`
(encoder_forms.rs). When plan-176-O splits `encode/emitter.rs`, that letter repoints this
include to the part holding `match instruction.op.mnemonic()`. `src/arch/aarch64/decode/mod.rs:3`
`//!` prose "`tests.rs` holds" → "`tests/` holds".

### 4.2 `src/arch/aarch64/encode/tests.rs` → `encode/tests/` (1251, 41 tests)

- `mod.rs` (54): 4 `use` lines, 7 `mod` lines; shared `fresh_encoder`, `encode_one`,
  `emit_words`, `plan`.
- `scalar_forms.rs` (199, 9) [src 6-237]: `instruction_size_expands_large_stack_and_memory_immediates`,
  `encoder_accepts_large_immediates_with_fallback_sequences`, `encodes_umulh_add_carry_and_rorv`,
  `encodes_fmov_d_from_d`, `encodes_sxtw`, `encodes_fabs_d`, `encodes_fminnm_fmaxnm_d`,
  `encodes_b_vs_branch`, `encodes_b_mi_and_b_ls_branches`.
- `memory.rs` (170, 7) [src 239-331, 625-709, 782-791]: `encodes_fp_scalar_spill_load_store`,
  `encodes_load_acquire_store_release`, `memory_ops_encode_all_widths`, `str_u16_encodes_strh`,
  `memory_ops_use_scratch_for_large_offsets`, `unaligned_memory_offsets_error`,
  `ldr_q_str_q_scratch_for_unaligned_or_large_offset`.
- `neon.rs` (198, 2) [src 346-531, 747-780]: `encodes_neon_vector_ops`, `neon_shift_and_umov_index`.
- `op_coverage.rs` (108, 4) [src 533-546, 567-623, 711-745, 1435-1448]: `codeop_sizes_cover_large_add_imm`,
  `every_scalar_op_encodes`, `float_scalar_ops_encode`, `float_precision_conversions_encode`.
- `carry_and_branches.rs` (194, 8) [src 793-1013]: `add_carry_and_sub_borrow_sizes`,
  `cmp_imm_size_matches_emit_for_out_of_range_immediates`, `all_conditional_branches_patch`,
  `unresolved_branch_label_errors`, `bl_relocation_bindings`, `symbol_ref_data_and_got_relocations`,
  `unsupported_instruction_errors`, `shift_immediate_out_of_range_errors`.
- `image.rs` (141, 4) [src 1015-1032, 1052-1184]: `encode_builds_image_with_symbols_data_and_labels`,
  `encode_requires_entry_and_rejects_bad_hex`, `encode_rejects_a_duplicate_label_within_a_function`,
  `encode_carries_imports_as_functions`; helpers `plan_fn`, `data_obj`.
- `operands_and_sizing.rs` (201, 7) [src 1186-1433]: `operand_reg_decoding`,
  `operand_field_missing_reports_op`, `sizing_covers_immediate_and_memory_paths`, `sizing_helpers_directly`,
  `sp_is_rejected_in_shifted_register_operand_slots`,
  `cmp_imm_against_sp_is_rejected_only_when_the_immediate_forces_register_form`,
  `an_absurdly_wide_add_sub_immediate_is_rejected_rather_than_looped_over`.
Hazards: `src/docs/spec/architecture/14_aarch64-instruction-set.md:231`
`[[src/arch/aarch64/encode/tests.rs:encodes_neon_vector_ops]]` → `…/encode/tests/neon.rs:…`;
`:284` `…:encodes_umulh_add_carry_and_rorv]]` → `…/encode/tests/scalar_forms.rs:…` (both
required: the file vanishes).

### 4.3 `src/arch/riscv64/encode/tests.rs` → `encode/tests/` (1166, 41 tests)

- `mod.rs` (83): 7 `use` lines, 8 `mod` lines; shared `encode_text`, `words`, `ci`, `plan_of`,
  `func`, `try_encode`.
- `scalar_ops.rs` (97, 6) [src 51-160]: `ret_and_ecall`, `add_and_sub_r_type`, `fp_min_max_d`,
  `fma_family_d`, `li_small_and_move`, `li_thirty_two_bit_uses_lui_addi`.
- `offsets_and_branches.rs` (147, 6) [src 162-340]: `load_store_word_offsets`,
  `large_offset_load_uses_rd_not_t0_as_address_scratch`,
  `large_offset_load_with_rd_aliasing_base_stages_through_t0`, `conditional_branch_is_long_form`,
  `bitmanip_expansions_size_matches_emitted_bytes`, `li_reconstructs_all_values`.
- `operand_decoding.rs` (121, 4) [src 382-505]: `integer_register_names_decode_to_numbers`,
  `fp_register_names_decode_to_numbers`, `immediate_and_shift_decoding`, `field_lookup_reports_missing_field`.
- `data_and_relocations.rs` (148, 9) [src 509-674]: `data_objects_string_and_raw_encode_and_symbolize`,
  `raw_data_object_rejects_malformed_hex`, `call_and_data_reference_to_import_and_internal_data`,
  `adrp_pageoff_to_import_uses_got_binding`, `call_to_internal_function_is_internal_binding`,
  `call_to_unresolved_symbol_errors`, `duplicate_label_in_function_errors`,
  `branch_to_unknown_label_errors`, `missing_entry_symbol_errors`.
- `emitter_ops.rs` (143, 7) [src 678-821, 1310-1322]: `integer_alu_and_logic_ops_encode_as_op`,
  `rotate_ops_expand_to_fixed_sequences`, `unary_and_muladd_ops_encode`, `shift_immediate_ops_encode`,
  `add_sub_immediate_small_and_large_paths`, `load_store_variants_and_large_offsets`,
  `float_precision_conversions_encode`.
- `emitter_control_and_fp.rs` (156, 5) [src 823-992]: `control_flow_and_system_ops_encode`,
  `explicit_carry_ops_are_fixed_length`, `scalar_fp_ops_encode_as_op_fp`,
  `immediate_materialization_true_and_wide_values`, `emitter_rejects_bad_operands`.
- `rvv.rs` (214, 2) [src 996-1229]: `vector_register_names_decode_to_numbers`,
  `rvv_ops_encode_to_reference_words`; helper `vword`.
- `rvv_sizing.rs` (73, 2) [src 1231-1308]: `rvv_ops_are_single_words_and_size_matches`,
  `rvv_encoder_rejects_unknown_vop_and_bad_registers`.
Hazards: none.

### 4.4 `src/arch/x86_64/encode/tests.rs` → `encode/tests/` (3185, 130 tests)

- `mod.rs` (57): the `//!` header (kept as the file's first lines), 9 `use` lines, 18 `mod`
  lines; shared `fresh_encoder`, `bytes`, `enc_err`, `enc`.
- `integer_alu.rs` (222, 17) [src 40-338]: `mov_reg_reg`, `mov_imm64`, `add_sub_and_or_eor`,
  `add_with_move`, `mvn_from_zero_token_materializes_minus_one`, `mov_from_zero_token_materializes_zero`,
  `sub_with_zero_lhs_negates`, `alu3_dst_equals_rhs_aliasing`, `mul_aliasing`, `mvn`, `mul_low`,
  `umulh`, `udiv_sdiv`, `msub`, `add_imm_sub_imm`, `add_sp_sub_sp`, `cmp_cmp_imm`.
- `shifts_memory_branches.rs` (154, 10) [src 340-533]: `shifts_imm`, `shifts_var`, `loads`,
  `stores`, `branch_self_ret_svc`, `blr`, `branches_are_fixed_size`, `call_emits_5_bytes`,
  `lea_rip_relative`, `add_pageoff_is_zero_bytes`.
- `carry_and_zero_tokens.rs` (162, 11) [src 535-742]: `add_carry_no_carry_in`,
  `add_carry_with_carry_in`, `sub_borrow_no_borrow_in`, `add_carry_zero_token_rhs_adds_immediate_zero`,
  `add_carry_zero_token_rhs_no_carry_in_is_move`, `sub_borrow_zero_token_rhs_subtracts_immediate_zero`,
  `cmp_zero_token_rhs_compares_immediate_zero`, `cmp_zero_token_lhs_rejected`,
  `cmp_imm_zero_token_lhs_rejected`, `add_carry_zero_token_lhs_commutes`, `sub_borrow_zero_token_lhs_rejected`.
- `bit_ops_and_extended.rs` (201, 13) [src 752-1013]: `label_and_pageoff_are_empty`, `clz_lzcnt`,
  `rev_word_and_quad`, `rbit_bit_reverse_sequence`, `smulh_signed_high`, `alu_zero_lhs_forms`,
  `alu_zero_rhs_rejected`, `alu_dst_equals_rhs_commutes_and_subtracts`, `lsrv_variable_shift`,
  `rorv_word_variable`, `u32_load_store_extended`, `str_u16_encodes_the_operand_size_prefixed_store`,
  `str_u8_extended_register`; helper `just_bytes`.
- `float_scalar.rs` (193, 6) [src 1015-1258]: `float_scalar_moves_and_arith`,
  `float_sqrt_compare_neg_abs`, `float_int_conversions`, `f2i_nearest_never_clobbers_its_own_dst`,
  `float_scalar_mem`, `float_mem_bad_offset_errors`.
- `v128_arith.rs` (193, 4) [src 1260-1481]: `v128_load_store_and_bad_offset`,
  `v128_three_operand_arith`, `v128_unary_and_neg_abs`, `v128_integer_compares`.
- `v128_compares_and_lanes.rs` (172, 4) [src 1483-1678]: `v128_float_compares`,
  `v128_zero_compares`, `v128_rounding`, `v128_shifts_dup_extract`.
- `v128_select_and_convert.rs` (131, 1) [src 1680-1833]: `v128_bit_select_fma_convert`.
- `aliasing_and_ranges.rs` (164, 7) [src 1835-2031]: `div_aliasing_and_preservation`,
  `msub_extended_registers`, `add_carry_extended_and_sub_borrow`, `x86_float_conditional_branches`,
  `immediate_true_false_and_shift_range`, `imm32_overflow_and_disp32_overflow`, `blr_and_unknown_register`.
- `image.rs` (222, 9) [src 2033-2266, 2301-2316]: `encode_produces_an_image_with_symbols_and_data`,
  `encode_requires_entry_symbol`, `encode_data_rejects_bad_hex`, `encode_carries_imports`,
  `encoder_emits_and_patches_a_local_branch`, `encoder_unresolved_label_errors`,
  `encoder_call_relocation_bindings`, `encoder_data_and_got_relocations`, `unsupported_op_errors`;
  helpers `encode_err`, `minimal_plan`, `simple_function`.
- `integer_coverage.rs` (215, 11) [src 2268-2299, 2329-2566]: `operand_decoding_edge_cases`,
  `label_and_add_pageoff_are_empty`, `rev_w_rev_x`, `rbit_reverse_bits`,
  `msub_disjoint_and_dst_aliases_lhs`, `div_aliasing_and_dividend_preservation`, `shifts_var_32bit`,
  `shift_imm_move_first`, `add_imm_move_first_and_str_u32_extended`, `str_u8_extended_and_u16_encode`,
  `extra_branch_conditions`.
- `scalar_double_coverage.rs` (182, 6) [src 2568-2787]: `scalar_double_moves_and_arith`,
  `scalar_double_min_max`, `scalar_double_fma_family`, `scalar_double_compares_and_signops`,
  `int_float_conversions`, `scalar_double_mem`.
- `v128_coverage_arith.rs` (172, 2) [src 2789-2971]: `v128_load_store_and_arith`, `v128_unary_and_negabs`.
- `v128_coverage_lanes.rs` (187, 3) [src 2973-3176]: `v128_compares_against_zero`,
  `v128_lane_shifts_and_moves`, `v128_bit_selects_fma_and_serial_conversions`.
- `error_arms.rs` (142, 4) [src 3178-3266, 3455-3539]: `alu3_and_zero_and_error_arms`,
  `add_carry_dst_not_lhs_and_sub_borrow_with_borrow_in`, `immediate_and_disp_overflow_errors`,
  `fixed_register_aliasing_is_rejected_rather_than_miscompiled`.
- `plan_encoding.rs` (159, 5) [src 3268-3453]: `encode_full_plan_labels_calls_and_data`,
  `duplicate_label_in_a_function_is_rejected`, `ordered_only_float_branch_jp_skips_only_its_own_jcc`,
  `encode_external_call_and_got_load`, `encode_unresolved_call_and_label_error`; helpers `ci`, `plan_with`.
- `ties_away.rs` (143, 7) [src 3541-3744]: `ties_away_model_matches_aarch64_semantics`,
  `ties_away_encodes_the_exact_fraction_sequence`, `sxtw_sign_extends_low_32`,
  `store_zero_immediate_by_width`, `variable_shift_targeting_rcx_shifts_in_a_scratch`,
  `variable_shift_reloads_value_aliasing_rcx`, `variable_shift_w_reloads_value_aliasing_rcx`.
- `carry_register_aliasing.rs` (150, 10) [src 3746-3920]: `add_carry_dst_aliases_rhs_no_carry_in`,
  `add_carry_dst_aliases_rhs_with_carry_in`, `add_carry_register_carry_in_distinct_operands`,
  `add_carry_register_carry_in_zero_rhs`, `sub_borrow_zero_rhs_no_borrow_in_moves`,
  `sub_borrow_dst_aliases_rhs_no_borrow_in`, `sub_borrow_zero_rhs_register_borrow_in`,
  `sub_borrow_dst_aliases_rhs_with_register_borrow_in_is_rejected`, `umulh_stages_rhs_aliasing_rax`,
  `float_precision_conversions_encode`.
Hazards: `src/docs/spec/architecture/15_x86_64-instruction-set.md:103`
`[[src/arch/x86_64/encode/tests.rs:1]]` cites the file's `//!` header ("Byte-exact encoding
tests — the correctness gate…") → `[[src/arch/x86_64/encode/tests/mod.rs:1]]`, with the header
kept at line 1 of `mod.rs`; `:232` `…:unsupported_op_errors]]` → `…/encode/tests/image.rs:unsupported_op_errors]]`.

### 4.5 `src/arch/riscv64/select.rs` inline (629, 39 tests) → `select/tests/`

`#[cfg(test)] mod tests { … }` (887-1651) → `#[cfg(test)] mod tests;` (host keeps 559 code LOC).
- `mod.rs` (21): `use super::*;`, the `lower_t…` import, 5 `mod` lines; shared `build`, `sel`,
  `values`, `fl`.
- `registers_and_abi.rs` (121, 9) [src 907-959, 1060-1083, 1592-1650]: `positional_abi_remap`,
  `explicit_abi_tokens_realize_to_positional_a_registers`, `link_register_maps_to_ra`,
  `arena_base_realizes_to_s11`, `fp_registers_map_to_abi_roles`, `scratch_register_homes`,
  `fp_register_homes`, `fp_register_out_of_range_panics`, `remap_register_cases`.
- `compare_branches.rs` (115, 10) [src 961-1029, 1091-1157]: `integer_compare_branch_native`,
  `compare_branch_swaps_for_gt`, `compare_imm_materializes_nonzero`, `compare_imm_zero_uses_zero_register`,
  `float_compare_branch_uses_fcmp_and_br`, `integer_branch_condition_table`,
  `integer_branch_unknown_condition_panics`, `float_branch_condition_table`,
  `float_branch_unknown_condition_panics`, `float_compare_against_zero_materializes_plus_zero`.
- `overflow.rs` (75, 7) [src 1031-1058, 1159-1211]: `overflow_add_expands_with_detection`,
  `addr_of_expands_to_auipc_addi_pair`, `overflow_subtract_expands_with_detection`,
  `fused_syscall_carry_idiom_is_rejected`, `fused_unexpected_setter_panics`,
  `overflow_branch_condition_mapping`, `overflow_branch_unknown_condition_panics`.
- `bare_compares.rs` (200, 6) [src 1215-1452]: `bare_compare_register_defers_to_standalone_branch`,
  `bare_compare_imm_zero_standalone_branch`, `bare_compare_on_abi_tokens_realizes_them`,
  `no_selected_sequence_holds_t0_across_a_virtual_register`,
  `float_compare_branch_keeps_its_bool_out_of_t0`, `bare_compare_imm_nonzero_standalone_branch`;
  helper `t0_held_across_a_vreg`.
- `pending_compares.rs` (104, 7) [src 1454-1588]: `pending_compare_invalidated_by_label`,
  `pending_compare_recovers_across_a_carry_out_def_of_its_rhs`,
  `pending_compare_recovers_across_a_borrow_out_def_of_its_rhs`,
  `pending_compare_recovers_across_a_reload_of_its_operand`,
  `pending_compare_invalidated_by_an_intervening_call`,
  `an_undisturbed_pending_compare_still_reaches_its_branch`, `v128_op_scalarizes_during_selection`.
Hazards: 14 citations, all to host code that stays; `.ai/arch-abi.md` names the host path. None.

### 4.6 `src/arch/riscv64/v128.rs` inline (913, 36 tests) → `v128/tests/`

`mod tests` (1442-2537) → `#[cfg(test)] mod tests;`. When plan-176-O turns `v128.rs` into
`v128/mod.rs` + parts, it re-exports from `mod.rs` every private fn these tests reach via
`super::*` (recorded for O).
- `mod.rs` (19): `use super::*;`, 6 `mod` lines; shared `map`, `big`, `fl`, `count`.
- `scalarize_checks.rs` (135, 9) [src 1450-1614]: `detects_vector_operands`,
  `fadd_v_scalarizes_to_two_lane_adds`, `umov_rejects_an_out_of_range_lane_index`,
  `umov_rejects_a_malformed_lane_index`, `umov_still_accepts_both_real_lanes`,
  `ldr_q_rejects_a_non_numeric_offset_on_its_own_terms`, `str_q_rejects_a_non_numeric_offset_on_its_own_terms`,
  `ldr_q_lanes_stay_eight_bytes_apart_for_a_real_offset`, `lane_shift_by_sixty_four_matches_aarch64`.
- `slots.rs` (138, 3) [src 1616-1775]: `slots_are_reused_across_disjoint_live_ranges`,
  `loop_carried_values_never_share_a_slot`, `vreg_map_reuse_loop_distinctness_and_overflow`;
  helpers `mir`, `peak`.
- `rvv_paths.rs` (183, 4) [src 1777-1994]: `dual_path_emits_guard_rvv_and_scalar_arms`,
  `mask_bridge_and_minmax_rvv_arms`, `per_run_single_guard_and_resident_chain`, `dup_broadcasts_both_lanes`.
- `fp_lanes.rs` (125, 9) [src 2012-2150]: `scratch_token_operands_are_vector_values`,
  `fp_three_same_ops_lower_per_lane`, `fp_min_max_use_number_semantics`, `fmla_fuses_and_fmls_subtracts`,
  `fp_two_reg_misc_ops_lower_per_lane`, `frint_directed_modes_and_ties_even`,
  `float_to_int_and_int_to_float_lane_converts`, `fp_lane_compares_build_masks`, `fp_compare_against_zero_masks`.
- `int_lanes.rs` (97, 8) [src 2152-2262]: `integer_add_sub_lanes`, `integer_lane_compares`,
  `integer_neg_and_abs`, `bitwise_ops_lower_per_lane`, `bit_select_and_bit_insert`,
  `quad_load_store_move_sixteen_bytes`, `umov_extracts_one_lane`, `unhandled_v128_op_panics`.
- `rvv_arm.rs` (225, 3) [src 2266-2536]: `rvv_arm_lowers_every_vector_op`,
  `rvv_lowerable_matches_the_emitter`, `residency_elides_only_a_proven_redundant_reload`; helper
  `slots_and_vregs`.
Hazards: 5 citations to host code; `tests/guards/no_operator_strings.rs` names the host path
(host code, unchanged). None.

### 4.7 `src/arch/x86_64/select.rs` inline (279, 13 tests) → `select/tests/`

`mod tests` (413-747) → `#[cfg(test)] mod tests;`.
- `mod.rs` (19): `use super::*;`, the `lower_t…` import, 2 `mod` lines; shared `ci`, `sel`, `values`.
- `registers.rs` (144, 7) [src 439-496, 635-746]: `addr_of_becomes_lea_and_pageoff_drops`,
  `sp_zero_and_fp_register_mapping`, `scratch_register_pool_wraps`, `x30_link_register_is_dropped`,
  `arena_base_realizes_to_r15`, `realize_abi_operand_maps_to_aligned_registers`,
  `explicit_abi_token_realizes_to_aligned_register`.
- `branches.rs` (117, 6) [src 498-633]: `float_compare_branch_rewrites`,
  `float_compare_ordered_only_and_multi_branch`, `ordered_only_skip_labels_are_unique_per_branch_site`,
  `fcmp_zero_branch_rewrite`, `float_branch_unmapped_condition_panics`, `integer_compare_branch_not_remapped`.
Hazards: 10 citations, all to host code. `[[src/arch/x86_64/select.rs:1]]`
(15_x86_64-instruction-set.md:23) cites the host's own `//!` header ("x86-64 instruction
selection…", `sed -n 1p src/arch/x86_64/select.rs`), which stays — no repoint. `.ai/arch-abi.md`
names the host path — unchanged.

### 4.8 `src/binary_repr/tests/cross_package_tests.rs` → `cross_package_tests/` (303, 7)

`src/binary_repr/tests/mod.rs` keeps `mod cross_package_tests;` unchanged (same for every
`binary_repr/tests/*.rs` below).
- `mod.rs` (39): `use super::fixtures::*; use super::*; use crate::ir::IrSourceLoc;`, 3 `mod`
  lines; shared `dep_with_type_project`, `dep_meta`, `dependency`.
- `external_calls.rs` (69, 3) [src 9-83]: `lower_package_project_resolves_external_calls`,
  `external_function_metadata_assigns_ids_and_hashes`, `build_package_binary_repr_bytes_round_trips`;
  helpers `dep_project`, `write_dep_mfp`.
- `foreign_type_reexport.rs` (101, 2) [src 122-254]: `foreign_type_reexport_chain_resolves_through_siblings`,
  `foreign_type_export_without_owner_sibling_skips_fill`.
- `traversing_owner.rs` (100, 2) [src 256-405]: `foreign_type_reexport_rejects_traversing_owner`,
  `foreign_type_abi_check_rejects_traversing_owner`; helpers `widget_owner_mfp`, `variant_widget_owner_mfp`.
Hazards: `scripts/coverage-exceptions.txt:46` (`src/binary_repr/mod.rs` entry) prose names
`src/binary_repr/tests/cross_package_tests.rs` → `src/binary_repr/tests/cross_package_tests/`
(entry key unchanged).

### 4.9 `src/binary_repr/tests/fixtures.rs` → `fixtures/` (251, 0 tests)

- `mod.rs` (85): `use` lines, `mod projects;` + `pub(super) use projects::{default_function_project, rich_project};`;
  helpers `loc`, `const_int`, `fn_named`, `empty_project`, `encode_project`, `wrap_mfp`, `temp_mfp`.
- `projects.rs` (168) [src 53-227]: `use super::*;`; `rich_project`, `default_function_project`,
  changed from `pub(super)` to `pub(crate)` (2 edits) so the re-export reaches the 12 sibling
  files that `use super::fixtures::*;`.
Hazards: none.

### 4.10 `src/binary_repr/tests/gap_tests.rs` → `gap_tests/` (282, 10)

- `mod.rs` (5): the 3 `use` lines, 2 `mod` lines.
- `types_and_unions.rs` (147, 4) [src 10-162]: `type_id_parses_thread_source_names`,
  `source_type_payload_encodes_enum_members_with_ordinals`, `concrete_union_variants_flattens_included_unions`,
  `union_with_includes_round_trips_variants_through_encode`.
- `abi_and_sections.rs` (134, 6) [src 164-314]: `encode_functions_emits_registers_and_cleanups`,
  `validate_abi_index_rejects_dep_request_and_symbol_mismatches`, `validate_abi_index_rejects_edge_set_mismatch`,
  `package_type_exports_errors_when_type_missing_from_table`, `read_binary_repr_package_rejects_duplicate_section`,
  `read_binary_repr_package_reports_missing_required_sections`.
Hazards: none.

### 4.11 `src/binary_repr/tests/reader_gap_tests.rs` → `reader_gap_tests/` (303, 12)

- `mod.rs` (4): the 2 `use` lines, 2 `mod` lines.
- `sections_and_types.rs` (110, 6) [src 8-135]: `mfp_payload_rejects_truncated_hash_and_signature_and_length`,
  `read_binary_repr_package_names_each_missing_section`,
  `read_binary_repr_package_without_optional_sections_still_decodes`,
  `primitive_type_name_covers_handle_and_term_types`, `type_entry_names_rejects_cyclic_type`,
  `decode_function_type_round_trips_via_type_entry_names`; helper `mfpc_missing`.
- `abi_validation.rs` (193, 6) [src 137-360]: `validate_abi_index_recomputes_type_export_hashes`,
  `checked_usize_rejects_a_length_beyond_the_address_space`,
  `abi_serializer_rejects_reserved_type_ids_without_overflow`, `abi_serializer_walks_composite_field_types`,
  `validate_abi_index_rejects_export_missing_function`, `read_function_table_rejects_nonempty_code_region`.
Hazards: none.

### 4.12 `src/binary_repr/tests/reader_tests.rs` → `reader_tests/` (727, 40)

- `mod.rs` (8): the 2 `use` lines, 6 `mod` lines.
- `package_round_trip.rs` (137, 9) [src 8-132, 817-897 incl. the `// ---` PACKAGE_META section
  comment]: `read_binary_repr_package_round_trips_rich_project`,
  `a_description_round_trips_through_section_eighteen`, `a_section_with_an_unknown_id_is_parsed_and_ignored`,
  `read_binary_repr_package_rejects_bad_magic_and_version`, `read_binary_repr_package_rejects_short_input`,
  `read_binary_repr_package_rejects_truncated_section_table`,
  `package_meta_section_round_trips_and_is_omitted_when_empty`,
  `an_unknown_package_meta_field_id_is_skipped_not_rejected`, `an_over_cap_description_is_rejected_at_read_time`;
  helper `decompose_sections`.
- `tables.rs` (116, 11) [src 134-277]: `primitive_type_name_and_type_name_resolution`,
  `string_at_bounds_check`, `export_kind_encode_decode_round_trips`,
  `the_retired_sub_export_kind_is_rejected_with_a_rebuild_message`, `doc_kind_name_maps_codes`,
  `read_doc_table_handles_absent_package`, `read_doc_table_rejects_truncation`,
  `read_doc_table_rejects_trailing_bytes`, `read_string_pool_rejects_trailing_and_truncation`,
  `read_type_entries_rejects_duplicate_name_and_kind`, `read_type_entries_rejects_bad_bounds`.
- `default_functions.rs` (118, 3) [src 279-404]: `read_binary_repr_package_round_trips_a_function_default`,
  `validate_default_functions_accepts_what_the_writer_produces`,
  `validate_default_functions_refuses_each_invalid_record`; helper `default_function_table`.
- `table_rejections.rs` (98, 7) [src 406-512]: `read_function_table_rejects_flat_code_and_trailing`,
  `read_manifest_rejects_trailing_bytes`, `read_import_table_rejects_bad_pin`,
  `read_resource_and_global_tables_reject_trailing`, `read_export_table_rejects_bad_kind_and_trailing`,
  `read_abi_index_rejects_bad_version_and_pin`, `decode_type_field_maps_all_visibilities`.
- `mfp_container.rs` (73, 6) [src 514-597]: `package_identity_id_is_deterministic_and_content_addressed`,
  `mfp_payload_rejects_bad_magic_version_and_size`, `validate_mfp_signature_header_accepts_valid_variants`,
  `read_package_binary_repr_round_trips_through_temp_file`, `package_info_from_mfp_matches_the_on_disk_reader`,
  `validate_container_identity_rejects_mismatch`.
- `abi_serializer.rs` (189, 4) [src 599-815]: `abi_serializer_serializes_composite_types`,
  `abi_serializer_hashes_state_composites_structurally`,
  `abi_serializer_hashes_set_mapentry_and_threadworker_structurally`,
  `abi_serializer_rejects_deep_acyclic_type_chain`.
Hazards: none.

### 4.13 `src/binary_repr/tests/sections_tests.rs` → `sections_tests/` (651, 18)

- `mod.rs` (5): `use super::*;`, 4 `mod` lines.
- `type_id_maps.rs` (187, 2) [src 73-280]: `type_id_maps_primitives_and_composites`,
  `no_encoder_emits_the_retired_term_color_id`.
- `type_round_trips.rs` (147, 6) [src 282-461]: `type_id_composites_decode_back_to_source_names`,
  `state_carrying_resource_type_round_trips`, `deep_acyclic_type_chain_is_rejected_not_overflow`,
  `thread_types_round_trip_with_and_without_resource`, `type_table_encode_decode_round_trips_payloads`,
  `a_nested_map_key_splits_at_the_top_level_separator`.
- `pools_and_imports.rs` (208, 7) [src 7-71, 463-622]: `function_sig_hash_distinguishes_a_function_default`,
  `string_pool_interns_and_dedups`, `const_pool_stores_every_scalar_kind`, `scalar_wire_id_and_reserved_band`,
  `const_pool_rejects_bad_values_and_types`, `import_table_from_metadata_and_encode_round_trip`,
  `import_table_records_used_symbols`.
- `wire_ids.rs` (112, 3) [src 624-777]: `abi_index_encode_decode_round_trips`,
  `type_id_falls_back_for_malformed_composites`, `wire_type_ids_are_unchanged_by_the_typed_encoder`.
Hazards: none.

### 4.14 `src/binary_repr/tests/writer_tests.rs` → `writer_tests/` (375, 20)

- `mod.rs` (6): the 3 `use` lines, 3 `mod` lines.
- `lowering.rs` (143, 6) [src 9-78, 365-448]: `lower_project_populates_all_tables`,
  `lower_project_without_entry_uses_sentinel`, `lower_project_missing_entry_function_errors`,
  `native_resources_add_type_and_resource_entry`, `lower_project_encodes_the_doc_table_from_ir`,
  `an_imported_builtin_type_is_not_exported`.
- `type_strings_and_fixed.rs` (93, 7) [src 80-196, 294-307]: `split_top_level_types_respects_nesting`,
  `parse_function_type_handles_isolated_and_plain`, `split_function_type_rest_finds_top_level_terminator`,
  `fixed_raw_from_decimal_covers_signs_fractions_and_rounding`,
  `fixed_raw_from_decimal_matches_the_numeric_converter`, `fixed_raw_from_decimal_rejects_malformed`,
  `fixed_raw_from_decimal_covers_long_fractions_and_carries`.
- `resources_and_payloads.rs` (139, 7) [src 198-292, 309-363]: `ir_uses_resource_type_detects_file_param`,
  `is_resource_type_name_matches_builtins`, `standard_resource_flags_marks_sendable_types`,
  `source_type_payload_encodes_union_and_enum`, `concrete_union_variants_rejects_unknown_include`,
  `source_type_payload_records_encode_field_visibilities`,
  `source_type_payload_ignores_a_non_composite_kind`.
Hazards: none.

### 4.15 `src/binary_repr/tests/writer_walker_tests.rs` → `writer_walker_tests/` (584, 7)

- `mod.rs` (26): the 6 `use` lines, 3 `mod` lines,
  `use resource_corpus::{every_op_body, every_value}; use plain_corpus::{every_plain_op_body, every_plain_value};`;
  shared `file_local`, `corpus_function`.
- `resource_corpus.rs` (227) [src 19-248]: `every_value`, `every_op_body` → `pub(super)` (2).
- `plain_corpus.rs` (224) [src 280-506]: `plain_local`, `every_plain_value`, `every_plain_op_body`;
  the last two → `pub(super)` (2).
- `resource_walks.rs` (113, 7) [src 262-278, 508-620]: `ir_uses_resource_type_walks_every_op_and_value_arm`,
  `resource_walkers_visit_every_arm_when_no_resource_present`,
  `collect_resource_names_walk_gathers_file_over_every_arm`, `collect_resource_type_names_gathers_file`,
  `lower_project_emits_file_resource_table_from_body`, `collect_imported_calls_records_used_symbols`,
  `socket_and_listener_resources_are_emitted_when_used`.
Hazards: none.

### 4.16 `src/os/linux/appimage/squashfs/tests.rs` → `squashfs/tests/` (726, 30)

`squashfs/mod.rs` keeps `mod tests;`.
- `mod.rs` (60): `use super::*; use std::process::Command;`, 4 `mod` lines; shared `tree`,
  `dir_with`, `file`, `symlink`, `read_u16`, `read_u32`, `read_u64`, `have`, `golden_fixture`.
- `metadata_and_layout.rs` (141, 8) [src 75-243]: `metadata_ref_upper_field_is_a_byte_offset_not_a_block_index`,
  `metadata_writer_second_block_starts_at_on_disk_offset_8194`,
  `metadata_writer_exactly_8192_emits_header_a000`, `metadata_writer_never_emits_a_zero_size_header`,
  `superblock_fields_match_the_documented_configuration`, `id_table_ends_exactly_at_bytes_used`,
  `inode_table_precedes_the_directory_table_strictly`,
  `image_is_padded_to_4096_but_bytes_used_excludes_the_padding`; helper `metadata_table_ranges`.
- `directories.rs` (150, 5) [src 245-427]: `empty_directory_has_file_size_three`,
  `directory_file_size_is_listing_plus_three`, `directory_entry_biases_are_all_applied`,
  `a_257_entry_directory_emits_two_headers`,
  `a_directory_straddling_an_inode_block_boundary_emits_multiple_headers`; helpers
  `directory_header_count`, `walk_root_listing`, `verify_directory_runs`.
- `inodes_and_limits.rs` (180, 12) [src 429-627]: `multi_block_file_gets_one_uncompressed_block_list_entry_per_block`,
  `symlink_inode_stores_an_unterminated_target`, `permission_bits_survive_and_carry_no_type_bits`,
  `nlink_counts_subdirectories`, `root_parent_is_itself_and_inode_numbers_are_in_range`,
  `write_is_deterministic`, `rejects_a_non_directory_root`, `rejects_an_over_long_entry_name`,
  `rejects_a_listing_that_would_need_an_extended_inode`, `rejects_an_empty_symlink_target`,
  `rejects_an_empty_entry_name`, `rejects_a_file_past_four_gib`.
- `golden_and_external.rs` (203, 5) [src 631-672, 688-896]: `golden_small_tree_is_byte_stable`,
  `unsquashfs_extracts_the_tree_it_was_given`, `unsquashfs_extracts_a_thousand_files_across_eight_levels`,
  `matches_mksquashfs_modulo_the_documented_normalizations`, `dir_constructor_builds_a_moded_empty_directory`;
  helpers `GOLDEN_TREE_SHA256`, `sha256_hex`, `insert_path`.
Hazards: `scripts/coverage-exceptions.txt:44` (`squashfs/mod.rs` entry) prose "(tests in the
sibling squashfs/tests.rs)" → "(tests in squashfs/tests/)". External `unsquashfs`/`mksquashfs`
via `Command` — not a self-respawn.

### 4.17 `src/os/linux/link/tests.rs` → `link/tests/` (1796, 51)

`link/mod.rs` keeps `mod tests;`.
- `mod.rs` (127): the 2 `use` lines, 8 topic `mod` lines + `mod debug_symbols;`; shared `none`,
  `versioned_exit_image`, `glob_dat_image`, `has_gnu_stack`, `phdr_range`.
- `static_elf.rs` (180, 7) [src 180-367, 690-725]: `encode_static_elf_x86_emits_two_pt_load_segments`,
  `encode_static_elf_places_data_where_relocations_expect_it`, `encode_static_elf_x86_appends_signing_section`,
  `write_executable_x86_static_writes_flavored_output`, `write_appdir_puts_the_same_elf_under_usr_bin`,
  `write_executable_rejects_entry_not_in_text`, `write_executable_x86_static_data_pc32`; helper `x86_static_image`.
- `dynamic_elf.rs` (173, 5) [src 369-499, 567-625]: `write_executable_x86_dynamic_covers_all_reloc_kinds`,
  `write_executable_x86_dynamic_musl_uses_musl_interpreter`,
  `write_executable_aarch64_dynamic_musl_uses_musl_interpreter`,
  `dynamic_build_rejects_missing_initializer_symbol`, `dynamic_elf_appends_signing_section`; helper
  `x86_dynamic_image`.
- `unbound_and_rejects.rs` (170, 5) [src 501-565, 727-790, 846-874, 1690-1714]:
  `write_executable_rejects_unbound_external_symbol`, `write_executable_rejects_unsupported_relocation`,
  `write_executable_rejects_unbound_external_all_kinds`, `write_executable_rejects_undefined_internal_symbol`,
  `refuses_to_write_an_executable_under_a_traversing_name`; helper `expect_unbound`.
- `libc_flavors.rs` (172, 6) [src 53-125, 876-966, 1037-1069]: `writes_glob_dat_glibc_elf`,
  `writes_glob_dat_musl_elf`, `writes_mfb_sign_section_to_static_elf`, `writes_init_array_glibc_elf`,
  `writes_versioned_glibc_elf`, `dynamic_elf_hash_chain_starts_at_the_null_symbol`; helper `init_array_image`.
- `riscv64_relocs.rs` (122, 7) [src 968-1035, 1071-1091, 1725-1789]: `riscv_pcrel_lo12_pairs_with_adjacent_auipc`,
  `riscv_pcrel_lo12_pairs_across_a_spill_gap`, `riscv_pcrel_lo12_pairs_with_nearest_preceding_hi_of_same_target`,
  `riscv_hi_lo_rejects_a_displacement_past_the_auipc_reach`,
  `the_pairing_index_is_keyed_by_kind_as_well_as_target`,
  `the_pairing_index_does_not_assume_relocations_arrive_sorted`,
  `a_lo12_with_no_preceding_hi20_keeps_its_diagnostic`; helper `riscv_reloc`.
- `arch_relocations.rs` (188, 3) [src 627-688, 792-844, 1093-1188]: `write_executable_aarch64_static_internal_relocs`,
  `write_executable_riscv64_internal_call_patches_auipc_jalr_pair`,
  `write_executable_riscv64_dynamic_covers_call_pcrel_and_got`; helper `rv_inst`.
- `provenance_and_relro.rs` (137, 4) [src 1190-1322, 1350-1385]: `static_elf_carries_the_mfbasic_provenance_note`,
  `dynamic_elf_carries_the_mfbasic_provenance_note_past_the_interpreter`,
  `provenance_note_coexists_with_the_signing_section`, `dynamic_elf_relro_covers_the_dynamic_segment`;
  helper `mfb_note`.
- `runpaths.rs` (213, 8) [src 1387-1688]: `a_non_vendor_elf_emits_no_runpath`,
  `a_vendoring_elf_emits_one_origin_relative_runpath`, `the_runpath_string_does_not_disturb_the_symbol_table`,
  `a_runpath_grows_the_dynamic_prefix_by_exactly_its_dynstr_bytes`,
  `the_emitted_dynstr_matches_the_prefix_computation_with_a_runpath`,
  `an_appdir_vendoring_elf_emits_the_usr_lib_relative_runpath`,
  `each_runpath_grows_the_dynamic_prefix_by_its_own_length`,
  `a_non_vendoring_appdir_elf_is_byte_identical_to_the_console_build`; helper `elf_runpaths`.
- `debug_symbols/mod.rs` (115) [src 1793-1919]: `use super::*;`, `mod static_images; mod dynamic_images;`;
  private parsers `ParsedSection` (private fields), `parse_sections`, `section_index`,
  `parse_symtab`, `debug_symbol`, `assert_symbols_match` (children see them; no edit).
- `debug_symbols/static_images.rs` (136, 4) [src 1921-1986, 2042-2117]: `debug_symbols_static_aarch64`,
  `debug_symbols_static_x86`, `debug_symbols_signed_keeps_mfbsign_findable`,
  `debug_symbols_off_writes_no_table`; helper `static_debug_image`.
- `debug_symbols/dynamic_images.rs` (85, 2) [src 1988-2040, 2119-2156]: `debug_symbols_dynamic_with_rodata`,
  `signed_dynamic_table_names_the_dynamic_segment`; helper `assert_dynamic_section_covers_pt_dynamic`.
Hazards: `src/docs/spec/linker/04_symbols-and-relocations.md:110` and
`src/docs/spec/linker/07_linux-aarch64.md:115` `[[src/os/linux/link/tests.rs:writes_init_array_glibc_elf]]`
→ `[[src/os/linux/link/tests/libc_flavors.rs:writes_init_array_glibc_elf]]` (required).

### 4.18 `src/os/macos/link/tests.rs` → `link/tests/` (1656, 45)

`link/mod.rs` keeps `mod tests;`.
- `mod.rs` (98): the 4 `use` lines, 8 topic `mod` lines + `mod debug_symbols;`; shared `import`,
  `none`, `text_symbol`, `data_symbol`, `encode_aarch64`, `code_fn`, `inst`.
- `binding.rs` (96, 3) [src 37-142]: `patches_external_data_relocations_to_got_entry`,
  `bind_info_uses_uleb_ordinal_past_fifteen`, `import_libraries_assigns_one_ordinal_per_distinct_library`.
- `initializers.rs` (143, 3) [src 146-300]: `rejects_initializer_without_text_symbol`,
  `runs_initializer_before_entry_without_imports`, `runs_initializer_before_entry_with_imports`.
- `signing_and_provenance.rs` (184, 4) [src 304-512]: `writes_mfb_sign_section_to_mach_o`,
  `mach_o_carries_the_mfbasic_provenance_note_inside_the_signed_region`,
  `provenance_note_coexists_with_the_mfb_sign_segment`, `noted_mach_o_verifies_and_runs`; helpers
  `mfb_note`, `code_signature_offset`.
- `patch_relocations.rs` (201, 5) [src 598-817]: `patches_internal_and_data_relocations`,
  `patch_relocations_rejects_unsupported_kind`, `patch_relocations_rejects_out_of_range_offset`,
  `patch_relocations_rejects_unbound_external_symbols`, `patches_external_got_page_relocations`.
- `symbols_and_dylibs.rs` (199, 9) [src 514-580, 819-974]: `links_and_runs_program_importing_from_two_dylibs`,
  `symbol_vmaddr_rejects_unknown_symbol`, `symbol_vmaddr_splits_constants_from_writable_data`,
  `dylib_path_covers_all_libraries_and_rejects_unknown`, `library_ordinal_rejects_absent_library`,
  `data_const_helpers_size_by_slots`, `plist_escape_handles_all_predefined_entities`,
  `rejects_entry_symbol_not_in_text`, `dylib_path_resolves_app_mode_frameworks`.
- `app_bundle.rs` (196, 7) [src 976-1170, 1285-1323]: `app_info_plist_has_required_bundle_keys`,
  `app_info_plist_publishes_manifest_version`, `app_info_plist_escapes_xml_metacharacters`,
  `app_info_plist_escapes_xml_metacharacters_in_version`, `writes_and_launches_app_bundle`,
  `links_and_launches_app_bundle_importing_libobjc`, `write_app_bundle_creates_layout_and_plist_host_neutral`.
- `layout.rs` (95, 2) [src 1229-1283, 1325-1381]: `writes_full_mach_o_with_imports_data_and_initializer`,
  `code_offset_and_layout_helpers_vary_with_presence`.
- `rpaths.rs` (167, 6) [src 1383-1591]: `a_non_vendor_image_emits_no_rpath`,
  `a_vendoring_image_emits_one_rpath_with_a_consistent_header`,
  `an_app_bundle_rpath_points_at_the_frameworks_directory`,
  `a_vendoring_bundle_differs_from_the_console_binary_by_exactly_the_rpath`,
  `a_vendoring_binary_still_launches`, `refuses_to_write_an_executable_under_a_traversing_name`;
  helpers `exit_zero_image`, `mach_o_rpaths`.
- `debug_symbols/mod.rs` (136) [src 1595-1742]: `use super::*;`, 2 `mod` lines; private
  `ParsedSymtab` (private fields), `parse_symtab`, `ordinal_of`, `section_addr`, `debug_image`.
- `debug_symbols/symbol_tables.rs` (74, 4) [src 1744-1828]: `debug_symbols_lists_locals_before_imports`,
  `debug_symbols_shift_indirect_table_and_iundefsym`, `debug_symbols_on_import_less_image`,
  `function_starts_lists_every_function`.
- `debug_symbols/launch_and_uuid.rs` (89, 2) [src 1830-1926]: `debug_symbols_image_with_imports_runs`,
  `debug_uuid_differs_between_images_and_is_stable`; helper `mach_o_uuid`.
Each `#[cfg(target_os = "macos")]` attribute moves with its item. Hazards: none (several tests
launch the produced Mach-O, not the test binary).

### 4.19 `src/os/windows/link/spike.rs` → `spike/` (327, 2)

`windows/link/mod.rs` keeps `#[cfg(test)] mod spike;`.
- `spike/mod.rs` (138): the `//!` header, `#![cfg(test)]`, `use` lines, `KERNEL32`, `USER32`,
  `Asm` + impl, `le_u16`, `le_u32`, both tests (`links_and_is_gui_subsystem`,
  `writes_spike_exe_when_env_set`), `mod image; use image::spike_image;`.
- `spike/image.rs` (191) [src 144-375]: `use super::*;`; `spike_image` → `pub(super)` (1 edit);
  reaches `Asm`'s private fields/methods as a child.
Hazards: prose in `src/target/win_x86_64/app/mod.rs` names `src/os/windows/link/spike.rs` (`//!`
line 6) and "mirrors spike.rs" (doc at the `_main` emitter, comment "byte-equivalent to spike.rs")
→ `src/os/windows/link/spike/` / `spike/image.rs` (the frame lives in `spike_image`).

### 4.20 `src/os/windows/link/mod.rs` inline (686, 23) → `windows/link/tests/`

`mod tests` (661-1490) → `#[cfg(test)] mod tests;`; `#[cfg(test)] mod spike;` (22-23) stays.
- `mod.rs` (101): `use super::*;`, the image import, 5 `mod` lines; shared `image`, `le_u16`,
  `le_u32`, `le_u64`, `section_named`, `read_at_rva`, `exit_process_42_image`.
- `pe_layout.rs` (144, 4) [src 686-743, 794-889, 1254-1259, 1290-1302]: `writes_runnable_exit42_exe_when_env_set`,
  `emitted_pe_is_aslr_capable`, `runnable_exit42_image_links_to_a_pe`, `gui_flag_selects_the_pe_subsystem`;
  helper `runnable_exit42_image`.
- `signing.rs` (68, 4) [src 891-955, 1261-1288]: `signing_metadata_emits_mfbsign_section`,
  `unsigned_build_emits_no_mfbsign_section`, `signed_build_emits_both_mfbnote_and_mfbsign_disjoint`,
  `provenance_marker_emitted_unconditionally`; helper `section_extent`.
- `text_and_calls.rs` (91, 4) [src 957-1011, 1050-1105]: `minimal_text_only_image_links`,
  `entry_not_in_text_is_rejected`, `internal_call_patches_rel32`,
  `exit_process_image_has_text_and_idata_and_bound_call`.
- `imports_and_relocations.rs` (126, 8) [src 1107-1252]: `imports_grouped_by_dll_first_seen_order`,
  `rdata_and_data_sections_are_both_emitted`, `data_import_is_skipped_by_thunks`,
  `external_call_with_no_iat_slot_is_rejected`, `external_data_reloc_binds_to_the_iat_slot`,
  `external_data_reloc_with_no_slot_is_rejected`, `unsupported_relocation_kind_is_rejected`,
  `relocation_offset_past_text_end_is_rejected`; helper `image_with_import_and_data_reloc`.
- `debug_symbols.rs` (161, 3) [src 1306-1489]: `debug_symbols_fill_coff_header_and_table`,
  `debug_symbols_off_leaves_header_zero`, `signed_debug_build_keeps_mfbsign_and_seals`; helpers
  `coff_symbols`, `section_by_number`, `debug_image`.
Hazards: 16 citations to host code; `.ai/arch-abi.md` names the host path and
`signed_build_emits_both_mfbnote_and_mfbsign_disjoint` by fn name — unchanged.

### 4.21 `src/os/linux/appimage/mod.rs` inline (254, 19) → `appimage/tests/`

`mod tests` (307-618) → `#[cfg(test)] mod tests;`; the two `#[cfg(test)] const RUNTIME_*_LEN`
(49-52) stay.
- `mod.rs` (3): `use super::*;`, 2 `mod` lines.
- `runtime_blobs.rs` (81, 9) [src 311-426]: `every_runtime_ends_exactly_at_its_own_length`,
  `every_runtime_carries_the_appimage_magic`, `every_runtime_matches_its_recorded_length`,
  `runtime_for_rejects_an_unpublished_arch`, `elf_image_end_rejects_a_non_elf_blob`,
  `elf_image_end_rejects_a_32_bit_elf`, `elf_image_end_rejects_a_truncated_section_table`,
  `elf_image_end_rejects_a_truncated_section_body`, `elf_image_end_skips_nobits_sections`; helper `blobs`.
- `appdir_and_seal.rs` (167, 10) [src 428-617]: `read_dir_node_rejects_a_non_file_non_dir_non_symlink`,
  `remove_appdir_reports_a_hard_failure`, `read_appdir_keeps_symlinks_as_symlinks`,
  `read_appdir_preserves_the_executable_bit`, `read_appdir_omits_usr_lib_when_the_build_vendors_nothing`,
  `seal_concatenates_the_runtime_and_a_valid_squashfs`, `seal_is_deterministic`,
  `seal_uses_the_arch_specific_runtime`, `seal_reports_a_missing_appdir`, `remove_appdir_is_idempotent`;
  helper `fixture_appdir`.
Hazards: `scripts/coverage-exceptions.txt:47` (this file's entry) names the inline test module's
assertion arms (":463-524") and a follow-up to move them out — both done by this move. The entry
is rewritten to keep its production reason only: "caps below the floor: the seal() blob-length
supply-chain guard is unreachable with the embedded runtime (proven by
every_runtime_ends_exactly_at_its_own_length); every production path is unit-covered". The entry is kept:
the seal() guard stays unreachable after the move, and an exception entry for a file that is at
or above the floor changes no gate result. Host `include_bytes!("runtime-…")` are code and stay.

### 4.22 `src/os/linux/object.rs` inline (259, 16) → `object/tests/`

- `mod.rs` (75): `use super::*;`, the plan import, 2 `mod` lines; shared `void_type`, `function`,
  `call`, `base_plan`, `full_plan`.
- `lowering.rs` (82, 6) [src 550-634, 751-762]: `lowers_minimal_plan_to_static_elf_object`,
  `lowers_x86_target`, `lowers_full_plan_covering_every_branch`, `to_json_emits_all_sections_and_round_trips_shape`,
  `to_json_minimal_plan_has_empty_collections`, `plan_without_entry_symbol_emits_no_entry_relocation`.
- `validation.rs` (103, 10) [src 636-749]: `validate_rejects_unknown_target`,
  `validate_rejects_wrong_container_and_status`, `validate_rejects_entry_not_defined`,
  `validate_rejects_duplicate_defined_symbol`, `validate_rejects_relocation_source_not_defined`,
  `validate_rejects_relocation_target_not_defined`,
  `validate_accepts_relocation_to_imported_or_external_symbol`,
  `push_relocation_deduplicates_identical_entries`, `push_unique_skips_repeats`, `align_rounds_up_to_multiple`.
Hazards: 7 citations and 3 prose mentions name the host — unchanged.

### 4.23 `src/os/macos/object.rs` inline (420, 17) → `object/tests/`

- `mod.rs` (94): `use super::*;`, the plan import, 3 `mod` lines; shared `void_type`, `function`,
  `plan_call`, `base_plan`, `full_plan`.
- `lowering.rs` (125, 5) [src 691-740, 839-914, 1077-1086]: `lowers_libsystem_import_to_mach_o_object_plan`,
  `lowers_full_plan_covering_every_branch`, `to_json_emits_all_mach_o_sections`,
  `to_json_minimal_plan_has_empty_collections`, `plan_without_entry_symbol_emits_no_entry_relocation`.
- `validation.rs` (102, 8) [src 916-1024]: `validate_rejects_wrong_target`,
  `validate_rejects_wrong_container_and_status`, `validate_rejects_entry_not_defined`,
  `validate_rejects_duplicate_defined_symbol_and_dylib`, `validate_rejects_code_unit_without_operations`,
  `validate_rejects_missing_text_section`, `validate_rejects_overlapping_sections`,
  `validate_rejects_relocation_source_and_target`.
- `dylibs_and_helpers.rs` (102, 4) [src 1026-1075, 1088-1140]: `dylib_for_library_maps_known_and_rejects_unknown`,
  `dylibs_rejects_unknown_library_import`, `helpers_dedup_and_align`,
  `section_name_formats_with_and_without_section`.
Hazards: 2 citations and 2 prose mentions name the host — unchanged.

### 4.24 `src/codegen/resource/tests/mod.rs` (325, 8)

Existing directory module gains three topics.
- `mod.rs` (4): `use super::*;`, 3 `mod` lines.
- `builtins.rs` (81, 3) [src 3-106]: `builtins_recognize_standard_resources`,
  `builtins_carry_close_op_and_sendability`, `a_member_named_close_is_its_packages_resource_close_op`.
- `close_refusal.rs` (106, 1) [src 108-238]: `every_builtin_close_refuses_an_already_closed_handle`.
- `registry_agreement.rs` (140, 4) [src 240-420]: `both_listen_members_default_the_same_backlog`,
  `every_builtin_resource_sendability_matches_the_documented_set`,
  `every_builtin_resource_has_a_close_op`, `free_helpers_match_registry`.
Hazards: `.ai/net-tls.md:37` cites `codegen::resource::tests::every_builtin_close_refuses_an_already_closed_handle`
→ `codegen::resource::tests::close_refusal::every_builtin_close_refuses_an_already_closed_handle`.

### 4.25 `src/codegen/io/input/corpus.rs` (903, 0) — exempt, not split

The file stays as is (`Case`, builders, `cases()`); it gets its `exempt` row (§4.0).
`tests.rs` keeps `use super::corpus::{cases, Case}`.

### 4.26 `src/codegen/io/input/model.rs` → `model/` (418, 0)

cfg(test) reference decoder (`#[cfg(test)] mod model;` in `input/mod.rs`).
- `model/mod.rs` (195) [src 1-250]: header, `use` lines (incl. `use super::{CSI_PARAM_CAP, GRAPHEME_CAP}`),
  `FINAL`, `EVENTS`, `CLUSTER`, `Text`, `Token`, `Outcome`, `event_default`, `named`, `unicode`,
  `scalar_len`, `decode`, `text`, `escape`, `alt_prefixed`, `alt_introducer`,
  `mod sequences; use sequences::{csi, ss3};`.
- `model/sequences.rs` (226) [src 251-499]: `use super::*;`; `parse_params`, `field`, `event_of`,
  `csi`, `utf8`, `code_token`, `kitty_token`, `ss3`; `csi`, `ss3` → `pub(super)` (2 edits).
Every imported name (`decode, Outcome, Text, Token, CLUSTER, EVENTS, FINAL`) stays in `mod.rs`.
Hazards: none.

### 4.27 `src/codegen/string/format/float_parse_ref.rs` → `float_parse_ref/` (763, 10)

cfg(test) reference (`#[cfg(test)] pub(crate) mod float_parse_ref;`), inline `mod tests {}`
(598-968); no outside importer (`grep -rn "float_parse_ref::" src tests` → none).
- `mod.rs` (67) [src 1-26, 282-363]: header, `use …float_parse_table`, the 5 consts, `assemble`,
  `parse`, `parse_without_fast_path`, `takes_exact_path`, `finish`, `finish_inner`;
  `mod scan; mod lemire; mod exact;`, `pub(crate) use scan::{scan, Scanned};`,
  `pub(crate) use lemire::{lemire, BiasedFp, DECLINED};`, `use exact::exact;`, and LAST
  `#[cfg(test)] mod tests;` (keeps every part and item above the guards' cut point).
- `scan.rs` (116) [src 28-169]: `Scanned`, `scan`.
- `lemire.rs` (93) [src 171-280]: `power`, `full_multiplication`, `compute_product_approx`,
  `BiasedFp`, `DECLINED`, `lemire`.
- `exact.rs` (178) [src 365-596, incl. the `// --- The exact fallback ---` comment]: `Nat` + impl,
  `exact` → `pub(super)` (1 edit), `approximate`, `midpoint_above`, `midpoint_below`, `decompose`,
  `next`, `previous`, `compare_scaled`.
- `tests/mod.rs` (18) [src 600-616]: `use super::*;`, `check`, 2 `mod` lines.
- `tests/rounding.rs` (113, 6) [src 618-730, 951-967]: `the_corpus_vectors_are_correctly_rounded`,
  `the_classic_torture_cases_are_correctly_rounded`, `matches_std_over_random_bit_patterns`,
  `matches_std_over_many_digit_decimals`, `zero_keeps_its_sign`, `overflow_and_underflow_saturate`.
- `tests/paths.rs` (183, 4) [src 732-949]: `the_scanner_accepts_and_rejects_what_the_old_one_did`,
  `lemire_alone_is_sufficient`, `the_original_corpus_never_reached_the_exact_fallback`,
  `the_exact_fallback_is_reachable_and_correct`; helpers `midpoint_decimal`, `exact_dyadic`,
  `num_to_string`, `mul_decimal_by_two`, `mul_decimal_by_five`, `mul_decimal_small`.
Hazards: prose `float_parse_ref.rs` → `float_parse_ref/` in `src/codegen/string/format/float_parse.rs:4`,
`src/codegen/string/format/float_format_sci_ref.rs:4` (moves into `float_format_sci_ref/mod.rs`)
and `.ai/codegen-invariants.md:832`.

### 4.28 `src/codegen/string/format/float_format_sci_ref.rs` → `float_format_sci_ref/` (320, 0)

cfg(test) reference; `float_format_sci_ref/tests/mod.rs` already exists (declared by the
file's `mod tests;` at line 445). No outside importer (`grep -rn "float_format_sci_ref::" src tests` → none).
- `mod.rs` (153) [src 1-30, 253-445]: header, `sci_18`, `round_to`, `stringify_number_via_18`,
  `place`, `stringify_number`, `mod exact; pub(crate) use exact::{sci_digits, Fraction}; use exact::split;`,
  and LAST the existing `#[cfg(test)] mod tests;`.
- `exact.rs` (171) [src 31-251]: `use super::*;`; `sci_digits`, `split` → `pub(super)` (1 edit),
  `Fraction` + impl, `Big` + impl. `sci_digits` calls the parent's private `place` (visible to a child).
Hazards: prose in `src/codegen/string/format/float_format_sci.rs:4` "`float_format_sci_ref.rs`"
→ "`float_format_sci_ref/`".

### 4.29 `src/codegen/io/input/encode_tests.rs` → `encode_tests/` (436, 4)

`input/mod.rs` keeps `#[cfg(test)] mod encode_tests;`.
- `mod.rs` (24): the 4 `use` lines (incl. `use super::ncode_emu::…`; `ncode_emu` is declared in
  `input/mod.rs` via `#[path]` and does not move), 3 `mod` lines; shared `TABLES`, `native_mod_sets`.
- `kitty_terminal.rs` (148, 1) [src 18-166]: `the_encoder_writes_what_a_kitty_terminal_sends`;
  helpers `APPKIT_SHIFT`, `APPKIT_CTRL`, `APPKIT_OPTION`, `APPKIT_COMMAND`, `GDK_SHIFT`, `GDK_CTRL`, `enc`.
- `decode_round_trip.rs` (172, 2) [src 168-262, 284-368]: `the_decoder_reads_every_encoded_key_back`,
  `the_decoder_reads_every_encoded_text_key_back`; helpers `key_mods`, `key_event`, `expected_key`,
  `FUNCTION_KEYS`.
- `app_backends.rs` (98, 1) [src 372-474]: `the_emitted_encoder_matches_the_model_on_every_app_backend`;
  helpers `NoPlatform` + `impl Externals for NoPlatform`, `app_dump`, `grid`.
Hazards: none.

### 4.30 `src/codegen/io/input/reader_tests.rs` → `reader_tests/` (284, 11) and `io/input/tests.rs` → `tests/` (386, 7)

`reader_tests`:
- `mod.rs` (166): 4 `use` lines, `mod reads;`; the `.ncode` harness `PROGRAM`, `dump`, `Process` +
  `impl Externals`, `Run` + both `impl Run`, `READ_LINE`, `READ_RAW`, `READ_SCALAR`,
  `READ_GRAPHEME`, `READ_KEY`, `UNBOUNDED`, `error_code`.
- `reads.rs` (120, 11) [src 182-192, 222-349]: `read_line_joins_buffered_bytes_and_the_rest_of_the_line`,
  `read_raw_is_byte_exact_through_the_buffer_and_the_source`,
  `read_key_decodes_a_sequence_split_across_the_buffer_and_the_source`,
  `read_scalar_skips_keys_without_text_and_releases`, `read_scalar_gives_the_rest_of_a_multi_scalar_key_back`,
  `read_grapheme_joins_a_cluster_from_a_pipe`, `a_timeout_leaves_a_half_sequence_in_the_buffer`,
  `a_lone_escape_from_a_keyboard_is_decided_after_the_delay`,
  `a_keyboard_escape_sequence_is_not_split_by_the_delay`, `a_timed_read_line_gives_a_half_line_back`,
  `a_negative_timeout_is_an_invalid_argument`.
`tests` (`input/mod.rs` keeps `#[cfg(test)] mod tests;`):
- `mod.rs` (156): both `use` groups (lines 3-4 and 102-106), the `// --- The emitted decoder ---`
  comment block, 3 `mod` lines; shared `run`, `NoPlatform` + impl, `decoder_dump`,
  `decoder_dump_for`, `KEY_NAMES`, `KEY_EVENTS`, `Emitted` + impl.
- `model_corpus.rs` (57, 3) [src 35-94]: `the_model_decodes_the_whole_corpus`,
  `the_corpus_covers_every_named_key`, `the_corpus_rejects_a_slightly_wrong_decoder`.
- `emitted_corpus.rs` (57, 2) [src 242-254, 390-435]: `the_decoder_flags_are_the_models`,
  `the_decoder_decodes_the_corpus_as_every_backend_lowers_it`.
- `emitted_vs_model.rs` (122, 2) [src 256-388]: `the_emitted_decoder_decodes_the_whole_corpus`,
  `the_emitted_decoder_agrees_with_the_model_on_random_input`; the `thread_local!` block and
  `emitted_decode_shim` (only users).
Hazards: none.

### 4.31 `src/codegen/collection/assign/self_update.rs` (2085 test LOC, 9 tests) → `self_update/tests/`

The host keeps `#[cfg(test)] mod tests;` at line 1442 (the first `#[cfg(test)]`, so the guards'
cut point is unchanged) and its 1059 code LOC (plan-176-Q's split). Everything from 1442 to the
end moves: the census data (1442-3314) and `mod tests` (3315-3829). Per-item `#[cfg(test)]`
attributes on census items move verbatim.
- `tests/mod.rs` (26): the 4 `use` lines of the old `mod tests` (`BTreeSet`, `super::*`,
  `registry::{registry, self_update_shaped}`, `NativeBuildMode::Console`, `testutil::{…}`),
  `mod census; use census::*;`, 3 topic `mod` lines; shared `NON_REGISTRY`, `tier_b_row`,
  `shaped_functions`.
- `tests/census/mod.rs` (159) [src 1442-1625]: `use super::*;`; `SelfUpdate`, `Probe`,
  `SelfUpdateRow`, the package consts `C`…`AST`, `probe`, `probe_with`, `str_probe`,
  `STR`…`NOT_DERIVED_REASON`; `mod self_update_table; mod arm_ids; mod field_kinds; mod sites; mod probe_impl;`,
  `pub(crate) use self_update_table::SELF_UPDATE_TABLE;`,
  `pub(crate) use field_kinds::{field_kind_class, FieldKindClass, FieldKindRow, FIELD_KIND_TABLE};`,
  `pub(crate) use sites::{Site, ENABLED_SITES, FIELD_NEVER, FIELD_SITES, OWNED_PARAM_NEVER, RETURN_NEVER};`.
- `tests/census/self_update_table.rs` (881, exempt) [src 1626-2519]: `use super::*;`; `SELF_UPDATE_TABLE`.
- `tests/census/arm_ids.rs` (77) [src 2520-2606]: `impl ArmId` (`markers`, `lowers_in`, `code`).
- `tests/census/field_kinds.rs` (148) [src 2607-2792]: `FieldKindClass`, `field_kind_class`,
  `FieldKindRow`, `SIZE_VARIES`, `STRING_PLAN`, `FIELD_KIND_TABLE`.
- `tests/census/sites.rs` (168) [src 2793-3069]: `RETURN_NEVER`, `OWNED_PARAM_NEVER`, `Site`,
  `ENABLED_SITES`, `FIELD_SITES`, `FIELD_NEVER`, `impl Site`; `Site::field` → `pub(super)` (1 edit;
  called from `impl Probe`, old :3207).
- `tests/census/probe_impl.rs` (219) [src 3070-3313]: `replace_word`, `impl Probe`.
- `tests/census_coverage.rs` (168, 4) [src 3353-3544]: `field_kind_census_covers_every_record_field_type`,
  `self_update_builtin_names_every_spelling`, `self_update_census_covers_every_registry_overload`,
  `tier_b_transforms_have_rows`; helper `field_kind_census`.
- `tests/table_rows.rs` (146, 2) [src 3546-3705]: `self_update_table_has_no_stale_rows`,
  `every_arm_row_fires_at_every_enabled_site`.
- `tests/never_lists.rs` (107, 3) [src 3707-3828]: `return_never_names_dispatched_arms_once`,
  `field_never_names_field_sites_once`, `an_arm_with_no_field_reach_is_deferred_at_every_field_site`.
Hazards (all in the same commit):
- `include_str!("../../../../tests/runtime/inplace_self_update/field_expect.tsv")` (old :3781,
  → `never_lists.rs`) → `include_str!("../../../../../../tests/runtime/inplace_self_update/field_expect.tsv")`
  (two directories deeper).
- `tests/guards/inplace_self_update_census.rs`: the `//!` line naming
  `self_update_census_covers_every_registry_overload` "in `src/codegen/collection/assign/self_update.rs`"
  → "in `src/codegen/collection/assign/self_update/tests/census_coverage.rs`"; the doc comment of
  `every_package_record_type_has_a_field_kind_line` naming `field_kind_census_covers_every_record_field_type`
  (`self_update.rs`) → (`self_update/tests/census_coverage.rs`). The guard reads only the `.tsv`
  files, not this source.
- `tests/runtime/rt_inplace_self_update.rs`: the comment "`RETURN_NEVER` in `self_update.rs`" →
  "in `self_update/tests/census/sites.rs`". (Its `include_str!` of the host reads `SCRATCH_ARMS`,
  host code — unchanged.)
- `tests/runtime/inplace_self_update/field_kinds_gen.py:18` "(`field_kind_class`, `self_update.rs`, …"
  → "(`field_kind_class`, `self_update/tests/census/field_kinds.rs`, …".
- `.ai/collections.md` (the seam list after "The seam is `src/codegen/collection/assign/self_update.rs`"):
  the `SELF_UPDATE_TABLE` (tests only) bullet gains its new home,
  `self_update/tests/census/self_update_table.rs`.
- 6 spec citations name host code (`try_inplace_self_update`, `install_record_temps`,
  `add_global_string_capacities`) — unchanged.

### 4.32 `src/codegen/collection/layout/builder_collection_layout.rs` (432, 19) → `builder_collection_layout/tests/`

Three contiguous inline modules from line 3739 to the end (`grep -n "cfg(test)"` → 3739, 3910,
4082), each with a doc comment above. The host keeps one `#[cfg(test)] mod tests;` at 3739.
- `tests/mod.rs` (4): `use super::*;`, `mod kind2_layout_tests; mod flatness_split_tests; mod res_field_record_layout_tests;`.
- `tests/kind2_layout_tests.rs` (118, 5) [src 3740-3900]: the module's existing `use super::*;`
  and body; tests `representation_selectors_agree`, `alloc_size_matches_free_size`,
  `literal_type_refinement_is_narrow`, `nested_list_outer_keeps_entries`,
  `resource_elements_keep_entries`; helpers `FIXED_WIDTH`, `VARIABLE_WIDTH`.
- `tests/flatness_split_tests.rs` (112, 5) [src 3901-4071]: the doc comment above the old module
  becomes the file's `//!`; tests `a_res_field_and_its_record_diverge_between_the_two_predicates`,
  `a_res_collection_is_a_block_of_aliases`, `a_bare_resource_nominal_is_flat_in_neither_mode`,
  `every_resource_free_type_answers_both_predicates_the_same`, `a_cyclic_record_is_flat_in_neither_mode`;
  helper `model_with_res_field_record`.
- `tests/res_field_record_layout_tests.rs` (196, 9) [src 4073-4374]: doc comment → `//!`; tests
  `the_handle_field_is_a_plain_value_slot_not_a_pointer_or_inlined_block`,
  `the_record_copies_by_memcpy_but_may_not_cross_an_arena`,
  `the_handle_field_contributes_exactly_eight_bytes_and_no_sub_block`,
  `the_record_block_is_freeable_but_the_resource_record_is_not`,
  `a_user_declared_resource_is_neither_memcpy_copyable_nor_arena_transferable`,
  `only_a_thread_sendable_declaration_reaches_the_deep_copy_arm`, `a_thread_handle_is_flat_in_neither_mode`,
  `a_result_wrapping_a_thread_handle_is_flat_in_neither_mode`,
  `widening_the_resource_predicates_moves_no_pre_existing_answer`; helpers `holder_model`,
  `declared_resource_model`.
Test paths become `…::builder_collection_layout::tests::kind2_layout_tests::…` (nothing names
them by path, §3). Hazards: spec citations name host code. None.

### 4.33 `src/codegen/link/thunk/link_thunk.rs` inline (551, 7) → `link_thunk/tests/`

`mod tests` (2971-3654) → `#[cfg(test)] mod tests;`.
- `mod.rs` (18): `use super::*;` + the 4 crate `use` items, 4 `mod` lines; shared `TEST_THUNK_CONTEXT`.
- `float_fail_labels.rs` (139, 2) [src 2982-3137]: `struct_out_cdouble_field_emits_float_fail_labels`,
  `out_slot_cdouble_emits_float_fail_labels`; helper `assert_branch_labels_resolve`.
- `slots_and_buffers.rs` (91, 2) [src 3139-3264]: `only_the_builtin_file_resource_uses_io_buffers`,
  `seven_integer_slots_are_rejected_on_x86_and_accepted_on_aarch64`.
- `ctypes.rs` (143, 1) [src 3278-3460]: `every_known_ctype_lowers`.
- `native_calls.rs` (165, 2) [src 3462-3653]: `native_call_result_is_captured_from_c_return_register`,
  `windows_link_initializer_uses_win32_loader_not_dlopen`; helpers `one_vendored_library`, `call_targets`.
Hazards: 19 citations to host code; `tests/guards/no_type_strings.rs` names the host — unchanged.

### 4.34 `src/codegen/runtime/canvas/vulkan.rs` inline (748, 8) → `vulkan/tests/`

Only `mod tests` (9047-9862) moves; the host keeps `#[cfg(test)] mod tests;` at 9047. Two
cfg(test) items STAY in `vulkan.rs` (parent decision, 2026-10-14): `#[cfg(test)] const SPIRV_MAGIC`
(255-256) and `#[cfg(test)] fn assert_layout` (9011-9045). `assert_layout` is not used only by
this module: `vulkan_present.rs`'s own test module calls it 10 times through `use super::*`, so
moving it would break that compile. `layout_tables.rs` reaches it through the `use super::*`
chain (topic → `tests/mod.rs` → `vulkan`). Host test LOC after: 2 + 29 + 2 = 33. When
plan-176-V turns `vulkan.rs` into `vulkan/mod.rs`, the declaration resolves to the same
`vulkan/tests/`, and V checks `grep -c 'fn assert_layout' src/codegen/runtime/canvas/vulkan.rs`
→ 1 before its own move.
- `mod.rs` (8): `use super::*;`, the 2 `crate::codegen::runtime::canvas::{…}` imports, 3 `mod` lines.
- `layout_tables.rs` (486, exempt) [src 9055-9542]: `use super::*;`,
  `layout_tables_are_self_consistent` (calls the host's `assert_layout` unchanged).
- `pipeline_and_spirv.rs` (118, 5) [src 9544-9721]: `the_readback_buffer_prefers_cached_memory`,
  `the_embedded_spirv_is_well_formed`, `each_backend_has_one_pipeline_slot_per_blend_mode`,
  `the_pointer_words_end_the_graphics_state`, `the_item_block_matches_the_std430_stride`.
- `regions.rs` (108, 2) [src 9723-9861]: `the_pipeline_specializes_every_region_base`,
  `the_shared_buffer_holds_every_region`.
Hazards: `include_str!("shaders/mfb_canvas.frag")` (old :9738 → `regions.rs`) →
`include_str!("../../shaders/mfb_canvas.frag")`. 14 citations to host code; `.ai/canvas-threading.md`
names the two region tests by fn name — unchanged.

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

Every phase uses the same gate, run in this order (time estimates are guesses, not
measurements):

- **G0 (before the phase's first edit):** `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$' > /tmp/j_before.txt`
  and `cargo check --all-targets 2>&1 | grep -c '^warning' > /tmp/j_warn_before.txt` (est. 5 min
  cold, 1 min warm).
- **G1:** the same two commands after the phase's edits → numbers equal the G0 files.
- **G2:** run the moved modules' tests (filter per phase) → all pass, 0 failed.
- **G3:** `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings`
  and `cargo test -p mfb --bin mfb spec_citations_resolve` → all pass (est. 3 min).
- **G4:** `cargo test --test file_size_budget 2>&1 | grep -E '^  (NEW|GREW)'` → no output, and
  every `TIGHTEN`/`STALE` row names a file of this phase; then
  `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget` and rerun →
  `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`. Commit `tests/guards/file_size_ceilings.txt`
  (and `tools/loc-census/census.py` when the phase adds an exemption) with the phase (est. 2 min).
- **G5:** `cargo fmt --check` → no output (est. 20 s).

### Phase 1 — `binary_repr` suites and the resource tests

Eight sibling suites under one existing `tests/` module, plus one existing directory module.

- [ ] Split `src/binary_repr/tests/fixtures.rs` → `fixtures/` per §4.9 (2 visibility edits +
      re-export).
- [ ] Split `cross_package_tests.rs` per §4.8; update `scripts/coverage-exceptions.txt:46` prose.
- [ ] Split `gap_tests.rs` per §4.10, `reader_gap_tests.rs` per §4.11, `reader_tests.rs` per
      §4.12, `sections_tests.rs` per §4.13, `writer_tests.rs` per §4.14,
      `writer_walker_tests.rs` per §4.15 (4 visibility edits).
- [ ] Split `src/codegen/resource/tests/mod.rs` per §4.24; repoint `.ai/net-tls.md:37`.
- [ ] G4: tighten the ceilings for the nine files.

Acceptance: the suites run from their topic modules and the counts did not move.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb binary_repr:: codegen::resource::` → 0
  failed, and `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^(binary_repr::tests::(cross_package_tests|gap_tests|reader_gap_tests|reader_tests|sections_tests|writer_tests|writer_walker_tests)|codegen::resource::tests)::'`
  → 7+10+12+40+18+20+7+8 = 122; G3; G4; G5 (est. 12 min).
Commit: —

### Phase 2 — `src/arch` encoder, decoder and selector tests

- [ ] Split `src/arch/aarch64/decode/tests.rs` per §4.1 (two `include_str!` repoints, the
      `decode/mod.rs` prose); add the `exempt` row for `decode/tests/table.rs` to the ceiling
      file and the census `EXEMPT` table.
- [ ] Split `src/arch/aarch64/encode/tests.rs` per §4.2; repoint the two cites in
      `src/docs/spec/architecture/14_aarch64-instruction-set.md`.
- [ ] Split `src/arch/riscv64/encode/tests.rs` per §4.3.
- [ ] Split `src/arch/x86_64/encode/tests.rs` per §4.4; repoint the two cites in
      `src/docs/spec/architecture/15_x86_64-instruction-set.md` (`:1` → `tests/mod.rs:1`,
      `unsupported_op_errors` → `tests/image.rs`).
- [ ] Split the inline tests of `src/arch/riscv64/select.rs` (§4.5), `src/arch/riscv64/v128.rs`
      (§4.6) and `src/arch/x86_64/select.rs` (§4.7).
- [ ] G4: tighten; the new `exempt` row stays.

Acceptance: every arch test runs from a topic module, the guards ignore the register-spelling
fixtures through the `/tests/` rule, and the four spec cites resolve.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb arch::` → 0 failed, and
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^arch::(aarch64::(decode|encode)|riscv64::(encode|select|v128)|x86_64::(encode|select))::tests::'`
  → 11+41+41+130+39+36+13 = 311; `grep -rn "encode/tests.rs" src/docs/spec` → no output; G3; G4;
  G5 (est. 14 min).
Commit: —

### Phase 3 — `src/os` linker and object tests

- [ ] Split `src/os/linux/appimage/squashfs/tests.rs` per §4.16; update
      `scripts/coverage-exceptions.txt:44` prose.
- [ ] Split `src/os/linux/link/tests.rs` per §4.17; repoint the `writes_init_array_glibc_elf`
      cites in `src/docs/spec/linker/04_symbols-and-relocations.md` and `07_linux-aarch64.md`.
- [ ] Split `src/os/macos/link/tests.rs` per §4.18.
- [ ] Split `src/os/windows/link/spike.rs` per §4.19 (1 visibility edit); update the three
      `spike.rs` mentions in `src/target/win_x86_64/app/mod.rs`.
- [ ] Split the inline tests of `src/os/windows/link/mod.rs` (§4.20), `src/os/linux/appimage/mod.rs`
      (§4.21; rewrite the `scripts/coverage-exceptions.txt:47` prose as stated there),
      `src/os/linux/object.rs` (§4.22) and `src/os/macos/object.rs` (§4.23).
- [ ] G4: tighten the ceilings for the eight files.

Acceptance: every os test runs from a topic module, including the macOS-only launch tests on
this host.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb os::` → 0 failed, and
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^os::(linux::(appimage::squashfs::tests|link::tests|appimage::tests|object::tests)|macos::(link::tests|object::tests)|windows::link::(spike|tests))::'`
  → 30+51+19+16+45+17+2+23 = 203; `grep -rn "linux/link/tests.rs\|link/spike.rs\|squashfs/tests.rs" src scripts .ai`
  → no output; G3; G4; G5 (est. 14 min).
Commit: —

### Phase 4 — Input decoder and float reference implementations

- [ ] Add the `exempt` row for `src/codegen/io/input/corpus.rs` (§4.25) to the ceiling file and
      the census `EXEMPT` table; the file is not edited.
- [ ] Split `src/codegen/io/input/model.rs` → `model/` per §4.26 (2 visibility edits).
- [ ] Split `src/codegen/io/input/encode_tests.rs` per §4.29, `reader_tests.rs` and `tests.rs`
      per §4.30.
- [ ] Split `src/codegen/string/format/float_parse_ref.rs` → `float_parse_ref/` per §4.27
      (1 visibility edit; `#[cfg(test)] mod tests;` last in `mod.rs`); update the prose in
      `float_parse.rs:4` and `.ai/codegen-invariants.md:832`.
- [ ] Split `src/codegen/string/format/float_format_sci_ref.rs` → `float_format_sci_ref/` per
      §4.28 (1 visibility edit); update the prose in `float_format_sci.rs:4`; carry the
      `float_parse_ref.rs` → `float_parse_ref/` prose fix into the new `mod.rs` header.
- [ ] G4: tighten; the new `exempt` row stays.

Acceptance: the reference implementations keep their surfaces and their suites pass.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb codegen::io::input:: codegen::string::format::`
  → 0 failed, and `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^codegen::(io::input::(encode_tests|reader_tests|tests)|string::format::float_parse_ref::tests)::'`
  → 4+11+7+10 = 32; `grep -rn "float_parse_ref\.rs\|float_format_sci_ref\.rs" src .ai` → no
  output; G3; G4; G5 (est. 12 min).
Commit: —

### Phase 5 — Collection, link and canvas hosts (largest blast radius)

- [ ] Split `src/codegen/collection/layout/builder_collection_layout.rs`'s three test modules
      per §4.32.
- [ ] Split `src/codegen/link/thunk/link_thunk.rs`'s tests per §4.33.
- [ ] Split `src/codegen/runtime/canvas/vulkan.rs`'s `mod tests` per §4.34 (`include_str!`
      repoint; `assert_layout` and `SPIRV_MAGIC` stay in `vulkan.rs`); add the `exempt` row for
      `vulkan/tests/layout_tables.rs` in both tables.
- [ ] Split `src/codegen/collection/assign/self_update.rs`'s census and tests per §4.31
      (1 visibility edit, `include_str!` repoint, the four prose updates and the
      `.ai/collections.md` bullet); add the `exempt` row for
      `self_update/tests/census/self_update_table.rs` in both tables.
- [ ] G4: tighten; the two new `exempt` rows stay.

Acceptance: the census, the three layout modules, the thunk and canvas suites run from `tests/`
modules with every count unchanged, and the four exemptions are recorded.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb codegen::collection:: codegen::link:: codegen::runtime::canvas::vulkan::`
  → 0 failed, and `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^codegen::(collection::(assign::self_update|layout::builder_collection_layout)|link::thunk::link_thunk|runtime::canvas::vulkan)::tests::'`
  → 9+19+7+8 = 43; `grep -c 'fn assert_layout' src/codegen/runtime/canvas/vulkan.rs` → 1
  (and `cargo test -p mfb --bin mfb canvas::vulkan::present::` → 0 failed: `vulkan_present.rs`'s tests, which call it through
  `use super::*`); `grep -c '^exempt test' tests/guards/file_size_ceilings.txt` → the value
  before J + 4; `cargo test --test inplace_self_update_census` → pass (est. 1 min);
  G3; G4; G5 (est. 12 min).
Commit: —

## Validation Plan

- Tests: no test is added or removed. Per phase the whole-binary `--list` count is unchanged
  (G1) and the moved modules' tests pass (G2); the per-phase greps sum to
  122 + 311 + 203 + 32 + 43 = 711, every test in J's 35 files.
- Coverage check: per-file coverage is not a plan-176 gate — author, 2026-10-17. Evidence: at 7a4b09970,
  `sh scripts/coverage.sh --bins` in a detached worktree (59 min; 4989 unit tests passed), then
  `sh scripts/coverage-check.sh` exited 1. 241 `src/` files are already below the 98% per-file
  floor ("Files below 98% line coverage (GATE FAILURE)"), and overall coverage is 93.67%.
  Examples among plan-176 hosts: `src/ir/shape.rs` 93.21%,
  `src/codegen/engine/control/builder_control.rs` 75.55%, `src/debugger/values.rs` 86.66%. A
  pure move changes no executed line, and moving an inline test block only removes test lines
  from its host's denominator. The pre-existing gate failure is reported to the owner
  separately and is not a plan-176 task.
- Runtime proof: the repointed `include_str!` paths compile and their tests pass
  (`decodes_words_as_lldb_does`, `the_table_covers_every_form_the_encoder_emits`,
  `an_arm_with_no_field_reach_is_deferred_at_every_field_site`,
  `the_pipeline_specializes_every_region_base`) — a wrong path is a compile error.
- Doc sync: `src/docs/spec/architecture/14_aarch64-instruction-set.md`,
  `src/docs/spec/architecture/15_x86_64-instruction-set.md`,
  `src/docs/spec/linker/04_symbols-and-relocations.md`, `src/docs/spec/linker/07_linux-aarch64.md`,
  `.ai/collections.md`, `.ai/codegen-invariants.md`, `.ai/net-tls.md`,
  `scripts/coverage-exceptions.txt` (3 entries), and the prose in `tests/guards/inplace_self_update_census.rs`,
  `tests/runtime/rt_inplace_self_update.rs`, `tests/runtime/inplace_self_update/field_kinds_gen.py`,
  `src/target/win_x86_64/app/mod.rs`, `src/arch/aarch64/decode/mod.rs`,
  `src/codegen/string/format/float_parse.rs`, `float_format_sci.rs`. No compiler contract changes.
- Final gate (once, after Phase 5): `cargo test -p mfb --bin mfb spec_citations_resolve`,
  `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings --test file_size_budget --test inplace_self_update_census`
  → all pass and the banner has 0 rows; `cargo clippy --all-targets 2>&1 | grep -c '^warning'`
  → equal to its value at G0 of Phase 1 (est. 6 min). The full workspace suite runs once, in
  plan-176-V.

## Decisions

- Rule: code ≤ 1000, test ≤ 250 non-comment non-blank LOC; scope `.rs`/`.mfb` under `src/`
  `packages/` `examples/` `repository/` `tests/` — owner, 2026-10-14.
- Inline test code counts against the 250 test budget; over 250 it moves to its own module,
  split further as needed — owner, 2026-10-14.
- Generated files are exempt. The guard warns with a banner and never fails. A new module is
  always an acceptable split — owner, 2026-10-14.
- A single test-side unit over 250 (an embedded program, a const table / fixture list, a single
  `#[test]` fn, a single trait impl) gets its own file and an `exempt` row with its reason in
  `tests/guards/file_size_ceilings.txt` and in `tools/loc-census/census.py`'s `EXEMPT` table, in
  the same commit — owner, 2026-10-14. J's four: `src/arch/aarch64/decode/tests/table.rs`,
  `src/codegen/io/input/corpus.rs` (unsplit; no section fns),
  `src/codegen/collection/assign/self_update/tests/census/self_update_table.rs`,
  `src/codegen/runtime/canvas/vulkan/tests/layout_tables.rs` (§4.0).
- Extracted src test modules go into `foo/tests/mod.rs` + topic files — plan-176-A. This
  includes `builder_collection_layout`'s three modules (one `tests/` directory, module names
  kept) — author, 2026-10-14.
- The architecture guards' `/tests/` rule is plan-176-H's; J depends on it and does not edit
  any guard's logic — author, 2026-10-14.
- Pre-existing defects found while mapping are fixed in the letter that touches them — author,
  2026-10-14. J fixes the stale `.ai/net-tls.md` module path, the stale
  `scripts/coverage-exceptions.txt` prose (3 entries) and the stale `spike.rs`/`_ref.rs` prose
  paths.
- Glob-cycle imports compile without warnings — verified (`/tmp/loc/probe2`, `/tmp/hij_probe`).
- Per-file coverage is not a plan-176 gate; the measured evidence is in §Validation Plan
  "Coverage check" — author, 2026-10-17.

## Corrections

## Summary

J is pure motion of backend and codegen test code into `tests/` directory modules and
reference-implementation parts: 35 hosts, 711 tests, 0 renamed, 4 exempt files. The risk is in
`self_update.rs` (a nested census tree, an `include_str!` two levels deeper, four prose
references in other trees; last phase), in the 130-test x86 encoder suite whose header the spec
cites by line, and in keeping the `_ref`/`model` surfaces resolving at their old paths. Host
code, every reference implementation's public surface and every guard's logic are untouched.
