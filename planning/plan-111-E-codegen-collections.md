# plan-111-E: type codegen's collections and layout cluster

Last updated: 2026-08-29
Effort: large (3h–1d)
Depends on: plan-111-D (the scalar cluster is typed; the element/key/value types
these files thread are the same `ParameterType` values D's emitters now accept).

The second mechanical codegen letter: **collections and layout** — the List/Set/
Map layout builder, the comparison engine, the mutation and loop paths, and the
per-function collection builtins.

161 violation sites across 25 files (§2), and the single worst file in the whole
compiler: `builder_collection_layout.rs` at 55 sites, 31 of them `&str` type
parameters. That file is where a type spelling gets decomposed to decide a
memory layout, which is exactly the work `ParameterType`'s variants exist to do.

See plan-111-A for the shared prerequisites, the five sanctioned boundaries, the
tiered gate policy, and the rejected alternatives.

References:

- `src/codegen/collection/layout/builder_collection_layout.rs` — 55 sites;
  `:2459` is `refined_list_literal_type`, whose `format!("List OF {element}")`
  letter C already removed (plan-106-E Correction 4). Verify that at kickoff.
- `src/codegen/collection/compare/builder_collection_compare.rs` — 15 spelling
  arms + 5 `&str` params.
- `.ai/collections.md` — List/Map/Set codegen: memory management, in-place
  mutation, native lowering, and the HOF-rewrite tradeoffs these files implement.
  **Read this before starting.**
- `.ai/codegen-invariants.md` — record layout and the inline-headroom rules;
  the `for_each_iterable_locals` aliasing hazard lives in this cluster.

## Prerequisites

See plan-111-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-111-D complete | D's 15 files read 0 on all six needle classes | **MET** (2026-08-30, `df6956caa`). All 15 absent from `census_by_file`; D's end gate 0 diffs on math/money/vector/strings. |
| Scope re-measured at kickoff | the four census commands from plan-111-A §2, restricted to this letter's file list | UNMEASURED — C and D may have reduced it **Use `census_by_file`, not `rg`** — `cargo test --test no_type_strings census_by_file -- --ignored --nocapture`, with `MFB_CENSUS_DETAIL=<substring>` for the offending lines. `rg` over-counts by including `#[cfg(test)]` modules (Corrections A3, C3) and this letter's §2 table additionally UNDER-counts, because it was built before plan-111-D Correction D1 strengthened three scanners: tuple match arms, `== Some("X")` compares, and ten missing `*type*: &str` parameter names. Expect this letter's real population to be LARGER than §2 says. **MET** (2026-08-30) — see the kickoff table below; it is larger, as predicted. |

### Kickoff re-measurement (2026-08-30)

`cargo test --test no_type_strings census_by_file -- --ignored --nocapture`.
All 25 files in §2 are still live; **none was cleared by C or D**, and the
population is **168, not 161**. The whole delta is `str_type_params`, from the
ten names Correction D1 added to `TYPE_PARAM_NAMES`:

| File (under `src/codegen/`) | §2 | live | delta |
|---|---|---|---|
| `collection/layout/builder_collection_layout.rs` | 55 | **61** | **+6** (`stride_type`, `record_type`) |
| `collection/compare/builder_collection_compare.rs` | 20 | **23** | **+3** (`stride_type`) |
| `builtins/collections/gen_list.rs` | 5 | 5 | — |
| `builtins/collections/func_sum.rs` | 4 | 4 | — |
| every other file | | | — |
| **Total** | **161** | **168** | **+7** |

## 1. Goal

- Every file in §2's list takes and matches `ParameterType`. Zero `&str` type
  parameters, zero spelling match arms, zero spelling compares, zero
  `ParameterType::parse` calls in any of them.
- `CollectionTypeLayout`'s key/value type codes
  (`src/codegen/engine/builder/mod.rs:590`) are derived from `ParameterType`
  variants, not from a decomposed spelling.
- The gate budgets for these 25 files read 0 across all six needle classes.

### Non-goals (explicit constraints)

- See plan-111-A §1 Non-goals — all apply.
- **No layout change.** Element strides, header sizes, inline-headroom capacity
  rules and the `CollectionTypeLayout` code values stay byte-for-byte what they
  are. This letter changes how a layout is *decided*, never what it is.
- **No change to in-place mutation or ownership.** The grow-in-place rules for an
  inlined last collection field, and the arena transfer paths, are untouched
  semantics.
- Do not "fix" the HOF-rewrite tradeoffs or the native-lowering choices described
  in `.ai/collections.md` while converting.
- Do not touch memory/arena/engine files — letter F.

## 2. Current State

The layout builder takes a collection type as a spelling and pulls it apart to
find the element type, the key type, and whether the element is itself a
collection. `ParameterType` models all three structurally
(`ListOf`, `MapOf`, `SetOf`, `MapEntryOf`), so every one of those decompositions
is a variant match that is currently a string operation.

### Measured populations

At HEAD (`fd09ea809`), 2026-08-29, tests excluded. `a` = spelling match arms,
`e` = spelling `==`/`!=`, `p` = `&str` type parameters, `parse` =
`ParameterType::parse` sites.

| File (under `src/codegen/`) | a | e | p | parse | total |
|---|---|---|---|---|---|
| `collection/layout/builder_collection_layout.rs` | 19 | 4 | 31 | 1 | 55 |
| `collection/compare/builder_collection_compare.rs` | 15 | 0 | 5 | 0 | 20 |
| `builtins/collections/gen_map.rs` | 3 | 2 | 7 | 4 | 16 |
| `builtins/collections/func_group_by.rs` | 0 | 3 | 2 | 4 | 9 |
| `collection/list/list_mutate.rs` | 0 | 0 | 8 | 0 | 8 |
| `collection/collection_loop.rs` | 0 | 1 | 6 | 0 | 7 |
| `builtins/collections/gen_memory.rs` | 0 | 2 | 1 | 2 | 5 |
| `builtins/collections/gen_list.rs` | 0 | 1 | 3 | 1 | 5 |
| `builtins/collections/func_sum.rs` | 3 | 0 | 0 | 1 | 4 |
| `collection/search/builder_search.rs` | 0 | 0 | 3 | 0 | 3 |
| `collection/map/map_mutate.rs` | 0 | 0 | 3 | 0 | 3 |
| `builtins/collections/gen_slice.rs` | 0 | 0 | 1 | 2 | 3 |
| `builtins/collections/func_sort_by.rs` | 0 | 2 | 0 | 1 | 3 |
| `builtins/collections/func_partition.rs` | 0 | 1 | 1 | 1 | 3 |
| `builtins/collections/gen_mutate.rs` | 0 | 0 | 2 | 0 | 2 |
| `builtins/collections/func_zip.rs` | 0 | 1 | 1 | 0 | 2 |
| `builtins/collections/func_window.rs` | 0 | 2 | 0 | 0 | 2 |
| `builtins/collections/func_sort.rs` | 0 | 1 | 0 | 1 | 2 |
| `builtins/collections/func_merge.rs` | 0 | 2 | 0 | 0 | 2 |
| `builtins/collections/func_find_last_index.rs` | 0 | 2 | 0 | 0 | 2 |
| `collection/assign/builder_inplace_assign.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/collections/gen_set.rs` | 0 | 0 | 1 | 0 | 1 |
| `builtins/collections/func_transform.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/collections/func_filter.rs` | 0 | 1 | 0 | 0 | 1 |
| `builtins/collections/func_chunks.rs` | 0 | 1 | 0 | 0 | 1 |
| **Total** | **40** | **26** | **75** | **20** | **161** |

### Verified properties

- **`ParameterType` already models every container this cluster decomposes** —
  read `src/types.rs:23-49`: `ListOf`, `SetOf`, `MapOf`, `MapEntryOf`,
  `ResultOf`, `Res`, plus `ThreadHandle`'s `msg`/`res`/`out` planes. The
  decompositions in `builder_collection_layout.rs` have a structural equivalent
  for each; none needs a new variant.
- **UNVERIFIED: whether `refined_list_literal_type`'s `format!` is already gone.**
  Letter C Phase 4 removes it. Confirm at kickoff and mark the task moot if so,
  rather than removing it twice.
- **UNVERIFIED: how many of the 75 `&str` params are on private helpers versus
  cross-module surface.** A private helper converts in isolation; a `pub(crate)`
  one may have callers in letter F's files. Phase 1 task 1 classifies them.

## 3. Design Overview

Four phases, ordered by dependency: the layout builder's *surface* first (so
callers in later phases have a typed thing to call), then the big file's body,
then the comparison engine, then the per-function builtins which are 25 small
independent edits.

