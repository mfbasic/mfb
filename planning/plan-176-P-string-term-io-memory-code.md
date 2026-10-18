# plan-176-P: String, term, I/O and memory code splits

Last updated: 2026-10-18
Effort: large (3h–1d)
Depends on: plan-176-O

P splits the ten over-limit code files under `src/codegen/{string,term,io,memory}` into module
directories: the terminal input decoder, the stdin broadcast helpers, the arena allocator, the
cross-arena transfer builder, the static data objects, the value-semantics builder, the string
builder, the Unicode property emitters, the terminal backend and the terminal grid. Every split is
pure code motion; module paths callers use stay the same through re-exports.

Three of the files are prefix-named (`builder_arena_transfer.rs`, `builder_value_semantics.rs`,
`builder_strings.rs`). P relies on plan-176-O Phase 1, which made the two prefix-keyed scans take
a file under a `builder_*` directory; without it the 2 overflow labels in `builder_strings.rs`
would leave `no_overflow_label_returns_through_the_result_tag_register` silently.

The single outcome: after P lands, `cargo test --test file_size_budget` shows no row for any of
the ten files or the files they become, and `tests/guards/file_size_ceilings.txt` has no
`ceiling` row for them. The rule (owner, 2026-10-18): code ≤ 1000, inline test ≤ 250, test-only
file ≤ 1000 non-comment non-blank LOC; there are no exemptions besides `GENERATED` files.

References:

- `planning/plan-176-A-file-size-guard.md` — rule, guard, ratchet, §Decisions, split conventions
  (`.ai/build-tooling.md` "Splitting an over-budget file").
- `planning/plan-176-O-backend-code.md` §4.0 — the prefix-scan widening P depends on.
- `.ai/codegen-invariants.md`, `.ai/collections.md` (lines 254, 526 name P files),
  `.ai/resources-packages.md` (lines 28, 61, 123, 193, 387, 419, 486, 608), `.ai/net-tls.md`
  (line 42), `.ai/testing-gates.md` § "Splits must sweep man AND spec citations",
  `.ai/build-tooling.md`.
- Spec topics whose citations move (`grep -rlE "\[\[src/codegen/(io/input/decode|io/stdin/stdin_broadcast|memory/arena/arena|memory/arena/builder_arena_transfer|memory/data/data_objects|memory/value/builder_value_semantics|string/repr/builder_strings|string/unicode_props|term/core/term|term/grid/term_grid)\.rs" src/docs/spec`
  → 22 files): `app/04_term-backend.md`, `architecture/04_ir.md`,
  `language/{04_types,13_modules-and-packages,15_resource-management,16_threads,18_builtin-functions}.md`,
  `memory/{02_fallible-call-abi,03_heap-values,04_arenas,05_collections,07_runtime-helper-abi,09_closures}.md`,
  `stdlib/{01_regex,22_io-input}.md`, `threading/{02_isolation,08_queue-semantics,09_os-integration}.md`,
  `tooling/09_debug-report.md`, `unicode/{01_tables-and-algorithms,02_strings-model,spec}.md`.
- Split map: `/tmp/loc/maps/P.md` (working tree of `17a9b5799`), restated below in decided form.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-O is archived | `ls planning/plan-176-O-* planning/completed/plan-176-O-*` → only the `planning/completed/` path | NOT MET (2026-10-14) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | NOT MET (2026-10-14; lands in plan-176-A) |
| P's files have no uncommitted changes | `git status --porcelain -- src/codegen/io/input/decode.rs src/codegen/io/stdin/stdin_broadcast.rs src/codegen/memory/arena/arena.rs src/codegen/memory/arena/builder_arena_transfer.rs src/codegen/memory/data/data_objects.rs src/codegen/memory/value/builder_value_semantics.rs src/codegen/string/repr/builder_strings.rs src/codegen/string/unicode_props.rs src/codegen/term/core/term.rs src/codegen/term/grid/term_grid.rs tests/codegen/codegen_trap_result_wrapper.rs .ai` → empty | MET (2026-10-14) |
| The spec topics P edits have no uncommitted changes | `git status --porcelain -- src/docs/spec` → no line for a file P edits (the 22 above) | MET (2026-10-14: only ` M src/docs/spec/app/06_canvas.md`, not one of the 22) |

Everything below is written against the world where these hold. The test-only file beside
`decode.rs`, `src/codegen/io/input/corpus.rs` (903 LOC), is under the 1000 test-only limit, so no
letter splits it and P does not touch it (`python3 -I /tmp/loc/census.py "$PWD"` at ce7a995ca).
No earlier letter moves an inline test block out of a P file: each P file's inline test LOC is
< 250 (§2) and stays in its new `mod.rs`.

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

- Each of the ten files becomes `foo/mod.rs` + parts, every resulting file ≤ 1000 code LOC
  and ≤ 250 inline test LOC.
- `cargo test --test file_size_budget 2>&1 | grep -E '<the ten paths>'` → no row; no `ceiling`
  row for them in the ceiling file.
- `sh scripts/artifact-gate.sh target/release/mfb all` → 0 diffs.

### Non-goals (explicit constraints)

- No behavior change: no `.ncode`/`.nobj`/`.nir` byte change on any target.
- No logic edit beyond visibility widenings, `use`/`mod`/re-export lines, the one `include_str!`
  path in `builder_arena_transfer`'s test, the expected path in one source-scanning test, and
  comment/doc path fixes.
- No crate path a caller uses changes (`memory::arena::*`, `data::*`, `stdin::*`, `term::core::*`,
  `term::grid::*`, `string::unicode_props::*`, `io::input::decode::*`).
- No `[[test]]` target renamed; no test fn renamed; the inline test modules stay in the new
  `mod.rs` (same module paths).

## 2. Current State

| File | Kind | Code LOC | Test LOC | Inline `#[test]` |
|---|---|---|---|---|
| `src/codegen/io/input/decode.rs` | code | 1296 | 0 | 0 |
| `src/codegen/io/stdin/stdin_broadcast.rs` | code | 1126 | 0 | 0 |
| `src/codegen/memory/arena/arena.rs` | code | 1614 | 80 | 2 |
| `src/codegen/memory/arena/builder_arena_transfer.rs` | code | 1569 | 80 | 3 |
| `src/codegen/memory/data/data_objects.rs` | code | 1471 | 174 | 4 |
| `src/codegen/memory/value/builder_value_semantics.rs` | code | 1325 | 0 | 0 |
| `src/codegen/string/repr/builder_strings.rs` | code | 2018 | 0 | 0 |
| `src/codegen/string/unicode_props.rs` | code | 1513 | 0 | 0 |
| `src/codegen/term/core/term.rs` | code | 2811 | 0 | 0 |
| `src/codegen/term/grid/term_grid.rs` | code | 1502 | 60 | 2 |

