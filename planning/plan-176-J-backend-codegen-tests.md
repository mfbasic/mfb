# plan-176-J: Backend and remaining codegen test code

Last updated: 2026-10-18
Effort: medium (1h–2h)
Depends on: plan-176-I

J brings the backend test code in `src/arch`, `src/os` and four `src/codegen` hosts (collection,
link, runtime/canvas) under the plan-176 limits: a test-only file is ≤ 1000 LOC, and a file that
is not test-only holds ≤ 250 inline test LOC. 16 files are in scope: five test-only files over
1000, each split into topic modules, and eleven code files whose inline `#[cfg(test)]` code is
over 250, each moved out into its own test-only module. No test is added, removed or renamed.
Only the five split test-only files and `builder_collection_layout.rs`'s three inline modules
change test paths, by gaining one module segment
(`arch::x86_64::encode::tests::mov_reg_reg` → `arch::x86_64::encode::tests::integer::mov_reg_reg`).
Host code does not move. The code splits of `v128.rs`, `self_update.rs`,
`builder_collection_layout.rs`, `link_thunk.rs` and `vulkan.rs` belong to letters O, Q and V.

The single outcome: after J lands, `cargo test --test file_size_budget` shows no `inline` or
`test` row for any of J's 16 files or the files J creates, and `tests/guards/file_size_ceilings.txt`
has no `ceiling inline` or `ceiling test` row for them. The five hosts over 1000 code keep their
`ceiling code` rows until O, Q and V.

References:

- `planning/plan-176-A-file-size-guard.md` — the rule, §Decisions, counting rules (the
  `#[cfg(test)]` extent rule that makes `self_update.rs` 2087 inline test LOC), and "Splitting an
  over-budget file" in `.ai/build-tooling.md`.
- `planning/plan-176-H-codegen-builtins-engine-tests.md` §4.1 — the `/tests/` component rule in
  `tests/guards/architecture_guards.rs` that J's `tests/` modules rely on.
- `.ai/testing-gates.md` § "Splits must sweep man AND spec citations", and the coverage section.
- `.ai/arch-abi.md` — names `src/arch/x86_64/select.rs`, `src/arch/riscv64/select.rs`,
  `src/os/windows/link/mod.rs` (host paths J keeps) and `signed_build_emits_both_mfbnote_and_mfbsign_disjoint`
  by fn name (stable).
- `.ai/collections.md` — the self-update seam; its `SELF_UPDATE_TABLE` bullet is repointed (§4.13).
- `.ai/canvas-threading.md` — names `the_pipeline_specializes_every_region_base` /
  `the_shared_buffer_holds_every_region` by fn name (stable).
- `.ai/resources-packages.md` — names `every_known_ctype_lowers` by fn name (stable).
- Spec topics whose citations move: `src/docs/spec/architecture/14_aarch64-instruction-set.md`
  (2), `src/docs/spec/architecture/15_x86_64-instruction-set.md` (2),
  `src/docs/spec/linker/04_symbols-and-relocations.md` (1), `src/docs/spec/linker/07_linux-aarch64.md` (1).
- `scripts/coverage-exceptions.txt` — the `src/os/linux/appimage/mod.rs` entry, whose prose names
  the inline test module J moves (§4.10).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-I is complete and archived | `ls planning/plan-176-I-* planning/completed/plan-176-I-*` → only `planning/completed/…` | NOT MET (2026-10-18) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | NOT MET (2026-10-18) |
| The architecture guards treat a `tests` directory component as test code (plan-176-H §4.1) | `grep -rn 'contains("/tests/")' tests/guards/architecture_guards.rs` → a hit in each of `is_physical_scan_exempt`, `no_hand_picked_vreg` and `emission_lines` | NOT MET (2026-10-18) |
| J's files and the docs/scripts J edits have no uncommitted changes | `git status --porcelain -- src/arch/aarch64/encode src/arch/riscv64 src/arch/x86_64 src/os/linux/link src/os/macos src/os/linux/appimage/mod.rs src/os/linux/object.rs src/os/windows/link/mod.rs src/codegen/collection/assign/self_update.rs src/codegen/collection/layout/builder_collection_layout.rs src/codegen/link/thunk/link_thunk.rs src/codegen/runtime/canvas/vulkan.rs src/docs/spec/architecture src/docs/spec/linker .ai/collections.md scripts/coverage-exceptions.txt tests/guards/inplace_self_update_census.rs tests/runtime` → empty | MET (2026-10-18) |

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

- Every host in §2's table holds ≤ 250 inline test LOC, and every test-only file J creates is
  ≤ 1000 LOC (`python3 -I tools/loc-census/census.py --file <path>` → second field ≤ 250 for a
  host, ≤ 1000 for a test-only file). Every part of a split is ≤ 900 (headroom); a single moved
  `tests/mod.rs` is ≤ 1000 (the largest, `v128/tests/mod.rs`, is 910).
- `cargo test --test file_size_budget` prints no `inline` or `test` row naming a J path; the
  ceiling file has no `ceiling inline` or `ceiling test` row for one.
- The unit-test binary lists the same number of tests before and after each phase.

### Non-goals (explicit constraints)

- No behavior change. No test body edit beyond moving it (dedented), a leading `use super::*;`
  in a new topic file, and the two `include_str!` path repoints of §4 (each the same file at its
  new relative path). No visibility edit.
- No `#[test]` fn renamed, added or removed. No `[[test]]` target renamed (J touches none).
- No host code moves; `vulkan.rs`'s `#[cfg(test)] const SPIRV_MAGIC` and `#[cfg(test)] fn assert_layout`
  and `appimage/mod.rs`'s two `#[cfg(test)] const RUNTIME_*_LEN` stay in their hosts.
- No guard weakens. Nothing is exempt (the only exemption in plan-176 is a `GENERATED` banner,
  and no J file carries one).

## 2. Current State

