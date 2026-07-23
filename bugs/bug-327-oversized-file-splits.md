# bug-327: ~20 files over 1,000 lines hold unrelated concerns; every one has a verified split seam

Last updated: 2026-07-22
Effort: huge (>3d — this is a multi-week work order; land it file-by-file, one commit per file, never as a batch)
Severity: LOW
Class: Other (cleanup / file organization)

Status: Open — Phase 0 + 8 splits landed (Tier 1: T1-1, T1-2, T1-3, T1-4, T1-7,
T1-8; Tier 2: T2-6, T2-9), each verified byte-identical (artifact-gate 0 diffs +
acceptance 1080). Remaining: T1-5/T1-6 blocked on `tests/common/mod.rs`
(Agent 21 #2); T1-9 blocked on bug-330 (audio dedup); the rest of Tier 2 (T2-1..T2-5,
T2-7, T2-8, T2-10 + the glob→explicit conversion) and Tier 3 (T3-1, T3-2) not yet
started. The `shared/code/` Tier-2 items (T2-3/4/5/8) are intertwined with the
glob→explicit conversion (a bidirectional-namespace change) — sequence them
together. T2-7 has a hard blocker (Agent 09 #1 constfold dup, extract to
`nir/constfold.rs` first). See the Phases checklist.
Regression Test: artifact gate + acceptance suite per split — `scripts/artifact-gate.sh <exe>` and `scripts/test-accept.sh <exe>`. **Byte-identical generated output is the acceptance criterion**; there is no new behavioral test, because a pure file split emits no new bytes.

The cleanup review measured ~20 files between 1,000 and 5,268 lines whose
contents span concerns that never interact. The problem is not the line count —
it is that a reader looking for one concern must page past three others, and
that a symbol's definition routinely sits thousands of lines from its only
caller (`emit_geometric_step` is defined at
`src/target/shared/code/builder_collection_mutate.rs:3434`, 2,539 lines after
its first caller at `:895`; `lower_devices` is the **last** function in
`src/target/shared/code/audio/macos.rs` at `:2602` while its dispatcher lists
`audio.devices` **first** at `:70`). Agent 04 #13 showed the concrete cost: a
duplicate pair (`type_utils::align` / `types::data_align`) survived unnoticed
because both files are dissolved into one flat glob namespace.

Every proposal below was re-verified against the current tree: line counts
measured with `wc -l`, seams confirmed by opening the file and listing symbol
boundaries, and — where a reviewer asserted a property that makes a split safe
(zero cross-references, exactly one call site per function, existing section
banners) — that property was checked directly. Two reviewer claims did not
survive; both are corrected in place below rather than dropped.

The single correct outcome: each file below is split along the seam recorded
here, the compiler emits **byte-identical machine code, relocations, and
artifact dumps**, and the acceptance goldens do not move. Nothing about a
compiled binary may change.

References:

- `/tmp/cleanup-findings/index.md` — Agent 01 #4, Agent 04 #3/#4/#20, Agent 05 #1,
  Agent 06 #1/#9/#10, Agent 07 #5, Agent 08 #9, Agent 09 #13, Agent 12 #4/#7,
  Agent 13 #9, Agent 14 #4, Agent 15 #3/#8/#11/#12, Agent 18 #10, Agent 21 #17/#18.
- `bugs/bug-322-arena-alloc-boilerplate.md` — the duplication work order these
  splits interleave with. See "Ordering against bug-322" below.
- `src/ast/testing.rs` (154 lines) — the in-tree one-construct-per-file precedent.
- `src/builtins/strings.rs:266-267` — the in-tree proof that a `.mfb` package
  source can be split across files and concatenated at load.

## Current State

Measured on the current tree. "Concerns" = groups of symbols that do not call
into each other and would compile unchanged in separate files.

| File | LOC | Distinct concerns | Worst locality symptom |
| --- | --- | --- | --- |
| `src/ir/verify/mod.rs` | 5268 | 13 rule groups in ONE `impl TypeEnv` | `impl` block runs `:533-4780` (4,248 lines) with **zero** section banners |
| `src/target/shared/code/builder_collection_mutate.rs` | 4471 | 4 (dispatch / list / map / buffer) | `emit_geometric_step` defined 2,539 lines after first caller |
| `src/ir/lower.rs` | 4036 | 8 pipeline stages | `LowerContext` declared at `:894`, first used at `:718` |
| `src/target/shared/code/tls/macos.rs` | 3960 | 3 (support / client / server) + tests | 1,712-line client half and 1,712-line server half share 3 helpers |
| `src/syntaxcheck/mod.rs` | 3332 | ~6, one of which never got a topic module | 680 lines of native-LINK checking in the module root |
| `src/cli/build.rs` | 2946 | 5 peer concerns + a 618-line hub fn | `build_project` is `:240-857` |
| `src/target/shared/code/audio/macos.rs` | 2884 | 4 (open / write / read / query+devices) | dispatcher order is the reverse of definition order |
| `src/target/shared/code/builder_codegen_primitives.rs` | 2437 | 6 | register alloc, error blocks, and 3 cleanup subsystems in one `impl` |
| `src/target/shared/code/entry_and_arena.rs` | 2379 | 5 (incl. a complete PCG64 RNG) | RNG at `:2019-2216` has nothing to do with program entry |
| `src/target/shared/code/io_helpers.rs` | 2290 | 3, **interleaved** | terminal-mode block `:785-1060` splits the stdin readers in half |
| `src/builtins/crypto_package.mfb` | 2262 | ~13 primitives | 22 banner-marked sections already mark every seam |
| `src/target/shared/code/os.rs` | 2116 | 4 | `emit_copy_counted:1874` defined after both callers (`:1779`, `:1799`) |
| `tests/repo_acceptance.rs` | 1968 | 5 | 18 independent `#[test]`s serialized into one binary |
| `src/target/shared/validate.rs` | 1720 | 5 | `validate_ops` + `validate_value` = 749 lines (44%) in 2 functions |
| `src/ast/items.rs` | 1621 | 3 unrelated parsers | items / LINK / DOC, no shared state |
| `src/manifest/package.rs` | 1562 | 4 + a layering inversion | hand-rolls `.mfp` decode that `src/binary_repr/` owns |
| `tests/native_io_runtime.rs` | 1333 | 2 (`io::` + `term::`) | `build_short_write_interposer:1199` sits below 20 tests |
| `src/testing/desugar.rs` | 1326 | 5 | `:906-1136` is 40 generic AST constructors, nothing test-specific |
| `src/doc.rs` | 1098 | 2 | `const STYLE` stranded at `:1053`, below `#[cfg(test)]` at `:635` |
| `src/target/shared/code/fs_helpers.rs` | 153 | 0 — a **vestigial name** | 2 functions; 3 flat `fs_helpers_*` siblings total 6,657 lines |