LOC: `python3 -I /tmp/loc/census.py` on `17a9b5799` (values from `/tmp/loc/census2.json`);
after plan-176-A, `python3 -I tools/loc-census/census.py --file <path>`. Part LOC below:
`python3 -I /tmp/loc/span.py <file> <a> <b>` summed over the ranges; each part adds 2–12 lines of
`use` / `impl CodeBuilder<'_> {` wrapper. Test fns: `grep -cE '#\[(tokio::)?test\]' <file>`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Spec citations, per file | decode 2, stdin_broadcast 4, arena 8, builder_arena_transfer 13, data_objects 1, builder_value_semantics 6, builder_strings 5, unicode_props 21, term 12, term_grid 5 | `grep -rnoE "\[\[<path>(:[^]]+)?\]\]" src/docs/spec \| wc -l` per path |
| `mfb man` citations; non-citation `src/docs` mentions | 0; 0 | same pattern over `src/docs/man`; `grep -rnE "(^\|[^a-z_/])(src/)?<path>" src/docs \| grep -v '\[\[<path>'` |
| Guard allowlist rows naming a P file | 0 | basename scan of `tests/guards/*.rs` (`architecture_guards`, `no_type_strings`, `no_operator_strings`, `inplace_self_update_census`) |
| Tests that read a P file by path | 1: `tests/codegen/codegen_trap_result_wrapper.rs` `the_result_wrapper_has_exactly_one_constructor` expects `src/codegen/memory/arena/builder_arena_transfer.rs` | `grep -rnE '"(src/)?codegen/(io\|memory\|string\|term)/[a-z_/]+\.rs"' src tests` |
| `include_str!` in P files | 1: `builder_arena_transfer.rs:2296` `include_str!("../../error/constants/error_constants.rs")` | `grep -n include_str <file>` |
| Overflow labels in P's prefix-named files | `builder_strings.rs` 2 (old 267, 575, both in `lower_replace*` → `replace.rs`); the other two 0 | `grep -c 'abi::label(&.*overflow' <file>` |
| Host-`cfg` gating; `#[path]`; `module_path!`; coverage exceptions | 0; 0; 0; 0 | `grep -cE "#\[cfg\((target_os\|windows\|unix\|target_family)" <file>`; `grep -n "#\[path" <file>`; `grep -nE "vector\|term\|arena\|data_objects\|unicode_props\|builder_strings\|stdin" scripts/coverage-exceptions.txt` → none for these paths |

### Verified properties

- **A parent `use` consumed only by a child through `use super::*;` does not warn**
  (`/tmp/locN-probe`: no warning, printed `3`); **glob-cycle imports compile without warnings**
  (`/tmp/loc/probe2`: printed `6`).
- **No name in a split part collides through a glob**: a moved item was a top-level item of the
  same file, which already could not coexist with an identically named import there; parts reach
  each other only through their parent's names.
- **The prefix-keyed scans read split part files** after plan-176-O Phase 1 (its predicate test
  pins `builder_x/part.rs` → scanned).
- **The source-scanning test names a file, but its intent is a function.**
  `the_result_wrapper_has_exactly_one_constructor` walks `src/codegen` for
  `type_: ParameterType::result_of(` and asserts the only file is `builder_arena_transfer.rs`;
  its message and doc say the construction is funnelled through `fresh_trapped_result_value`. The
  one site (`grep -n "type_: ParameterType::result_of(" src/codegen/memory/arena/builder_arena_transfer.rs`
  → 528) is inside `fresh_trapped_result_value` (fn at 518), which moves to `result.rs`.
- **The terminal backend, input decoder and grid are reached by the `term` and `io` fixtures**;
  the arena allocator and data objects by every fixture; the transfer builder by `thread` and
  `resource-xfer-slots`; each selector covers 5 targets (`find tests/byte-identity -name "*.ncodesum"`).
- **No cfg-gated host code** (table above): no Windows-target or box-2223 check is needed for P.
- **No `module_path!`/`file!`/`line!` and no link-order registration** (plan-176-A).

## 3. Design Overview

Conventions are plan-176-A's: `foo.rs` → `foo/mod.rs` + parts; `use super::*;` first in each part;
`CodeBuilder` methods split into several `impl CodeBuilder<'_> { … }` blocks (no import needed);
a part item used by `mod.rs` or a sibling gets `pub(super)`. Free functions: `mod.rs` adds
`pub(crate) use <part>::{…};` for items callers outside the file name, and a private
`use <part>::*;` for every part that defines a `pub(super)` free item, so siblings reach it via
their `use super::*;`. Line ranges are lines **as of 17a9b5799**; item names are authoritative if
lines drift.

**Gate class: byte-identity.** Per phase: `cargo build --release`, the scoped artifact gate for
the selectors that reach the moved code (named per phase), the moved code's unit tests,
`cargo check --all-targets`, `spec_citations_resolve`, the guards, the tighten step. Letter end,
once: `sh scripts/artifact-gate.sh target/release/mfb all`. A diff = diff ONE fixture's dump,
find the moved item that changed, fix, re-run.

Risk: the arena allocator and transfer builder (Phase 5) are emitted into every program and own
the cross-thread copy; they land last. The `term` backend has the most files (7) but one fixture
family. The I/O split lands first: two small files, 2 + 4 citations.

Rejected alternatives:

- *Splitting `lower_arena_alloc` (707 LOC), `lower_stdin_next_byte` (618) or `string_symbols`
  (485)* — under the 1000 limit; extract-method is reserved for functions over it (S–V).
- *Moving `StampCtx` into `draw.rs`* — its 13 private fields are read from `text.rs`; keeping it in
  `term/mod.rs` needs no visibility edit.
- *Re-exporting every part with `pub(crate) use part::*`* — widens private helpers' reach; the
  explicit lists keep today's surface exactly.

## 4. Detailed Design

### 4.1 `src/codegen/io/input/decode.rs` (1296)

- `decode/mod.rs` (419): 1-445 (module doc, `use`s, every `pub(crate) const` incl.
  `FLAG_*`/`TOKEN_*`/`STATUS_*`/`TEXT_*`, private `NONE`/`PARAM_MAX`/`ESC`, `struct Dec` +
  `impl Dec`, `const fn k`, `lower_input_decode`, `struct Exits`, `emit_body`,
  `emit_named_const`, `emit_c0`) + 1406-1506 (`emit_exits`). Declares `mod csi; mod text;` and
  `use csi::*; use text::*;`.
- `decode/text.rs` (292): 446-692 (`emit_scalar_len`, `emit_copy_to_grapheme`, `emit_codepoint`,
  `emit_break_props`, `emit_text`) + 1038-1107 (`emit_encode_utf8`). Carries the
  `crate::codegen::string::unicode_props::{…}` import (old 34).
- `decode/csi.rs` (585): 693-1037 (`emit_alt_introducer`, `emit_modifier_bits`, `emit_csi`,
  `struct Params`, `emit_parse_params`, `emit_store_param`) + 1108-1405 (`emit_event`, `table_of`,
  `emit_csi_final`, `emit_code_token`, `emit_ss3`).
- Visibility: no re-export — every outside user (`io/input/mod.rs` `decode::TOKEN_BYTES`,
  `reader.rs` `super::decode::{TOKEN_*}`, `engine/builder/mod.rs` `decode::lower_input_decode`)
  names an item that stays in `mod.rs`. `pub(super)`: 4 — `emit_text`, `emit_encode_utf8`
  (text.rs; used by `mod.rs` / csi.rs), `emit_csi`, `emit_ss3` (csi.rs; used by `mod.rs`).
  `mod.rs`'s `use super::keys::*; use super::{CSI_PARAM_CAP, GRAPHEME_CAP};` stay (its `super` is
  still `input`); the parts reach them through `use super::*;`.
