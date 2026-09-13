# plan-134-F: A non-recursive per-type drop, proven the inverse of the copy

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-134-E

Emit a drop walker for recursive values — the exact inverse of plan-134-B's copy walker — and
prove the two are inverses, **without calling it from any user code path**. `mfb spec language
memory-semantics` §14.5: "dropping a recursive value recursively drops its owned children …
Implementations may use iterative drop internally to avoid stack overflow."

Behavioural outcome: a test-only hook shows **copy-then-drop of any recursive value returns the
arena's `live_bytes` to its value before the copy**, for every recursive type in the builtins
and for user types, at depth 1 000 000.

References: plan-134-A; plan-134-B (walker, edge enumeration); `mfb spec memory heap-values`
(block sizes), `mfb spec tooling debug-report` (`arena.live_bytes`); `src/codegen/debug/arena.rs`;
`tests/runtime/rt_debug_arena.rs` (how a test reads the counters).

Prerequisites: see plan-134-A; plan-134-E complete (`ls planning/completed/plan-134-E-*`). — MET
2026-09-13 (`planning/completed/plan-134-E-deep-copy-at-construction-stores.md`).

## 1. Goal

- `_mfb_rt_graph_drop(kind, ptr)` is emitted beside `_mfb_rt_graph_copy` for every module with a
  recursive type.
- The symmetry test (Phase 2) passes for `json::Json`, `__regex_Node`, `__regex_Cont`,
  `__regex_Choices`, a user `TYPE Node`, a user recursive `UNION`, and ~~`dom::Node`~~ (no such
  type — see Corrections).

### Non-goals

- No caller outside the test hook (letter G registers it).
- Resources inside a recursive value are not closed here (`type_contains_resource` values are
  excluded from the class, plan-134-D).

## 2. Current State

- `_mfb_arena_free(ptr, size)` (`src/codegen/memory/arena/arena.rs::lower_arena_free`) needs the
  size; there is no allocator header.
- Sizes are recomputable per shape: a `String` from its length word; a data union and a flat
  `Result` from the size word at +8; a collection from its capacity words
  (`lower_drop_owned_collection_helper`); a record from `emit_record_block_size_to_slot`.