## Root Cause

Append-only growth with no structural checkpoint. Two mechanisms:

1. **A flat glob namespace removes the cost signal.**
   `src/target/shared/code/mod.rs` glob-imports **18** sibling modules
   (`:3061-3151`, measured — Agent 04 #20 said 20), and **53 of the 64** `.rs`
   files in that directory open with `use super::*;`. The whole subtree is one
   namespace, so moving a symbol between files in it costs nothing and buys
   nothing at the `use` line. That is exactly why `type_utils::align` and
   `types::data_align` could coexist undetected (Agent 04 #13). **This is the
   central interaction for ranking**: splits inside `shared/code/` are the
   *lowest compile risk* in this document and the *lowest navigability payoff*
   until the globs become explicit `use` lists.

2. **No file-size or ordering convention exists.** Agent 22 #8 measured 118
   inline test modules vs 12 sibling `tests.rs` (26,686 lines) with the split
   *not* size-driven — `shared/code/tests.rs` was extracted at 131 lines while
   `syntaxcheck/inference.rs` keeps 1,085 lines inline.

## Goal

- Each file in the Current State table is split along the seam recorded in its
  item below, with each split landed as its own commit.
- `scripts/artifact-gate.sh` and `scripts/test-accept.sh` are green after every
  individual split, with **zero golden diffs**.
- After the `shared/code/` splits land, `mod.rs`'s 18 glob imports are converted
  to explicit `use` lists (this is what converts low-risk motion into an actual
  navigability gain).

### Non-goals (must NOT change)

- **Any emitted byte.** Machine code, relocations, data-object contents and
  order, symbol names, `.mfp` wire format, artifact dumps (`-ir`, `-nir`,
  `-plan`, `-obj`, `-mir`, `-ast`), and every acceptance golden must be
  byte-identical before and after each split. If a golden moves, the split was
  not a split.
- **Any behavior, diagnostic, rule code, or diagnostic ordering.** Rule
  emission order in `ir/verify` is observable through goldens; reordering the
  rule groups within `impl TypeEnv` is **forbidden** — only the file boundary
  moves.
- **Merging duplicates while splitting.** Tempting and wrong: several of these
  files are also cited in bug-322 and in the duplication findings. Deduplicating
  during a move makes the "byte-identical" check meaningless, because a real
  behavioral diff and a move artifact become indistinguishable in one commit.
  Move first, dedupe in a separate commit. See "Ordering against bug-322".
- **Renaming symbols during a move.** Same reason. `fs_helpers.rs`'s misleading
  name (item T2-4) is fixed by *creating a directory*, not by renaming functions.
- **Deleting dead code during a move.** `syntaxcheck::check_link_function`
  (`src/syntaxcheck/mod.rs:688-696`) is dead (Agent 13 #6). Extracting the LINK
  topic module moves it as-is; deleting it is a separate bug's job.

## Blast Radius

Actual search, not memory. The blast radius of a pure file split is the set of
`use` sites that must be updated, plus any `path:line` citation that becomes
stale.

- **`src/target/shared/code/*` splits** — blast radius is *nil* at the `use`
  level: 53 of 64 files in the directory do `use super::*;` and `mod.rs`
  glob-re-exports 18 of them, so new sibling modules are visible without
  touching any consumer. Only the `mod` declaration block needs a new line.
- **`src/ir/lower.rs`, `src/ir/verify/mod.rs`** — `src/ir/mod.rs` re-exports;
  `pub(crate) fn collect_project_docs` has exactly one external consumer
  (`src/cli/build.rs:647`, already via `ir::collect_project_docs`).
- **`src/ast/items.rs`** — all 32 items are `impl Parser` methods reached
  through `self`; splitting into multiple `impl Parser` blocks in sibling files
  requires only `mod` lines in `src/ast/mod.rs`. Three trailing free functions
  (`normalize_ws:1576`, `split_first_word:1581`, `dedent:1597`) have external
  consumers via `src/ast/mod.rs:22`.
- **Spec citations** — the specs carry 1,263 `[[path:Symbol]]` citations, of
  which Agent 19 #24 already found 10 broken from *earlier* file→directory
  moves. Every split in this document creates the same hazard. **Each split
  commit must re-run a citation resolution sweep** over `src/docs/spec/`. Agent
  19 #27 notes there is no automated guard; the 5 raw-line-number spec anchors
  into `builder_values.rs` (Agent 02 #10) are the known-fragile ones.
- **Not affected**: `repository/` (a separate workspace, Agent 21 #3),
  `benchmark/`, `bindings/`. No file in those trees is in scope.

## Fix Design

Ranked strictly by value ÷ risk. **Tier 1 is lift-and-shift**: every symbol
moved keeps its visibility, no call graph changes, no glob namespace is broken.
**Tier 2** requires a visibility decision or splits a symbol set that another
module reaches through a glob. **Tier 3** requires design work *before* any
motion, because the honest fix is not a move.

Land in tier order. Within a tier, order is arbitrary — the splits are
independent.

---

## Tier 1 — lift-and-shift (no call-graph surgery)

### T1-1 — `src/ir/lower.rs` → extract `src/ir/docs.rs` and the LINK block
*Agent 12 #7. Verified; recommended first split in the whole document.*

Two contiguous, self-contained blocks at the head of a 4,036-line file:

- **DOC collection, `:3-162`** — `doc_prose:3`, `collect_project_docs:13`,
  `function_param_types:151`, `normalize_types:159`. Every caller is inside the
  block (`:64`, `:109`, `:50`) except `collect_project_docs`, called once from
  `:283` and once from `src/cli/build.rs:647` — and it is already
  `pub(crate)`. → `src/ir/docs.rs`.
- **LINK lowering, `:294-652`** — `link_cstructs:294`, `link_functions:323`,
  `link_aliases:411`, `eval_link_const_opt:449`, `eval_link_const:487`,
  `link_const_bits:500`, `lower_bind_in_field:515`, `lower_link_expr:551`,
  `native_resources:594`. Every caller is inside the block except four
  assignments at `:279-282`, all inside `lower_project_with_external_functions`.
  → `src/ir/lower_link.rs` (**not** `src/ir/link.rs`, which already exists at
  719 lines and owns the *C-ABI type predicates* — merging them would conflate
  two concerns).

*Verified non-cross-reference*: `src/audit/collect/source.rs:7` calls a
`link_aliases` that is **its own** local function at `source.rs:594`, not
`ir::lower`'s. Likewise `src/audit/json.rs:163`'s `native_resources` takes a
`report`, not an `ast`. Neither is a consumer.

Remaining stage seams for a later pass (measured, all exact): `lower_statement`
`:979-1442` (464 lines), `expression_type` `:2204-2586` (383), and
`lower_expression_with_expected` `:2868-3658` (791). Those three are the real
mass; they need extraction *within* a stage module and are not lift-and-shift.

### T1-2 — `src/target/shared/code/entry_and_arena.rs` (2,379) → 5 files
*Agent 05 #1. The one-call-site property is verified and is what makes this mechanical.*

**Verified**: all 15 `pub(super) fn` in this file have **exactly one call site
each**, and all 15 are in `lower_module_for_platform` in
`src/target/shared/code/mod.rs` (`:765, 897, 898, 899, 900, 901, 902, 905, 906,
907, 908, 977, 984, 1127, 1128`). No `pub(super) fn` is called from anywhere
else in the tree.

| New file | Symbols | Lines |
| --- | --- | --- |
| `entry.rs` | `lower_program_entry:4`, `emit_entry_args_list_materialization:549`, `emit_cleanup_failure_audit_report:719` | 4-792 |
| `arena.rs` | `lower_arena_alloc:793`, `lower_simd_alloc_list:1376`, `lower_arena_insert_free:1590`, `lower_arena_free:1706`, `lower_arena_destroy:1794` | 793-1463, 1590-1856 |
| `error_result.rs` | `lower_build_error_loc:1464`, `lower_make_error_result:1548` | 1464-1589 |
| `process_lifecycle.rs` | `lower_shutdown:1857`, `lower_signal_handler:1927`, `lower_closure_descriptor_initializer:1960` | 1857-1999 |
| `rng_pcg64.rs` | `emit_pcg_step:2019`, `lower_rng_next:2049`, `emit_rng_draw:2062`, `lower_rng_seed_at:2088`, `emit_seed_dance:2102`, `lower_arena_fill_seed:2138`, `lower_arena_fill_next:2150`, `lower_arena_fill_random:2164` | 2019-2216 |

Two carve-outs before the split, both already filed:

- `pub(super) struct Vregs` (`:2000-2018`) is used by **8 modules**
  (`float_format.rs`, `os.rs`, `fs_helpers_io.rs`, `fs_helpers_atomic.rs`,
  `link_thunk.rs`, `codegen_utils.rs`, `fs_helpers_paths.rs`, and this file).
  It belongs in `codegen_utils.rs` — Agent 05 #8. Move it in the same commit or
  it lands in an arbitrary one of the five new files.
- `emit_write_string_object:2217`, `emit_write_integer_to_stderr:2251`,
  `emit_write_integer_to_stderr_with_labels:2269` are stderr helpers used by
  the entry audit path; keep them with `entry.rs`.

### T1-3 — `src/target/shared/code/tls/macos.rs` (3,960) → 4 files
*Agent 07 #5. **The zero-cross-reference claim is FALSE as stated** — corrected below. The split is still safe, with one helper reassigned.*

The banner is real and sits exactly where claimed:

```
src/target/shared/code/tls/macos.rs:1819
// ===========================================================================
// Server side: tls.listen / tls.accept / tls.closeListener
// (plan-06-tls-server.md §7)
// ===========================================================================
```

Boundaries verified: support `:1-392`, client `:394-1817`, server `:1819-3530`,
tests `:3532-3960` (`mod encoding_error_release_tests` opens at `:3532`).

**Correction.** The reviewer's claim of "zero cross-references between the
client and server halves" does not hold. The server half calls **two** helpers
defined in the client half:

- `emit_dlopen_libssl_macos` — defined `:1762`, called from the server at
  `:3013` and `:3396` (and from the client at `:1085`, `:1464`, `:1655`).
- `emit_dlopen_at` — defined `:1786`, called from the server at `:2242`,
  `:2252`, `:2262` (and from the client at `:1771`).

The reverse direction *is* clean: nothing in `:394-1817` calls any symbol
defined in `:1819-3530` (`emit_read_whole_file:1830`,
`emit_import_pem_item:1918`, `emit_cf_release_slot:2092` are called only from
within the server half).

So the split is 4-way, not 3-way, and the two dlopen helpers move **down** into
support rather than staying with the client:

| New file | Contents |
| --- | --- |
| `tls/macos/mod.rs` | constants `:1-153`, `raw_cstr:154`, `data_objects:165`, `dlsym:210`, `emit_build_block:238`, `emit_fresh_sem:303`, `emit_wait:361`, **plus `emit_dlopen_libssl_macos:1762` and `emit_dlopen_at:1786`** |
| `tls/macos/client.rs` | `lower_tls_connect_macos:394`, `lower_tls_read_macos:1029`, `lower_tls_write_macos:1391`, `lower_tls_close_macos:1626` |
| `tls/macos/server.rs` | `emit_read_whole_file:1830` … `lower_tls_close_listener_macos:3356` |
| `tls/macos/tests.rs` | `:3532-3960` |

Note `emit_dlopen_libssl_macos` does not open libssl (it forwards to
`MACLIB` = Network.framework, Agent 07 #15) — do **not** fix that name in this
commit.

### T1-4 — `src/builtins/crypto_package.mfb` (2,262) → 4-5 sources
*Agent 18 #10. Banners verified; the concatenation mechanism is proven in-tree.*

The seams are already drawn by the author. **Measured: 22 `' --- <topic> ---`
banners plus 2 `' ====` section headers** (the reviewer said 15):

`:48` shared 32-bit helpers · `:116` SHA-256/224 · `:339` 64-bit helpers ·
`:573` HMAC · `:643` HKDF · `:699` PBKDF2 · `:785` little-endian helpers ·
`:811` ChaCha20 · `:902` Poly1305 · `:1079` ChaCha20-Poly1305 · `:1144`
AES-256 · `:1304` GHASH · `:1364` AES-256-GCM · `:1482` constant-time compare ·
`:1509` CSPRNG glue · `:1564` (`====`) Ed25519 · `:1570`-`:2119` its six
sub-banners · `:2134` NIST EC key generation.

**Correction to the Ed25519 range**: the reviewer gave `:1564-2262 (~700
lines)`. The `:2134` banner marks **NIST EC key generation**, a different
primitive. Ed25519 is `:1564-2133` (570 lines); `:2134-2262` (129 lines) is its
own unit.

Suggested: `crypto_hash.mfb` (`:48-784`), `crypto_aead.mfb` (`:785-1481`),
`crypto_util.mfb` (`:1482-1563`), `crypto_ed25519.mfb` (`:1564-2133`),
`crypto_ecdsa.mfb` (`:2134-2262`).

**The mechanism is proven**: `src/builtins/strings.rs:266-267` already
concatenates two `.mfb` sources with
`format!("{}\n{}", include_str!(..), include_str!(..))` before parsing.
`src/builtins/crypto.rs:362-366` uses the identical single-source shape and
extends to N sources with no other change. Ordering matters (MFBASIC resolution
is order-sensitive) — concatenate in the order above.

### T1-5 — `tests/native_io_runtime.rs` (1,333) → 2 test binaries
*Agent 21 #18. Verified exactly.*

Four `native_term_*` tests at `:956`, `:1022`, `:1074`, `:1137` — exactly the
lines cited — sit among 16 `native_io_*` tests. They share only the generic
project/build/PTY helpers, not the `io::` fixtures.

Split → `tests/native_io_runtime.rs` and `tests/native_term_runtime.rs`. The
13 local helpers (`temp_project:7` … `run_pty_prompt_interaction_inner:338`,
plus `build_short_write_interposer:1199` which sits **below 20 tests** — fix
that ordering in the move) go to `tests/common/mod.rs`, which Agent 21 #2
already proposes for the 17-way `temp_project` duplication. Sequence T1-5 after
that helper module exists, or it creates a third copy.

### T1-6 — `tests/repo_acceptance.rs` (1,968) → 4 test binaries
*Agent 21 #17. Verified: 18 `#[test]` functions, 7 shared helpers at `:22-99`.*

The 18 tests are fully independent (each starts its own repo process via
`start_repo:43`); today they are serialized into one cargo target. Split by
concern:

- **identity/auth** — `:101`, `:150`, `:183`, `:212`
- **signing + publish** — `:241`, `:823`, `:1674`, `:1732`
- **install/resolve/lock** — `:534`, `:941`, `:1154`, `:1546`
- **governance** — `:375`, `:671`, `:1071`, `:1257`, `:1360`, `:1436`

Helpers `:22-99` → `tests/common/mod.rs` (same dependency as T1-5).

Note: this file works around Agent 21 #3 (`repository/` is a separate workspace,
so its tests never run in CI) by shelling out to `cargo build --manifest-path`
from inside a test at `:26-40`. **Do not try to fix that here** — splitting the
file does not change it, and fixing it is a workspace change.

### T1-7 — `src/doc.rs` (1,098) → model + renderer
*Agent 15 #12. Verified exactly.*

`:13-380` is the document model (`DocPage:13`, `DocGroup:25`, `Prose:31`,
`DocDecl:37`, plus `from_package:149` and `from_source:198`). `:382-634` is the
HTML renderer (`escape:382` … `render_empty_html:623`). `const STYLE` is
stranded at `:1053`, *below* the `#[cfg(test)]` module at `:635` — the file-order
defect Agent 22 #7 catalogues.

Split → `src/doc/mod.rs` (model) + `src/doc/html.rs` (renderer + `STYLE`, which
returns above the tests as a side effect). Agent 15 #10 separately proposes the
whole feature move under `src/testing/`; that is a different, larger decision —
do this split first, it is compatible with either outcome.

### T1-8 — `src/ast/items.rs` (1,621) → 3 parsers
*Agent 14 #4. Verified exactly at the claimed boundaries.*

Three `impl Parser` regions with no shared state beyond `self`:

- `:4-597` — ordinary top-level items (`parse_top_level_binding:4`,
  `parse_function:43`, `parse_type_decl:208`, `parse_params:373`,
  `parse_visibility:418`, six `check_top_level_*` predicates,
  `parse_top_level_resource:538`, `parse_top_level_func_alias:572`)
- `:598-1268` — LINK (`parse_link_block:598`, `parse_cstruct:683`,
  `parse_link_function:748`, `parse_bind_state:940`, `parse_bind_in:967`,
  `parse_free_block:1036`, `parse_abi_spec:1133`, `parse_const_pin:1208`,
  `parse_optional_state:1257`) — **671 lines**
- `:1269-1575` — DOC (`parse_doc_block:1269`, `parse_header_signature:1514`)
- `:1576-1621` — three generic string helpers

`src/ast/testing.rs` (154 lines, TESTING/TGROUP/TCASE) already sets the
one-construct-per-file precedent. Split → `items.rs` / `link_items.rs` /
`doc_items.rs`. `normalize_ws:1576` is exported from the DOC parser though it
does TYPE-NAME normalization for `resolver`/`ir`/`doc` (Agent 14 #20) — park it
in `items.rs`, and file its real home separately.

### T1-9 — `src/target/shared/code/audio/macos.rs` (2,884) → reorder, then split
*Agent 08 #9. Verified: the ordering defect is exactly as described.*

`lower_audio_macos:56` dispatches `audio.devices` **first** (`:70`); its handler
`lower_devices` is the **last** function in the file at `:2602`. There is no
top-to-bottom order to preserve, which makes this a rare case where reordering
is free.

Split by dispatcher arm: `audio/macos/mod.rs` (dispatcher `:56` + shared
`emit_pthread1:106`, `emit_validate_open:127`, `emit_select_device:484`,
`emit_open_cleanup:582`, `build_propaddr:627`, `call_get_property:641`,
`emit_cfstring_field:681`, `emit_channel_flag:787`, `emit_alloc_byte_list:1557`,
`emit_id_matches:2865`) / `output.rs` (`lower_open_output:183`,
`lower_write:844`, `lower_close_output:1043`) / `input.rs`
(`lower_open_input:1614`, `lower_read:1968`, `lower_close_input:2310`) /
`devices.rs` (`lower_query:1193`, `lower_devices:2602`).

**Do this after bug-322's audio work**, not before: Agent 08 #1 and #2 propose
an `audio/common.rs` extracting ~700-900 duplicated lines shared with
`alsa.rs`, and Agent 08 #6 wants the three overlapping frame-offset const
schemes (`:32-54`, `:163-180`, `:1533-1552`, 7 pairs aliasing exactly) grouped
into modules. Splitting first means doing the const disentangling twice.

---

## Tier 2 — mechanical, but touches visibility or a glob namespace

Every Tier 2 item inside `src/target/shared/code/` is **low compile risk**
(the glob namespace absorbs the move) and **low immediate payoff** for the same
reason. Sequence the glob-to-explicit conversion (Agent 04 #20) alongside them.

### T2-1 — `src/ir/verify/mod.rs` (5,268): add banners first, split second
*Agent 12 #4. Verified — and the cheapest single item in this document.*

**Verified**: `impl TypeEnv` spans `:533-4780` — **4,248 lines, one impl block,
65 methods, zero section banners** (`grep` for indented `// ==` / `// --` inside
the impl returns nothing). By contrast `src/ir/verify/tests.rs` has 76 banners.

The rules do cluster in source order. Measured groups — **13, not 12**:

| # | Range | Concern |
| --- | --- | --- |
| 1 | 534-747 | construction, diagnostic emission, closure arity |
| 2 | 748-1430 | `check_ops` (one 683-line function) |
| 3 | 1431-1844 | value walk, literal ranges, const literals |
| 4 | 1845-1960 | member access + visibility |
| 5 | 1961-2182 | operand typing (binary, money, comparability, map keys) |
| 6 | 2183-2377 | type declarations, union includes, record cycles |
| 7 | 2378-2749 | resource moves, defaultability, collection RES axis |
| 8 | 2750-3382 | native LINK (cstructs + functions) + resource classification |
| 9 | 3383-3595 | match exhaustiveness + patterns |
| 10 | 3596-3955 | call arity/arg types, thread + STATE agreement |
| 11 | 3956-4271 | result-type checks + builtin call args |
| 12 | 4272-4644 | compatibility + typed statement checks |
| 13 | 4645-4780 | type-model lookup helpers (`record_fields`, `union_variants`, `infer_type`, `field_type`) |

**Land the banners as a standalone commit first** — 13 comment lines, zero risk,
and it makes the subsequent split reviewable as pure motion. Then split into
`verify/{ops,values,types,resources,link,calls,matching,compat,lookup}.rs`, each
holding an `impl TypeEnv` block. Rust permits multiple `impl` blocks for one
type across modules in the same crate, so no visibility changes at all — the
only reason this is Tier 2 and not Tier 1 is the sheer size and the fact that
**diagnostic emission order is golden-observable**: preserve the *call* order in
`check_ops` exactly, even as the callee definitions move.

Group 8 (`:2750-3382`, native LINK, 633 lines) is a ~320-line hand-synced mirror
of `syntaxcheck/mod.rs`'s LINK checking (Agent 13 #6). Split them into
same-named files on both sides (`verify/link.rs` and T2-6's
`syntaxcheck/link.rs`) so the unenforced parity is at least visible.

### T2-2 — `src/target/shared/code/builder_collection_mutate.rs` (4,471)
*Agent 01 #4. Four subsystems verified — but they are **not contiguous**.*

`emit_geometric_step` is defined at `:3434` and first called at `:895` —
**2,539 lines earlier**, confirming the reviewer's "2,500 lines" claim. Callers:
`:895, 915, 1303, 1321, 1613, 1632, 2424, 2726, 2745`.

**Correction**: the file is entirely `impl CodeBuilder` methods, and list and
map mutation **interleave**. The split must be by symbol membership, not by line
range:

| New file | Symbols (line = definition) |
| --- | --- |
| `collection_mutate.rs` (dispatch) | `lower_collection_append:4`, `lower_collection_prepend:48`, `lower_collection_insert:93`, `lower_collection_remove_at:250`, `lower_collection_set:283`, `lower_collection_remove_key:438` |
| `list_mutate.rs` | `lower_list_insert_collection:475`, `lower_list_append_in_place:799`, `lower_list_bulk_append_in_place:1230`, `lower_list_prepend_in_place:1517`, `lower_list_set_in_place:1929`, `lower_list_remove_at:3064`, `lower_reserved_list:3563` |
| `map_mutate.rs` | `lower_map_set_in_place:2147`, `lower_map_concat:3800`, `lower_map_remove_key:4119`, `emit_copy_one_map_entry:4355` |
| `collection_buffer.rs` | `collection_argument_as_list_slot:153`, `free_intermediate_collection:201`, `emit_free_pre_grow_buffer:238`, `emit_write_list_header_from_registers:3340`, `emit_write_collection_header_full:3357`, `emit_geometric_step:3434`, `emit_bulk_copy_entries_shift:3482`, `emit_offset_compaction_fixup:3660`, `emit_copy_collection_entries:3705` |

Sequence **after** bug-322 and after Agent 01 #3 (`lower_list_append_in_place`
vs `lower_list_prepend_in_place` share ~350 identical lines) — otherwise the
duplicate pair lands in the same new file and the dedupe is a second rewrite.

### T2-3 — `src/target/shared/code/builder_codegen_primitives.rs` (2,437) → 6
*Agent 04 #3. Six concerns verified; boundaries match the reviewer within 2 lines.*

| New file | Range | Contents |
| --- | --- | --- |
| `builder_registers.rs` | 4-265 | `allocate_register:4`, `allocate_fp_register:57`, `run_register_allocation:95`, `temporary_vreg:178`, `mark_register_used:209`, local-constant save/restore `:222-243`, `allocate_stack_object:244`, `label:256`, `emit:262` |
| `builder_error_emission.rs` | 266-1280 | 12 one-line `emit_*_return` wrappers `:266-313` (Agent 04 #23), checked size arithmetic `:318-374`, error-block plumbing `:375-1280` |
| `builder_thread_cleanup.rs` | 1282-1360 | `is_thread_type:1282` … `deactivate_moved_thread_arguments:1333` |
| `builder_resource_cleanup.rs` | 1361-1808 | `resource_uses_io_buffers:1361` … `collection_resource_close_symbol:1797` |
| `builder_owned_cleanup.rs` | 1809-2021 | `setup_owned_list:1809` … `emit_owned_value_drop:1973` |
| `builder_exits.rs` | 2022-2437 | `emit_cleanup_sequence:2022`, `trap_cleanup_floor:2035`, … |

The 12 wrappers at `:266-313` differ only in error code — Agent 04 #23 wants
them collapsed. Move them intact; collapse separately.

### T2-4 — `src/target/shared/code/fs_helpers.rs`: create `code/fs/`
*Agent 06 #1. The vestigial-name claim is verified and is the clearest structural defect in the tree.*

**Verified**: `fs_helpers.rs` is **153 lines** containing exactly two functions
(`emit_errno_error_mapping:3`, `emit_fs_path_errno_error_mapping:58`), with no
`mod` declaration, no `//!` module doc, and no submodules. Meanwhile
`fs_helpers_paths.rs` (1,961) + `fs_helpers_io.rs` (2,841) +
`fs_helpers_atomic.rs` (1,855) = **6,657 lines** of flat siblings that are *not*
its children. `src/target/shared/code/net/` next door **is** a real directory
module, so the convention exists and fs is the outlier.

Fix: `mkdir src/target/shared/code/fs/`, move the four files to
`fs/{mod,paths,io,atomic}.rs`, with today's `fs_helpers.rs` body becoming
`fs/mod.rs`'s shared errno mapping. Pure motion — all four already do
`use super::*;`, and `code/mod.rs:3079-3086` replaces four `mod`+`use` pairs
with one.

This is the highest-payoff Tier 2 item and the natural first step of the
glob-to-explicit conversion, since `fs/mod.rs` gives the four files a real
namespace boundary for the first time.

### T2-5 — `src/target/shared/code/os.rs` (2,116) → `code/os/`
*Agent 06 #9. Four concerns verified; two ordering defects confirmed.*

- `build_string_from_len:1026` sits **750 lines** below its sibling
  `build_string_from_cstr:276` (reviewer said 675; measured 750).
- `emit_copy_counted:1874` is defined **after both** of its callers (`:1779`,
  `:1799`) — verified, and the plausible cause of `lower_environ:764` and
  `lower_args:1920` open-coding the same copy loop.

| New file | Symbols |
| --- | --- |
| `os/mod.rs` | dispatcher `lower_os_helper:148`, `os_family:190`, `os_arch:199`, `alloc_reloc:209`, `marshal_cstring:224`, `build_string_from_cstr:276`, `build_string_from_len:1026`, `push_alloc_error:339`, `emit_copy_counted:1874`, `emit_store_byte_advance:1904` |
| `os/env.rs` | `module_uses_env_lock:53`, `os_env_lock_init_hex:66`, `emit_env_lock:78`, `emit_env_unlock_return:106`, `lower_get_env:351`, `lower_has_env:494`, `lower_set_env:570`, `lower_unset_env:690`, `lower_environ:764` |
| `os/introspect.rs` | `lower_const_string:1078`, `lower_pid:1136`, `lower_cpu_count:1169`, `lower_host_name:1221`, `lower_user_name:1293`, `lower_args:1920` |
| `os/paths.rs` | `emit_executable_path_into:1403`, `lower_executable_path:1474`, `resource_base_offset:1556`, `lower_resource_path:1573`, `emit_reject_dot_component:1850` |

### T2-6 — `src/syntaxcheck/mod.rs`: extract the LINK topic module
*Agent 13 #9. Verified; the "every sibling has a topic module" claim holds.*

`src/syntaxcheck/` contains `builtins.rs`, `checking.rs`, `helpers.rs`,
`inference.rs`, `resources.rs`, `types.rs` — every concern has a topic module
**except** native LINK, which lives in the root:

- `:351-1030` — `collect_native_resources:351`, `check_resource_decl:394`,
  `check_link_block:403`, `record_fields_of:415`, `check_struct_slots:434`,
  `check_cstruct_escape:614`, `check_link_cstructs:655`,
  `check_link_function:688`, `check_link_function_in:698`. **680 lines**
  measured (reviewer said 640).
- `:1745-1830` — `collect_native_functions:1745`, `native_function_sig:1785`.

→ `src/syntaxcheck/link.rs` as another `impl` block. Note `check_link_function`
at `:688-696` is **dead** (Agent 13 #6) — move it as-is; deleting it here would
make the commit non-mechanical.

### T2-7 — `src/target/shared/validate.rs` (1,720) → 3 files
*Agent 09 #13. Verified exactly, including the 44% figure.*

`validate_ops:793-1210` (418 lines) + `validate_value:1211-1541` (331) = **749
lines, 43.5% of the file, in two functions**. `validate_project:23` is a no-op
stub (`_ir`, `_packages`, returns `Ok`).

| New file | Range | Contents |
| --- | --- | --- |
| `validate/mod.rs` | 16-184 | `validate_target:16`, `validate_project:23`, `validate_nir:27`, `type_owns_resource:119`, `validate_resource_rules:141` |
| `validate/capabilities.rs` | 185-573 | `validate_capabilities:185`, `collect_bind_types:228`, the runtime-call walkers `:259-455`, and `native_constant_value:457` … `native_primitive_text:547` |
| `validate/names.rs` | 574-698 | `unique_global_names:574`, `type_value_names:603`, `unique_function_names:646`, `unique_import_names:677` |
| `validate/body.rs` | 699-1565 | `validate_entry:699`, `validate_function:730`, `validate_param:764`, `validate_ops:793`, `validate_value:1211`, `validate_type_name:1542`, `is_function_type:1550`, `push_unique:1554` |

**Blocking dependency**: `:457-573` is one half of Agent 09 #1 — five
`native_*` constfold helpers duplicated **byte-for-byte** with
`src/target/shared/plan/symbols.rs:709-825`, differing only in five `pub(super)`
keywords. Extract those to `nir/constfold.rs` **first**; otherwise this split
cements the duplicate into a new file.

### T2-8 — `src/target/shared/code/io_helpers.rs` (2,290) → 3 files
*Agent 06 #10. Three concerns confirmed — but, like T2-2, they **interleave**.*

**Correction**: the reviewer's ranges (`:13-623` stdout, `:785-1270` terminal,
`:667-1685` stdin) overlap and are not usable as written. Measured membership:

- **stdout buffering** — `lower_stdout_drain:13`,
  `emit_append_to_stdout_buffer:132`, `lower_io_write_helper:283`,
  `lower_io_flush_helper:515`, `lower_io_is_buffered_helper:578`,
  `lower_io_set_buffered_helper:623` (`:13-666`, contiguous)
- **terminal mode** — `termios_storage_size:785`,
  `emit_configure_stdin_terminal:808`, `emit_restore_stdin_terminal:922`,
  `emit_console_raw_line_mode:994`, `lower_io_is_terminal_helper:1270`
  (`:785-1060` **and** `:1270-1316`)
- **stdin reads** — `lower_io_poll_input_helper:667`,
  `emit_continuation_read:1061`, `emit_stdin_byte_read:1099`,
  `lower_io_read_byte_helper:1146`, `lower_io_read_char_helper:1317`,
  `lower_io_read_line_helper:1685` (`:667-784`, `:1061-1269`, `:1317-2290`)

The terminal-mode block has a **verified external consumer**:
`src/target/shared/code/term.rs:418` calls `emit_configure_stdin_terminal`
(alongside 6 in-file callers at `:1195, 1257, 1377, 1672, 1825, 2262`). That is
the argument for extracting it as a named module rather than leaving it inline —
today `term.rs` reaches it only through the `use super::*;` glob.

### T2-9 — `src/testing/desugar.rs` (1,326) → 5 files
*Agent 15 #11. Verified — and it is **five** jobs, not four.*

The reviewer's first range (`:49-402`) bundles two distinct jobs:

| New file | Range | Contents |
| --- | --- | --- |
| `desugar/expect.rs` | 49-236 | `desugar_case_body:49`, `expand_expect:64`, `expand_eq:90`, `expand_trap:130`, `expand_ntrap:200`, `fail_test:217` |
| `desugar/driver.rs` | 237-403 | `build_driver:237`, `case_call:310`, `assertion_detail:354`, `runtime_detail:363`, `error_location:374`, `summary_line:386` |
| `desugar/coverage.rs` | 404-694 | `instrument_coverage:404` … `dump_list_to_file:639` |
| `desugar/placement.rs` | 695-905 | `validate_expect_placement:695`, `walk_statements:720`, `walk_statement:726`, `walk_expression:818` |
| `ast/build.rs` | 906-1136 | **40 generic AST constructors** (`str_lit:906` … `global_mut:1124`) — verified: not one mentions testing. These do not belong under `src/testing/` at all |

`call_arg_value:888` and `constructor_arg_value:895` are two of the copies in
Agent 13 #7's 5×/5× duplication; move them with `placement.rs` and dedupe later.

### T2-10 — `src/cli/build.rs` (2,946) → 5 files
*Agent 15 #3. Five concerns verified — but the "never call each other" claim needs qualifying.*

**Correction**: `build_project` (`:240-857`, **618 lines**) is a hub that calls
into all four peer concerns — `load_build_signing_info` at `:449`,
`copy_vendor_libraries` at `:559`, `copy_resources` at `:572`,
`run_test_binary` at `:607`, `generate_coverage_report` at `:614`. The accurate
statement is that the **four peers never call each other**; only the hub calls
them. That is still enough for the split, but the hub keeps five new `use`
lines rather than zero.

| New file | Range | Contents |
| --- | --- | --- |
| `build/options.rs` | 147-239, 859-918 | `parse_build_options:147`, `parse_test_options:859` (which copies the former's arms verbatim — Agent 15 #5) |
| `build/mod.rs` | 240-857 | `build_project` (the hub) |
| `build/test_mode.rs` | 919-994 | `generate_coverage_report:919`, `make_temp_output_dir:941`, `run_test_binary:967` |
| `build/signing.rs` | 995-1380 | `signing_ident:995` … `decode_trust_anchor:1370`, plus `apply_signing_metadata:1878`, `executable_signing_metadata_json:1894` |
| `build/native_libs.rs` | 1381-1744, 1846-1877 | `assemble_native_library_table:1381` … `copy_vendor_libraries:1666`, `assemble_native_libraries:1846` |
| `build/resources.rs` | 1745-1845 | `resource_src_fixed_prefix:1745`, `collect_files_recursive:1765`, `copy_resources:1787` |

Tests `:1920-2946` split to match (they already cluster: vendor tests at
`:2682-2788`, resource test at `:2890`).

---

## Tier 3 — needs design work before any motion

### T3-1 — `lower_module_for_platform` (`src/target/shared/code/mod.rs:405-1313`)
*Agent 04 #4. Verified: **909 lines** (reviewer said 915), and the duplication is real.*

This is **not** a file split. The function builds data objects inline at
`:496-661` (string objects `:496-500`, through `data_objects.extend(...)` at
`:659`) and again at `:1220-1283` (`unicode_runtime_data_objects()` at `:1282`),
duplicating the job of `src/target/shared/code/data_objects.rs` — a 1,334-line
module that already owns `string_symbols:69`, `raw_data_object:724`,
`unicode_runtime_data_objects:620`, and `builtin_function_refs:1171`. Between
them, `:663-1148` is a ~485-line runtime-symbol closure of one repeated shape.

Moving the inline blocks into `data_objects.rs` **without first reconciling the
two** would just relocate the duplication. Design decision required: does
`data_objects.rs` own construction (and `lower_module_for_platform` call it), or
does it own only the primitives? Resolve that, then move. The runtime-symbol
closure (`:663-1148`) is separable into `runtime_symbol_closure.rs` on its own
merits and can go first.

Note `mod.rs`'s `mod` declaration block sits at `:3061-3156` of a 3,548-line file
(Agent 04 #2, converging with Agents 01/06/09/10 on the same pattern). Hoisting
it to the top is a **zero-risk one-line-per-item move** worth doing immediately,
independent of everything else in this document.

### T3-2 — `src/manifest/package.rs` (1,562): the layering inversion is a **deletion**, not a move
*Agent 15 #8. Verified — and it is worse than reported.*

Four subsystems, measured (the reviewer's ranges overlap contradictorily; these
are corrected):

- `:47-112` — `file://` URL handling (`package_file_url_path:47`,
  `percent_decode_path:73`, `hex_value:98`)
- `:113-291` — package-name validation `:113` + **hand-rolled `.mfp` container
  decode** `:127-291` (`read_mfp_header:127`, `read_mfp_string:227`,
  `read_mfp_bytes:242`, `read_u16:269`, `read_u32:276`, `read_u64:283`)
- `:292-546` — package metadata + dependencies
- `:547-830` — a bespoke JSON *text* scanner for rewriting `project.json`
  in place (`project_json_with_package:547` … `json_string_end:809`)

**The inversion, verified**: `src/binary_repr/reader.rs:186` declares
`const MFP_MAGIC: [u8; 8] = [0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];`
and `src/manifest/package.rs:10` declares the **byte-identical constant under
the same name**, with duplicated error text ("does not have the MFP package
magic", `reader.rs:191` vs `package.rs:138`) and a duplicated 1.0-version gate
(`reader.rs:196-198` vs `package.rs:144-148`). And `package.rs:6` already reads
`use crate::binary_repr;` — the file imports the module that owns the format and
then re-decodes the header by hand anyway.

So the right fix for `:127-291` is to **delete it** and route `read_mfp_header`
through `binary_repr`, which is a behavioral change (different error strings on
malformed input, possibly different acceptance) and therefore **out of scope for
a file split**. File it separately. The other three subsystems can then be split
mechanically: `manifest/url.rs`, `manifest/package.rs`, `manifest/json_edit.rs`.

---

## Ordering against bug-322

bug-322 (arena-alloc / internal-call / error-tail boilerplate, ~1,500 lines)
targets many of the same files. The two work orders must not run concurrently on
one file, because both claim "byte-identical output" as their proof and a
concurrent edit destroys the signal.

Rule: **for any given file, land the bug-322 dedupe first, then the split.**
Deduplication shrinks the file (sometimes below the threshold that motivated the
split) and changes which symbols cluster. Affected overlaps: T2-2
(`builder_collection_mutate.rs`, Agent 01 #2/#3), T1-9 (`audio/macos.rs`,
Agent 08 #1/#2), T1-3 (`tls/macos.rs`, Agent 07 #4 — 27 `emit_fail` sites in
this file alone), T2-3 (`builder_codegen_primitives.rs`, Agent 04 #23),
T2-7 (`shared/validate.rs`, Agent 09 #1 — a hard blocker, see the item).

Tier 1 items T1-1, T1-2, T1-4, T1-5, T1-6, T1-7, T1-8 have **no bug-322
overlap** and can start immediately.

## Phases

Land one file per commit. There is no meaningful "phase 1 failing test" here —
the test is that nothing changes.

### Phase 0 — establish the invariant harness (once)

- [x] Record a baseline: build the compiler, run `scripts/artifact-gate.sh
      <exe>` and `scripts/test-accept.sh <exe>`, confirm both green **before**
      any split. (2026-07-22: artifact-gate 1318 goldens 0 diffs; acceptance
      1080 tests passed.)
- [ ] Confirm `scripts/artifact-gate.sh` covers the artifact kinds each split
      could disturb. Agent 21 #7 found it has **drifted** from
      `scripts/test-accept.sh` — it is missing rows for the mfp/info/app-mode
      goldens that `test-accept.sh:426-434` checks. Fix that drift first, or the
      gate will pass on splits it should catch.

Acceptance: baseline green; the gate's artifact list matches `test-accept.sh`'s.

### Phase 1 — Tier 1 (7 splits with no bug-322 overlap)

- [x] T1-1 `ir/lower.rs` → `ir/docs.rs` + `ir/lower_link.rs` (commit bfe4cc3ca)
- [x] T1-2 `entry_and_arena.rs` → 5 files + Vregs move (commit d653d5642)
- [x] T1-7 `doc.rs` → model + `html.rs` (commit 7c2782ba8)
- [x] T1-8 `ast/items.rs` → 3 parsers (commit 6dd854d3a)
- [x] T1-4 `crypto_package.mfb` → 5 sources (commit 7e13a7a7a)
- [ ] T1-6 / T1-5 test-file splits — **BLOCKED**: `tests/common/mod.rs` does not
      exist yet. Per T1-5/T1-6 these must land after it (Agent 21 #2's 17-way
      `temp_project` dedup), or they create a third helper copy — and that dedup
      is out of scope for a pure split (a separate bug's job). Deferred until the
      common helper module exists.

Acceptance: after **each** commit, `artifact-gate.sh` and `test-accept.sh` green
with zero golden diffs; `git show --stat` shows only moves plus `mod`/`use`
lines.

### Phase 2 — Tier 1 items gated on bug-322

- [x] T1-3 `tls/macos.rs` → 4 files (commit 73e733de5)
- [ ] T1-9 `audio/macos.rs` → 4 files — **BLOCKED** on the audio dedup
      (Agent 08 #1/#2/#6), which is **bug-330** (Open); `audio/common.rs` does
      not exist yet. Splitting first would disentangle the frame-offset const
      schemes twice — land bug-330's audio work first.

Acceptance: as Phase 1.

### Phase 3 — Tier 2, interleaved with the glob-to-explicit conversion

- [ ] T2-1 **banners first** (13 comment lines, standalone commit), then split
      `ir/verify/mod.rs`
- [ ] T2-4 `code/fs/` directory — do this **first** among the `shared/code/`
      items; it is the natural anchor for the glob conversion
- [ ] Convert `src/target/shared/code/mod.rs`'s 18 glob imports (`:3061-3151`)
      to explicit `use` lists; delete the `align`/`data_align` duplicate this
      exposes (Agent 04 #13)
- [x] T2-6 `syntaxcheck/link.rs` extracted (commit cb2d3b8ac)
- [x] T2-9 `testing/desugar.rs` → desugar/ + ast/build.rs (commit 6102f80b3)
- [ ] T2-5, T2-7, T2-8, T2-10, T2-3, T2-2 remain

Acceptance: as Phase 1, plus the explicit-`use` conversion compiles with no
`pub(super)` widened to `pub(crate)`.

### Phase 4 — Tier 3 design items

- [ ] T3-1: decide `data_objects.rs` ownership, then move
- [ ] T3-1 side-quest: hoist `code/mod.rs`'s `mod` block from `:3061` to the top
- [ ] T3-2: file the `.mfp`-decode-via-`binary_repr` change as its own bug, then
      split the remaining three subsystems

Acceptance: T3-1's move introduces no duplicate construction path; T3-2's split
commit contains no `.mfp` decoding change.

## Validation Plan

- **Regression test**: none new. The regression guard is the existing artifact
  gate plus the acceptance suite, run per split. A pure file split changes no
  emitted byte, so **any** golden diff is a defect in the split, not a golden
  that needs regenerating. Do not run `scripts/sync-goldens.sh` during this
  work — needing it means the split was wrong.
- **Runtime proof**: `scripts/test-accept.sh <exe>` end-to-end (it links and
  runs, unlike the execution-free gate). For T1-3 and T1-9 also exercise the
  platform paths the gate cannot reach.
- **Doc sync**: after each split, sweep `src/docs/spec/` for `[[path]]` and
  `[[path:Symbol]]` citations into the moved file. Agent 19 #24 found 10 already
  broken by earlier file→directory moves, and Agent 19 #27 confirms there is no
  automated guard — a ~20-line resolution check written during Phase 0 would pay
  for itself across ~20 splits.
- **Full suite**: `cargo test` + `scripts/artifact-gate.sh` +
  `scripts/test-accept.sh`, per commit.

## Open Decisions

- **Glob conversion timing** — recommended: convert `code/mod.rs`'s 18 globs
  *during* Phase 3, anchored on T2-4's `fs/` directory, rather than before
  (churns 53 files against in-flight splits) or after (Tier 2 delivers no
  navigability gain until it happens). (§Root Cause 1)
- **`ir/verify` split granularity** — recommended: 9 files from the 13 measured
  groups (merge 1+13 into a `mod.rs` core; merge 9 into the matching file) vs.
  one file per group (13 files, several under 200 lines). (§T2-1)
- **`doc.rs` destination** — recommended: split in place now (`doc/mod.rs` +
  `doc/html.rs`), leaving Agent 15 #10's move under `src/testing/` as a separate
  decision, vs. doing both at once. (§T1-7)
- **`tests/` naming** — recommended: adopt Agent 21 #31's prefix convention
  (`mfbcmd_*.rs`) when splitting T1-5/T1-6, since cargo requires integration
  tests at `tests/` root and the split adds 5 more files there. (§T1-6)

## Summary

The engineering risk here is **not** in any individual split — every one was
verified to be symbol motion with a known consumer set, and the byte-identical
acceptance criterion makes a bad split loudly visible. The risk is in
**sequencing**: splitting a file that bug-322 is about to dedupe wastes the work
twice and destroys the byte-identical signal that proves both correct. Hence the
per-file rule (dedupe, then split) and the per-commit gate.

Three reviewer claims were corrected on verification and one materially changes
a design: `tls/macos.rs`'s client and server halves are **not** cross-reference
free (the server calls two client-half dlopen helpers, which must move to shared
support instead), `builder_collection_mutate.rs` and `io_helpers.rs` have
**interleaved** rather than contiguous concerns (so their splits are by symbol
list, not line range), and `manifest/package.rs`'s layering inversion is a
**deletion in favor of `src/binary_repr/`**, not a move — which puts it out of
scope for a split entirely.

Left untouched: every emitted byte, every diagnostic and its ordering, every
duplicate (bug-322's job), every dead item (its own bug's job), and every
misleading symbol name.