- Citations 2: `lower_input_decode`, `FLAG_EVENTS` (`stdlib/22_io-input.md:73-74`) →
  `[[src/codegen/io/input/decode/mod.rs:…]]`.

### 4.2 `src/codegen/io/stdin/stdin_broadcast.rs` (1126)

- `stdin_broadcast/mod.rs` (222): 1-306 (consts, `stdin_log_data_object`, `push_log_address`,
  `field_addr`, `emit_libc`, `emit_stdin_next_byte`, `emit_stdin_poll_ready_check`,
  `PollReadyRegs`, `emit_stdin_poll_ready_check_with`).
- `stdin_broadcast/next_byte.rs` (618): 307-993 — `lower_stdin_next_byte`.
- `stdin_broadcast/subscription.rs` (286): 995-1309 — `lower_stdin_recompute_base`,
  `lower_stdin_subscribe`, `lower_stdin_unsubscribe`.
- Visibility: `stdin/mod.rs` does `pub(crate) use stdin_broadcast::*;` and
  `engine/builder/mod.rs` calls all four lowerers → `mod.rs` adds
  `pub(crate) use next_byte::lower_stdin_next_byte; pub(crate) use subscription::{lower_stdin_recompute_base, lower_stdin_subscribe, lower_stdin_unsubscribe};`.
  `pub(super)`: 0 (parts use only `mod.rs` items: `emit_libc`, `field_addr`, consts).
- Citations 4: `emit_stdin_poll_ready_check` (`stdlib/22_io-input.md:112`) → `mod.rs`;
  `lower_stdin_next_byte` (`threading/09_os-integration.md:145`) → `next_byte.rs`;
  `lower_stdin_subscribe`, `lower_stdin_unsubscribe` (`threading/09_os-integration.md:153-154`) →
  `subscription.rs`. Prose `io/input/buffer.rs:36` `stdin_broadcast::field_addr` stays valid.

### 4.3 `src/codegen/memory/arena/arena.rs` (1614 + 80 test)

- `arena/arena/mod.rs` (269 + 80 test): 1-177 (`use`s, `emit_perf_arena_call`,
  `emit_large_bin_slot`, `emit_park_chunk`, `emit_debug_alloc_hit`) + 1869-2018
  (`lower_arena_fill_zero`, `lower_arena_destroy`) + 2020-2109 `#[cfg(test)] mod zero_fill_tests`
  (stays inline; `the_arena_fill_stores_only_zero_words`,
  `fresh_blocks_and_freed_chunks_are_zero_filled`).
- `arena/arena/alloc.rs` (799): 178-1200 — `lower_arena_alloc`, `lower_simd_alloc_list`.
- `arena/arena/free.rs` (546): 1202-1867 — `lower_arena_insert_free`, `lower_arena_flush_coalesce`,
  `lower_arena_free`.
- Visibility: `memory/arena/mod.rs` has `pub(crate) use arena::*;`; `arena/mod.rs` adds
  `pub(crate) use alloc::{lower_arena_alloc, lower_simd_alloc_list}; pub(crate) use free::{lower_arena_flush_coalesce, lower_arena_free, lower_arena_insert_free};`
  (callers `engine/builder/mod.rs`, `target/macos_aarch64/app/mod.rs`,
  `collection/assign/self_update.rs`, `builtins/tests/declared_errors_all.rs`). `pub(super)`: 0
  (the private helpers stay in `mod.rs`). `mod arena` inside `arena` already exists
  (`module_inception` unchanged).
- Citations 8: `emit_large_bin_slot` ×2 (`memory/04_arenas.md:209,280`), `lower_arena_fill_zero`
  (:319), `lower_arena_destroy` (:338) → `arena/arena/mod.rs`; `lower_arena_alloc` ×2
  (`04_arenas.md:161`, `07_runtime-helper-abi.md:161`) → `alloc.rs`;
  `lower_arena_flush_coalesce` (:243), `lower_arena_free` (:268) → `free.rs`.

### 4.4 `src/codegen/memory/arena/builder_arena_transfer.rs` (1569 + 80 test)

- `builder_arena_transfer/mod.rs` (61 + 80 test): 1-133 (`use`s, `RawSuccessBlock`, `BlockLength`,
  `TrappedErrorTagClass`, `trapped_error_tag_class`, `adoptable_error_tag`, `TrappedErrorSource`)
  + 2266-2278 (`PayloadEdgeShape`) + 2280-2394 `#[cfg(test)] mod tests` (stays inline;
  `the_result_tag_partition_is_total`, `exactly_one_tag_is_adoptable_and_the_emitter_uses_it`,
  `every_trapped_error_source_is_accounted_for`).
- `…/result.rs` (438): 135-817 — `emit_build_result_inline`, `materialize_current_result`,
  `materialize_current_result_boxed`, `fresh_trapped_result_value`, `emit_trapped_error_result`,
  `emit_trapped_error_block`, `emit_free_borrowed_guarded`, `emit_free_trapped_error_payload`.