- Each pointer edge of a copied graph is its own arena block (plan-134-B's walker allocates one
  per edge, as today's copy does).
- `--debug` arena counters: `alloc_calls`, `alloc_bytes`, `free_calls`, `free_bytes`,
  `live_bytes`, `peak_live_bytes`, `double_free_skips`, … (`src/codegen/debug/arena.rs`, 18
  counters mirrored in `rt_debug_arena.rs`).

## 3. Design

- **Walker.** Same kind indices and growable arena work stack as `_mfb_rt_graph_copy`. Push
  `{kind, ptr}`. Pop: null → skip. Read every cycle-type edge pointer of the block **first**
  (pushing each), free every non-cycle pointer edge inline (a flat `String`/collection field
  via the existing one-block frees), then compute the block's own size and `arena_free` it.
  Reading edges before freeing the parent is what makes freeing in LIFO order safe.
- **Edge enumeration is shared** with the copy walker (one generator, two uses). This is the
  inverse proof's structural half; the symmetry test is the behavioural half.
- **Work stack free.** The walker frees its own work stack before returning.
- **Test hook.** A `--debug`-only intrinsic is not added to the language. Instead the symmetry
  test uses an existing, already-owned copy path: plan-134-G is not in yet, so the test builds
  a value, copies it with `collections::get` (owned copy, today never freed), and calls the drop
  through a `#[cfg(test)]` codegen unit test that emits a tiny module invoking
  `_mfb_rt_graph_drop` on the copy — or, if a unit-level harness cannot run native code, a test
  fixture program compiled with a hidden `MFB_TEST_DROP_HOOK` build option that exists only in
  test builds. **Open Decision** below.

Risk: a size computed differently from the size allocated (arena free-list corruption — shows as
`double_free_skips` or a crash under churn) and an edge freed that the copy never allocated
separately (an inline payload's field). Both are caught by live_bytes returning exactly to its
start and by a 20 000-iteration churn loop with arena scrubbing (plan-01 §6.5 entropy fill —
freed chunks are scrubbed, so a use-after-free reads garbage).

## Phases

### Phase 1 — the walker

- [x] `src/codegen/engine/builder/mod.rs` / `builder_arena_transfer.rs` (or a new
      `src/codegen/cleanup/owned/graph_drop.rs`): emit `_mfb_rt_graph_drop` using the shared
      edge enumeration and the per-shape size functions. — `src/codegen/memory/arena/graph_drop.rs`
      (beside `graph_copy.rs`, whose work-stack layout, grow helper and `GraphCopyWalker` it
      reuses), emitted after `_mfb_rt_graph_stack_grow` for every module with a recursive type.
      Shared lists extracted from the copy's edge sites: `record_pointer_edges`,
      `union_variants_by_tag`, `collection_payload_edges`, `payload_edge_shape`; block sizes from
      `emit_inlined_block_size_from_ptr_slot` (the sizer the copy allocates with).
- [x] Unit test: the drop walker visits exactly the edges the copy walker visits, per kind
      (table over the builtin recursive types and a hand-built model). —
      `graph_drop_edges_match_the_copy_edges` (Node, Json-shaped union, bridged second cycle,
      a `Tree` union whose variant holds two `Tree` fields) and
      `..._for_the_builtin_types` (`#regex_Node`/`#regex_Cont`/`#regex_Choices` and `json::Json`
      from the regex/json bench programs lowered to NIR); each also asserts the drop body calls
      no per-type copy and neither walker.

Acceptance: the edge tables match for every recursive type.
  Check: `cargo test --release --bin mfb -- graph_drop` → passed (est. 3 min).
  Result: `cargo test --release --bin mfb -- graph_drop graph_copy` → `3 passed; 0 failed`
  (`graph_copy_edges_match_the_copy_calls` still green after the copy's edge sites moved onto
  the shared lists).
Commit: 62292f3af

### Phase 2 — symmetry, depth and churn

- [x] Resolve the test-hook Open Decision and build it. — `MFB_TEST_GRAPH_DROP` build-time
      environment hook (Corrections: test hook), landed with Phase 1 in `62292f3af`.
- [x] `tests/runtime/rt_recursive_value_drop_symmetry.rs` (register in `Cargo.toml`): for each
      type in §1, build a value, record `arena.live_bytes`, copy, drop the copy, assert
      `live_bytes` equals the recorded value and `double_free_skips` is 0; repeat at depth
      1 000 000 (chain) and 20 000 iterations (churn), reading the result of the source after
      each drop to prove the drop freed only the copy. — Six cases, each a plain vs a hooked
      `--debug` build: user `TYPE Node` (hook `probe,side,cur`), user `UNION Tree` with a
      list-of-self variant and a two-`Tree`-field variant (`probeTree,leaf,branch,t`),
      `json::Json` (`probeJson`), the regex engine's `stack`/`cont`/`node` locals (one build
      each), a 1 000 000-deep chain, and a 20 000-iteration churn whose hooked peak must stay
      within 64 KiB of the plain peak. Each asserts equal stdout, equal `live_bytes`, extra
      `free_bytes` == extra `alloc_bytes`, `double_free_skips` 0, and more `alloc_calls` hooked.
- [x] Added: runtime proof on box 2223, and a check that the test can fail. —
      `bash /tmp/p134-f-linux.sh` (plain/hooked `-target linux-aarch64 --debug` builds run on
      2223): deep chain `top=1000000 1000000` both, `live_bytes` 208000128 both, extra
      alloc 96003168 = extra free 96003168; churn `total=300000` both, `live_bytes` 60162208
      both, peaks 60163760 / 60164016; `double_free_skips` 0; `status=0`. Mutation (every drop
      frees 16 bytes too many): `churn_frees_every_copy_as_it_goes` FAILED (hooked program
      crashed), `a_million_deep_chain_drops_exactly_its_copy` FAILED ("freed 3248 bytes of the
      96003168"); reverted. A +8 mutation is equivalent for blocks ≡ 8 mod 16 (`arena_free`
      rounds sizes up to 16), which is why only the union case failed under it.

Acceptance: copy-then-drop is exact for every recursive type, at depth and under churn.
  Check: `cargo test --release --test rt_recursive_value_drop_symmetry` → passed (est. 4 min).
  Result: `6 passed; 0 failed` (196.6 s) on macOS; box 2223 as above.
Commit: —

### Phase 3 — goldens

- [ ] Artifact gate; expected diffs: the modules that emit the new drop walker (json, regex,
      recursive-user-type fixtures). Regenerate.

Acceptance: diffs confined to modules with a recursive type.
  Check: the gate (est. 15 min).
Commit: —

## Validation Plan

- Tests: the edge-table unit test; `rt_recursive_value_drop_symmetry.rs`.
- Runtime proof: the symmetry test on macOS and box 2223.
- Doc sync: none until G makes the drop reachable.
- Final gate: plan-134-H.

## Open Decisions

- **Test hook** — recommended: a test-build-only compiler option that makes a fixture call
  `_mfb_rt_graph_drop` on a named local, compiled out of release builds; alternative: defer the
  symmetry proof to letter G's registered drops (no hook, but then the first time the drop runs
  is also the first time a user program depends on it).
