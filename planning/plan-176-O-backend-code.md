# plan-176-O: Backend code splits

Last updated: 2026-10-18
Effort: large (3h–1d)
Depends on: plan-176-N

O splits the twelve over-limit code files of the backend into module directories:
the aarch64 decoder and encoder, the riscv64 `v128` lowering, the x86-64 encoder, the `.mfp`
codec (`binary_repr` reader/sections/writer), the owned-list cleanup builder, the `--debug` arena
report, the native link thunk, the shared socket lowering, and the Linux ELF writer. Every split
is pure code motion; the module paths callers use stay the same through re-exports.

O is also the first code letter to split a file whose NAME is a scan key
(`builder_owned_cleanup.rs`). Two scans select files by a `builder_`/`func_`/`gen_` file-name
prefix, so the code in a split's part files would silently leave them. Phase 1 widens both scans
to also take a file under a directory with that prefix. Letters P–V rely on this fix.

The single outcome: after O lands, `cargo test --test file_size_budget` shows no row for any of
the twelve files or the files they become, and `tests/guards/file_size_ceilings.txt` has no
`ceiling` row for them. The rule (owner, 2026-10-18): code ≤ 1000, inline test ≤ 250, test-only
file ≤ 1000 non-comment non-blank LOC; there are no exemptions besides `GENERATED` files.

References:

- `planning/plan-176-A-file-size-guard.md` — rule, guard, ratchet, §Decisions, and the split
  conventions in `.ai/build-tooling.md` "Splitting an over-budget file".
- `.ai/arch-abi.md` (per-architecture encoder/ABI traps), `.ai/codegen-invariants.md` (line 98
  names the owned-cleanup drop), `.ai/resources-packages.md` (lines 58, 211, 271, 553),
  `.ai/testing-gates.md` § "Splits must sweep man AND spec citations" and line 774,
  `.ai/build-tooling.md`.
- Spec topics whose citations move (`grep -rlE "\[\[src/(arch/aarch64/encode/emitter|arch/riscv64/v128|arch/x86_64/encode/emitter|binary_repr/(reader|sections|writer)|codegen/cleanup/owned/builder_owned_cleanup|codegen/debug/arena|codegen/link/thunk/link_thunk|os/linux/link/elf)\.rs" src/docs/spec`
  → 36 files): `architecture/{05_binary-representation,06_native,14_aarch64-instruction-set,15_x86_64-instruction-set,16_riscv64-instruction-set,23_escape-analysis}.md`,
  `language/{02_lexical-structure,04_types,13_modules-and-packages,15_resource-management,17_native-libraries}.md`,
  `linker/{03_import-selection,04_symbols-and-relocations,05_static-and-dynamic-output,07_linux-aarch64,08_linux-x86_64,09_linux-riscv64,10_windows-x86_64,11_package-linking,13_provenance-marker}.md`,
  `memory/{02_fallible-call-abi,04_arenas}.md`,
  `package/{01_container-format,02_binary-representation,03_metadata-encoding,04_type-table,05_constant-pool,06_globals,07_functions,08_ir-section,09_resource-regions,10_native-bindings,12_verifier-rules,14_compact-summary,15_package-meta-section}.md`,
  `tooling/09_debug-report.md`.
- Split map: `/tmp/loc/maps/O.md` (working tree of `17a9b5799`), restated below in decided form.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-N is archived | `ls planning/plan-176-N-* planning/completed/plan-176-N-*` → only the `planning/completed/` path | MET (2026-10-19) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | MET (2026-10-19) |
| O's files have no uncommitted changes | `git status --porcelain -- src/arch/aarch64/decode src/arch/aarch64/encode src/arch/riscv64/v128.rs src/arch/x86_64/encode src/binary_repr src/codegen/cleanup/owned src/codegen/debug src/codegen/link/thunk src/codegen/os/socket src/os/linux/link src/codegen/engine/tests tests/guards .ai src/docs/spec` → empty | MET (2026-10-19: 0 lines) |

Everything below is written against the world where these hold. plan-176-J (an earlier letter)
has already moved the two inline test blocks this letter's files carried over the 250 inline
limit: `src/arch/riscv64/v128.rs`'s `mod tests` (913) → the test-only module
`src/arch/riscv64/v128/tests/mod.rs`, and `src/codegen/link/thunk/link_thunk.rs`'s `mod tests`
(551) → `src/codegen/link/thunk/link_thunk/tests/mod.rs` (each ≤ 1000, so one file). J replaced
only the trailing test blocks, so every non-test line range below is unchanged by J. The
test-only file `src/arch/aarch64/decode/tests.rs` (742 LOC) is under the 1000 test-only limit and
no letter touches it: its `the_table_covers_every_form_the_encoder_emits` still reads
`include_str!("../encode/emitter.rs")`. `tests/guards/architecture_guards.rs` (370) and
`src/codegen/engine/tests/tests.rs` (404) are likewise under 1000 and unsplit; Phase 1 still
locates its two scans by fn name (`python3 -I /tmp/loc/census.py "$PWD"` at ce7a995ca for the
counts).

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

- Each of the twelve files becomes `foo/mod.rs` + parts, every resulting file ≤ 1000 code LOC
  and ≤ 250 inline test LOC.
- The two prefix-keyed scans take a file under a `builder_*`/`func_*`/`gen_*` directory.
- `cargo test --test file_size_budget 2>&1 | grep -E '<the twelve paths>'` → no row, and the
  ceiling file has no `ceiling` row for them.
- `sh scripts/artifact-gate.sh target/release/mfb all` → 0 diffs; the `.mfp`/`.info` goldens,
  the Linux executables and the `--debug` native dumps are byte-identical before/after.

### Non-goals (explicit constraints)

- No behavior change: no `.ncode`/`.nobj`/`.mfp`/executable byte changes, no diagnostic change.
- No logic edit beyond: visibility widenings, `use`/`mod`/re-export lines, the one
  `include_str!` path in the decoder test, comment/doc path fixes, and the Phase 1 scan
  widening (test/guard code only).
- No crate path a caller uses changes (`encode::emitter::Encoder`, `super::v128::*`,
  `binary_repr::*` globs, `thunk::*`, `socket::shared::*`, `link::elf::*`).
- No `[[test]]` target renamed; no test fn renamed. Inline test code that stays inline
  (x86 emitter 4 test LOC, owned cleanup 78) stays in the new `mod.rs`.

## 2. Current State

| File | Kind | Code LOC | Test LOC | Inline `#[test]` |
|---|---|---|---|---|
| `src/arch/aarch64/decode/vector.rs` | code | 1202 | 0 | 0 |
| `src/arch/aarch64/encode/emitter.rs` | code | 1133 | 0 | 0 |
| `src/arch/riscv64/v128.rs` | code | 1155 | 913 at planning; 0 after J | 36 at planning; 0 after J |
| `src/arch/x86_64/encode/emitter.rs` | code | 1837 | 4 | 0 |
| `src/binary_repr/reader.rs` | code | 1180 | 0 | 0 |
| `src/binary_repr/sections.rs` | code | 1161 | 0 | 0 |
| `src/binary_repr/writer.rs` | code | 1111 | 0 | 0 |
| `src/codegen/cleanup/owned/builder_owned_cleanup.rs` | code | 1106 | 78 | 6 |
| `src/codegen/debug/arena.rs` | code | 1097 | 0 | 0 |
| `src/codegen/link/thunk/link_thunk.rs` | code | 2140 | 551 at planning; 0 after J | 7 at planning; 0 after J |
| `src/codegen/os/socket/shared.rs` | code | 1199 | 0 | 0 |
| `src/os/linux/link/elf.rs` | code | 1037 | 0 | 0 |