- `…/transfer.rs` (582): 819-1652 — `copy_value_to_current_arena`, `emit_thread_copy_call`,
  `emit_thread_copy_real`, `copy_record_to_current_arena`, `copy_resource_to_current_arena`,
  `emit_copy_resource_live_slots`, `emit_copy_resource_arena_block`,
  `emit_flag_resource_source_moved`, `record_field_is_pointer_in`, `record_pointer_edges`, the
  fn at old 1547, `record_needs_pointer_field_fix`, `copy_union_to_current_arena`.
- `…/collection_transfer.rs` (488): 1654-2263 — `collection_needs_transfer_fix`,
  `copy_collection_to_current_arena`, `fix_collection_transfer_payloads`,
  `collection_payload_edges`, `payload_edge_shape`, `own_collection_payload_edges`,
  `collection_payload_needs_transfer_fix`, `fix_collection_transfer_payload`,
  `copy_record_fields_into_existing`, `copy_union_fields_into_existing`.
- Visibility: no re-export (outside callers name only `TrappedErrorSource` and the enums, which
  stay; the rest are methods). `pub(super)`: 5 methods — transfer.rs
  `copy_resource_to_current_arena`, `record_needs_pointer_field_fix` (used by
  collection_transfer.rs); collection_transfer.rs `copy_collection_to_current_arena`,
  `copy_record_fields_into_existing`, `copy_union_fields_into_existing` (used by transfer.rs).
- `include_str!` (must change): test const `ERROR_CONSTANTS_SOURCE` (old 2296)
  `include_str!("../../error/constants/error_constants.rs")` → in `builder_arena_transfer/mod.rs`
  `include_str!("../../../error/constants/error_constants.rs")` (one level deeper; both resolve to
  `src/codegen/error/constants/error_constants.rs`).