Where correctness risk sits:

1. **`builder_collection_layout.rs`'s 31 `&str` parameters.** These form a call
   graph inside one file; converting them piecemeal means a half-typed graph with
   `.name()` renders bridging the gap. Phase 2 converts the graph in dependency
   order, leaf helpers first, so no bridge render is ever written.
2. **`collection_loop.rs` and `list_mutate.rs`.** These touch `FOR EACH`
   iteration over a member collection, where aliasing the buffer is a UAF unless
   member-iterables are tracked (`for_each_iterable_locals` tracks plain locals
   only — the inline-headroom memory). A signature change here must not perturb
   which locals get tracked.
3. **`gen_map.rs`'s 7 `&str` params + 4 parses.** Map key/value threading, where
   `MapOf(k, v)` decomposition replaces a ` TO `-split. Note the standing bug
   class: a leftmost ` TO ` split mis-parses a nested `Map` key (plan-106-E
   Correction 3). If any code in this file splits on ` TO `, converting it is a
   **bug fix**, and it must be called out in the commit and in Corrections
   rather than landed silently.

Per plan-111-A §3, the per-phase gate is `cargo test --no-fail-fast -- --skip artifact_gate_all` —
the `--skip` keeps the full cross-target artifact sweep out of the loop, since
`tests/golden.rs`'s only test shells out to `artifact-gate.sh all`. Goldens,
`test-accept.sh` and the artifact gate are swept **once, in letter G**.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial with one line on what remains; mark
> moot tasks `- [x] ~~text~~ — moot: <evidence>`; fill `Commit:` when a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — classify the layout builder's call graph

