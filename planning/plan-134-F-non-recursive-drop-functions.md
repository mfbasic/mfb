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

Prerequisites: see plan-134-A; plan-134-E complete (`ls planning/completed/plan-134-E-*`).

## 1. Goal

- `_mfb_rt_graph_drop(kind, ptr)` is emitted beside `_mfb_rt_graph_copy` for every module with a
  recursive type.
- The symmetry test (Phase 2) passes for `json::Json`, `__regex_Node`, `__regex_Cont`,
  `__regex_Choices`, a user `TYPE Node`, a user recursive `UNION`, and `dom::Node`.

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

- [ ] `src/codegen/engine/builder/mod.rs` / `builder_arena_transfer.rs` (or a new
      `src/codegen/cleanup/owned/graph_drop.rs`): emit `_mfb_rt_graph_drop` using the shared
      edge enumeration and the per-shape size functions.
- [ ] Unit test: the drop walker visits exactly the edges the copy walker visits, per kind
      (table over the builtin recursive types and a hand-built model).

Acceptance: the edge tables match for every recursive type.
  Check: `cargo test --release --bin mfb -- graph_drop` → passed (est. 3 min).
Commit: —

### Phase 2 — symmetry, depth and churn

- [ ] Resolve the test-hook Open Decision and build it.
- [ ] `tests/runtime/rt_recursive_value_drop_symmetry.rs` (register in `Cargo.toml`): for each
      type in §1, build a value, record `arena.live_bytes`, copy, drop the copy, assert
      `live_bytes` equals the recorded value and `double_free_skips` is 0; repeat at depth
      1 000 000 (chain) and 20 000 iterations (churn), reading the result of the source after
      each drop to prove the drop freed only the copy.

Acceptance: copy-then-drop is exact for every recursive type, at depth and under churn.
  Check: `cargo test --release --test rt_recursive_value_drop_symmetry` → passed (est. 4 min).
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

(Filled in during execution.)

## Summary

The inverse of the copy, proven exact before any program depends on it. Its risks — size
mismatch and freeing inline fields — are exactly what `live_bytes` returning to its start
detects.