LOC: `python3 -I /tmp/loc/census.py "$PWD"` at HEAD `ce7a995ca` (2026-10-18; output
`/tmp/loc/census.json`, the `test` field; for a test-only file it is the file's total LOC);
after plan-176-A, `python3 -I tools/loc-census/census.py --file <path>`. `self_update.rs` is
(1059 code, 2087 test) under plan-176-A's extent rule: every top-level item from line 1442 to the
end is `#[cfg(test)]`. Tests: `grep -cE '#\[(tokio::)?test\]' <file>`.

| File | Kind | Test LOC | Tests |
|---|---|---|---|
| `src/arch/aarch64/encode/tests.rs` | test | 1251 | 41 |
| `src/arch/riscv64/encode/tests.rs` | test | 1166 | 41 |
| `src/arch/x86_64/encode/tests.rs` | test | 3185 | 130 |
| `src/os/linux/link/tests.rs` | test | 1796 | 51 |
| `src/os/macos/link/tests.rs` | test | 1656 | 45 |
| `src/arch/riscv64/select.rs` | inline | 629 | 39 |
| `src/arch/riscv64/v128.rs` | inline | 913 | 36 |
| `src/arch/x86_64/select.rs` | inline | 279 | 13 |
| `src/codegen/collection/assign/self_update.rs` | inline | 2087 | 9 |
| `src/codegen/collection/layout/builder_collection_layout.rs` | inline | 432 | 19 |
| `src/codegen/link/thunk/link_thunk.rs` | inline | 551 | 7 |
| `src/codegen/runtime/canvas/vulkan.rs` | inline | 748 | 8 |
| `src/os/linux/appimage/mod.rs` | inline | 254 | 19 |
| `src/os/linux/object.rs` | inline | 259 | 16 |
| `src/os/macos/object.rs` | inline | 420 | 17 |
| `src/os/windows/link/mod.rs` | inline | 686 | 23 |

Totals: 16 files, 16,312 test LOC (9054 in the five test-only files, 7258 inline), 514 tests
(sum of the per-file greps).

Since the `17a9b5799` line ranges of §4 were taken, three hosts drifted (`git diff --numstat 17a9b5799 ce7a995ca -- <file>`):
`self_update.rs` +3 lines inside `FIELD_KIND_TABLE` (after line 2722), `builder_collection_layout.rs`
two in-place line edits in its test modules, `vulkan.rs` +45/−20 in host code (its `mod tests`
now starts at 9073). Item and test NAMES are authoritative; re-locate by name.

### Verified properties

- **Topic layouts compile warning-free.** `/tmp/hij_probe`: a non-`mod.rs` host `src/host.rs`
  with `#[cfg(test)] mod tests;` loads `src/host/tests/mod.rs` (`use super::*;` + `mod` lines),
  whose topics see the host's private items and private `use` imports; `cargo test` → no
  warning. `/tmp/loc/probe2`: glob cycles (`use part::*;` + `use super::*;`) → no warning. A
  `foo.rs` host and a `foo/` directory coexist (the same probe), which is the layout J uses for
  every non-`mod.rs` host. None of the nine directories J creates beside a host exists today
  (`ls -d src/arch/riscv64/select src/arch/riscv64/v128 src/arch/x86_64/select src/os/linux/object src/os/macos/object src/codegen/link/thunk/link_thunk src/codegen/runtime/canvas/vulkan src/codegen/collection/assign/self_update src/codegen/collection/layout/builder_collection_layout`
  → all "No such file or directory").
- **`include_str!` resolves relative to the file that contains the macro.** Two moved tests
  include files by relative path; each is repointed by the extra directory depth (§4.13, §4.16).
  No other moved region uses `include_str!`, `include_bytes!`, `#[path]`, `file!()`,
  `module_path!` or `current_exe` (grep over the 16 files' moved ranges → none).
- **What the architecture guards scan does not grow.** After plan-176-H, any path with a
  `/tests/` component is exempt from `is_physical_scan_exempt`, `no_hand_picked_vreg` and
  `emission_lines`. Every file J creates is under a `tests/` directory, and each was already
  test code in its parent (a `tests.rs` file or a `#[cfg(test)]` region below the host's cut
  point). The `no_type_strings` / `no_operator_strings` guards key `is_test_file` on `/tests/` the
  same way.
- **The self-update census leaves the host's scanned region unchanged.** Its first
  `#[cfg(test)]` region (line 1442) stays the cut point: the host keeps `#[cfg(test)] mod tests;`
  there. The census goes under `self_update/tests/census/`, so it stays exempt from the string
  guards (`rel.contains("/tests/")`) once its per-item `#[cfg(test)]` no longer sits in a scanned
  file.
- **No census item is used outside `self_update.rs`.** `grep -rn "SELF_UPDATE_TABLE\|FIELD_KIND_TABLE\|ENABLED_SITES\|FIELD_NEVER\b\|RETURN_NEVER\|field_kind_class" src tests`
  outside the file → only prose/doc mentions (`registry/mod.rs:3324` doc, `tests/runtime/rt_inplace_self_update.rs`
  defines its own `ENABLED_SITES` and names `RETURN_NEVER` in a comment,
  `field_kinds_gen.py` and `inplace_self_update_census.rs` prose). The external
  `include_str!("../../src/codegen/collection/assign/self_update.rs")` in
  `tests/runtime/rt_inplace_self_update.rs::scratch_arms_match_the_compiler` reads
  `SCRATCH_ARMS` (host code at line 706), which does not move. The census's `mod tests` names no
  private census item outside prose (`sed -n 3318,3832p … | grep -owE 'probe|probe_with|str_probe|replace_word|SIZE_VARIES|STRING_PLAN'`
  → only string/comment hits), so the `pub(crate)` items reach it through a glob.
- **Only macOS- and unix-gated code moves, and this host compiles both.**
  `grep -n "cfg(target_os\|cfg(unix\|cfg(windows"` over the 16 files' moved code:
  `os/macos/link/tests.rs` (`#[cfg(target_os = "macos")]` ×8, plus 2 comment mentions),
  `appimage/mod.rs` (`#[cfg(unix)]` ×3 in tests; its `#[cfg(windows)]` at 291 is host code that
  stays). No Windows- or Linux-only line moves, so no cross-target check is needed.
- **No test path is named outside the tree.** `grep -rnE "(encode|link)::tests::|builder_collection_layout::tests|kind2_layout_tests|flatness_split_tests|res_field_record_layout_tests" scripts .ai src/docs tests/guards .github AGENTS.md`
  → only `.ai/testing-gates.md`'s `arch::x86_64::encode::tests::*` glob, which still matches.
- **Moving an inline test module out of a code file removes covered lines from its coverage
  denominator** (`.ai/testing-gates.md`). Per-file coverage is not a plan-176 gate
  (§Validation Plan "Coverage check"), so J runs no coverage measurement.

## 3. Design Overview

The H convention: a whole test-only file `x.rs` over 1000 becomes `x/mod.rs` + topic parts; an
inline module becomes `#[cfg(test)] mod tests;` → `<host>/tests/mod.rs`, ONE test-only file unless
it is over 1000. `mod.rs` keeps the `use` lines, shared helpers and one `mod` per part; each part
starts with `use super::*;`; a helper with one user moves with it. Non-`mod.rs` hosts stay
`foo.rs`; their tests go to `foo/tests/`. When O/Q/V later turn a host into `foo/mod.rs`, its
`#[cfg(test)] mod tests;` resolves to the same `foo/tests/` directory.

Parts are the fewest that keep each ≤ 900 LOC, formed by merging neighbouring topics of the
`17a9b5799` map (`/tmp/loc/maps/J.md`); a part's LOC is the sum of the map topics it merges
minus the `use super::*;` lines the merge removes ("sum of map parts"). Every topic name below is
the map's, so line ranges and test lists carry over unchanged.

Gate class: test letter (same G0–G5 as H).

Risk concentrates in:
- **`self_update.rs`** — 1644 LOC of census data moves into a nested `tests/census/` module whose
  `SELF_UPDATE_TABLE` (881 LOC) is its own file; one `include_str!` two levels deeper; four prose
  references in other trees. Last phase.
- **`x86_64/encode/tests.rs`** — 130 tests into 4 parts; the spec cites line 1 of the file.

Rejected alternatives:
- *One file per map topic* — 55 topic files where 12 parts suffice; under the 1000 test-only
  limit the extra seams are pure churn.
- *A part per responsibility regardless of count* (e.g. three parts for `linux/link/tests.rs`) —
  the fewest-parts rule wins while every part stays ≤ 900; the chosen merges still follow topic
  neighbours.
- *Splitting `self_update`'s census into per-topic files* (`arm_ids`, `field_kinds`, `sites`,
  `probe_impl`) — they fit one 766-LOC `census/mod.rs` with the census types, which also removes
  the `Site::field` visibility edit that a separate `sites.rs` / `probe_impl.rs` pair needed.
- *Flattening `builder_collection_layout`'s three inline modules into `tests/mod.rs`* — their
  private helpers and `use super::*` scopes would merge; J moves them verbatim as three inline
  modules inside one `tests/mod.rs`.
- *`builder_collection_layout/kind2_layout_tests.rs` beside the host* — outside a `tests/`
  directory, so coverage counts it and the guards scan it.

## 4. Detailed Design

Line ranges are source lines as of `17a9b5799` (see §2 for the three drifted hosts); item and
test NAMES are authoritative if lines drift. Shared items stay in `mod.rs`. Hazard commands per
file: `grep -rnoE "\[\[<path>:[^]]+\]\]" src/docs/spec`;
`grep -rn "<path>" tests/guards scripts .ai AGENTS.md src/docs .github Cargo.toml`;
`grep -n "<path>" scripts/coverage-exceptions.txt`. "Hazards: none" means all returned nothing
to repoint.

An inline move (§4.4–§4.6, §4.9–§4.12, §4.15, §4.16) is the same mechanical step every time: the
host's `mod tests { … }` becomes `#[cfg(test)] mod tests;` (the attribute already present) and
the body, dedented, becomes `<host>/tests/mod.rs` with its existing `use super::*;`. Test paths
are unchanged. The new file's LOC is
`python3 -I /tmp/loc/span.py <host> <first body line> <last body line>` at `ce7a995ca`.

### 4.1 `src/arch/aarch64/encode/tests.rs` → `encode/tests/` (1251, 41 tests; 2 parts)

- `mod.rs` (49): 4 `use` lines, 2 `mod` lines; shared `fresh_encoder`, `encode_one`,
  `emit_words`, `plan`.
- `forms.rs` (672, 22; map topics `scalar_forms` + `memory` + `neon` + `op_coverage`)
  [src 6-331, 346-546, 567-791, 1435-1448]:
  `instruction_size_expands_large_stack_and_memory_immediates`,
  `encoder_accepts_large_immediates_with_fallback_sequences`, `encodes_umulh_add_carry_and_rorv`,
  `encodes_fmov_d_from_d`, `encodes_sxtw`, `encodes_fabs_d`, `encodes_fminnm_fmaxnm_d`,
  `encodes_b_vs_branch`, `encodes_b_mi_and_b_ls_branches`, `encodes_fp_scalar_spill_load_store`,
  `encodes_load_acquire_store_release`, `memory_ops_encode_all_widths`, `str_u16_encodes_strh`,
  `memory_ops_use_scratch_for_large_offsets`, `unaligned_memory_offsets_error`,
  `ldr_q_str_q_scratch_for_unaligned_or_large_offset`, `encodes_neon_vector_ops`,
  `neon_shift_and_umov_index`, `codeop_sizes_cover_large_add_imm`, `every_scalar_op_encodes`,
  `float_scalar_ops_encode`, `float_precision_conversions_encode`.
- `branches_image_operands.rs` (534, 19; map topics `carry_and_branches` + `image` +
  `operands_and_sizing`) [src 793-1433]: `add_carry_and_sub_borrow_sizes`,
  `cmp_imm_size_matches_emit_for_out_of_range_immediates`, `all_conditional_branches_patch`,
  `unresolved_branch_label_errors`, `bl_relocation_bindings`, `symbol_ref_data_and_got_relocations`,
  `unsupported_instruction_errors`, `shift_immediate_out_of_range_errors`,
  `encode_builds_image_with_symbols_data_and_labels`, `encode_requires_entry_and_rejects_bad_hex`,
  `encode_rejects_a_duplicate_label_within_a_function`, `encode_carries_imports_as_functions`,
  `operand_reg_decoding`, `operand_field_missing_reports_op`, `sizing_covers_immediate_and_memory_paths`,
  `sizing_helpers_directly`, `sp_is_rejected_in_shifted_register_operand_slots`,
  `cmp_imm_against_sp_is_rejected_only_when_the_immediate_forces_register_form`,
  `an_absurdly_wide_add_sub_immediate_is_rejected_rather_than_looped_over`; helpers `plan_fn`,
  `data_obj`.
Hazards: `src/docs/spec/architecture/14_aarch64-instruction-set.md:231`
`[[src/arch/aarch64/encode/tests.rs:encodes_neon_vector_ops]]` → `…/encode/tests/forms.rs:…`;
`:284` `…:encodes_umulh_add_carry_and_rorv]]` → `…/encode/tests/forms.rs:…` (both required: the
file vanishes).

### 4.2 `src/arch/riscv64/encode/tests.rs` → `encode/tests/` (1166, 41 tests; 2 parts)

- `mod.rs` (77): 7 `use` lines, 2 `mod` lines; shared `encode_text`, `words`, `ci`, `plan_of`,
  `func`, `try_encode`.
- `encoder.rs` (510, 25; map topics `scalar_ops` + `offsets_and_branches` + `operand_decoding` +
  `data_and_relocations`) [src 51-674]: `ret_and_ecall`, `add_and_sub_r_type`, `fp_min_max_d`,
  `fma_family_d`, `li_small_and_move`, `li_thirty_two_bit_uses_lui_addi`, `load_store_word_offsets`,
  `large_offset_load_uses_rd_not_t0_as_address_scratch`,
  `large_offset_load_with_rd_aliasing_base_stages_through_t0`, `conditional_branch_is_long_form`,
  `bitmanip_expansions_size_matches_emitted_bytes`, `li_reconstructs_all_values`,
  `integer_register_names_decode_to_numbers`, `fp_register_names_decode_to_numbers`,
  `immediate_and_shift_decoding`, `field_lookup_reports_missing_field`,
  `data_objects_string_and_raw_encode_and_symbolize`, `raw_data_object_rejects_malformed_hex`,
  `call_and_data_reference_to_import_and_internal_data`, `adrp_pageoff_to_import_uses_got_binding`,
  `call_to_internal_function_is_internal_binding`, `call_to_unresolved_symbol_errors`,
  `duplicate_label_in_function_errors`, `branch_to_unknown_label_errors`, `missing_entry_symbol_errors`.
- `emitter_and_rvv.rs` (583, 16; map topics `emitter_ops` + `emitter_control_and_fp` + `rvv` +
  `rvv_sizing`) [src 678-1322]: `integer_alu_and_logic_ops_encode_as_op`,
  `rotate_ops_expand_to_fixed_sequences`, `unary_and_muladd_ops_encode`, `shift_immediate_ops_encode`,
  `add_sub_immediate_small_and_large_paths`, `load_store_variants_and_large_offsets`,
  `float_precision_conversions_encode`, `control_flow_and_system_ops_encode`,
  `explicit_carry_ops_are_fixed_length`, `scalar_fp_ops_encode_as_op_fp`,
  `immediate_materialization_true_and_wide_values`, `emitter_rejects_bad_operands`,
  `vector_register_names_decode_to_numbers`, `rvv_ops_encode_to_reference_words`,
  `rvv_ops_are_single_words_and_size_matches`, `rvv_encoder_rejects_unknown_vop_and_bad_registers`;
  helper `vword`.
Hazards: none.

### 4.3 `src/arch/x86_64/encode/tests.rs` → `encode/tests/` (3185, 130 tests; 4 parts)

- `mod.rs` (43): the `//!` header (kept as the file's first lines), 9 `use` lines, 4 `mod`
  lines; shared `fresh_encoder`, `bytes`, `enc_err`, `enc`.
- `integer.rs` (736, 51; map topics `integer_alu` + `shifts_memory_branches` +
  `carry_and_zero_tokens` + `bit_ops_and_extended`) [src 40-1013]: `mov_reg_reg`, `mov_imm64`,
  `add_sub_and_or_eor`, `add_with_move`, `mvn_from_zero_token_materializes_minus_one`,
  `mov_from_zero_token_materializes_zero`, `sub_with_zero_lhs_negates`, `alu3_dst_equals_rhs_aliasing`,
  `mul_aliasing`, `mvn`, `mul_low`, `umulh`, `udiv_sdiv`, `msub`, `add_imm_sub_imm`, `add_sp_sub_sp`,
  `cmp_cmp_imm`, `shifts_imm`, `shifts_var`, `loads`, `stores`, `branch_self_ret_svc`, `blr`,
  `branches_are_fixed_size`, `call_emits_5_bytes`, `lea_rip_relative`, `add_pageoff_is_zero_bytes`,
  `add_carry_no_carry_in`, `add_carry_with_carry_in`, `sub_borrow_no_borrow_in`,
  `add_carry_zero_token_rhs_adds_immediate_zero`, `add_carry_zero_token_rhs_no_carry_in_is_move`,
  `sub_borrow_zero_token_rhs_subtracts_immediate_zero`, `cmp_zero_token_rhs_compares_immediate_zero`,
  `cmp_zero_token_lhs_rejected`, `cmp_imm_zero_token_lhs_rejected`, `add_carry_zero_token_lhs_commutes`,
  `sub_borrow_zero_token_lhs_rejected`, `label_and_pageoff_are_empty`, `clz_lzcnt`,
  `rev_word_and_quad`, `rbit_bit_reverse_sequence`, `smulh_signed_high`, `alu_zero_lhs_forms`,
  `alu_zero_rhs_rejected`, `alu_dst_equals_rhs_commutes_and_subtracts`, `lsrv_variable_shift`,
  `rorv_word_variable`, `u32_load_store_extended`, `str_u16_encodes_the_operand_size_prefixed_store`,
  `str_u8_extended_register`; helper `just_bytes`.
- `float_v128_and_ranges.rs` (849, 22; map topics `float_scalar` + `v128_arith` +
  `v128_compares_and_lanes` + `v128_select_and_convert` + `aliasing_and_ranges`) [src 1015-2031]:
  `float_scalar_moves_and_arith`, `float_sqrt_compare_neg_abs`, `float_int_conversions`,
  `f2i_nearest_never_clobbers_its_own_dst`, `float_scalar_mem`, `float_mem_bad_offset_errors`,
  `v128_load_store_and_bad_offset`, `v128_three_operand_arith`, `v128_unary_and_neg_abs`,
  `v128_integer_compares`, `v128_float_compares`, `v128_zero_compares`, `v128_rounding`,
  `v128_shifts_dup_extract`, `v128_bit_select_fma_convert`, `div_aliasing_and_preservation`,
  `msub_extended_registers`, `add_carry_extended_and_sub_borrow`, `x86_float_conditional_branches`,
  `immediate_true_false_and_shift_range`, `imm32_overflow_and_disp32_overflow`, `blr_and_unknown_register`.
- `image_and_coverage.rs` (788, 28; map topics `image` + `integer_coverage` +
  `scalar_double_coverage` + `v128_coverage_arith`) [src 2033-2971]:
  `encode_produces_an_image_with_symbols_and_data`, `encode_requires_entry_symbol`,
  `encode_data_rejects_bad_hex`, `encode_carries_imports`, `encoder_emits_and_patches_a_local_branch`,
  `encoder_unresolved_label_errors`, `encoder_call_relocation_bindings`, `encoder_data_and_got_relocations`,
  `unsupported_op_errors`, `operand_decoding_edge_cases`, `label_and_add_pageoff_are_empty`,
  `rev_w_rev_x`, `rbit_reverse_bits`, `msub_disjoint_and_dst_aliases_lhs`,
  `div_aliasing_and_dividend_preservation`, `shifts_var_32bit`, `shift_imm_move_first`,
  `add_imm_move_first_and_str_u32_extended`, `str_u8_extended_and_u16_encode`, `extra_branch_conditions`,
  `scalar_double_moves_and_arith`, `scalar_double_min_max`, `scalar_double_fma_family`,
  `scalar_double_compares_and_signops`, `int_float_conversions`, `scalar_double_mem`,
  `v128_load_store_and_arith`, `v128_unary_and_negabs`; helpers `encode_err`, `minimal_plan`,
  `simple_function`.
- `lanes_plans_and_aliasing.rs` (777, 29; map topics `v128_coverage_lanes` + `error_arms` +
  `plan_encoding` + `ties_away` + `carry_register_aliasing`) [src 2973-3920]:
  `v128_compares_against_zero`, `v128_lane_shifts_and_moves`, `v128_bit_selects_fma_and_serial_conversions`,
  `alu3_and_zero_and_error_arms`, `add_carry_dst_not_lhs_and_sub_borrow_with_borrow_in`,
  `immediate_and_disp_overflow_errors`, `fixed_register_aliasing_is_rejected_rather_than_miscompiled`,
  `encode_full_plan_labels_calls_and_data`, `duplicate_label_in_a_function_is_rejected`,
  `ordered_only_float_branch_jp_skips_only_its_own_jcc`, `encode_external_call_and_got_load`,
  `encode_unresolved_call_and_label_error`, `ties_away_model_matches_aarch64_semantics`,
  `ties_away_encodes_the_exact_fraction_sequence`, `sxtw_sign_extends_low_32`,
  `store_zero_immediate_by_width`, `variable_shift_targeting_rcx_shifts_in_a_scratch`,
  `variable_shift_reloads_value_aliasing_rcx`, `variable_shift_w_reloads_value_aliasing_rcx`,
  `add_carry_dst_aliases_rhs_no_carry_in`, `add_carry_dst_aliases_rhs_with_carry_in`,
  `add_carry_register_carry_in_distinct_operands`, `add_carry_register_carry_in_zero_rhs`,
  `sub_borrow_zero_rhs_no_borrow_in_moves`, `sub_borrow_dst_aliases_rhs_no_borrow_in`,
  `sub_borrow_zero_rhs_register_borrow_in`, `sub_borrow_dst_aliases_rhs_with_register_borrow_in_is_rejected`,
  `umulh_stages_rhs_aliasing_rax`, `float_precision_conversions_encode`; helpers `ci`, `plan_with`.
Four is the fewest: three parts cannot stay ≤ 900 (3164 topic LOC / 3 = 1055).
Hazards: `src/docs/spec/architecture/15_x86_64-instruction-set.md:103`
`[[src/arch/x86_64/encode/tests.rs:1]]` cites the file's `//!` header ("Byte-exact encoding
tests — the correctness gate…") → `[[src/arch/x86_64/encode/tests/mod.rs:1]]`, with the header
kept at line 1 of `mod.rs`; `:232` `…:unsupported_op_errors]]` →
`…/encode/tests/image_and_coverage.rs:unsupported_op_errors]]`.

### 4.4 `src/arch/riscv64/select.rs` inline (629, 39 tests) → `select/tests/mod.rs`

`#[cfg(test)] mod tests { … }` (889-1651 at `ce7a995ca`) → `#[cfg(test)] mod tests;` (host keeps
559 code LOC). `select/tests/mod.rs` (626; span 890-1650): the whole body — `use super::*;`, the
`lower_t…` import, shared `build`, `sel`, `values`, `fl`, all 39 tests and helper
`t0_held_across_a_vreg`.
Hazards: 14 citations, all to host code that stays; `.ai/arch-abi.md` names the host path. None.

### 4.5 `src/arch/riscv64/v128.rs` inline (913, 36 tests) → `v128/tests/mod.rs`

`mod tests` (1443-2537 at `ce7a995ca`) → `#[cfg(test)] mod tests;`. `v128/tests/mod.rs` (910;
span 1444-2536): the whole body (shared `map`, `big`, `fl`, `count`, helpers `mir`, `peak`,
`slots_and_vregs`, all 36 tests). It is ≤ 1000, so one file. When plan-176-O turns `v128.rs`
into `v128/mod.rs` + parts, it re-exports from `mod.rs` every private fn these tests reach via
`super::*` (recorded for O).
Hazards: 5 citations to host code; `tests/guards/no_operator_strings.rs` names the host path
(host code, unchanged). None.

### 4.6 `src/arch/x86_64/select.rs` inline (279, 13 tests) → `select/tests/mod.rs`

`mod tests` (414-747 at `ce7a995ca`) → `#[cfg(test)] mod tests;`. `select/tests/mod.rs` (276;
span 415-746): shared `ci`, `sel`, `values` and all 13 tests.
Hazards: 10 citations, all to host code. `[[src/arch/x86_64/select.rs:1]]`
(15_x86_64-instruction-set.md:23) cites the host's own `//!` header ("x86-64 instruction
selection…", `sed -n 1p src/arch/x86_64/select.rs`), which stays — no repoint. `.ai/arch-abi.md`
names the host path — unchanged.

### 4.7 `src/os/linux/link/tests.rs` → `link/tests/` (1796, 51 tests; 2 parts)

`link/mod.rs` keeps `mod tests;`.
- `mod.rs` (120): the 2 `use` lines, 2 `mod` lines; shared `none`, `versioned_exit_image`,
  `glob_dat_image`, `has_gnu_stack`, `phdr_range`.
- `elf_writing.rs` (833, 30; map topics `static_elf` + `dynamic_elf` + `unbound_and_rejects` +
  `libc_flavors` + `riscv64_relocs`) [src 53-125, 180-625, 690-790, 846-1091, 1690-1789]:
  `encode_static_elf_x86_emits_two_pt_load_segments`,
  `encode_static_elf_places_data_where_relocations_expect_it`, `encode_static_elf_x86_appends_signing_section`,
  `write_executable_x86_static_writes_flavored_output`, `write_appdir_puts_the_same_elf_under_usr_bin`,
  `write_executable_rejects_entry_not_in_text`, `write_executable_x86_static_data_pc32`,
  `write_executable_x86_dynamic_covers_all_reloc_kinds`,
  `write_executable_x86_dynamic_musl_uses_musl_interpreter`,
  `write_executable_aarch64_dynamic_musl_uses_musl_interpreter`,
  `dynamic_build_rejects_missing_initializer_symbol`, `dynamic_elf_appends_signing_section`,
  `write_executable_rejects_unbound_external_symbol`, `write_executable_rejects_unsupported_relocation`,
  `write_executable_rejects_unbound_external_all_kinds`, `write_executable_rejects_undefined_internal_symbol`,
  `refuses_to_write_an_executable_under_a_traversing_name`, `writes_glob_dat_glibc_elf`,
  `writes_glob_dat_musl_elf`, `writes_mfb_sign_section_to_static_elf`, `writes_init_array_glibc_elf`,
  `writes_versioned_glibc_elf`, `dynamic_elf_hash_chain_starts_at_the_null_symbol`,
  `riscv_pcrel_lo12_pairs_with_adjacent_auipc`, `riscv_pcrel_lo12_pairs_across_a_spill_gap`,
  `riscv_pcrel_lo12_pairs_with_nearest_preceding_hi_of_same_target`,
  `riscv_hi_lo_rejects_a_displacement_past_the_auipc_reach`,
  `the_pairing_index_is_keyed_by_kind_as_well_as_target`,
  `the_pairing_index_does_not_assume_relocations_arrive_sorted`,
  `a_lo12_with_no_preceding_hi20_keeps_its_diagnostic`; helpers `x86_static_image`,
  `x86_dynamic_image`, `expect_unbound`, `init_array_image`, `riscv_reloc`.
- `sections_and_symbols.rs` (867, 21; map topics `arch_relocations` + `provenance_and_relro` +
  `runpaths` + the three `debug_symbols` topics) [src 627-688, 792-844, 1093-1688, 1793-2156]:
  `write_executable_aarch64_static_internal_relocs`,
  `write_executable_riscv64_internal_call_patches_auipc_jalr_pair`,
  `write_executable_riscv64_dynamic_covers_call_pcrel_and_got`,
  `static_elf_carries_the_mfbasic_provenance_note`,
  `dynamic_elf_carries_the_mfbasic_provenance_note_past_the_interpreter`,
  `provenance_note_coexists_with_the_signing_section`, `dynamic_elf_relro_covers_the_dynamic_segment`,
  `a_non_vendor_elf_emits_no_runpath`, `a_vendoring_elf_emits_one_origin_relative_runpath`,
  `the_runpath_string_does_not_disturb_the_symbol_table`,
  `a_runpath_grows_the_dynamic_prefix_by_exactly_its_dynstr_bytes`,
  `the_emitted_dynstr_matches_the_prefix_computation_with_a_runpath`,
  `an_appdir_vendoring_elf_emits_the_usr_lib_relative_runpath`,
  `each_runpath_grows_the_dynamic_prefix_by_its_own_length`,
  `a_non_vendoring_appdir_elf_is_byte_identical_to_the_console_build`, `debug_symbols_static_aarch64`,
  `debug_symbols_static_x86`, `debug_symbols_signed_keeps_mfbsign_findable`,
  `debug_symbols_off_writes_no_table`, `debug_symbols_dynamic_with_rodata`,
  `signed_dynamic_table_names_the_dynamic_segment`; helpers `rv_inst`, `mfb_note`, `elf_runpaths`,
  `static_debug_image`, `assert_dynamic_section_covers_pt_dynamic`, and the private parsers
  `ParsedSection` (private fields), `parse_sections`, `section_index`, `parse_symtab`,
  `debug_symbol`, `assert_symbols_match` (one file, so no nested module and no edit).
Hazards: `src/docs/spec/linker/04_symbols-and-relocations.md:110` and
`src/docs/spec/linker/07_linux-aarch64.md:115` `[[src/os/linux/link/tests.rs:writes_init_array_glibc_elf]]`
→ `[[src/os/linux/link/tests/elf_writing.rs:writes_init_array_glibc_elf]]` (required).

### 4.8 `src/os/macos/link/tests.rs` → `link/tests/` (1656, 45 tests; 2 parts)

`link/mod.rs` keeps `mod tests;`.
- `mod.rs` (91): the 4 `use` lines, 2 `mod` lines; shared `import`, `none`, `text_symbol`,
  `data_symbol`, `encode_aarch64`, `code_fn`, `inst`.
- `mach_o.rs` (819, 24; map topics `binding` + `initializers` + `signing_and_provenance` +
  `patch_relocations` + `symbols_and_dylibs`) [src 37-974]:
  `patches_external_data_relocations_to_got_entry`, `bind_info_uses_uleb_ordinal_past_fifteen`,
  `import_libraries_assigns_one_ordinal_per_distinct_library`, `rejects_initializer_without_text_symbol`,
  `runs_initializer_before_entry_without_imports`, `runs_initializer_before_entry_with_imports`,
  `writes_mfb_sign_section_to_mach_o`, `mach_o_carries_the_mfbasic_provenance_note_inside_the_signed_region`,
  `provenance_note_coexists_with_the_mfb_sign_segment`, `noted_mach_o_verifies_and_runs`,
  `patches_internal_and_data_relocations`, `patch_relocations_rejects_unsupported_kind`,
  `patch_relocations_rejects_out_of_range_offset`, `patch_relocations_rejects_unbound_external_symbols`,
  `patches_external_got_page_relocations`, `links_and_runs_program_importing_from_two_dylibs`,
  `symbol_vmaddr_rejects_unknown_symbol`, `symbol_vmaddr_splits_constants_from_writable_data`,
  `dylib_path_covers_all_libraries_and_rejects_unknown`, `library_ordinal_rejects_absent_library`,
  `data_const_helpers_size_by_slots`, `plist_escape_handles_all_predefined_entities`,
  `rejects_entry_symbol_not_in_text`, `dylib_path_resolves_app_mode_frameworks`; helpers `mfb_note`,
  `code_signature_offset`.
- `bundles_and_debug.rs` (750, 21; map topics `app_bundle` + `layout` + `rpaths` + the three
  `debug_symbols` topics) [src 976-1926]: `app_info_plist_has_required_bundle_keys`,
  `app_info_plist_publishes_manifest_version`, `app_info_plist_escapes_xml_metacharacters`,
  `app_info_plist_escapes_xml_metacharacters_in_version`, `writes_and_launches_app_bundle`,
  `links_and_launches_app_bundle_importing_libobjc`, `write_app_bundle_creates_layout_and_plist_host_neutral`,
  `writes_full_mach_o_with_imports_data_and_initializer`, `code_offset_and_layout_helpers_vary_with_presence`,
  `a_non_vendor_image_emits_no_rpath`, `a_vendoring_image_emits_one_rpath_with_a_consistent_header`,
  `an_app_bundle_rpath_points_at_the_frameworks_directory`,
  `a_vendoring_bundle_differs_from_the_console_binary_by_exactly_the_rpath`,
  `a_vendoring_binary_still_launches`, `refuses_to_write_an_executable_under_a_traversing_name`,
  `debug_symbols_lists_locals_before_imports`, `debug_symbols_shift_indirect_table_and_iundefsym`,
  `debug_symbols_on_import_less_image`, `function_starts_lists_every_function`,
  `debug_symbols_image_with_imports_runs`, `debug_uuid_differs_between_images_and_is_stable`;
  helpers `exit_zero_image`, `mach_o_rpaths`, `mach_o_uuid`, and the private `ParsedSymtab`
  (private fields), `parse_symtab`, `ordinal_of`, `section_addr`, `debug_image`.
Each `#[cfg(target_os = "macos")]` attribute moves with its item. Hazards: none (several tests
launch the produced Mach-O, not the test binary).

### 4.9 `src/os/windows/link/mod.rs` inline (686, 23 tests) → `windows/link/tests/mod.rs`

`mod tests` (662-1490 at `ce7a995ca`) → `#[cfg(test)] mod tests;`; `#[cfg(test)] mod spike;`
(22-23) stays. `windows/link/tests/mod.rs` (681; span 663-1489): the image import, shared `image`,
`le_u16`, `le_u32`, `le_u64`, `section_named`, `read_at_rva`, `exit_process_42_image`, every
helper and all 23 tests.
Hazards: 16 citations to host code; `.ai/arch-abi.md` names the host path and
`signed_build_emits_both_mfbnote_and_mfbsign_disjoint` by fn name — unchanged.

### 4.10 `src/os/linux/appimage/mod.rs` inline (254, 19 tests) → `appimage/tests/mod.rs`

`mod tests` (308-618 at `ce7a995ca`) → `#[cfg(test)] mod tests;`; the two
`#[cfg(test)] const RUNTIME_*_LEN` (49-52) stay. `appimage/tests/mod.rs` (247; span 309-617):
helpers `blobs`, `fixture_appdir` and all 19 tests, with their three `#[cfg(unix)]` attributes.
Hazards: `scripts/coverage-exceptions.txt:47` (this file's entry) names the inline test module's
assertion arms (":463-524") and a follow-up to move them out — both done by this move. The entry
is rewritten to keep its production reason only: "caps below the floor: the seal() blob-length
supply-chain guard is unreachable with the embedded runtime (proven by
every_runtime_ends_exactly_at_its_own_length); every production path is unit-covered". The entry is kept:
the seal() guard stays unreachable after the move, and an exception entry for a file that is at
or above the floor changes no gate result. Host `include_bytes!("runtime-…")` are code and stay.

### 4.11 `src/os/linux/object.rs` inline (259, 16 tests) → `object/tests/mod.rs`

`mod tests` (465-763 at `ce7a995ca`) → `#[cfg(test)] mod tests;`. `object/tests/mod.rs` (256;
span 466-762): the plan import, shared `void_type`, `function`, `call`, `base_plan`, `full_plan`
and all 16 tests.
Hazards: 7 citations and 3 prose mentions name the host — unchanged.

### 4.12 `src/os/macos/object.rs` inline (420, 17 tests) → `object/tests/mod.rs`

`mod tests` (685-1141 at `ce7a995ca`) → `#[cfg(test)] mod tests;`. `object/tests/mod.rs` (417;
span 686-1140): the plan import, shared `void_type`, `function`, `plan_call`, `base_plan`,
`full_plan` and all 17 tests.
Hazards: 2 citations and 2 prose mentions name the host — unchanged.

### 4.13 `src/codegen/collection/assign/self_update.rs` (2087 inline test LOC, 9 tests) → `self_update/tests/`

The host keeps `#[cfg(test)] mod tests;` at line 1442 (the start of the first `#[cfg(test)]`
region, so the guards' cut point is unchanged) and its 1059 code LOC (plan-176-Q's split).
Everything from 1442 to the end moves: the census data (1442-3317 at `ce7a995ca`) and
`mod tests` (3318-3832). Per-item `#[cfg(test)]` attributes on census items move verbatim. The
2087 LOC are over the 1000 test-only limit, so the block becomes three files, each ≤ 900
(measured with `python3 -I /tmp/loc/span.py src/codegen/collection/assign/self_update.rs <a> <b>`
at `ce7a995ca`):
- `tests/mod.rs` (442; span 3318-3832 = 443 minus the 3 wrapper lines, plus 2): the old
  `mod tests` body verbatim — its 5 `use` lines (`BTreeSet`, `super::*`,
  `registry::{registry, self_update_shaped}`, `NativeBuildMode::Console`, `testutil::{…}`),
  shared `NON_REGISTRY`, `tier_b_row`, `shaped_functions`, helper `field_kind_census`, and all 9
  tests (`field_kind_census_covers_every_record_field_type`, `self_update_builtin_names_every_spelling`,
  `self_update_census_covers_every_registry_overload`, `tier_b_transforms_have_rows`,
  `self_update_table_has_no_stale_rows`, `every_arm_row_fires_at_every_enabled_site`,
  `return_never_names_dispatched_arms_once`, `field_never_names_field_sites_once`,
  `an_arm_with_no_field_reach_is_deferred_at_every_field_site`) — plus `mod census; use census::*;`.
  Test paths stay `…::self_update::tests::<name>`.
- `tests/census/mod.rs` (766; spans 1442-1628 = 149 and 2521-3317 = 614, plus 3): `use super::*;`;
  `SelfUpdate`, `Probe`, `SelfUpdateRow`, the package consts `C`…`AST`, `probe`, `probe_with`,
  `str_probe`, `STR`…`NOT_DERIVED_REASON`; `impl ArmId` (`markers`); `FieldKindClass`,
  `field_kind_class`, `FieldKindRow`, `SIZE_VARIES`, `STRING_PLAN`, `FIELD_KIND_TABLE`;
  `RETURN_NEVER`, `OWNED_PARAM_NEVER`, `Site`, `ENABLED_SITES`, `FIELD_SITES`, `FIELD_NEVER`,
  `impl Site` (`lowers_in`, `code`, private `field`); `replace_word`, `impl Probe` (`source`,
  private `field_source`, `field_program`); `mod self_update_table;
  pub(crate) use self_update_table::SELF_UPDATE_TABLE;`. `impl Site` and `impl Probe` share the
  file, so the private `Site::field` needs no visibility edit. Every item the tests name is
  already `pub(crate)` (`awk 'NR>=1442 && NR<=3317 && /^pub\(crate\)/' src/codegen/collection/assign/self_update.rs` → `SelfUpdate`, `Probe`,
  `SelfUpdateRow`, `SELF_UPDATE_TABLE`, `FieldKindClass`, `field_kind_class`, `FieldKindRow`,
  `FIELD_KIND_TABLE`, `RETURN_NEVER`, `OWNED_PARAM_NEVER`, `Site`, `ENABLED_SITES`, `FIELD_SITES`,
  `FIELD_NEVER`), so `use census::*;` reaches it.
- `tests/census/self_update_table.rs` (882; span 1629-2520 = 881, plus 1): `use super::*;`;
  `SELF_UPDATE_TABLE` (the one const, 880 LOC of rows, which reaches the parent's private
  package consts and `probe*` fns as a child). It is one indivisible literal, and at 882 it is
  within both the 1000 limit and the 900 part target, so it is not split.
Hazards (all in the same commit):
- `include_str!("../../../../tests/runtime/inplace_self_update/field_expect.tsv")` (line 3784 at
  `ce7a995ca`, → `tests/mod.rs`) → `include_str!("../../../../../../tests/runtime/inplace_self_update/field_expect.tsv")`
  (two directories deeper).
- `tests/guards/inplace_self_update_census.rs`: the `//!` line naming
  `self_update_census_covers_every_registry_overload` "in `src/codegen/collection/assign/self_update.rs`"
  (line 18) → "in `src/codegen/collection/assign/self_update/tests/mod.rs`"; the doc comment of
  `every_package_record_type_has_a_field_kind_line` naming `field_kind_census_covers_every_record_field_type`
  (`self_update.rs`, line 427) → (`self_update/tests/mod.rs`). The guard reads only the `.tsv`
  files, not this source.
- The comment "`RETURN_NEVER` in `self_update.rs`" (`tests/runtime/rt_inplace_self_update.rs:379`
  at `ce7a995ca`; plan-176-B splits that file first, so locate it with
  `grep -rn 'RETURN_NEVER. in .self_update.rs' tests/runtime`) → "in `self_update/tests/census/mod.rs`".
  (Its `include_str!` of the host reads `SCRATCH_ARMS`, host code — unchanged.)
- `tests/runtime/inplace_self_update/field_kinds_gen.py:18` "(`field_kind_class`, `self_update.rs`, …"
  → "(`field_kind_class`, `self_update/tests/census/mod.rs`, …".
- `.ai/collections.md:80` (the seam list after "The seam is `src/codegen/collection/assign/self_update.rs`"):
  the `SELF_UPDATE_TABLE` (tests only) bullet gains its new home,
  `self_update/tests/census/self_update_table.rs`.
- 6 spec citations name host code (`try_inplace_self_update`, `install_record_temps`,
  `add_global_string_capacities`) — unchanged.

### 4.14 `src/codegen/collection/layout/builder_collection_layout.rs` (432, 19 tests) → `builder_collection_layout/tests/mod.rs`

Three contiguous inline modules from line 3739 to the end (`grep -n "cfg(test)"` → 3739, 3910,
4082 at `17a9b5799`), each with a doc comment above. The host keeps one `#[cfg(test)] mod tests;`
at 3739. `tests/mod.rs` (430; span 3741-4374 at `ce7a995ca` = 429, plus 1): `use super::*;`, then
the three modules moved verbatim as inline modules, each with its doc comment and its own
`use super::*;`:
- `mod kind2_layout_tests` [src 3740-3900]: `representation_selectors_agree`,
  `alloc_size_matches_free_size`, `literal_type_refinement_is_narrow`, `nested_list_outer_keeps_entries`,
  `resource_elements_keep_entries`; helpers `FIXED_WIDTH`, `VARIABLE_WIDTH`.
- `mod flatness_split_tests` [src 3901-4071]: `a_res_field_and_its_record_diverge_between_the_two_predicates`,
  `a_res_collection_is_a_block_of_aliases`, `a_bare_resource_nominal_is_flat_in_neither_mode`,
  `every_resource_free_type_answers_both_predicates_the_same`, `a_cyclic_record_is_flat_in_neither_mode`;
  helper `model_with_res_field_record`.
- `mod res_field_record_layout_tests` [src 4073-4374]:
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
them by path, §2). Hazards: spec citations name host code. None.

### 4.15 `src/codegen/link/thunk/link_thunk.rs` inline (551, 7 tests) → `link_thunk/tests/mod.rs`

`mod tests` (2972-3654 at `ce7a995ca`) → `#[cfg(test)] mod tests;`. `link_thunk/tests/mod.rs`
(548; span 2973-3653): the 4 crate `use` items, shared `TEST_THUNK_CONTEXT`, helpers
`assert_branch_labels_resolve`, `one_vendored_library`, `call_targets`, and all 7 tests
(`struct_out_cdouble_field_emits_float_fail_labels`, `out_slot_cdouble_emits_float_fail_labels`,
`only_the_builtin_file_resource_uses_io_buffers`, `seven_integer_slots_are_rejected_on_x86_and_accepted_on_aarch64`,
`every_known_ctype_lowers`, `native_call_result_is_captured_from_c_return_register`,
`windows_link_initializer_uses_win32_loader_not_dlopen`).
Hazards: 19 citations to host code; `tests/guards/no_type_strings.rs` names the host and
`.ai/resources-packages.md` names `every_known_ctype_lowers` by fn name — unchanged.

### 4.16 `src/codegen/runtime/canvas/vulkan.rs` inline (748, 8 tests) → `vulkan/tests/mod.rs`

Only `mod tests` (9073-9887 at `ce7a995ca`) moves; the host keeps `#[cfg(test)] mod tests;` at
9072. Two cfg(test) items STAY in `vulkan.rs` (parent decision, 2026-10-14): `#[cfg(test)] const SPIRV_MAGIC`
(256-257) and `#[cfg(test)] fn assert_layout` (from 9042). `assert_layout` is not used only by
this module: `vulkan_present.rs`'s own test module calls it 10 times through `use super::*`, so
moving it would break that compile. `vulkan/tests/mod.rs` reaches it through `use super::*`.
Host test LOC after: 2 + 29 + 2 = 33. When plan-176-V turns `vulkan.rs` into `vulkan/mod.rs`,
the declaration resolves to the same `vulkan/tests/`, and V checks
`grep -c 'fn assert_layout' src/codegen/runtime/canvas/vulkan.rs` → 1 before its own move.
`vulkan/tests/mod.rs` (714; span 9074-9886): `use super::*;`, the 2
`crate::codegen::runtime::canvas::{…}` imports and all 8 tests — `layout_tables_are_self_consistent`
(485 LOC of hand-transcribed Vulkan C layouts, calls the host's `assert_layout` unchanged),
`the_readback_buffer_prefers_cached_memory`, `the_embedded_spirv_is_well_formed`,
`each_backend_has_one_pipeline_slot_per_blend_mode`, `the_pointer_words_end_the_graphics_state`,
`the_item_block_matches_the_std430_stride`, `the_pipeline_specializes_every_region_base`,
`the_shared_buffer_holds_every_region`. One `#[test]` of 485 LOC in a 714-LOC test-only file is
within the limit, so it is neither split nor given its own file.
Hazards: `include_str!("shaders/mfb_canvas.frag")` (line 9763 at `ce7a995ca`) →
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
  with the phase (est. 2 min).
- **G5:** `cargo fmt --check` → no output (est. 20 s).

### Phase 1 — `src/arch` encoder and selector tests

- [ ] Split `src/arch/aarch64/encode/tests.rs` per §4.1; repoint the two cites in
      `src/docs/spec/architecture/14_aarch64-instruction-set.md` to `tests/forms.rs`.
- [ ] Split `src/arch/riscv64/encode/tests.rs` per §4.2.
- [ ] Split `src/arch/x86_64/encode/tests.rs` per §4.3; repoint the two cites in
      `src/docs/spec/architecture/15_x86_64-instruction-set.md` (`:1` → `tests/mod.rs:1`,
      `unsupported_op_errors` → `tests/image_and_coverage.rs`).
- [ ] Move the inline tests of `src/arch/riscv64/select.rs` (§4.4), `src/arch/riscv64/v128.rs`
      (§4.5) and `src/arch/x86_64/select.rs` (§4.6) to their `tests/mod.rs`.
- [ ] G4: tighten the ceilings for the six files (three `ceiling test` rows and three
      `ceiling inline` rows go; `v128.rs`'s `ceiling code` row stays for plan-176-O).

Acceptance: every arch test runs from its new module, the guards ignore the register-spelling
fixtures through the `/tests/` rule, and the four spec cites resolve.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb arch::` → 0 failed, and
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^arch::(aarch64::encode|riscv64::(encode|select|v128)|x86_64::(encode|select))::tests::'`
  → 41+41+130+39+36+13 = 300; `grep -rn "encode/tests.rs" src/docs/spec` → no output;
  `find src/arch/aarch64/encode/tests src/arch/riscv64/encode/tests src/arch/x86_64/encode/tests src/arch/riscv64/select src/arch/riscv64/v128 src/arch/x86_64/select -name '*.rs' | xargs -n1 python3 -I tools/loc-census/census.py --file`
  → every second field ≤ 910 (v128's single `tests/mod.rs`; every other ≤ 900); G3; G4; G5
  (est. 12 min).
Commit: —

### Phase 2 — `src/os` linker and object tests

- [ ] Split `src/os/linux/link/tests.rs` per §4.7; repoint the `writes_init_array_glibc_elf`
      cites in `src/docs/spec/linker/04_symbols-and-relocations.md` and `07_linux-aarch64.md` to
      `tests/elf_writing.rs`.
- [ ] Split `src/os/macos/link/tests.rs` per §4.8.
- [ ] Move the inline tests of `src/os/windows/link/mod.rs` (§4.9), `src/os/linux/appimage/mod.rs`
      (§4.10; rewrite the `scripts/coverage-exceptions.txt:47` prose as stated there),
      `src/os/linux/object.rs` (§4.11) and `src/os/macos/object.rs` (§4.12) to their `tests/mod.rs`.
- [ ] G4: tighten the ceilings for the six files (two `ceiling test` and four `ceiling inline`
      rows go).

Acceptance: every os test runs from its new module, including the macOS-only launch tests on
this host.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb os::` → 0 failed, and
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^os::(linux::(link::tests|appimage::tests|object::tests)|macos::(link::tests|object::tests)|windows::link::tests)::'`
  → 51+19+16+45+17+23 = 171; `grep -rn "linux/link/tests.rs" src/docs/spec` → no output;
  `grep -c "463-524" scripts/coverage-exceptions.txt` → 0; G3; G4; G5 (est. 12 min).
Commit: —

### Phase 3 — Collection, link and canvas hosts (largest blast radius)

- [ ] Move `src/codegen/collection/layout/builder_collection_layout.rs`'s three test modules into
      `builder_collection_layout/tests/mod.rs` per §4.14.
- [ ] Move `src/codegen/link/thunk/link_thunk.rs`'s tests per §4.15.
- [ ] Move `src/codegen/runtime/canvas/vulkan.rs`'s `mod tests` per §4.16 (`include_str!`
      repoint; `assert_layout` and `SPIRV_MAGIC` stay in `vulkan.rs`).
- [ ] Move `src/codegen/collection/assign/self_update.rs`'s census and tests per §4.13
      (`include_str!` repoint, the four prose updates and the `.ai/collections.md` bullet).
- [ ] G4: tighten (four `ceiling inline` rows go; the four hosts' `ceiling code` rows stay for
      Q and V).

Acceptance: the census, the three layout modules, the thunk and canvas suites run from `tests/`
modules with every count unchanged, and no file J created is over 900.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb codegen::collection:: codegen::link:: codegen::runtime::canvas::vulkan::`
  → 0 failed, and `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^codegen::(collection::(assign::self_update|layout::builder_collection_layout)|link::thunk::link_thunk|runtime::canvas::vulkan)::tests::'`
  → 9+19+7+8 = 43; `grep -c 'fn assert_layout' src/codegen/runtime/canvas/vulkan.rs` → 1
  (and `cargo test -p mfb --bin mfb canvas::vulkan::present::` → 0 failed: `vulkan_present.rs`'s
  tests, which call it through `use super::*`); `find src/codegen/collection/assign/self_update src/codegen/collection/layout/builder_collection_layout src/codegen/link/thunk/link_thunk src/codegen/runtime/canvas/vulkan -name '*.rs' | xargs -n1 python3 -I tools/loc-census/census.py --file`
  → every second field ≤ 900 (largest `self_update/tests/census/self_update_table.rs`, 882);
  `cargo test --test inplace_self_update_census` → pass (est. 1 min); G3; G4; G5 (est. 12 min).
Commit: —

## Validation Plan

- Tests: no test is added or removed. Per phase the whole-binary `--list` count is unchanged
  (G1) and the moved modules' tests pass (G2); the per-phase greps sum to
  300 + 171 + 43 = 514, every test in J's 16 files.
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
  (`an_arm_with_no_field_reach_is_deferred_at_every_field_site`,
  `the_pipeline_specializes_every_region_base`) — a wrong path is a compile error.
- Doc sync: `src/docs/spec/architecture/14_aarch64-instruction-set.md`,
  `src/docs/spec/architecture/15_x86_64-instruction-set.md`,
  `src/docs/spec/linker/04_symbols-and-relocations.md`, `src/docs/spec/linker/07_linux-aarch64.md`,
  `.ai/collections.md`, `scripts/coverage-exceptions.txt` (1 entry), and the prose in
  `tests/guards/inplace_self_update_census.rs`, the `rt_inplace_self_update` tree,
  `tests/runtime/inplace_self_update/field_kinds_gen.py`. No compiler contract changes.
- Final gate (once, after Phase 3): `cargo test -p mfb --bin mfb spec_citations_resolve`,
  `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings --test file_size_budget --test inplace_self_update_census`
  → all pass and the banner has 0 rows; `cargo clippy --all-targets 2>&1 | grep -c '^warning'`
  → equal to its value at G0 of Phase 1 (est. 6 min). The full workspace suite runs once, in
  plan-176-V.

## Decisions

- Rule revised — owner, 2026-10-18. Three kinds, three limits, all non-comment, non-blank LOC
  (counting rules unchanged from plan-176-A §2); scope `.rs`/`.mfb` under `src/` `packages/`
  `examples/` `repository/` `tests/`:

  | Kind | Applies to | Limit | Over the limit → |
  |---|---|---|---|
  | `code` | code LOC of a file that is not a test-only file | 1000 | split the file into modules |
  | `inline` | inline test LOC (`#[cfg(test)]` items / MFBASIC `TESTING … END TESTING`) inside a file that is not a test-only file | 250 | move the tests out into their own test-only file(s) |
  | `test` | total LOC of a test-only file (path component `tests`, `test_*.mfb`, `#![cfg(test)]`, or a `#[cfg(test)] mod x;` target and its subtree) | 1000 | split the test-only file into modules |

  A moved inline block becomes one `tests/mod.rs` unless it is over 1000; parts target ≤ 900.
  Ceiling rows are `ceiling <code|inline|test> <n> <path>`; banner rows name the kind.
- No exemptions — owner, 2026-10-18. Generated files (`GENERATED` banner) are skipped; that is
  the only exemption. The guard warns with a banner and never fails. A new module is always an
  acceptable split — owner, 2026-10-14.
- Extracted src test modules go into `foo/tests/mod.rs` (+ parts only when over 1000) —
  plan-176-A. This includes `builder_collection_layout`'s three modules (one `tests/mod.rs`,
  module names kept as inline modules) — author, 2026-10-18.
- Parts are the fewest that keep each ≤ 900, merging neighbouring map topics — author,
  2026-10-18 (§3).
- The architecture guards' `/tests/` rule is plan-176-H's; J depends on it and does not edit
  any guard's logic — author, 2026-10-14.
- Pre-existing defects found while mapping are fixed in the letter that touches them — author,
  2026-10-14. J fixes the stale `scripts/coverage-exceptions.txt` prose of the appimage entry
  (§4.10).
- Glob-cycle imports compile without warnings — verified (`/tmp/loc/probe2`, `/tmp/hij_probe`).
- Per-file coverage is not a plan-176 gate; the measured evidence is in §Validation Plan
  "Coverage check" — author, 2026-10-17.

## Corrections

- 2026-10-18: Rule revised by the owner: test-only files limit 1000 (was 250), inline tests 250,
  no exemptions. Dropped 19 files (each within the new limits, `python3 -I /tmp/loc/census.py "$PWD"`
  at `ce7a995ca`): `src/arch/aarch64/decode/tests.rs` (742), the eight
  `src/binary_repr/tests/{cross_package_tests,fixtures,gap_tests,reader_gap_tests,reader_tests,sections_tests,writer_tests,writer_walker_tests}.rs`,
  `src/codegen/io/input/{corpus,encode_tests,model,reader_tests,tests}.rs`,
  `src/codegen/resource/tests/mod.rs`, `src/codegen/string/format/{float_format_sci_ref,float_parse_ref}.rs`,
  `src/os/linux/appimage/squashfs/tests.rs`, `src/os/windows/link/spike.rs`. Old exemptions:
  aarch64 decode `table()` and `src/codegen/io/input/corpus.rs` (903) — their files are ≤ 1000,
  not split, no row; `SELF_UPDATE_TABLE` — its own test-only part
  `self_update/tests/census/self_update_table.rs` at 882 (≤ 900), no row;
  `layout_tables_are_self_consistent` — stays in the one 714-LOC `vulkan/tests/mod.rs`, no row.
  Structural: for the 16 kept files, 115 planned files → 30 (5 test-only splits: 2+2+4+2+2
  parts plus a `mod.rs` each; 10 inline moves: one `tests/mod.rs` each; `self_update`:
  `tests/mod.rs` + `tests/census/mod.rs` + `tests/census/self_update_table.rs`), phases 5 → 3, tests 711 → 514, visibility edits 19 → 0,
  `include_str!` repoints 5 → 2. Fixed: `lowers_in` and `code` are `impl Site` methods, not
  `impl ArmId` (`grep -n "fn lowers_in\|fn code" src/codegen/collection/assign/self_update.rs`).

## Summary

J is pure motion of backend and codegen test code into `tests/` directory modules: 16 hosts,
514 tests, 0 renamed, 0 exempt. Five test-only files over 1000 become 12 topic parts (each
≤ 900) behind a `mod.rs`; ten inline blocks each become one `tests/mod.rs`; `self_update.rs`'s
2087-LOC census becomes three files, the 882-LOC `SELF_UPDATE_TABLE` alone in one. The risk is
in `self_update.rs` (a nested census module, an `include_str!` two levels deeper, four prose
references in other trees; last phase) and in the 130-test x86 encoder suite whose header the
spec cites by line. Host code and every guard's logic are untouched.