- **Emit-only-if-referenced** — recommended: emit the drop walker unconditionally with the copy
  walker (simpler; letter G references it anyway); alternative: emit only when referenced (keeps
  F's goldens unchanged, adds a reference census).

## Corrections

- **Prerequisite re-run** (2026-09-13): `ls planning/completed/plan-134-E-*` → one file — MET.
- **Test hook: an inert-unless-set environment variable, not a test-build-only option.** The
  recommended hook is "a test-build-only compiler option ... compiled out of release builds",
  but the integration tests run the RELEASE `mfb` binary (`tests/common/mod.rs::mfb_exe`, the
  binary every `rt_*` test builds programs with), so an option compiled out of release could
  never be exercised by the test that needs it. The compiler already carries inert-unless-set
  environment hooks read at codegen (`MFB_BENCH_LOWERING` in `engine/regalloc/mod.rs`,
  `MFB_BUG387_SELFMOVE` in `arch/aarch64/select.rs`), so the hook follows that precedent: unset,
  the emitted code is byte-identical; set by the symmetry test, it makes the program drop a named
  copy through `_mfb_rt_graph_drop`. The alternative (defer the proof to letter G) is still
  rejected for the plan's own reason — the first run of the drop would be a user program's.
  Built as: `MFB_TEST_GRAPH_DROP=<local>[,<local>…]` at build time makes every `LET` or
  assignment of a named local whose type is a resource-free cycle member deep-copy the value
  and immediately `_mfb_rt_graph_drop` the copy (`graph_drop.rs::emit_test_graph_drop_hook`,
  called from `lower_ops_inner`). The symmetry test compares a plain `--debug` build with a
  hooked one: equal stdout, equal final `live_bytes`, extra bytes freed == extra bytes
  allocated, `double_free_skips` 0, and more `alloc_calls` hooked (non-vacuous). Because the
  hook copies a live value rather than dropping the program's own, it also reaches the regex
  engine's private types through its locals (`stack`, `cont`, `node`), which no user program
  can name.
- **`dom::Node` does not exist.** §1 lists it, but `grep -rn "dom::Node" src tests` → 3 hits,
  all in comments (`builder_collection_layout.rs`, `builder_arena_transfer.rs`,
  `engine/builder/mod.rs`), and `ls src/codegen/builtins` has no `dom` package. It is dropped
  from the symmetry list; the user recursive `UNION` case adds a variant whose record fields are
  the union itself (`Couple`), the shape a DOM node's parent/child fields would have.
- **"One generator" is the per-block edge lists.** The copy's edge sites interleave writes to
  the destination block, so the instruction stream cannot be shared. What is shared is every
  decision about which words are edges: `record_pointer_edges`, `union_variants_by_tag`,
  `collection_payload_edges` and `payload_edge_shape` were extracted from the copy's three edge
  sites (`builder_arena_transfer.rs`), which now iterate them, and the drop iterates the same
  lists and asks the copy's `graph_copy_edge_kind` whether to push. The edge-table test pins
  the result on the hand-built models and on the builtin types' real models (the json and regex
  bench programs lowered to NIR).

## Summary

The inverse of the copy, proven exact before any program depends on it. Its risks — size
mismatch and freeing inline fields — are exactly what `live_bytes` returning to its start
detects.