No behavior change. Produces the conversion order Phase 2 executes.

- [x] List all ~~31~~ **38** `&str` type parameters in
      `collection/layout/builder_collection_layout.rs` with their visibility
      (private / `pub(crate)`) and their callers, and record the leaf-first
      conversion order in this file. **Done — see the table below.** The 38 hits
      belong to **30 functions**; 7 are file-private with no external caller and
      23 are `pub(crate)`.
- [x] Identify any that are called from letter F's files; those convert here (the
      callee) and their call sites update in F. **Done, and this is the finding
      that reshapes the letter** — see the note below the table.
- [x] Confirm whether `refined_list_literal_type` (`:2459`) still builds a type
      by `format!`; if letter C removed it, mark E's copy of the task moot with
      the commit hash as evidence. **Moot: letter C removed it** (`34f300996`,
      "plan-111-C Phase 4: the last format! type construction"). It is now
      `refined_list_literal_type(declared: &ParameterType, first_element_type:
      Option<&ParameterType>) -> Option<ParameterType>`, a `ListOf` variant match
      building `ParameterType::list_of(element.clone())`, and the caller no
      longer parses the result back.
- [x] Grep this cluster for ` TO ` and ` OF ` string splits and record every hit
      — each is a potential nested-`Map`-key mis-split.
      **Count: 0.** `grep -rn 'split_once(" TO ")|split_once(" OF ")|
      strip_prefix("List OF |strip_prefix("Map OF |strip_prefix("Set OF '
      over `src/codegen/collection src/codegen/builtins/collections` returns
      nothing. So §3's risk 3 — "if any code in this file splits on ` TO `,
      converting it is a **bug fix**" — does not arise: there is no ` TO ` split
      left in this cluster to fix. Every remaining violation here is a spelling
      *match* or a `&str` *parameter*, not a hand-rolled decomposition.

#### The 30 functions, and the conversion order

`in-file` is call sites inside `builder_collection_layout.rs`; `external` is the
files that call it.

| fn | vis | param(s) | in-file | external call sites |
|---|---|---|---|---|
| `collection_payload_types` | private | `type_` | 1 | — |
| `type_is_flat_inner` | private | `type_` | 5 | — |
| `emit_load_payload_with_stride` | private | `stride_type`, `type_` | 2 | — |
| `emit_flat_block_size` | pub(crate) | `type_` | 2 | — |
| `is_pointer_string_record` | pub(crate) | `type_` | 4 | — |
| `record_has_inline_data` | pub(crate) | `record_type` | 3 | — |
| `type_components` | pub(crate) | `type_` | 3 | — |
| `is_pointer_collection_payload_type` | pub(crate) | `type_` | 4 | compare ×3 |
| `collection_payload_alignment` | pub(crate) | `type_` | 4 | map_mutate ×2 |
| `list_element_padding_alignment` | pub(crate) | `type_` | 2 | list_mutate ×6 |
| `type_participates_in_cycle` | pub(crate) | `type_` | 1 | arena_transfer ×1 |
| `record_field_is_pointer` | pub(crate) | `field_type` | 2 | arena_transfer, builder_control |
| `list_block_kind` | pub(crate) | `element_type` | 3 | arena ×3, validation ×1 |
| `emit_wrap_record_in_union` | pub(crate) | `member_type` | 0 | value_semantics, builder_values |
| `thread_copy_symbol` | pub(crate) | `type_` | 0 | arena_transfer, builder/mod ×2 |
| `list_element_is_fixed_width` | pub(crate) | `element_type` | 3 | list_mutate ×2, func_set, engine/tests ×2 |
| `emit_record_block_size_to_slot` | pub(crate) | `record_type` | 4 | arena_transfer, error_emission |
| `type_is_flat` | pub(crate) | `type_` | 4 | arena_transfer ×2, thread_cleanup, builder_values |
| `emit_load_map_payload` | pub(crate) | `type_` | 0 | gen_map ×4, builder_control ×3 |
| `union_is_data` | pub(crate) | `type_` | 6 | marshal/record, value_semantics ×3, arena_transfer ×2, thread_cleanup, builder_values |
| `emit_copy_payload_to_collection` | pub(crate) | `stride_type` | 2 | map_mutate ×4, list_mutate ×5 |
| `emit_element_value_offset` | pub(crate) | `element_type` | 0 | group_by, gen_list, flatten, func_sort ×3, sort_by |
| `emit_load_collection_payload` | pub(crate) | `type_` | 0 | collection_loop ×2, for_each, group_by, gen_list, sort, sort_by, builder_control |
| `record_field_is_inlined` | pub(crate) | `field_type`, `record_type` | 5 | compare, marshal/record ×2, value_semantics ×2, arena_transfer, builder_control ×3 |
| `emit_inlined_block_size_from_ptr_slot` | pub(crate) | `field_type` | 4 | collection_loop, arena_transfer, func_zip, owned_cleanup ×2, thread_cleanup, resource_cleanup, error_emission |
| `inline_collection_payload_size` | pub(crate) | `type_` | 7 | compare ×3, arena_transfer, builder_strings, builder_exits, error_emission |
| `emit_build_inlined_record` | pub(crate) | `record_type` | 0 | marshal/record, value_semantics ×2, crypto ×3, astrings ×2, partition, zip, vector_inline, builder_values |
| `kind2_payload_size` | pub(crate) | `element_type` | 2 | collection_loop ×6, search ×3, list_mutate ×3, for_each, find_last_index ×2, slice, zip ×2, contains, sum ×2, builder_strings |
| `list_entry_stride` | pub(crate) | `element_type` | 15 | list_mutate ×8, marshal/record, arena ×2, gen_memory, gen_mutate, merge, builder_strings |
| `emit_collection_data_pointer_for` | pub(crate) | `element_type` | 3 | **24 files, ~80 sites** — list_mutate ×23, compare ×4, map_mutate ×4, simd_math ×7, simd_float ×5, simd_fixed ×4, func_sort ×4, gen_mutate ×4, merge ×4, gen_pow ×3, slice ×2, zip ×2, map_values ×2, arena_transfer ×2, builder_strings ×2, search, gen_with_any, gen_graphemes, func_split, func_join, grapheme_at, gen_memory, flatten, sum |

**Conversion order = the table's own order** (leaf-first): the three private
helpers, then the four `pub(crate)` ones with no external caller, then the rest
by ascending external fan-out. Following it means no intermediate commit needs a
`.name()` bridge inside this file.

#### The finding: this cluster's surface is the whole of codegen

The plan assumed a handful of these helpers reach letter F. The measurement says
**23 of the 30 are `pub(crate)`, with roughly 250 external call sites across
letters D's, E's, F's and G's files** — `emit_collection_data_pointer_for` alone
has ~80 across 24 files, including files letter D has already converted.

This is not a scope error to re-split; it is the shape of the thing. The layout
builder is codegen's type→memory oracle, so *every* emitter that touches memory
calls it. Two consequences, both recorded rather than worked around:

1. **Letter F is pulled forward.** Converting a callee here forces its call sites
   to compile, so `memory/arena/builder_arena_transfer.rs`,
   `memory/value/builder_value_semantics.rs`, `memory/marshal/record.rs`,
   `cleanup/*` and `engine/control|value` are edited in this letter. F's boxes
   for those sites become moot-with-evidence as they land; F keeps whatever the
   census still shows when E closes.
2. **The direction of travel is right.** Nearly every one of those ~250 sites
   passes `&something.name()` — a render of a `ParameterType` it already holds.
   Typing the callee deletes the render at the caller; it does not create work
   there. That is the opposite of the boundary-render cost letter D paid, and it
   is why this letter shrinks the tree-wide count far more than its own 168.

Acceptance: **MET.** The conversion order is recorded above, every `&str` param
has a visibility and a caller list, and the ` TO `-split census is recorded with
its command and its count (0). No code changed.
Commit: —

### Phase 2 — the layout builder (55 sites, the worst file in the compiler)

- [x] Convert the ~~31~~ **38** `&str` type parameters to `&ParameterType` in the
      leaf-first order from Phase 1, so no intermediate commit needs a `.name()`
      bridge. Followed exactly; the file never held a bridge render.
- [x] Convert 19 spelling arms and 4 compares to variant matches.
- [x] Delete the 1 remaining parse.
- [x] ~~Derive `CollectionTypeLayout`'s `kind`/`key_type_code`/`value_type_code`
      from variants.~~ — **moot: it already does.** `CollectionTypeLayout::from_type`
      takes a `&ParameterType` and matches `ListOf`/`SetOf`/`MapOf` structurally;
      the `code` values come from `payload_type_code`, itself a variant match.
      Nothing in it decomposes a spelling, so there was no derivation to change.
- [x] Lower this file's gate budgets to 0.
- [x] ~~Tests: a layout unit test asserting `CollectionTypeLayout` codes for six
      shapes.~~ — **moot on the same evidence, and replaced by something
      stronger.** A unit test pinning codes that are already derived from
      variants would assert that a match arm returns what it returns. What
      actually needed proving is that the *whole conversion* is byte-neutral,
      and the end-of-letter artifact gate proves it over 28 real goldens across
      every target — including the nested-key and stateful-resource shapes the
      proposed test would have covered synthetically. Recorded rather than
      silently dropped.

Acceptance: **MET.** The file reads 0 on all seven needle classes.
`cargo test --no-fail-fast -- --skip artifact_gate_all` → exit 0, 0 failures.
The tree-wide census moved **584 → 192** on this phase alone.
Commit: 4997520c4

### Phase 3 — comparison, mutation and loop paths (43 sites)

- [x] `collection/compare/builder_collection_compare.rs` — 15 arms, 5 params.
- [x] `collection/list/list_mutate.rs` — 8 params.
- [x] `collection/collection_loop.rs` — 6 params, 1 compare. Verify that
      `for_each_iterable_locals` tracking is byte-for-byte unaffected; a member
      collection iterated by `FOR EACH` must still be tracked exactly as today.
      **Verified, and the strongest evidence is the artifact gate**: the
      `collections` builtin's 7 goldens across every target are byte-identical,
      and a change in which locals get tracked would move the emitted frame.
      The tracking code itself is untouched — this phase changed signatures and
      match forms, not the predicate.
- [x] `collection/map/map_mutate.rs` — 3 params;
      `collection/search/builder_search.rs` — 3 params;
      `collection/assign/builder_inplace_assign.rs` — 1 parse.
- [x] Lower these files' gate budgets to 0.
- [x] Tests: an rt fixture iterating a record's inlined collection field with
      `FOR EACH` and mutating it, pinning the no-UAF behavior across the change.
      **Written: `tests/rt-behavior/arena/member-iterable-mutate`.**

      I first went looking for an existing one to mark this moot, and the two
      candidates do NOT cover it — both iterate a member collection read-only:
      `rt-behavior/arena/flat-record-collection:26` (`FOR EACH n IN b.items`,
      summing) and `byte-identity/collections:123` (`FOR EACH x IN part.matched`,
      printing). The hazard needs iterate **and** mutate, so the fixture is new:

      * `copyItems` iterates the inlined `items` field while each iteration
        rebuilds the whole record (`WITH out { seen := append(out.seen, n) }`),
        so the cursor is live across a reallocation of the block it walks;
      * `sumNested` does the same one level down — a member iterable of a record
        that is itself a collection element;
      * the last block reads the final element through a member iterable and
        then reads element 0 back from the same field, so a cursor that had
        walked a stale buffer would show up as a wrong value rather than a
        crash.

      Golden output `5 3 5 0 5 21 9 7`, each value checked by hand against the
      source before it became a golden (5 appended in order, first 3 and last 5,
      source `start.seen` still empty at 0, `items` intact at 5, nested sum
      1+2+3+4+5+6 = 21, tail 9 and head 7 of `[7,8,9]`).

Acceptance: **MET.** All six files read 0 on all seven needle classes;
`cargo test --no-fail-fast -- --skip artifact_gate_all` → exit 0, 0 failures.
Commit: 4997520c4

### Phase 4 — the per-function collection builtins (63 sites, 18 files)

25 small independent files; batch them by commit but keep each file's change
self-contained.

- [x] `builtins/collections/gen_map.rs` (16) — ~~call out any ` TO ` split removed
      as the bug fix it is~~: there was none to remove. Phase 1's census found
      **zero** ` TO `/` OF ` splits in this whole cluster, so §3's risk 3 never
      arose. The file's real content was 7 `&str` params, 4 parses, 3 arms and
      2 compares, all mechanical.
- [x] `builtins/collections/func_group_by.rs` (9), `gen_memory.rs` (5),
      `gen_list.rs` (5), `func_sum.rs` (4), `gen_slice.rs` (3),
      `func_sort_by.rs` (3), `func_partition.rs` (3).
- [x] `gen_mutate.rs` (2), `func_zip.rs` (2), `func_window.rs` (2),
      `func_sort.rs` (2), `func_merge.rs` (2), `func_find_last_index.rs` (2),
      `gen_set.rs` (1), `func_transform.rs` (1), `func_filter.rs` (1),
      `func_chunks.rs` (1).

      Most of these were already at 0 from Phase 2's cascade. What remained was
      one shared idiom worth naming (**Correction E2**): a monomorph SUFFIX
      compared to a spelling — `target.strip_prefix("#collections_window$")`
      then `t == "String"`. The suffix is a name the NIR symbol carries, so each
      is parsed once at the symbol boundary and the decision below it is on
      variants. Same treatment in `func_partition`, `func_group_by`, `func_zip`,
      `func_chunks`, `func_find_last_index`, `func_merge`, `func_sort_by`,
      `func_window`.
- [x] Lower every remaining budget in this cluster to 0.
- [x] Tests: the collections `rt_*` suite covers these; run it explicitly and
      record the count. **Correction D2's pattern again, third time**: there is
      no collections `rt_*` suite. `ls tests/ | grep -iE 'rt_.*collection'`
      returns `rt_inline_headroom_collection_field.rs` and nothing else. The
      real coverage is `tests/acceptance/src/collections.mfb`, run scoped:
      **passed, 1 test ran**.

Acceptance: **MET.** All 25 files in §2 read 0 on all seven needle classes —
verified by their joint absence from `census_by_file`, not by reading budgets.
Commit: —

### End-of-letter spot-check (scoped, read-only)

Before closing this letter, run the scoped artifact gate on the builtins it
touched — **`collections`, `json`, `csv`** (collections and layout; `json`/`csv` are the heaviest nested-container consumers):

```
scripts/artifact-gate.sh target/release/mfb collections
scripts/artifact-gate.sh target/release/mfb json
scripts/artifact-gate.sh target/release/mfb csv
```

Measured cost: ~31s per builtin (one builtin = 1 test, 6 builds, 7 goldens).
This is **read-only diffing**: it regenerates nothing and updates no golden. It
is multi-target — per-target goldens (`*.linux-aarch64.ncode` and friends) are
discovered by filename and rebuilt with `-target`, so cross-arch drift is caught
on a macOS host, which no other per-letter check can see.

Expect **0 diffs**. A diff here is this letter's, which is the entire point of
running it now instead of discovering it in G behind six letters of churn —
root-cause it with objdump on one fixture and fix the conversion. **Do not
regenerate a golden here.** All regeneration happens once, in letter G, after
attribution (plan-111-A §3).

**Result: 0 diffs on all three, MET.**

```
artifact-gate [collections]: 1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
artifact-gate [json]:        1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
artifact-gate [csv]:         1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
```

21 goldens across every target, byte-identical, after retyping codegen's entire
type-to-memory oracle and its ~250 call sites. Nothing regenerated. `json` and
`csv` are the nested-container consumers, so a `MapOf(MapOf(..), ..)`
decomposition that had gone wrong would surface here rather than in
`collections`.

## Validation Plan

- Tests: `cargo test --no-fail-fast -- --skip artifact_gate_all` — the `--skip` keeps the full
  cross-target artifact sweep out of the per-phase loop (plan-111-A §3), and
  `--no-fail-fast` is required or the `rt_*` tests are silently skipped.
- Gate: `cargo test --test no_type_strings` — all 25 files at 0, budgets tight.
- Coverage check: `.ai/collections.md` documents native-lowering paths that the
  default fixture set may not reach. Confirm the converted lowering paths are in
  the suite's denominator rather than assuming a green run covers them.
- Runtime proof: **deferred to letter G.** No `test-accept.sh` run in this
  letter — the acceptance corpus and its goldens are swept once, at the end
  (plan-111-A §3). The per-phase `rt_*` runtime tests are this letter's
  behavioral signal.

- Artifact gate: **scoped spot-check only** — the builtins above, ~31s each,
  read-only. The full `artifact-gate.sh all`, `tests/golden.rs`,
  `test-accept.sh` and every golden regeneration run once, in letter G.
- Diagnostics: **not run in this letter** — this letter touches codegen, which
  emits no source diagnostics (plan-111-A §3). G re-checks it.
- Doc sync: `.ai/collections.md` if any documented signature changes.
- Formatting: `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **A ` TO `-split found in Phase 1's census** — recommended: convert it and
  report it as a bug fix in the commit and in Corrections, with the nested-`Map`
  input that mis-parses. The alternative (convert silently) hides a real
  correctness improvement inside a refactor and makes the byte-identity
  expectation wrong without explanation. (§Phase 1, §Phase 4)

## Corrections

**E1 — the letter's real reach is ~250 external call sites, and that is the
design, not a scope error.** §2 measured 161 sites in 25 files; the kickoff
re-measurement found 168, and Phase 1's classification then found that 23 of the
30 functions carrying `builder_collection_layout.rs`'s 38 `&str` parameters are
`pub(crate)` with callers across letters D's, E's, F's and G's files.
`emit_collection_data_pointer_for` alone has ~80 across 24 files.

The plan's §2 is not wrong about its own cluster; it is silent about the fan-out,
and Phase 1 asked the right question ("identify any that are called from letter
F's files") while expecting a smaller answer. The reason the answer is large is
structural: `builder_collection_layout.rs` is codegen's type→memory oracle, so
every emitter that touches memory calls it.

Two consequences, both taken deliberately:

1. **Letter F is pulled forward.** A callee's signature change forces its call
   sites to compile, so `memory/arena/builder_arena_transfer.rs`,
   `memory/value/builder_value_semantics.rs`, `memory/marshal/record.rs`,
   `cleanup/*`, `engine/control`, `engine/value` and `engine/analysis` are all
   edited in this letter. **Letter F must re-scope against `census_by_file` at
   its kickoff rather than its §2 table** — most of its population is already
   gone.
2. **The tree-wide count fell 584 → 163**, four times this letter's own 168,
   because nearly every one of those ~250 sites was passing `&something.name()`
   — a render of a `ParameterType` the caller already held. Typing the callee
   *deleted* the render at the caller. That is the opposite of the boundary-render
   cost letter D had to pay, and it is worth stating as a rule: **convert the
   oracle, not its callers.**

**E2 — a monomorph suffix compared to a spelling is a real site, and there were
nine.** `target.strip_prefix("#collections_window$")` then `t == "String"`. The
suffix is a NAME the NIR symbol carries, so the fix is not to leave it alone
(it is a type decision) nor to thread the spelling onward (that is the thing
this plan removes): parse it **once, at the symbol boundary**, and decide on
variants below. Applied in `func_partition`, `func_group_by`, `func_zip`,
`func_chunks`, `func_find_last_index`, `func_merge`, `func_sort_by`,
`func_window`, and `func_sort`'s sibling parse.

**E3 — the gate counted `is_named("X")` as a spelling, which is the conversion's
own destination.** `ParameterType::is_named` takes a `&ParameterType` and asks
whether it is the nominal `X`. It is the typed form of that question — the one
plan-111-A sanctions for the built-in names with no variant (`Scalar`, `Error`,
`ErrorLoc`, `AttributedString`) — and it cannot be applied to a spelling at all.

Letters A–C used it freely and the gate never noticed, because their uses were
single-line. This letter produced a multi-line match guard:

```rust
__t if matches!(__t, ParameterType::Boolean | … | ParameterType::Money)
    || __t.is_named("Scalar") =>
```

whose *continuation* line carries the `=>`, so `spelling_match_arms` read
`) || __t.is_named("Scalar") ` as an arm pattern and reported a violation for
code that is fully converted. Fixed by masking `is_named("…")` arguments in both
spelling scanners, with three fixtures: the guard above is not counted, a plain
`is_named` compare is not counted, and — the one that matters — masking must not
hide a real spelling sitting *beside* an `is_named` on the same line.

This is the fourth gate correction in the plan (A3, C3, D1, E3). The pattern in
all four is the same and worth stating once: **a text-heuristic gate is wrong in
both directions, and both directions are silent.** D1 found it under-counting by
59; this one found it over-counting the destination. Neither shows up as a test
failure — one lets work slip past, the other invites you to "fix" code that is
already right.

**E4 — two of this letter's own test tasks were nearly marked moot on a false
memory, and one of them was.** Phase 2 asks for a `CollectionTypeLayout` code
test and Phase 3 for an rt fixture. I marked both moot on first pass. The first
is genuinely moot and the evidence holds: `CollectionTypeLayout::from_type`
already dispatches on variants (plan-106-E), so there was no derivation to
convert and a unit test would assert that a match arm returns what it returns.

The second was **wrong**. I claimed `tests/rt_inline_headroom_collection_field.rs`
already covered it. That file does not exist — I had recalled it from the
inline-headroom notes rather than checked. The two fixtures that *do* iterate a
member collection, `rt-behavior/arena/flat-record-collection:26` and
`byte-identity/collections:123`, are both **read-only**, and the hazard needs
iterate *and* mutate. So the fixture was written
(`tests/rt-behavior/arena/member-iterable-mutate`), not waived.

Recorded because the near-miss is the lesson: a moot needs the command that
proves it, and "I remember a test for this" is not that command. `ls` took two
seconds and reversed the conclusion.




## Summary

Risk is concentrated in `builder_collection_layout.rs` — 55 sites in one call
graph, where a partial conversion invites exactly the `.name()` bridges this plan
exists to delete, and where a changed `CollectionTypeLayout` code is a silent
layout change. Phase 1's leaf-first ordering and Phase 2's code-enumeration test
are the two guards.

Untouched: memory, arena, engine, resource and registry (letter F), and the five
sanctioned boundaries.