LOC: `python3 -I /tmp/loc/census.py` on `17a9b5799` (values from `/tmp/loc/census2.json`);
after plan-176-A, `python3 -I tools/loc-census/census.py --file <path>`. Test fns:
`grep -cE '#\[(tokio::)?test\]' <file>`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Spec citations, per file | vector 0, aarch64 emitter 12, v128 5, x86 emitter 15, reader 44, sections 18, writer 13, owned cleanup 17, debug arena 3, link_thunk 19, socket shared 0, elf 31 | `grep -rnoE "\[\[<path>(:[^]]+)?\]\]" src/docs/spec \| wc -l` per path |
| `mfb man` citations, non-citation `src/docs` mentions | 0, 0 | same pattern over `src/docs/man`; `grep -rnE "(^\|[^a-z_/])(src/)?<path>" src/docs \| grep -v '\[\[<path>'` |
| Guard rows naming these paths | `no_type_strings.rs`: 3 `BOUNDARY_FILES` + 1 `TYPE_KEYED_TABLES`, and 3 closed-list entries in `tests/guards/no_type_strings/self_checks.rs` after plan-176-E; `no_operator_strings.rs`: 2 | `grep -n '"src/binary_repr/\|"src/codegen/link/thunk\|"src/arch/riscv64/v128' tests/guards/*.rs` |
| Source-reading tests naming these files | 1 (`include_str!("../encode/emitter.rs")` in the aarch64 decode tests) | `grep -rn "include_str" src \| grep emitter` |
| Files the engine overflow-label scan reads today | 747, holding 42 overflow labels | `find src/codegen -type f -name '*.rs' \( -name 'builder_*' -o -name 'func_*' -o -name 'gen_*' \) \| wc -l`; `… \| xargs grep -h 'abi::label(&' \| grep -c overflow` |
| `.rs` files under a prefixed directory today (missed by both scans) | 7 (`src/codegen/builtins/tls/gen_macos/*` ×6, `src/codegen/error/emission/builder_error_emission/tests/mod.rs`), 0 overflow labels, 0 `_native(` | `find src/codegen -type f -name '*.rs' -path '*/builder_*/*' -o -type f -name '*.rs' -path '*/func_*/*' -o -type f -name '*.rs' -path '*/gen_*/*' \| wc -l`; per-file `grep -c` |
| Host-`cfg` gating in these files | 0 | `grep -cE "#\[cfg\((target_os\|windows\|unix\|target_family)" <file>` → 0 each |
| Coverage exceptions for these files | 0 | `grep -nE "vector\|emitter\|v128\|binary_repr\|owned_cleanup\|debug/arena\|link_thunk\|socket/shared\|linux/link/elf" scripts/coverage-exceptions.txt` → only `src/binary_repr/mod.rs` (untouched) |

### Verified properties

- **A parent `use` consumed only by a child through `use super::*;` does not warn**
  (`/tmp/locN-probe`: no warning, printed `3`); **glob-cycle imports compile without warnings**
  (`/tmp/loc/probe2`: printed `6`, no warning).
- **The artifact gate covers encoders but not linking, packaging or `--debug`.** It compares
  `ast ir hex` and `nir nplan nobj ncode mir` (`scripts/artifact-kinds.sh`), never links, and
  builds without `--debug`. Every byte-identity selector carries 5 targets except
  `link-const-pins` (4, no windows) (`find tests/byte-identity -name "*.ncodesum"`).
- **Linux executables are deterministic**, so a before/after `cmp` is a valid gate for `elf.rs`:
  `mfb build -q --target linux-x86_64` of a copy of `tests/byte-identity/general` twice →
  `cmp` identical for both `-glibc.out` and `-musl.out`; `mfb build -q --debug --target linux-aarch64`
  twice → identical (`/tmp/elfprobe`, 2026-10-14).
- **`mfb build --debug --ncode` emits the arena report code** (`_mfb_debug_arena_register`,
  `_mfb_debug_arena_sample`, `_mfb_debug_report_arena`, …: `grep -o "_mfb_debug[a-z_]*"` on the
  dump, `/tmp/elfprobe`), so a before/after `--debug --ncode` diff gates `debug/arena.rs`.
- **The `.mfp` codec is checked by `test-accept.sh`**, which compares `.mfp`/`.info` goldens for
  18 fixtures (`find tests -path "*golden/*.mfp" -o -path "*golden/*.info" | sed 's|/golden/.*||' | sort -u`):
  every `tests/syntax/packages/package-*`, `tests/syntax/native/native-library-*`,
  `tests/syntax/native/native-resource-state-export-valid`,
  `tests/syntax/resources/native-resource-link-valid`, `tests/syntax/astrings/roundtrip-package`.
- **The aarch64 decoder is reached only by the debugger** (`grep -rln "aarch64::decode" src` →
  `src/debugger/disassemble.rs` and a spec topic); its gate is its unit tests plus the debugger's.
- **The `emit_instruction` dispatch the decoder test scans stays in `emitter/mod.rs`**:
  `match instruction.op.mnemonic()` is old line 37 and its `other => Err(` old 446, both inside
  `emit_instruction` (35-450).
- **No cfg-gated host code** (table above), so no Windows-target check and no box-2223 check is
  needed for O.
- **No `module_path!`/`file!`/`line!` and no link-order registration** (plan-176-A), so motion
  cannot change output.

## 3. Design Overview

Conventions are plan-176-A's (`foo.rs` → `foo/mod.rs` + parts; `use super::*;` first in each
part; `pub(super)` for a part item used by `mod.rs` or a sibling; `pub(in <grandparent>)` where
the old item reached the grandparent; a private `use <part>::*;` or explicit `use` in `mod.rs`
for free items siblings call; impls split into several `impl X { … }` blocks). Line ranges are
as of 17a9b5799; item names are authoritative if lines drift.

**Gate class: byte-identity.** Per phase: `cargo build --release`, the scoped artifact gate for
the selectors whose code moved, the moved code's unit tests, the before/after differentials where
§2 shows the artifact gate is blind (Linux executables, `--debug` dumps, `.mfp`/`.info`),
`cargo check --all-targets`, `spec_citations_resolve`, the guards, the tighten step. Letter end,
once: `sh scripts/artifact-gate.sh target/release/mfb all`. A diff = diff ONE fixture's dump,
find the moved item that changed, fix, re-run; never a premise stop.

Before/after differential: before each phase, `cp target/release/mfb /tmp/mfb-before-O`; the
phase's check builds the same input with both binaries and `cmp`s.

Risk: Phase 1 changes test and guard code (scan widening) — it must widen, never narrow, each
scan. Among the splits, `link_thunk.rs` (2140 LOC, 12 visibility edits, 19 citations, the FFI
marshalling of every `LINK` call) lands last. `lower_link_thunk` is one 921-LOC function; its
file `lower_thunk.rs` is 922 LOC, under the 1000 code limit (no guard row).

Rejected alternatives:

- *Decomposing `lower_link_thunk`* — it is under the 1000 limit; extract-method is reserved for
  functions over the limit (letters S–V, owner decision). Its 103 top-level statements share
  locals, so no pure-motion cut exists.
- *Renaming the split dir to drop the prefix* (e.g. `owned_cleanup/`) — changes a module path
  callers use (`…::builder_owned_cleanup::{…}` in `function_lowering.rs`, `builder_control.rs`,
  `ir/resource_escape.rs`).
- *Listing split part files in the scans by name* — a list rots at every later split; the
  ancestor-directory rule covers P–V without edits.

## 4. Detailed Design

### 4.0 Prefix-keyed scans take split directories

Two scans select files by their own name:

1. `no_overflow_label_returns_through_the_result_tag_register` (at 17a9b5799 in
   `src/codegen/engine/tests/tests.rs`, fn `collect_builder_sources`: `n.starts_with("builder_")
   || n.starts_with("func_") || n.starts_with("gen_")`). It asserts no overflow label returns
   through the result-tag register. At letter U's split, 10 of its 42 overflow labels would leave
   the scan.
2. `no_cross_member_os_seam_dispatcher` (at 17a9b5799 `tests/guards/architecture_guards.rs:339`,
   `name.starts_with("func_")` gating the `*_native(` shell check).

Change: each scan gets a predicate taking the file path and the scan root. A file qualifies when
its own name has the prefix (as today) OR any directory component between the scan root and the
file has the prefix. So `builder_x.rs`, `builder_x/mod.rs` and `builder_x/part.rs` all qualify;
`other/part.rs` does not.

- Engine test: `fn is_lowering_source(root: &Path, path: &Path) -> bool` beside
  `collect_builder_sources`; `collect_builder_sources` keeps the root and calls it for every
  `.rs` file. Prefixes `builder_`, `func_`, `gen_`; a directory qualifies by the same prefixes.
- Architecture guard: `fn func_named(root: &Path, path: &Path) -> bool` (prefix `func_`, own
  name or an ancestor directory under `src/codegen/builtins`) replaces `name.starts_with("func_")`.
- Each gets a `#[test]` beside it (negative case for the old rule): the predicate returns `true`
  for `<root>/a/builder_x/part.rs` and `<root>/a/builder_x/mod.rs` (resp. `func_x/part.rs`),
  `true` for `<root>/a/builder_x.rs`, and `false` for `<root>/a/other/part.rs` and for a
  prefixed directory ABOVE the root. The old `file_name().starts_with` rule returns `false` for
  `part.rs` — the case this test pins.

Effect today: the engine scan grows from 747 files by the 7 under prefixed directories (§2),
which hold 0 overflow labels, so 0 new offenders; the guard scan gains the
`tls/gen_macos/*` files only for its `_os_seam` arm (unchanged — that arm reads every file), and
no `func_*` directory exists yet, so 0 new offenders.