- Source-scanning test (must change):
  `tests/codegen/codegen_trap_result_wrapper.rs` `the_result_wrapper_has_exactly_one_constructor`
  expected `vec!["src/codegen/memory/arena/builder_arena_transfer.rs".to_string()]` →
  `vec!["src/codegen/memory/arena/builder_arena_transfer/result.rs".to_string()]`.
  Justification (AGENTS.md test rule): (1) written in `1bf2a4a94` (bug-568, "an inline TRAP
  binds the Result wrapper, and no longer copies it"); (2) protects: a `Result`-typed
  `ValueResult` is built only by `fresh_trapped_result_value`, which is what makes binding it
  without a deep copy sound; (3) dependents: none (`grep -rn "the_result_wrapper_has_exactly_one_constructor" src tests`
  → its definition); (4) proof: the test's own message names `fresh_trapped_result_value` as the
  single constructor, and that fn — holding the one `type_: ParameterType::result_of(` site —
  moves verbatim to `result.rs`; the walk still finds exactly one file, so the assertion still
  pins "one constructor", at the new path.
- Stale in-code reference (fixed here): the comment in `copy_value_to_current_arena` (old 909,
  → `transfer.rs`) "the `Result` wrap at `:145` is reached by any `TRAP`" names a line in another
  file after the split; old 141-145 is `emit_build_result_inline` → "the `Result` wrap in
  `emit_build_result_inline` is reached by any `TRAP`".
- Citations 13: `RawSuccessBlock` (`architecture/04_ir.md:268`) → `mod.rs`;
  `materialize_current_result_boxed` (`memory/03_heap-values.md:221`), `emit_trapped_error_result`
  (`memory/02_fallible-call-abi.md:67`) → `result.rs`; `copy_resource_to_current_arena` ×5
  (`memory/04_arenas.md:459`, `language/15_resource-management.md:350`, `language/16_threads.md:66`,
  `threading/08_queue-semantics.md:100,105`), `copy_value_to_current_arena`
  (`threading/02_isolation.md:60`), `emit_thread_copy_real` (`memory/09_closures.md:135`) →
  `transfer.rs`; `payload_edge_shape` (`memory/05_collections.md:357`),
  `own_collection_payload_edges` ×2 (`05_collections.md:566,1115`) → `collection_transfer.rs`.
- `.ai`: `.ai/collections.md:254` `copy_collection_to_current_arena`
  (`memory/arena/builder_arena_transfer.rs`) → (`memory/arena/builder_arena_transfer/collection_transfer.rs`);
  `.ai/resources-packages.md:123` `materialize_current_result` (`memory/arena/builder_arena_transfer.rs`)
  → (`…/builder_arena_transfer/result.rs`); `:193` `copy_resource_to_current_arena`
  (`builder_arena_transfer.rs`) → (`builder_arena_transfer/transfer.rs`); `:608`
  "the `Result` wrap at `builder_arena_transfer.rs:145`" (stale line) → "the `Result` wrap in
  `emit_build_result_inline` (`builder_arena_transfer/result.rs`)".

### 4.5 `src/codegen/memory/data/data_objects.rs` (1471 + 174 test)

- `data_objects/mod.rs` (640 + 174 test): 1-753 (`push_symbol_address`,
  `push_error_message_address`, `raise_error_into`, `error_status_prefix_symbol`,
  `decode_error_status_prefix_symbol`, `err_msg`, `string_symbols`) + 1033-1079
  (`powers_of_ten_data_object`, `raw_data_object`, `string_data_object`) + 1739-1955
  `#[cfg(test)] mod tests` (stays; `fail_status_prefix_symbol_round_trips`,
  `fail_status_prefix_symbol_rejects_malformed`,
  `typename_fold_agrees_with_the_authoritative_resolver`,
  `unicode_runtime_data_objects_emit_only_referenced_tables`).
- `…/string_values.rs` (463): 755-834 (the detached doc block at 755-757,
  `collect_type_name_values`, `collect_type_name_values_from_ops`) + 1081-1498
  (`collect_string_values_from_function`, `collect_string_values_from_ops_with_constants`,
  `collect_string_values_from_value`, `push_string_value`, `static_string_value_with_constants`).
- `…/unicode_tables.rs` (181): 836-1031 — `unicode_string_call_is_static`,
  `unicode_runtime_data_objects`.
- `…/static_names.rs` (187): 1500-1737 — `static_type_name_with_types`,
  `static_type_name_for_fold_with_types`, `builtin_function_symbol_for_type`,
  `builtin_function_refs`, `collect_function_ref_names`, `collect_builtin_function_refs_in_ops`.
- Visibility: `data/mod.rs` does `pub(crate) use data_objects::*;` → `data_objects/mod.rs` adds
  `pub(crate) use string_values::static_string_value_with_constants;`
  `pub(crate) use unicode_tables::{unicode_runtime_data_objects, unicode_string_call_is_static};`
  `pub(crate) use static_names::{builtin_function_refs, builtin_function_symbol_for_type, collect_function_ref_names, static_type_name_for_fold_with_types, static_type_name_with_types};`
  (8), and `use string_values::*;`. `pub(super)`: 3 in string_values.rs —
  `collect_type_name_values`, `collect_string_values_from_function`, `push_string_value` (called
  by `string_symbols`).
- Citations 1: `unicode_runtime_data_objects` (`unicode/01_tables-and-algorithms.md:250`) →
  `unicode_tables.rs`.
- `.ai`: `.ai/resources-packages.md:486` "`data_objects.rs`'s NIR walk" →
  "`collect_string_values_from_function`'s NIR walk (`memory/data/data_objects/string_values.rs`)";
  `.ai/net-tls.md:42` and `.ai/resources-packages.md:387` "`data_objects.rs`'s per-package gate" →
  "`string_symbols`' per-package gate (`memory/data/data_objects/mod.rs`)" — the gate is the
  `module_uses_any_call(…)` blocks inside `string_symbols` (`awk 'NR>=164 && NR<=753 && /module_uses_any_call/' src/codegen/memory/data/data_objects.rs`).
  Stale (pre-existing, fixed here): `:387` says the gate "pushes `ERR_*_MESSAGE`s"; those
  constants were deleted by plan-88-D (`grep -rn "ERR_[A-Z_]*_MESSAGE" src | grep -v '^src/docs'`
  → only the `err_msg` doc comment saying they are deleted) → "pushes the `_mfb_str_error_*`
  strings, each message read through `err_msg` from `ERRORCODE_CONSTANTS`".

### 4.6 `src/codegen/memory/value/builder_value_semantics.rs` (1325)

One `impl CodeBuilder<'_>` (12-1874), re-opened per file.

- `builder_value_semantics/mod.rs` (224): 1-12 (`use`s) + 865-1058 (`lower_string_concat`,
  `lower_string_concat_chain`) + 1787-1873 (`lower_match_compare`, `result_payload_is_block`).
- `…/default_value.rs` (382): 13-513 — `emit_resource_record_ptr`, `emit_resource_state_init`,
  `emit_closed_resource_record`, `lower_default_value`, `lower_default_value_inner`,
  `default_value_materializable`, `default_record_materializable`.
- `…/field_access.rs` (269): 515-863 — `lower_field_access`, `scalar_field_of_borrowable_get`,
  `get_element_type`, `member_chain_type`, `is_inline_scalar`, `lower_borrowed_chain_base`,
  `lower_field_access_of`, `lower_with_update`.
- `…/static_eval.rs` (450): 1060-1785 — `global_value`, `load_global_address`,
  `local_constant_value`, `static_string_value_vr`, `static_string_value`,
  `static_primitive_text`, `static_item_type`, `static_operand_type`, `static_call_type`,
  `static_composite_type`, `static_type_name`, `member_type_of`, `static_type_name_for_fold`,
  `overload_arg_type`, `thread_runtime_return_type`, `thread_handle_arg_type`.
- Visibility: methods only, no re-export; `pub(super)`: 0 (each of the 14 private methods is
  used only inside its own part).
- Citations 6: `lower_default_value` (`memory/09_closures.md:149`), `lower_default_value_inner`
  (`language/04_types.md:582`), `emit_closed_resource_record`
  (`language/15_resource-management.md:369`) → `default_value.rs`;
  `scalar_field_of_borrowable_get` (`memory/05_collections.md:599`) → `field_access.rs`;
  `static_string_value` (`unicode/02_strings-model.md:125`) → `static_eval.rs`;
  `result_payload_is_block` (`memory/03_heap-values.md:157`) → `mod.rs`.
- `.ai`: `.ai/collections.md:526` `static_item_type` (`src/codegen/memory/value/builder_value_semantics.rs`)
  → (`…/builder_value_semantics/static_eval.rs`); `.ai/resources-packages.md:28`
  `emit_resource_record_ptr` (`builder_value_semantics.rs`) → (`builder_value_semantics/default_value.rs`);
  `:61` `builder_value_semantics.rs:lower_default_value` →
  `builder_value_semantics/default_value.rs:lower_default_value`.

### 4.7 `src/codegen/string/repr/builder_strings.rs` (2018)

One `impl CodeBuilder<'_>` (15-2353).

- `builder_strings/mod.rs` (176): 1-15 (`use`s) + 2165-2352 (`lower_string_comparison_binary`,
  `lower_string_ordering_binary`).
- `…/replace.rs` (763): 16-864 — `lower_replace`, `lower_replace_out`, `lower_list_replace`. Holds
  the file's 2 overflow labels (old 267, 575), scanned through plan-176-O's widening.
- `…/to_string.rs` (463): 866-1435 — `lower_enum_to_string`, `lower_to_string`,
  `lower_boolean_to_string`, `emit_integer_to_string_value`, `emit_byte_list_to_string_value`.
- `…/numeric_text.rs` (616): 1437-2163 — `emit_decimal_sign_prologue`,
  `emit_decimal_integer_render`, `emit_decimal_alloc_and_copy_integer`,
  `emit_decimal_fraction_render`, `emit_fixed_to_string_value`, `emit_money_to_string_value`,
  `emit_float_to_string_value`, `emit_sci_parts_call`.
- Visibility: methods only; `pub(super)`: 0 (the 5 private methods `lower_enum_to_string`,
  `emit_decimal_*` are used only in their own part).
- Citations 5: `lower_replace` ×2 (`unicode/02_strings-model.md:194,200`) → `replace.rs`;
  `lower_to_string` (`language/18_builtin-functions.md:165`), `lower_enum_to_string`
  (`language/13_modules-and-packages.md:260`) → `to_string.rs`; `emit_fixed_to_string_value`
  (`18_builtin-functions.md:36`) → `numeric_text.rs`.

### 4.8 `src/codegen/string/unicode_props.rs` (1513)

- `unicode_props/mod.rs` (511): 1-50 (every `UNICODE_*`/`GRAPHEME_BOUNDCLASS_*`/`INDIC_*`/
  `UTF8_CONTINUATION_*` const) + impl methods 52-307 (`emit_scalar_skip_continuations`,
  `emit_scalar_count_loop`, `emit_load_data_address`, `emit_utf8_decode_next`) + 549-666
  (`emit_utf8_encoded_width`, `emit_utf8_encode_next`) + 1093-1261
  (`emit_string_byte_range_equal_branch`, `emit_unicode_whitespace_branch`).
- `…/properties.rs` (331): impl 309-547 (`emit_unicode_property_lookup`,
  `emit_unicode_range_lookup`, `lower_unicode_range_member`,
  `emit_unicode_property_{boundclass,u16,combining_class,comb_index,comb_length,flags,indic_conjunct_break,charwidth}`)
  + 668-823 (`emit_unicode_u32_mapping_lookup`, `emit_hangul_composition_attempt`).
- `…/grapheme.rs` (246): impl 825-1091 — `emit_grapheme_break_branch`,
  `emit_seed_indic_conjunct_state`, `emit_grapheme_state_update`.
- `…/free_mirrors.rs` (425): 1262-1756 (the "Free-function Unicode primitives" section:
  `emit_load_data_address_free`, `emit_utf8_codepoint_by_len`, `emit_unicode_property_ptr_free`,
  `emit_seed_indic_conjunct_state_free`, `emit_seed_indic_conjunct_state_instrs`,
  `emit_read_boundclass_icb_charwidth_free`, `emit_grapheme_break_branch_free`,
  `emit_grapheme_state_update_free`).
- Visibility: `mod.rs` adds
  `pub(crate) use free_mirrors::{emit_grapheme_break_branch_free, emit_grapheme_state_update_free, emit_load_data_address_free, emit_read_boundclass_icb_charwidth_free, emit_seed_indic_conjunct_state_free, emit_unicode_property_ptr_free, emit_utf8_codepoint_by_len};`
  (7; callers `term.rs`, `term_grid.rs`, `io/input/decode.rs`, `string/format/float_parse.rs`
  import them as `string::unicode_props::…`; `grep -rln "unicode_props::" src`) and
  `use free_mirrors::*;`. `UTF8_CONTINUATION_*` stay in `mod.rs` (used by `builder_search.rs`).
  `pub(super)`: 1 — `emit_seed_indic_conjunct_state_instrs` (free_mirrors.rs; used by
  `emit_seed_indic_conjunct_state` in grapheme.rs, reached through the glob). Private consts stay
  in `mod.rs`.
- Citations 21: `mod.rs` — `UNICODE_PROPERTY_SIZE` (`unicode/01_tables-and-algorithms.md:155`),
  `UNICODE_NFD_ENTRY_SIZE` (:219), `GRAPHEME_BOUNDCLASS_CR` (:348), `emit_utf8_decode_next`
  (:292), `emit_utf8_encode_next` (:293), `emit_scalar_count_loop`
  (`unicode/02_strings-model.md:32`), `emit_unicode_whitespace_branch` (`02_strings-model.md:169`);
  `properties.rs` — `emit_unicode_property_lookup` ×2 (`01_tables…:34,129`),
  `emit_unicode_property_u16` (:130), `emit_unicode_range_lookup` ×2 (`01_tables…:80`,
  `stdlib/01_regex.md:395`), `emit_unicode_u32_mapping_lookup` ×2 (:228, 274),
  `emit_hangul_composition_attempt` (:381); `grapheme.rs` — `emit_grapheme_break_branch` ×2
  (`01_tables…:323`, `unicode/spec.md:21`), `emit_grapheme_state_update` (:328),
  `emit_seed_indic_conjunct_state` (:339); `free_mirrors.rs` — `emit_grapheme_break_branch_free`
  (:341), `emit_grapheme_state_update_free` (:342).

### 4.9 `src/codegen/term/core/term.rs` (2811)

All free functions (`grep -nE "^(pub\(crate\) )?fn " src/codegen/term/core/term.rs`).

- `term/mod.rs` (524): 1-548 (all consts, `kitty_data_objects`, `esc_entries`,
  `mouse_data_objects`, `console_data_objects`, `data_reloc`, `load_data_address`,
  `emit_write_const`, `emit_gate_inactive`, `emit_unsupported`, `lower_term_helper`) + 1198-1241
  (`emit_is_on`, `emit_did_resize`) + 1927-1946 (`struct StampCtx`, kept here so its 13 private
  fields need no edit) + 3371-3454 (`emit_terminal_size`).
- `term/screen_mode.rs` (233): 550-847 — `emit_on`, `emit_off`.
- `term/kitty.rs` (296): 849-1196 — `emit_kitty_enable`, `emit_kitty_keep_byte`,
  `emit_store_input_events`, `emit_kitty_disable`, `emit_kitty_line_window`.
- `term/mouse.rs` (287): 1243-1666 — `emit_enable_mouse`, `emit_mouse_withdraw`,
  `emit_mouse_tracking_window`, `emit_store_mouse_mode`, `emit_mouse_inject`, `emit_poll_mouse`.
- `term/attributes.rs` (164): 1668-1766 (`emit_set_color`, `emit_set_attr`,
  `emit_set_cursor_visible`) + 3271-3369 (`emit_get_color`, `emit_get_attr`).
- `term/draw.rs` (726): 1768-1926 (`emit_clear_grid`, `emit_move_to`, `packed_glyph`,
  `emit_select_glyph`) + 1947-2632 (`emit_stamp_run`, `emit_clear_wide_pair`, `emit_stamp_cell`,
  `emit_load_grid`, `emit_draw_line`, `emit_draw_box`, `emit_fill_rect`).
- `term/text.rs` (581): 2634-3270 — `emit_encode_utf8`, `emit_draw_glyph`, `emit_draw_text`.
- Visibility: `term/core/mod.rs` does `pub(crate) use term::*;` → `term/mod.rs` adds
  `pub(crate) use kitty::emit_kitty_line_window; pub(crate) use mouse::{emit_mouse_inject, emit_mouse_tracking_window, emit_store_mouse_mode}; pub(crate) use text::emit_encode_utf8;`
  (5; callers `gen_read_line_family.rs`, `func_enable_mouse.rs`, `target/linux_gtk/term_draw.rs`)
  and `use attributes::*; use draw::*; use kitty::*; use mouse::*; use screen_mode::*; use text::*;`.
  `pub(super)`: 21 — screen_mode `emit_on`, `emit_off`; kitty `emit_kitty_enable`,
  `emit_kitty_disable` (used by screen_mode); mouse `emit_enable_mouse`, `emit_poll_mouse`
  (`mod.rs`), `emit_mouse_withdraw` (screen_mode); attributes `emit_set_color`, `emit_set_attr`,
  `emit_set_cursor_visible`, `emit_get_color`, `emit_get_attr`; draw `emit_clear_grid`,
  `emit_move_to`, `emit_draw_line`, `emit_draw_box`, `emit_fill_rect` (`mod.rs`),
  `emit_stamp_cell`, `emit_load_grid` (text); text `emit_draw_glyph`, `emit_draw_text` (`mod.rs`).
- Citations 12: `mod.rs` — `emit_gate_inactive` (`app/04_term-backend.md:17`), `emit_did_resize`
  (:181), `KITTY_PROBE_MS` (`stdlib/22_io-input.md:191`); `screen_mode.rs` — `emit_on`
  (`04_term-backend.md:94`); `kitty.rs` — `emit_kitty_enable`, `emit_kitty_disable`,
  `emit_kitty_line_window` (`22_io-input.md:188-190`); `mouse.rs` — `emit_enable_mouse`
  (`04_term-backend.md:237`), `emit_mouse_tracking_window` (:317), `emit_mouse_inject`
  (`tooling/09_debug-report.md:158`); `draw.rs` — `emit_draw_line` (`04_term-backend.md:134`);
  `text.rs` — `emit_draw_text` (`unicode/02_strings-model.md:56`).
- `.ai/resources-packages.md:419` names `src/codegen/term/core/term.rs`'s `emit_set_color` /
  `emit_get_color` "twenty lines away" → `src/codegen/term/core/term/attributes.rs`; both land
  there. The "twenty lines away" wording is inaccurate at HEAD (old 1671 vs 3272) and becomes true
  after the move; it stays.

### 4.10 `src/codegen/term/grid/term_grid.rs` (1502 + 60 test)

- `term_grid/mod.rs` (320 + 60 test): 1-141 (consts incl. `TERM_STATE_GRID_OFFSET`, `HDR_SIZE`,
  `CELL_SIZE`, `C_*`, `WIDE_TRAIL`, `OUTBUF_PER_CELL`, `POOL_BYTES_PER_CELL`, `GLYPH_POOLED_TAG`,
  `TRAILER_SLACK`, `DEFAULT_*`) + 310-652 (`ScrollScratch`, `emit_scroll_back`, `emit_grid_alloc`,
  `emit_grid_free`, `emit_grid_free_previous`, `GridSizeRegs` + impl, `emit_grid_block_free`) +
  1857-1960 `#[cfg(test)] mod tests` (stays; `outbuf_per_cell_covers_the_worst_case_escape_run`,
  `present_write_loop_retries_eintr`).
- `term_grid/present.rs` (524): 142-308 (`append_const`, `AppendScratch`, `append_decimal`,
  `append_rgb`, `append_glyph`) + 1392-1855 (`emit_grid_present`).
- `term_grid/write.rs` (406): 654-1106 — `emit_grid_write`.
- `term_grid/resize.rs` (252): 1108-1390 — `emit_grid_resize`.
- Visibility: `term/grid/mod.rs` does `pub(crate) use term_grid::*;` → `term_grid/mod.rs` adds
  `pub(crate) use present::emit_grid_present; pub(crate) use resize::emit_grid_resize; pub(crate) use write::emit_grid_write;`
  (callers `term.rs`, `gen_write_family.rs`; the inline test calls `emit_grid_present` via
  `use super::*`). `pub(super)`: 0.
- Citations 5: `OUTBUF_PER_CELL`, `TRAILER_SLACK` (`app/04_term-backend.md:448`),
  `POOL_BYTES_PER_CELL` (:456), `emit_grid_free_previous` (:463), `emit_scroll_back` (:970) → all
  `term_grid/mod.rs`.

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
record the number in this line as the P baseline (est. 4 min).

Every phase's Check also runs: `cargo check --all-targets` → no new warning (est. 2 min);
`cargo test -p mfb --bin mfb spec_citations_resolve` → pass (est. 1 min);
`grep -rn "<each old path of the phase>" src/docs .ai tests` → none (est. 5 s); and
`cargo test --test file_size_budget 2>&1 | grep -E '<the phase's old paths>'` → one `STALE` row per
old (file, kind) ceiling before `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`, none
after, and no `NEW` row for a part file (est. 2 min). The tightened ceiling file is committed with
the phase.

### Phase 1 — I/O: `io/input/decode.rs`, `io/stdin/stdin_broadcast.rs`

- [ ] §4.1: split `decode.rs` into `decode/{mod,text,csi}.rs`; 4 `pub(super)` edits; `use csi::*;
      use text::*;`; rewrite the 2 citations.
- [ ] §4.2: split `stdin_broadcast.rs` into `stdin_broadcast/{mod,next_byte,subscription}.rs`;
      the two `pub(crate) use` lines; rewrite the 4 citations.
- [ ] Tighten and commit the ceiling file.

Acceptance: terminal input decoding and stdin broadcast emit identical code.
  Check: `cargo build --release && cargo test -p mfb --bin mfb codegen::io` → pass, count before
  == after (est. 6 min). `sh scripts/artifact-gate.sh target/release/mfb io`, then `… term`, then
  `… thread` (stdin broadcast serves worker subscriptions) → 0 diffs each (est. 1 min each).
Commit: —

### Phase 2 — Strings: `unicode_props.rs`, `builder_strings.rs`

- [ ] §4.8: split `unicode_props.rs` into `unicode_props/{mod,properties,grapheme,free_mirrors}.rs`;
      the 7-item re-export and `use free_mirrors::*;`; 1 `pub(super)` edit; rewrite the 21
      citations.
- [ ] §4.7: split `builder_strings.rs` into `builder_strings/{mod,replace,to_string,numeric_text}.rs`;
      rewrite the 5 citations.
- [ ] Tighten and commit the ceiling file.

Acceptance: string and Unicode lowering emit identical code, and the 2 overflow labels in
`replace.rs` stay under the overflow-label scan.
  Check: `cargo build --release && cargo test -p mfb --bin mfb -- codegen::string no_overflow_label_returns_through_the_result_tag_register`
  → pass, count before == after (est. 6 min). `sh scripts/artifact-gate.sh target/release/mfb strings`,
  `… regex`, `… money`, `… term` → 0 diffs each (`money` renders fixed/money text; `term` uses the
  free mirrors; est. 1 min each).
Commit: —

### Phase 3 — Terminal: `term/core/term.rs`, `term/grid/term_grid.rs`

- [ ] §4.9: split `term.rs` into `term/{mod,screen_mode,kitty,mouse,attributes,draw,text}.rs`;
      the 5-item re-export and six private globs; 21 `pub(super)` edits; rewrite the 12 citations;
      `.ai/resources-packages.md:419`.
- [ ] §4.10: split `term_grid.rs` into `term_grid/{mod,present,write,resize}.rs`; the three
      `pub(crate) use` lines; rewrite the 5 citations.
- [ ] Tighten and commit the ceiling file.

Acceptance: the terminal backend and grid emit identical code on every target.
  Check: `cargo build --release && cargo test -p mfb --bin mfb codegen::term` → pass (incl. the 2
  grid tests), count before == after (est. 6 min). `sh scripts/artifact-gate.sh target/release/mfb term`
  and `… io` → 0 diffs each (est. 1 min each). The app-mode `.app.ncode` goldens outside
  `tests/byte-identity` are swept by the letter-end `all` run.
Commit: —

### Phase 4 — Memory values: `data_objects.rs`, `builder_value_semantics.rs`

- [ ] §4.5: split `data_objects.rs` into `data_objects/{mod,string_values,unicode_tables,static_names}.rs`;
      the 8-item re-export and `use string_values::*;`; 3 `pub(super)` edits; rewrite the 1
      citation; `.ai/resources-packages.md:387, 486` (incl. the stale `ERR_*_MESSAGE` fix),
      `.ai/net-tls.md:42`.
- [ ] §4.6: split `builder_value_semantics.rs` into `builder_value_semantics/{mod,default_value,field_access,static_eval}.rs`;
      rewrite the 6 citations; `.ai/collections.md:526`, `.ai/resources-packages.md:28, 61`.
- [ ] Tighten and commit the ceiling file.

Acceptance: static data and value semantics emit identical code.
  Check: `cargo build --release && cargo test -p mfb --bin mfb -- codegen::memory::data codegen::memory::value`
  → pass (incl. the 4 `data_objects` tests), count before == after (est. 6 min).
  `sh scripts/artifact-gate.sh target/release/mfb general`, `… strings`, `… collections`,
  `… fs` (resource default values) → 0 diffs each (est. 1 min each).
Commit: —

### Phase 5 — Arena: `arena/arena.rs`, `builder_arena_transfer.rs` (largest blast radius)

- [ ] §4.3: split `arena.rs` into `arena/arena/{mod,alloc,free}.rs`; the two `pub(crate) use`
      lines; rewrite the 8 citations.
- [ ] §4.4: split `builder_arena_transfer.rs` into `builder_arena_transfer/{mod,result,transfer,collection_transfer}.rs`;
      5 `pub(super)` edits; the `include_str!` → `"../../../error/constants/error_constants.rs"`;
      the old-909 comment fix; rewrite the 13 citations; `.ai/collections.md:254`,
      `.ai/resources-packages.md:123, 193, 608`.
- [ ] §4.4: `tests/codegen/codegen_trap_result_wrapper.rs` expected path →
      `src/codegen/memory/arena/builder_arena_transfer/result.rs` (4-point justification in the
      commit message).
- [ ] Tighten and commit the ceiling file.

Acceptance: every program's allocator and every cross-arena copy emit identical code.
  Check: `cargo build --release && cargo test -p mfb --bin mfb codegen::memory::arena` → pass
  (the 2 zero-fill and 3 transfer tests), count before == after (est. 6 min).
  `cargo test --test codegen_trap_result_wrapper` → pass (est. 2 min).
  `sh scripts/artifact-gate.sh target/release/mfb <s>` for `s` in `general`, `thread`,
  `resource-xfer-slots`, `collections`, `vector` (SIMD list alloc) → 0 diffs each (est. 1 min each).
Commit: —

## Validation Plan

- Tests: none added or removed. Whole-binary count
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` → equals the P baseline
  (est. 4 min); `cargo test --test codegen_trap_result_wrapper -- --list 2>/dev/null | grep -c ': test$'`
  → same before and after.
- Coverage check: every moved line stays under its `src/codegen/…` directory, inside the coverage
  denominator; no coverage exception names these files.
- Runtime proof: byte-identity. Letter end, once: `sh scripts/artifact-gate.sh target/release/mfb all`
  → `0 diff(s)` (est. 15–20 min; the allocator and data objects are in every fixture's native
  code on every target, and the app-mode term goldens live outside `tests/byte-identity`, so only
  the full sweep covers them).
- Guards: `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings`
  → pass (est. 2 min); `cargo test -p mfb --bin mfb no_overflow_label_returns_through_the_result_tag_register`
  → pass.
- Budget: `cargo test --test file_size_budget 2>&1 | grep -E 'io/input/decode|stdin_broadcast|arena/arena|builder_arena_transfer|data_objects|builder_value_semantics|builder_strings|unicode_props|term/core/term|term_grid'`
  → no row; the same pattern over `tests/guards/file_size_ceilings.txt` → no `ceiling` row.
- Doc sync: `spec_citations_resolve` → pass; the §2 citation pattern over every old path → 0. No
  `mfb man` page cites these files. No compiler contract changes.
- Full workspace suite: once, in plan-176-V's Validation Plan.

## Decisions

- Rule revised — owner, 2026-10-18: three kinds, non-comment non-blank LOC, scope `.rs`/`.mfb`
  under `src/`, `packages/`, `examples/`, `repository/`, `tests/`: `code` (a non-test-only file)
  ≤ 1000 → split into modules; `inline` (`#[cfg(test)]`/`TESTING` LOC in a non-test-only file)
  ≤ 250 → move to a test-only file; `test` (a test-only file) ≤ 1000 → split into modules. Parts
  target ≤ 900. Ceiling rows are `ceiling <code|inline|test> <n> <path>`. No exemptions except
  `GENERATED` files; the guard warns with a banner, never fails; a new module is always an
  acceptable split. P's inline test modules are each < 250 (80, 80, 174, 60) and stay inline.
- A single function over the code limit is decomposed by extract-method only in S–V — owner,
  2026-10-14. No P function is over the limit (largest `lower_arena_alloc`, 707).
- P relies on plan-176-O Phase 1's prefix-scan widening for its three `builder_*` splits —
  parent planner, 2026-10-14.
- The source-scanning test's expected path moves with `fresh_trapped_result_value`, with the
  AGENTS.md 4-point justification — author.
- Pre-existing defects fixed here: the stale `:145` line references (code comment and
  `.ai/resources-packages.md:608`) and the deleted `ERR_*_MESSAGE` constants named in
  `.ai/resources-packages.md:387` — author (AGENTS.md "never leave a bug").
- No exemptions exist (rule revised 2026-10-18); P adds no guard row.

## Corrections

- 2026-10-18: Rule revised by the owner (code ≤ 1000, inline ≤ 250, test-only file ≤ 1000, no exemptions); file list and split design unchanged; rule text restated; `io/input/corpus.rs` (903) is no longer an exempt file J splits — it stays whole and untouched.

## Summary

The engineering risk is in Phase 5: the arena allocator and cross-arena transfer are in every
program and every thread hand-over. Byte identity over every fixture and target, plus the one
source-scanning test kept pointing at the single `Result` constructor, contains it. No
caller-visible path, emitted byte or test changes.