Justification (AGENTS.md test rule):
(1) the engine scan was written in `054a5d905` (bug-352, "size-overflow guards report the OOM
code, not a computed size"); the guard check in `5224f5263` ("add
no_cross_member_os_seam_dispatcher architecture guard").
(2) Protects: every overflow label in builder/func/gen lowering code returns the OOM code, not
the computed size in x0; every `func_*` member file carries its own lowering instead of a
`*_native(` shell over a shared dispatcher.
(3) Dependents: none outside the two tests (`grep -rn "collect_builder_sources\|no_cross_member_os_seam_dispatcher" src tests`
→ only their own definitions).
(4) Proof: both tests' doc comments state the intent as "every current and future `builder_*.rs`
legacy carrier plus the clean-room `func_*.rs`/`gen_*.rs` member lowerings" and "each
`abi_function` member … in its `func_*.rs`". A split moves the same code to
`builder_x/part.rs`; the file-name rule drops it from the scan with no failure. The widened rule
scans a superset, so nothing that passes today can be hidden.

Letters P–V split `builder_*`/`func_*`/`gen_*` files and rely on this widening.

### 4.1 `src/arch/aarch64/decode/vector.rs` (1202)

- `decode/vector/mod.rs` (853 + 2 lines): 1-936 — imports, `decode`, the arrangement/element
  helpers, crypto (`aes`, `sha_*`), `table`, `permute`, `extract`, `copy`, the
  three-same/two-misc/three-different families, `Misc`, `across`, modified/shift immediates,
  `by_element`, `float_by_element`, `indexed`. Adds `mod scalar; use scalar::scalar;`.
- `decode/vector/scalar.rs` (349): 937-1302 — `scalar`, `scalar_copy`, `scalar_three`,
  `scalar_three_same`, `scalar_float_three_same`, `scalar_half_three_same`,
  `scalar_three_different`, `scalar_two_misc`, `scalar_pairwise`, `scalar_shift`, `scalar_indexed`.
- Visibility: `scalar` → `pub(super)`. scalar.rs uses parent-private `by_element`, `element`,
  `float_by_element`, `scalar_register`, `shift_size` (visible). `decode` (called by
  `decode/mod.rs:187` as `vector::decode`) stays. The `use super::{bit, field, …}` /
  `use super::memory::…` lines stay in `vector/mod.rs` (`super` is still `decode`); no `super::`
  in 937-1302.
- Hazards: 0 citations, 0 path refs, no inline tests.

### 4.2 `src/arch/aarch64/encode/emitter.rs` (1133)

The 1058-LOC `impl Encoder` (35-1302) splits into 4 `impl Encoder` blocks.

- `emitter/mod.rs` (526 + 3 `mod` lines): 1-450 (imports, `STLR_X`/`LDAR_*`, `Encoder`,
  `LabelPatch`, `impl Encoder {` + `emit_instruction`, the dispatch) + 1258-1364
  (`patch_labels`, the impl's `}`, `impl InstructionEncoder for Encoder`).
- `emitter/vector.rs` (126 + 3): 451-615 — `emit_ldr_q`, `emit_str_q`, `emit_v_three_same`,
  `emit_v_two_misc`, `emit_v_shift_imm`, `emit_dup_v_from_x`, `emit_fma_d`, `emit_umov_x_from_v`.
- `emitter/scalar.rs` (319 + 3): 616-1065 — `emit_word`, `emit_mov`, `emit_mov_imm`, `emit_rrr`,
  `emit_rr`, the integer ALU/shift/div/imm/sp/cmp emitters, `emit_fmov_*`, `emit_f*_d`,
  `emit_fcmp*`, `emit_scvtf_d_from_x`, `emit_fcvt*_x_from_d`.
- `emitter/memory.rs` (162 + 3): 1066-1257 — `emit_ldr_*`/`emit_str_*` (u64/u32/u16/u8/d),
  `emit_label_branch`, `emit_bl`, `emit_blr`, `emit_symbol_ref`.
- Visibility: `Encoder`, `LabelPatch`, `emit_instruction`, `patch_labels` stay
  (`encode/mod.rs:15 use emitter::Encoder`; `sizing.rs:13 Encoder::new` is the trait impl). 80
  methods become `pub(super)`: vector 8 (all), scalar 58 (every method in 616-1065 except
  `emit_rrr`, `emit_add_imm_chunk`, `emit_sub_imm_chunk`, which stay private), memory 14 (all).
  No `super::` in 451-1257.
- Hazards: 12 citations, all rewritten: `emit_instruction` ×3, `patch_labels` → `emitter/mod.rs`;
  `emit_v_three_same` → `emitter/vector.rs`; `emit_add_imm`, `emit_cmp_imm`, `emit_mov_imm`,
  `emit_scvtf_d_from_x` → `emitter/scalar.rs`; `emit_ldr_u64`, `emit_bl`, `emit_symbol_ref` →
  `emitter/memory.rs`. Doc links `super::emitter::Encoder::patch_labels` (`relax.rs:6,38`) and
  `super::emitter::Encoder::emit_instruction` (`sizing.rs:4`) still resolve.
- Source-reading test (must change): the decoder test
  `the_table_covers_every_form_the_encoder_emits` (in the test-only file
  `src/arch/aarch64/decode/tests.rs`) reads `include_str!("../encode/emitter.rs")` → `include_str!("../encode/emitter/mod.rs")`.
  Justification: (1) written in `1b6a632e6` ("arch/aarch64: decode, the encoder's counterpart");
  (2) protects: every mnemonic `emit_instruction` dispatches on has a decoder table row;
  (3) dependents: none (`grep -rn "encode/emitter" src tests` → this line); (4) proof:
  `match instruction.op.mnemonic()` … `other => Err(` (old 37…446) is inside `emit_instruction`,
  which stays verbatim in `emitter/mod.rs` (§2); the test reads the same text at a new path.

### 4.3 `src/arch/riscv64/v128.rs` (1155)

- `v128/mod.rs` (226 + mod/use lines): 1-329 (docs, imports, `SLOT_COUNT`, scratch-register
  consts, `ci`, `is_v128`, `is_vector_operand`, `build_slot_map`, `build_vreg_map`,
  `v128_live_ranges`, `f`) + J's `#[cfg(test)] mod tests;` and the doc comment above it.
- `v128/scalarize.rs` (392): 330-816 — `scalarize_v128`.
- `v128/rvv.rs` (537): 817-1436 — `load_has_rvv_flag`, `rvv_arm`, `lanes_from_mask`,
  `rvv_prologue`, `rvv_lowerable`, `drop_redundant_reloads`, `lower_v128_run`.
- Visibility: outside users `super::v128::{SLOT_COUNT, build_slot_map, build_vreg_map, is_v128,
  rvv_lowerable, lower_v128_run, scalarize_v128}` (`src/arch/riscv64/select.rs:474-530`).
  `mod.rs` adds `pub(crate) use rvv::{lower_v128_run, rvv_lowerable}; pub(crate) use scalarize::scalarize_v128;`
  (already `pub(crate)`; rvv.rs reaches `scalarize_v128` through that re-export). `rvv_arm` →
  `pub(super)`, plus `#[cfg(test)] use rvv::rvv_arm;` in `mod.rs` so the J-moved tests'
  `use super::*` sees it (they use `rvv_arm`, `lower_v128_run`, `rvv_lowerable`,
  `scalarize_v128`, `build_*`, `is_vector_operand`). Children use parent-private `ci`, `f`,
  `T0`…`ZERO`. Absolute `crate::arch::riscv64::regmodel::…` (old 350, 1302) unaffected.
- Guard (must change): `tests/guards/no_operator_strings.rs` `OPERATOR_SHAPED_NON_OPERATORS` row
  `"src/arch/riscv64/v128.rs"` and its entry in `operator_shaped_exemptions_are_closed` →
  `"src/arch/riscv64/v128/scalarize.rs"`, same position, same reason.
  `every_exemption_is_still_load_bearing` fails for a listed file with no `op: &str` site; the
  only site, `fn high_lane_offset(offset: &str, op: &str)` (old 397,
  `grep -nE "(op|operator):\s*&" src/arch/riscv64/v128.rs`), lands in `scalarize.rs`.
  Justification: (1) written in `875189218` (plan-112 Phase 5, the no-operator-strings gate);
  (2) protects: only files whose `op` strings are not language operators are exempt;
  (3) dependents: the two lists and `every_exemption_is_still_load_bearing` /
  `exempt_files_are_still_scanned_for_decisions` in the same file; (4) proof: the one site
  moves verbatim; the exemption narrows from the whole old file to the part holding the site,
  so no production file becomes newly exempt.
- Citations 5: `SLOT_COUNT`, `build_slot_map`, `build_vreg_map` → `v128/mod.rs`;
  `scalarize_v128` → `v128/scalarize.rs`; `lower_v128_run` → `v128/rvv.rs`.
- `.ai/testing-gates.md:774` `riscv64/v128.rs`'s slot overlap check → `riscv64/v128/mod.rs`'s.
  `src/codegen/error/constants/error_constants.rs:952,959` name the module path
  `arch::riscv64::v128::SLOT_COUNT` — still valid.

### 4.4 `src/arch/x86_64/encode/emitter.rs` (1837 + 4 test)

- `emitter/mod.rs` (256 + mod/use lines + 4 test): 1-297 (imports, `Encoder`, `LabelPatch`,
  `SideEffect`, `Encoded` + impl incl. its `#[cfg(test)]` method, `impl Encoder`, `rex`, `modrm`,
  `sib`, `enc_push_reg`, `enc_pop_reg`, `alu_rr`, `mem_disp32`, `mem_rip`, `checked_disp32`,
  `checked_imm32`) + 2421-2482 (`impl InstructionEncoder for Encoder`).
- `emitter/instruction.rs` (802 + 1): 298-1371 — the section banner and `encode_instruction`.
- `emitter/integer.rs` (558): 1372-1448, 1739-2420 — `alu3`, `enc_neg`, `enc_mov`,
  `enc_mov_imm64`, `enc_imul_rr`, `enc_alu_imm32`, `enc_bt_imm0`, `MemWidth`, `mem_load`,
  `mem_store`, `JccKind`, `jmp_label`, `var_shift`, `var_shift_w`, `shift_imm`, `div_seq`,
  `enc_setcc_to`, `enc_add_carry`, `enc_sub_borrow`.
- `emitter/sse.rs` (225): 1449-1738 — `enc_sse_rr` … `sse_arith` (SSE/AVX encoders, `three_fp`,
  `vec3`, `vec3_op`, `vec_sse38_commutative`, `vec_cmppd_swapped`).
- Visibility: `encode_instruction` is used by `mod.rs`, `encode/sizing.rs:7` and the encode tests
  (`use super::emitter::{encode_instruction, Encoder}`) → widen `pub(super)` →
  `pub(in crate::arch::x86_64::encode)`, and `mod.rs` adds
  `pub(super) use instruction::encode_instruction;`. 37 items become `pub(super)` (all used only
  by `encode_instruction`): integer 16 (`alu3`, `enc_mov`, `enc_mov_imm64`, `enc_imul_rr`,
  `enc_alu_imm32`, `MemWidth`, `mem_load`, `mem_store`, `JccKind`, `jmp_label`, `var_shift`,
  `var_shift_w`, `shift_imm`, `div_seq`, `enc_add_carry`, `enc_sub_borrow`); sse 21
  (`enc_sse_rr`, `enc_movq_xmm_r64`, `enc_movq_r64_xmm`, `enc_roundsd`, `enc_shift_imm_reg`,
  `enc_sse_cvt`, `enc_psxlq`, `enc_movsd_mem`, `enc_movups_mem`, `enc_movaps`, `enc_sse38_rr`,
  `enc_cmppd`, `enc_pshufd`, `enc_vfma231pd`, `enc_vfma231sd`, `enc_roundpd`, `three_fp`,
  `vec3_op`, `vec_sse38_commutative`, `vec_cmppd_swapped`, `sse_arith`). `mod.rs` adds private
  `use integer::*; use sse::*;`. No `super::` in 298-2420.
- Citations 15: `rex` → `emitter/mod.rs`; `encode_instruction` ×9 → `emitter/instruction.rs`;
  `alu3` ×2, `enc_add_carry` → `emitter/integer.rs`; `enc_vfma231sd`, `sse_arith` →
  `emitter/sse.rs`. Doc links in `encode/mod.rs:12-33` and `sizing.rs:2-3` stay valid.

### 4.5 `src/binary_repr/reader.rs` (1180)

`binary_repr/mod.rs:20` does `use reader::*;` and `binary_repr/tests/**` reach reader fns through
`binary_repr`'s globs, so a moved `pub(super)` fn must stay visible in `binary_repr`: in the parts
`pub(super)` → `pub(in crate::binary_repr)`, and `reader/mod.rs` adds
`pub(in crate::binary_repr) use abi_index::*; pub(in crate::binary_repr) use type_decode::*;`.

- `reader/mod.rs` (572 + 4): 1-402 (package meta, `SectionKind` + impl, identity, container
  framing, `read_binary_repr_package`) + 800-1105 (per-section table decoders: function table,
  const pool, default-function validation, manifest, import, used-symbol, resource, global,
  export tables).
- `reader/type_decode.rs` (356): 403-799 — `decode_type_export`, `decode_type_field`,
  `read_string_pool`, `read_type_entries`, `type_entry_names`, `decode_type_name`,
  `decode_type_name_body`, `decode_function_type`, `read_payload_type`, `primitive_type_name`.
- `reader/abi_index.rs` (252): 1106-1405 — `read_abi_index`, `validate_abi_index`,
  `validate_manifest_counts`, `abi_export_for_decoded`, `decode_callable_export_kind`,
  `retired_sub_error`, `decode_export_kind`, `type_name`, `string_at`.
- 18 widenings (`sed -n 403,799p src/binary_repr/reader.rs | grep -c "pub(super)"` = 9;
  `sed -n 1106,1405p … | grep -c` = 9). Private `decode_type_name_body` and `SectionKind` are used
  only in their own file or by children. `use super::*;` at line 1 stays in `reader/mod.rs`.
- Guard (must change): `BOUNDARY_FILES` row `"src/binary_repr/reader.rs"` (the decoder half of
  boundary #4) and its `boundary_list_is_closed` entry (in `tests/guards/no_type_strings/self_checks.rs` after plan-176-E) → three rows `src/binary_repr/reader/mod.rs`,
  `…/reader/type_decode.rs`, `…/reader/abi_index.rs` in the same position, same reason, in both.
  Justification: (1) written in `4c5cc03a7` (plan-111-G Phase 2); (2) protects: only the `.mfp`
  codec may convert type spellings; (3) dependents: `is_boundary_file`, `boundary_list_is_closed`,
  the justification test `boundary_files_exist_and_are_justified` (in `tests/guards/no_type_strings/self_checks.rs`); (4) proof: the old file was exempt as a whole,
  and its three parts are exactly that code — group #4 is unchanged. Prose line 1736 → `reader/`.
- Citations 44: `reader/mod.rs` — `encode_package_meta`, `read_package_meta` ×3,
  `package_identity_id`, `mfp_binary_repr_payload` ×6, `validate_container_manifest_identity` ×4,
  `read_binary_repr_package` ×6, `read_function_table` ×3, `read_const_pool`,
  `validate_default_functions` ×2, `read_global_table`, bare `[[src/binary_repr/reader.rs]]`
  (`package/01_container-format.md:210`) → `[[src/binary_repr/reader/mod.rs]]`; `type_decode.rs` —
  `read_type_entries`, `decode_type_name_body` ×2, `primitive_type_name` ×3; `abi_index.rs` —
  `validate_abi_index` ×2, `validate_manifest_counts` ×2, `decode_callable_export_kind`,
  `retired_sub_error` ×2, `type_name`, `string_at`. `src/binary_repr/mod.rs:40` prose
  `reader::mfp_binary_repr_payload` stays valid.

### 4.6 `src/binary_repr/sections.rs` (1161)

- `sections/mod.rs` (509 + 2): 1-27 (`StringPool` impl) + 625-1202 (`ConstPool`,
  `ResourceTable`, `ImportTable`, `AbiIndex` impls, native-library table codec,
  `encode_export_kind`, `is_exported_function`, `function_sig_hash`, `type_sig_hash`).
- `sections/type_table.rs` (439 + 1): 28-624 — `impl TypeTable { … }` whole.
- `sections/abi_serializer.rs` (213 + 1): 1203-1457 — `impl<'a> AbiSerializer<'a> { … }` whole.
- The structs live in `binary_repr/mod.rs` (`struct AbiSerializer` at `mod.rs:910`), so their
  private fields stay visible. The moved impls' `pub(super)` methods widen to
  `pub(in crate::binary_repr)`: type_table 17, abi_serializer 11
  (`sed -n 28,624p src/binary_repr/sections.rs | grep -c "pub(super) fn"` = 17; `1203,1457` = 11).
  Private `opaque_entry_kind`, `collect_reachable`, `serialize_type_inner` are used only in their
  own impl. Free fns stay in `mod.rs`, so `binary_repr/mod.rs:21 use sections::*;` is unchanged.
- Guard (must change): `BOUNDARY_FILES` row `("src/binary_repr/sections.rs", ".mfp type-table codec")`
  and its closed-list entry → `src/binary_repr/sections/mod.rs`, `…/sections/type_table.rs`,
  `…/sections/abi_serializer.rs`, same position and reason, in both. Justification as §4.5
  (row written in `9174857b8`, plan-111-A Phase 1). Prose 50, 940, 1064 → `sections/`.
- Citations 18: `type_table.rs` — `type_id` ×3, `reserve_source_type`, `state_type`, `set_type`,
  `thread_type`, `foreign_type`; `abi_serializer.rs` — `serialize_type`; `mod.rs` — `add`,
  `add_native`, `from_project`, `encode_native_library_table`, `read_native_library_table`,
  `encode_export_kind`, `is_exported_function`, `function_sig_hash`, `type_sig_hash`.
- `.ai/resources-packages.md:211` `TypeTable::type_id` (`binary_repr/sections.rs`) →
  (`binary_repr/sections/type_table.rs`).

### 4.7 `src/binary_repr/writer.rs` (1111)

Same visibility recipe as the reader (`binary_repr/mod.rs:54 use writer::*;`): parts
`pub(super)` → `pub(in crate::binary_repr)`, and `writer/mod.rs` adds
`pub(in crate::binary_repr) use functions::*; pub(in crate::binary_repr) use resource_scan::*; pub(in crate::binary_repr) use type_payload::*;`.

- `writer/mod.rs` (496 + 6): 1-392 (`lower_project`, `lower_package_project`,
  `external_type_metadata`, `external_function_metadata`, `lower_project_with_external_functions`)
  + 1083-1294 (`impl BinaryReprProject` encode, doc-section lowering `docs_from_ir`).
- `writer/resource_scan.rs` (252): 393-657 — `ir_uses_resource_type`, `ops_use_resource_type`,
  `value_uses_resource_type`, `is_resource_type_name`, `collect_resource_type_names`,
  `collect_resource_names_in_ops`, `collect_resource_names_in_value`.
- `writer/functions.rs` (260): 658-945 — `lower_param_default`, `lower_function`,
  `collect_imported_calls_op`, `collect_imported_calls_value`.
- `writer/type_payload.rs` (103): 946-1082 — `FunctionTypeSignature` (+3 fields),
  `parse_function_type`, `source_type_payload`, `concrete_union_variants`, `put_field_payload`,
  `fixed_raw_from_decimal`, `standard_resource_flags`.
- 20 widenings (resource_scan 7, functions 3, type_payload 10 incl. the struct and its 3 fields;
  `sed -n <range>p src/binary_repr/writer.rs | grep -c "pub(super)"`). `lower_param_default`
  stays private.
- Guard (must change): `BOUNDARY_FILES` row `("src/binary_repr/writer.rs", ".mfp wire codec")` and
  its closed-list entry → the four `src/binary_repr/writer/…` files (`mod.rs`, `resource_scan.rs`,
  `functions.rs`, `type_payload.rs`), same position and reason, in both. Justification as §4.5
  (row written in `9174857b8`).
- Citations 13: `mod.rs` — `encode` ×3, `encode_functions` ×2, `encode_manifest` ×2, bare
  `[[src/binary_repr/writer.rs]]` (`language/13_modules-and-packages.md:41`) →
  `[[src/binary_repr/writer/mod.rs]]`; `functions.rs` — `lower_function`, `lower_param_default`;
  `type_payload.rs` — `concrete_union_variants`, `fixed_raw_from_decimal`, `standard_resource_flags`.
- Prose `binary_repr::writer::concrete_union_variants` (`codegen/builtins/canvas/mod.rs:2558`,
  `codegen/registry/mod.rs:814`) still resolves through the re-export.
  `.ai/resources-packages.md:271` `binary_repr/writer.rs` → `binary_repr/writer/`.

### 4.8 `src/codegen/cleanup/owned/builder_owned_cleanup.rs` (1106 + 78 test)

- `builder_owned_cleanup/mod.rs` (430 + 2; 78 test): 1-32 (imports, node consts,
  `OwnedListPass`, `impl CodeBuilder<'_> {`) + 477-804 (`call_assigned_owner_containers`,
  `emit_assigned_container_float`, `emit_stored_item_float`, `by_ref_resource_captures`,
  `ref_cell_size`, `setup_ref_cell`, `emit_captured_store_float`,
  `emit_owned_list_seed_from_collection`) + 1350-1651 (the impl's `}`, free fns
  `call_result_holds_only_argument_resources` … `type_variables`, second `impl CodeBuilder`, the
  inline tests, 6 `#[test]`).
- `builder_owned_cleanup/owned_list.rs` (315 + 3): 33-476 — `setup_owned_list`,
  `setup_owned_list_at`, `deactivate_owned_list`, `retire_returned_owner_block`,
  `is_resource_owning_container`, `emit_owned_list_push`, `emit_owned_list_push_as`,
  `emit_owned_list_push_node`, in `impl CodeBuilder<'_> { }`.
- `builder_owned_cleanup/drain.rs` (361 + 3): 805-1349 — `emit_owned_list_drain`,
  `emit_owned_list_reclaims`, `emit_owned_list_reclaim`, `emit_owned_list_walk`,
  `emit_owned_list_take`, `emit_owned_value_drop`, `emit_loop_item_drop`, likewise.
- Visibility: external paths `…::builder_owned_cleanup::{trap_call_result_locals,
  value_may_carry_statement_resources, collections_call_member, type_variables}`
  (`function_lowering.rs:1410`, `builder_control.rs:2979,3233,3290`, `ir/resource_escape.rs:798`)
  are free fns that stay in `mod.rs`. Moved `pub(crate)` methods unchanged. 2 edits:
  `setup_owned_list_at`, `emit_owned_list_push_as` → `pub(super)`. No `super::` in moved ranges.
- Stale in-code reference (fixed here): old 1252 comment "clears `builder_owned_cleanup.rs:201`"
  (in `drain.rs` after the move) → "clears `emit_owned_list_push`'s" — line 201 is
  `pub(crate) fn emit_owned_list_push(` (`awk 'NR<=201 && /fn [a-z_]+/ {l=NR": "$0}; NR==201{print l}'`),
  and the path no longer exists after the split.
- Citations 17: `mod.rs` — `OwnedListPass`, `call_assigned_owner_containers`,
  `emit_assigned_container_float` ×2, `emit_stored_item_float`, `setup_ref_cell`,
  `emit_captured_store_float`, `call_result_holds_only_argument_resources`,
  `value_may_carry_statement_resources`, `trap_call_result_locals`; `owned_list.rs` —
  `retire_returned_owner_block`, `emit_owned_list_push`, `emit_owned_list_push_as`,
  `emit_owned_list_push_node`; `drain.rs` — `emit_owned_list_drain` ×2, `emit_owned_list_reclaims`.
- `.ai`: `:58` of `.ai/resources-packages.md` `builder_owned_cleanup.rs` →
  `builder_owned_cleanup/drain.rs`. Stale (pre-existing, fixed here): `.ai/codegen-invariants.md:98`
  "`emit_owned_value_drop` / `emit_closure_drop` (`src/target/shared/code/builder_owned_cleanup.rs`)"
  names a path that does not exist (`ls src/target/shared/code/builder_owned_cleanup.rs` → no such
  file) and a fn deleted by `f37874271` (bug-781, single-owner closures;
  `grep -rn "fn emit_closure_drop" src` → none) → "`emit_owned_value_drop`
  (`src/codegen/cleanup/owned/builder_owned_cleanup/drain.rs`)"; the same line's "`arena.rs`'s
  immediate-double-free idempotency guard" → "`lower_arena_free`'s" (the arena helper; plan-176-P
  moves it to `memory/arena/arena/free.rs`, so the line names the symbol).
- This is the first split of a prefix-named file; the §4.0 widening lands in Phase 1, before it.

### 4.9 `src/codegen/debug/arena.rs` (1097)

- `arena/mod.rs` (519 + 2): 1-538 (consts, counter tables, `ArenaFeature`, `family_of`,
  `emit_debug_arena_*` (`pub(crate)`), `emit_registry_lock`, `lower_register`) + 1138-1244
  (`impl DebugFeature for ArenaFeature`). Adds `mod report; mod sample; use report::lower_report;
  use sample::lower_sample;`.
- `arena/report.rs` (421): 539-971 — `emit_slot_line`, `emit_series_line`, `lower_report`.
- `arena/sample.rs` (157): 972-1137 — `lower_sample`.
- Visibility: `debug/mod.rs:28 pub(crate) use arena::{…}` and `&arena::ArenaFeature` name only
  `mod.rs` items. `lower_report`, `lower_sample` → `pub(super)`. The `use super::clock::…` /
  `super::process` / `super::write` imports stay in `arena/mod.rs`; no `super::` in 539-1137.
- Citations 3: `ArenaFeature` ×2 → `arena/mod.rs`; `lower_sample` → `arena/sample.rs`.
- Gate: the artifact goldens are built without `--debug`; the phase diffs
  `mfb build -q --debug --ncode` dumps before/after for every target (§Phase 4).

### 4.10 `src/codegen/link/thunk/link_thunk.rs` (2140)

- `link_thunk/mod.rs` (381 + mod/use lines): 1-534 (imports, canaries, `LinkSupport`, symbol
  helpers, `emit_data_address`, `emit_posix_dlopen/dlsym`, Windows consts, `LinkCodegenOptions`,
  `ThunkContext`, `emit_link_support`, `lower_link_initializer`) + 1864-1874 (`ReturnMarshal`) +
  2279-2305 (`ThunkScratch` + impl) + J's `#[cfg(test)] mod tests;`.
- `link_thunk/lower_thunk.rs` (921 + 1): 535-1863 — `lower_link_thunk`.
- `link_thunk/value_marshal.rs` (321): 1875-2278 — `emit_return_passthrough`,
  `emit_copy_string_to_cstring`, `RESULT_SAVE_OFF`, `STATUS_SCRATCH_OFF`,
  `emit_copy_cstring_to_string`, `emit_narrow_to_cfloat`, `emit_nan_box_cfloat`,
  `emit_widen_from_cfloat`, `function_narrows_cfloat`, `c_literal_bits`.
- `link_thunk/link_expr.rs` (121): 2306-2474 — `emit_link_expr`, `link_expr_is_boolean`,
  `emit_link_bool`.
- `link_thunk/struct_marshal.rs` (396): 2475-2970 — `store_field`, `load_field`,
  `marshal_struct_in`, `narrow_signed_bits`, `marshal_struct_out`.
- `ReturnMarshal` and `ThunkScratch` stay in `mod.rs`, so their private fields need no edit.
- Visibility: `thunk/mod.rs` does `pub(crate) use link_thunk::*;` and re-exports only
  `pub(crate)` items, none of which move (`grep -n "pub(crate)"` in 535-2970 → none). 12 items
  become `pub(super)`: lower_thunk 1 (`lower_link_thunk`); value_marshal 7
  (`emit_return_passthrough`, `emit_copy_string_to_cstring`, `emit_narrow_to_cfloat`,
  `emit_widen_from_cfloat`, `function_narrows_cfloat`, `c_literal_bits`, `RESULT_SAVE_OFF`);
  link_expr 1 (`emit_link_expr`); struct_marshal 3 (`marshal_struct_in`, `marshal_struct_out`,
  `narrow_signed_bits`). `mod.rs` adds private `use link_expr::*; use lower_thunk::*;
  use struct_marshal::*; use value_marshal::*;` so siblings and the J-moved tests
  (`lower_link_thunk`, `emit_link_support`, `ThunkContext`, `LinkCodegenOptions`) see them. No
  `super::` in 535-2970.
- Guard: `tests/guards/no_type_strings.rs` `TYPE_KEYED_TABLES` row
  `("src/codegen/link/thunk/link_thunk.rs", "record_native_resources")` →
  `src/codegen/link/thunk/link_thunk/mod.rs`. The typed local at old 317 (in `emit_link_support`)
  stays in `mod.rs`; the parameter at old 545 is `&HashSet<…>`, which the scanner's
  `HashMap<`/`HashSet<` prefix match does not take (`grep -n "record_native_resources\s*:" src/codegen/link/thunk/link_thunk.rs`
  → 317, 545), so `lower_thunk.rs` needs no row. Justification: (1) written in `9c3c2f704`
  (plan-111-B Phase 1/2); (2) protects: this resource table stays keyed by `ParameterType`;
  (3) dependents: `curated_type_keyed_tables_all_exist`, `string_keyed_type_maps`; (4) proof: the
  same declaration at a new path.
- Citations 19: `mod.rs` — `_mfb_linker_init`, `emit_link_support` ×3, `lower_link_initializer`;
  `lower_thunk.rs` — `lower_link_thunk` ×5; `value_marshal.rs` — `emit_copy_string_to_cstring` ×3,
  `emit_copy_cstring_to_string`, `emit_narrow_to_cfloat`, `emit_widen_from_cfloat`,
  `c_literal_bits`; `link_expr.rs` — `emit_link_expr`; `struct_marshal.rs` — `marshal_struct_out`.
- `.ai/resources-packages.md:553` (`link_thunk.rs`, `if function.return_resource`) →
  (`link_thunk/lower_thunk.rs`, …).

### 4.11 `src/codegen/os/socket/shared.rs` (1199)

- `shared/mod.rs` (654 + 2): 1-698 (imports, `NetSymbol`, `net_symbol`, pollfd/cloexec/accept
  helpers, socket consts, cstring/hints helpers, `AddrVregs`, `AddressSlots`, address record
  helpers, `emit_make_handle`) + 1357-1527 (`NetBodyParts`, `void_result`,
  `lower_net_address_helper`).
- `shared/endpoint.rs` (545): 699-1356 — the `net.connectTcp / net.listenTcp` banner,
  `lower_net_endpoint_helper`, `lower_net_connect_tcp_helper`, `lower_net_connect_tcp_addr_helper`,
  `lower_net_listen_tcp_helper`.
- Visibility: users glob-import `crate::codegen::os::socket::shared::*`
  (`builtins/net|udp|tcp/gen_io.rs`, `socket/poll.rs`) and call
  `gen_shared::lower_net_{connect_tcp,connect_tcp_addr,listen_tcp}_helper`
  (`builtins/tcp/func_connect.rs:99-101`, `func_listen.rs:88`) → `mod.rs` adds
  `pub(crate) use endpoint::{lower_net_connect_tcp_addr_helper, lower_net_connect_tcp_helper, lower_net_listen_tcp_helper};`.
  0 visibility edits. No `super::` in 699-1356.
- Hazards: 0 citations, 0 guard/script/`.ai` refs.

### 4.12 `src/os/linux/link/elf.rs` (1037)

- `elf/mod.rs` (413 + mod/use lines): 1-446 (`.note` helpers, `encode_static_elf`,
  `encode_static_elf_x86`, `encode_dynamic_elf`) + 796-873 (`section_header`, `program_header`,
  `interpreter`, `e_machine`, `e_flags`).
- `elf/sections.rs` (295): 447-795 — `ElfSection`, `ElfSectionContents`, `append_section_table`,
  `SHT_*`/`SHF_*`/`STB_*`/`STT_*` consts, `mfbsign_section`, `LoadedSections`, `DynamicSections`,
  `SYMTAB_ENTRY_SIZE`, `append_elf_sections`, `symtab_bytes`.
- `elf/dynamic.rs` (329): 874-1309 — `DynamicPayload` + impl, `runpath_string`,
  `runpath_dynstr_len`, `dynamic_prefix_size`.
- Visibility: `link/mod.rs:56 use elf::*;` and `link/tests.rs` use only `encode_static_elf`,
  `encode_dynamic_elf` (stay) and `dynamic_prefix_size` (`link/mod.rs:367`, `tests.rs` ×6).
  `dynamic_prefix_size` `pub(super)` → `pub(in crate::os::linux::link)` + `pub(super) use dynamic::dynamic_prefix_size;`
  in `elf/mod.rs`; `DynamicPayload`, its 5 fields (`bytes`, `dynamic_offset`, `dynamic_size`,
  `dynstr_offset`, `dynstr_size`) and `build` → `pub(super)`; `elf/mod.rs` adds
  `use dynamic::DynamicPayload; use sections::*;`. sections.rs items keep their existing
  `pub(super)` (now = `elf`, their only user); sections.rs uses parent-private `section_header`.
  `use super::*;` (line 1) stays in `elf/mod.rs` with `super` = `link`.
- Citations 31: `mod.rs` — `encode_dynamic_elf` ×14, `encode_static_elf` ×3,
  `encode_static_elf_x86`, `mfb_note_bytes`, `note_program_header`, `e_machine`, `e_flags`,
  `PT_GNU_RELRO` (defined in `link/mod.rs:35`; the cited elf comment at old 266 stays in
  `elf/mod.rs`); `sections.rs` — `append_elf_sections` ×3, `append_section_table`, `symtab_bytes`;
  `dynamic.rs` — `build` ×3.
- Gate: the artifact gate never links; the phase `cmp`s Linux executables built before/after.

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

Before Phase 1, once: `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` →
record the number in this line as the O baseline (est. 4 min). Baseline: 5017 (at `5a9ab63f5`). Before each phase:
`cargo build --release && cp target/release/mfb /tmp/mfb-before-O`.

### Phase 1 — Prefix scans take split directories; aarch64 decoder

The scan widening lands first, alone with the lowest-risk split (code no artifact contains).

- [x] §4.0: locate `collect_builder_sources` and `no_cross_member_os_seam_dispatcher`
      (`grep -rn "fn collect_builder_sources\|fn no_cross_member_os_seam_dispatcher" src tests`);
      add `is_lowering_source(root, path)` and `func_named(root, path)`, route both scans through
      them, and add the two predicate `#[test]`s (part file, `mod.rs`, plain prefixed file → true;
      unprefixed part, prefixed dir above the root → false). The commit message carries the §4.0
      4-point justification.
      Done: `is_lowering_source` is a module-level fn in `src/codegen/engine/tests/tests.rs` (the walker
      `collect_builder_sources` is nested inside the test, so the predicate and its test sit beside that test);
      `func_named` in `architecture_guards.rs` (the scan root is now cloned into `rs_files`). Tests
      `is_lowering_source_takes_files_under_prefixed_directories` and
      `func_named_takes_files_under_func_directories`.
- [x] §4.1: `git mv src/arch/aarch64/decode/vector.rs src/arch/aarch64/decode/vector/mod.rs`;
      create `vector/scalar.rs` (937-1302); `scalar` → `pub(super)`; `mod scalar; use scalar::scalar;`.
      Done: `codesplit.py` → scalar 11 items; `motion.py` → only the wiring and the one `pub(super)`.
- [x] Tighten and commit the ceiling file.

Acceptance: both scans still pass and now read files under prefixed directories; the decoder
decodes identically.
  Check: `cargo test -p mfb --bin mfb -- no_overflow_label_returns_through_the_result_tag_register is_lowering_source`
  → pass, including the new predicate test (est. 5 min). `cargo test --test architecture_guards`
  → pass, including the `func_named` test (est. 1 min). The predicate tests are
  the proof that a part file is now scanned (the old rule returns `false` for `part.rs`); the
  population the widened engine walk reads is
  `find src/codegen -type f -name '*.rs' \( -name 'builder_*' -o -name 'func_*' -o -name 'gen_*' -o -path '*/builder_*/*' -o -path '*/func_*/*' -o -path '*/gen_*/*' \) | wc -l`
  → 754 (747 + 7, §2) (est. 5 s). `cargo test -p mfb --bin mfb -- arch::aarch64::decode debugger::`
  → pass, count before == after (11 decoder tests incl. the lldb-corpus parity; est. 3 min).
  `cargo check --all-targets` → no new warning (est. 2 min). `cargo test --test file_size_budget 2>&1 | grep 'decode/vector'`
  → one `STALE` row before the tighten, none after (est. 2 min).
  Verified 2026-10-19: `no_overflow_label_returns_through_the_result_tag_register is_lowering_source` → `ok. 2
  passed`; `architecture_guards` → `ok. 9 passed` (incl. `func_named_…`); widened population → 760 files (the
  file-name rule alone → 751; the planning-time 754/747 predate letters E–N); `arch::aarch64::decode debugger::`
  → 190 passed + 1 ignored (= the G0 list's 191); check warnings 0; list 5018 (baseline + 1, the engine predicate
  test); budget → one `STALE code (ceiling 1202) …decode/vector.rs` before, none after; census → 855, 350.
Commit: 23e8dfcaa

### Phase 2 — Encoders: aarch64 emitter, x86-64 emitter, riscv64 `v128`

- [x] §4.2: split `src/arch/aarch64/encode/emitter.rs` into `emitter/{mod,vector,scalar,memory}.rs`;
      80 `pub(super)` edits; rewrite the 12 citations.
      Done: `items.py split` by §4.2's ranges → 8, 61, 14 methods; `pub(super)` from the compiler's errors → exactly
      80 (scalar keeps `emit_rrr`, `emit_add_imm_chunk`, `emit_sub_imm_chunk` private); `motion.py` → only the
      wiring, three impl blocks and the 80; `citesweep.py` → 12.
- [x] §4.2: decoder test `include_str!("../encode/emitter.rs")` →
      `include_str!("../encode/emitter/mod.rs")` (4-point justification in the commit message).
- [x] §4.4: split `src/arch/x86_64/encode/emitter.rs` into `emitter/{mod,instruction,integer,sse}.rs`;
      widen `encode_instruction`; the re-export and two private globs; 37 `pub(super)` edits;
      rewrite the 15 citations.
      Done: `codesplit.py` → 1, 19, 22 items; `pub(super)` → exactly 37 (16 + 21); `motion.py` → only the
      wiring, the two globs, the re-export and the visibility edits; `citesweep.py` → 15.
- [x] §4.3: split `src/arch/riscv64/v128.rs` into `v128/{mod,scalarize,rvv}.rs`; the re-exports,
      `rvv_arm` → `pub(super)` + `#[cfg(test)] use rvv::rvv_arm;`; rewrite the 5 citations.
      Done: `codesplit.py` → scalarize 1, rvv 7 items; only `rvv_arm` widened; `citesweep.py` → 5.
- [x] §4.3: `tests/guards/no_operator_strings.rs`: both `v128.rs` entries →
      `src/arch/riscv64/v128/scalarize.rs` (justification in the commit message);
      `.ai/testing-gates.md:774`.
      Done: before the edit `no_operator_strings` → 4 FAILED; after → 8 passed; `.ai/testing-gates.md` (now :792).
- [x] Fix five pre-existing arch citations the phase's `citedef.py` check surfaced (added task):
      `[[src/arch/x86_64/select.rs:RETS]]` named a constant removed by `8e317bd5e` (plan-85), and its sentence
      ("the error-`Result` convention returns its four fields in `RETS = [rax, rdx, rcx, rsi]`") is stale — MFB
      results come from the call-argument bank (`realize_abi_operand`, `(AbiConvention::Mfb, _) => call_args`), C
      returns from `C_RETS`; rewritten to say so with those cites. The four `[[src/arch/{aarch64,riscv64}/encode/
      operand.rs:{field,shift,immediate}]]` cites named a re-export → `[[src/arch/encode_operand.rs:…]]`, the
      definition. `citedef.py src/arch/` → 142 cites, 0 not defined.
- [x] Tighten and commit the ceiling file.

Acceptance: every target's machine code is byte-identical.
  Check: `cargo test -p mfb --bin mfb -- arch::aarch64 arch::x86_64::encode arch::riscv64` → pass,
  count before == after (est. 5 min). `sh scripts/artifact-gate.sh target/release/mfb general`
  and `… vector` → 0 diffs each (`.ncode`/`.nobj` on all 5 targets; `vector` exercises NEON, SSE/AVX
  and RVV paths; est. 1 min each). `cargo test --test no_operator_strings` → pass (est. 1 min).
  `cargo check --all-targets` → no new warning; `cargo test -p mfb --bin mfb spec_citations_resolve`
  → pass. `cargo test --test file_size_budget 2>&1 | grep -E 'encode/emitter|riscv64/v128'` → one
  `STALE` code row per old file before the tighten, none after (est. 2 min).
  Verified 2026-10-19: `arch::aarch64 arch::x86_64::encode arch::riscv64` → 324 passed + 1 ignored (= G0's 325);
  G2 general, vector → 0 diffs; `no_operator_strings` → 8 passed; check warnings 0; `spec_citations_resolve` ok;
  budget → three `STALE` code rows (1133, 1155, 1837) before, none after; census → aarch64 emitter 529/148/322/170,
  x86 emitter 258/4 + 805/559/226, v128 230/4 + 393/538.
Commit: d70bef28c

### Phase 3 — `.mfp` codec: `binary_repr` reader, sections, writer

- [x] §4.5: split `reader.rs` into `reader/{mod,type_decode,abi_index}.rs`; 18 widenings; the two
      `pub(in crate::binary_repr) use` globs; rewrite the 44 citations.
      Done: `codesplit.py` → 10, 9 items; the 9 + 9 `pub(super)` → `pub(in crate::binary_repr)`, no other edit
      needed; `citesweep.py` → 43 + the bare cite by hand.
- [x] §4.6: split `sections.rs` into `sections/{mod,type_table,abi_serializer}.rs`; 28 widenings;
      rewrite the 18 citations; `.ai/resources-packages.md:211`.
      Done: whole impls moved; 17 + 11 widened; `citesweep.py` → 18.
- [x] §4.7: split `writer.rs` into `writer/{mod,resource_scan,functions,type_payload}.rs`;
      20 widenings; the three globs; rewrite the 13 citations; `.ai/resources-packages.md:271`.
      Done: `codesplit.py` → 7, 4, 7 items; 20 widened (`lower_param_default` stays private); `citesweep.py` → 12
      + the bare cite. Also two unlisted code comments (`codegen/builtins/mod.rs:627`,
      `builtins/tests/package_format.rs:7`) → the directories; `git grep` of the three old paths → 0.
- [x] `tests/guards/no_type_strings.rs`: replace the three `binary_repr` rows in `BOUNDARY_FILES`,
      and in `tests/guards/no_type_strings/self_checks.rs`: `boundary_list_is_closed`, with their part files (§4.5–4.7), same order; prose
      50, 940, 1064, 1736 (justifications in the commit message).
      Done: before the edit 2 FAILED (`no_type_strings`, `boundary_files_exist_and_are_justified`); after → 7 + 1
      ignored. Prose now at 50, 961, 1085 and `self_checks.rs:472`, whose "six files in five groups" also gains the
      measured path count (19, after N's and O's splits).
- [x] Tighten and commit the ceiling file.

Acceptance: packages encode and decode to the same bytes.
  Check: `cargo test -p mfb --bin mfb binary_repr` → pass, count before == after (est. 4 min).
  `bash scripts/test-accept.sh target/release/mfb /tmp/accept-O 'package-*' 'native-library-*' 'native-resource-*' 'roundtrip-package'`
  → 0 failures over the 18 `.mfp`/`.info` fixtures (est. 3 min, guess).
  `sh scripts/artifact-gate.sh target/release/mfb thread` → 0 diffs (its worker package is read
  from `packages/thread_cover_worker.mfp`; est. 1 min). `cargo test --test no_type_strings` → pass.
  `cargo check --all-targets` → no new warning; `spec_citations_resolve` → pass.
  `cargo test --test file_size_budget 2>&1 | grep 'binary_repr'` → three `STALE` rows before the
  tighten, none after (est. 2 min).
  Verified 2026-10-19: `binary_repr` → `ok. 181 passed` (= G0's 181); `test-accept.sh … 'package-*'
  'native-library-*' 'native-resource-*' 'roundtrip-package'` → `acceptance tests passed (42 test(s) ran)`; G2
  thread → 0 diffs; `no_type_strings` → 7 + 1 ignored; check warnings 0; `spec_citations_resolve` ok; `citedef.py
  src/binary_repr/` → 80, 0 not defined; budget → three STALE rows before, none after; census → reader 576/360/260,
  sections 511/460/226, writer 502/256/261/106.
Commit: —

### Phase 4 — Linker and debug report: `elf.rs`, `debug/arena.rs`

- [ ] §4.12: split `src/os/linux/link/elf.rs` into `elf/{mod,sections,dynamic}.rs`; the
      visibility edits and imports; rewrite the 31 citations.
- [ ] §4.9: split `src/codegen/debug/arena.rs` into `arena/{mod,report,sample}.rs`; two
      `pub(super)` edits and imports; rewrite the 3 citations.
- [ ] Tighten and commit the ceiling file.

Acceptance: Linux executables and `--debug` native code are byte-identical.
  Check: `cargo test -p mfb --bin mfb -- os::linux::link codegen::debug` → pass, count before ==
  after (est. 3 min). Copy `tests/byte-identity/general` to `/tmp/o4/a` and `/tmp/o4/b`; for each
  of `linux-aarch64`, `linux-x86_64`, `linux-riscv64`, build `a` with `/tmp/mfb-before-O build -q --target <t>`
  and `b` with `target/release/mfb build -q --target <t>`, then the same pair with `--debug` added;
  `cmp` each `build/*-glibc.out` and `build/*-musl.out` pair → identical (the dynamic-ELF and
  `--debug` section/symbol-table paths; `encode_static_elf`/`encode_static_elf_x86` are pinned by
  the `os::linux::link` unit tests above; est. 3 min). For each of the 5 targets,
  `mfb build -q --debug --ncode [--target <t>]` with both binaries → `cmp` of the `.ncode` dumps →
  identical (est. 2 min). `cargo check --all-targets` → no new warning; `spec_citations_resolve`
  → pass. `cargo test --test file_size_budget 2>&1 | grep -E 'link/elf|debug/arena'` → two
  `STALE` rows before the tighten, none after (est. 2 min).
Commit: —

### Phase 5 — Codegen: owned cleanup, socket shared, link thunk (largest blast radius)

- [ ] §4.8: split `builder_owned_cleanup.rs` into `builder_owned_cleanup/{mod,owned_list,drain}.rs`;
      two `pub(super)` edits; the old-1252 comment fix; rewrite the 17 citations;
      `.ai/resources-packages.md:58`; the stale `.ai/codegen-invariants.md:98` fix.
- [ ] §4.11: split `src/codegen/os/socket/shared.rs` into `shared/{mod,endpoint}.rs`; the
      `pub(crate) use endpoint::{…}` re-export.
- [ ] §4.10: split `link_thunk.rs` into `link_thunk/{mod,lower_thunk,value_marshal,link_expr,struct_marshal}.rs`;
      12 `pub(super)` edits; the four private globs; rewrite the 19 citations;
      `.ai/resources-packages.md:553`.
- [ ] `tests/guards/no_type_strings.rs`: the `record_native_resources` row →
      `src/codegen/link/thunk/link_thunk/mod.rs` (justification in the commit message).
- [ ] Tighten and commit the ceiling file.

Acceptance: resource cleanup, sockets and native calls emit identical code, and the scans read
the owned-cleanup parts.
  Check: `cargo test -p mfb --bin mfb -- codegen::cleanup codegen::link codegen::os no_overflow_label_returns_through_the_result_tag_register`
  → pass, count before == after (est. 5 min). `sh scripts/artifact-gate.sh target/release/mfb <s>`
  for `s` in `resource-xfer-slots`, `fs`, `net`, `tcp`, `udp`, `link-const-pins` → 0 diffs each
  (est. 1 min each). `cargo test --test no_type_strings --test architecture_guards` → pass.
  `cargo check --all-targets` → no new warning; `spec_citations_resolve` → pass.
  `cargo test --test file_size_budget 2>&1 | grep -E 'builder_owned_cleanup|socket/shared|link_thunk'`
  → one `STALE` code row per old path (three; J removed `link_thunk.rs`'s inline ceiling and
  `builder_owned_cleanup.rs`'s 78 inline test LOC never had one) before the
  tighten, none after, and no `NEW` row (so `lower_thunk.rs`, 922, is under 1000) (est. 2 min).
Commit: —

## Validation Plan

- Tests: added — the two scan-predicate tests (Phase 1). Whole-binary count:
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` → O baseline + 1 (the
  engine predicate test; the guard test is in the `architecture_guards` target) (est. 4 min).
- Coverage check: every moved line stays in its directory under `src/`; no coverage exception
  names these files, so none is repointed.
- Runtime proof: byte-identity. Letter end, once: `sh scripts/artifact-gate.sh target/release/mfb all`
  → `0 diff(s)` (est. 15–20 min; the encoders and cleanup feed every fixture on every target, and
  only the full sweep covers all of them). Plus the Phase 3 `.mfp` and Phase 4 executable and
  `--debug` differentials, which the artifact gate cannot see.
- Guards: `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings`
  → pass (est. 2 min).
- Budget: `cargo test --test file_size_budget 2>&1 | grep -E 'aarch64/(decode/vector|encode/emitter)|riscv64/v128|x86_64/encode/emitter|binary_repr/(reader|sections|writer)|builder_owned_cleanup|debug/arena|link_thunk|socket/shared|linux/link/elf'`
  → no row; the same pattern over `tests/guards/file_size_ceilings.txt` → no `ceiling` row.
- Doc sync: `spec_citations_resolve` → pass; the citation pattern of §2 over every old path → 0.
  No `mfb man` page cites these files. No compiler contract changes.
- Full workspace suite: once, in plan-176-V's Validation Plan.

## Decisions

- Rule revised — owner, 2026-10-18: three kinds, non-comment non-blank LOC, scope `.rs`/`.mfb`
  under `src/`, `packages/`, `examples/`, `repository/`, `tests/`: `code` (a non-test-only file)
  ≤ 1000 → split into modules; `inline` (`#[cfg(test)]`/`TESTING` LOC in a non-test-only file)
  ≤ 250 → move to a test-only file (J did this for `v128.rs` and `link_thunk.rs`); `test` (a
  test-only file) ≤ 1000 → split into modules. Parts target ≤ 900. Ceiling rows are
  `ceiling <code|inline|test> <n> <path>`. No exemptions except `GENERATED` files; the guard
  warns with a banner, never fails; a new module is always an acceptable split.
- A single function over the code limit is decomposed by extract-method only in S–V — owner,
  2026-10-14. `lower_link_thunk` (921) is under the limit and stays whole in `lower_thunk.rs`
  (922 LOC) — author.
- The prefix-keyed scans (engine overflow-label test, `no_cross_member_os_seam_dispatcher`) take
  a file under a `builder_*`/`func_*`/`gen_*` directory, landed in O Phase 1 with a predicate
  test — decided by the parent planner, 2026-10-14. Letters P–V rely on it.
- Guard and source-reading-test edits land in the same commit as the move, each with the
  AGENTS.md 4-point justification; no production file becomes newly exempt — author.
- Pre-existing defects fixed here: the stale `.ai/codegen-invariants.md:98` path and
  `emit_closure_drop` name; the `builder_owned_cleanup.rs:201` comment line reference — author
  (AGENTS.md "never leave a bug").
- No exemptions exist (rule revised 2026-10-18); O adds only the scan widening, no guard row.

## Corrections

- 2026-10-18: Rule revised by the owner (code ≤ 1000, inline ≤ 250, test-only file ≤ 1000, no exemptions); file list and split design unchanged; rule text restated, J's moved blocks named as single `tests/mod.rs` files, `aarch64/decode/tests.rs` (742, no longer split by J) keeps its `include_str!("../encode/emitter.rs")` path, `architecture_guards.rs`/`engine/tests/tests.rs` noted unsplit.

## Summary

Risk sits in two places: the Phase 1 scan widening (test code that must only ever widen — the
predicate test pins it) and the link thunk, whose marshalling feeds every native call. Byte
identity across all targets, plus the executable, `--debug` and `.mfp` differentials the
artifact gate cannot see, covers every moved line. No caller-visible path, emitted byte or test
changes.
