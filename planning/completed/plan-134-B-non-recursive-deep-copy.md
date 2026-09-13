# plan-134-B: A non-recursive deep copy for recursive values

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-134-A

Replace the per-type deep copy that recurses on the native stack with a walker that keeps its
own work stack in the arena. Today a copy of a chain deeper than about 60 000 levels crashes
(measured: `deep_chain` 50 000 exits 0, 70 000 exits 139 — plan-134-A §2.1), and that copy
runs on every thread send and every `collections::get` of a recursive element. Letters D and
E make this copy run at every owning store, so it must be safe at any depth first.

Behavioural outcome: **deep-copying a recursive value of any depth succeeds** (a 1 000 000-deep
chain copies and reads back), and every existing caller produces the same values as before.

References: plan-134-A (§2 current state, §3 design, rejected alternatives); `mfb spec
language memory-semantics` §14.5; `mfb spec memory heap-values` (record, union and collection
layouts the walker reads); `.ai/arch-abi.md` "Win64 stack growth".

Prerequisites: see plan-134-A. Additionally: plan-134-A is complete (`ls
planning/completed/plan-134-A-*` → one file).

## 1. Goal

- `deep_chain` (plan-134-A §2.2) exits 0 and prints `top=<n>` for n = 70 000, 100 000 and
  1 000 000, on macOS and on Linux box 2223.
- Every existing thread-transfer and `get` test that copies a recursive value passes unchanged.

### Non-goals

- No new copy call sites (letters D and E add them). The walker replaces the body behind
  `copy_value_to_current_arena` for cycle types only.
- Non-recursive values keep `emit_thread_copy_real`'s inline copy, byte-identical.
- The copied graph's layout is identical to today's (same blocks, same sizes, same field
  contents); only the traversal order may differ.

## 2. Current State

- `copy_value_to_current_arena` (`src/codegen/memory/arena/builder_arena_transfer.rs`)
  calls `emit_thread_copy_call` for a type where `type_participates_in_cycle` holds, else
  inlines `emit_thread_copy_real`.
- `src/codegen/engine/builder/mod.rs` emits one function per member of
  `recursive_transfer_types` (`lower_thread_copy_function`, symbol `thread_copy_symbol`); its
  body is `emit_thread_copy_real(type_, arg0)`.
- `emit_thread_copy_real` copies by shape: `copy_record_to_current_arena` (size via
  `emit_record_block_size_to_slot`, alloc, memcpy, then `copy_record_fields_into_existing`
  replaces each pointer slot with a copy), `copy_union_to_current_arena` (size word at +8,
  then `copy_union_fields_into_existing`), `copy_collection_to_current_arena` (one block,
  then a per-entry copy of pointer payloads and in-place field fixes for inline record/union
  payloads), resources moved or `copy_resource_to_current_arena`d. Each pointer edge calls
  back into `copy_value_to_current_arena`, which for a cycle type is a **native call** — the
  recursion that overflows.
- Measured: 2 copy functions emitted for `TYPE Node (kids AS List OF Node)` (the record and its
  list); 0 of them called from `main` in `node_copies` (plan-134-A §2.1).

## 3. Design

A module-level **copy walker** `_mfb_rt_graph_copy(kind, src) -> dst`, emitted when
`recursive_transfer_types` is non-empty, replacing the per-type functions' recursion:

- **Kind index.** Each member of `recursive_transfer_types` (already a sorted
  `BTreeSet<String>`, so the index is deterministic) gets a small integer. The walker
  dispatches on it with a compare chain (the set is small: 2 per recursive record, measured).
- **Work stack.** An arena-allocated array of `{kind, src, dst_slot_address}` entries, 24 bytes
  each, starting at 64 entries and doubling (alloc new, memcpy, free old) — the growable
  pattern `lower_list_append_in_place` already uses. `ErrOutOfMemory` on failure, through the
  existing `raise_error_bare` path the copy functions use.
- **Loop.** Push `{root kind, src, &result}`. Pop an entry: shallow-copy its block exactly as
  today's `copy_*_to_current_arena` does for one level (size, alloc, memcpy), write the new
  pointer to `dst_slot_address`, then for each pointer edge of that block whose type is a cycle
  type **push** `{edge kind, old child pointer, address of the edge slot in the new block}`
  instead of calling. Non-cycle pointer edges (a flat `String`, a non-recursive collection)
  keep today's inline `emit_thread_copy_real` copy. Null/absent edges (offset-0 sentinel) are
  skipped as today.
- **Per-type entry points** `thread_copy_symbol(type)` stay (callers unchanged) as a two-line
  shim: push the root and run the walker.
- **Order independence.** Every entry writes only its own new block and its own destination
  slot, so LIFO order yields the same graph as today's depth-first recursion.

Risk: getting one shape's "edges" list wrong (a missed edge stays shared; an extra edge copies a
non-pointer). The shapes are exactly the three `copy_*_fields_into_existing` functions — the
walker's edge enumeration must be generated from the same per-field predicates
(`record_field_is_pointer_in`, the union variant fields, `is_pointer_collection_payload_type`),
never re-derived.

Rejected: one walker per type (N stacks); recursion with a depth counter that falls back to the
walker (two paths to prove). See also plan-134-A §3.

## Phases

### Phase 1 — RED: the depth tests

- [x] `tests/runtime/rt_recursive_value_copy_depth.rs` (register in `Cargo.toml`; the
      `test_targets_registered` guard prints the stanza): builds `deep_chain` and asserts exit 0
      and `top=100000` at n = 100 000; a second case copies through a thread send
      (a worker returning a `List OF Node` holding the chain, read back with `thread::waitFor`
      — see Corrections) at the same depth. Confirm both fail today (exit 139). — both fail,
      killed by signal 11.

Acceptance: both cases fail on main with exit 139.
  Check: `cargo test --release --test rt_recursive_value_copy_depth` → 2 failed (est. 2 min).
  Result: `cargo test --release --no-fail-fast --test rt_recursive_value_copy_depth` → `0 passed;
  2 failed`, both "killed by signal 11 (SIGSEGV)".
Commit: —

### Phase 2 — the walker

- [x] `src/codegen/memory/arena/builder_arena_transfer.rs`: split each
      `copy_*_fields_into_existing` into "copy this block one level" + "enumerate this block's
      cycle-type edges as (kind, slot offset)"; keep the non-cycle edges inline. — done as a
      push at each of the three edge sites (`graph_copy_edge_kind` → `emit_graph_copy_push`),
      not a split; see Corrections. Non-cycle edges keep the inline copy.
- [x] `src/codegen/engine/builder/mod.rs`: assign kind indices over `recursive_transfer_types`;
      emit `_mfb_rt_graph_copy` (work stack, loop, dispatch); turn each per-type copy function
      into the push-root-and-run shim. — walker and `_mfb_rt_graph_stack_grow` in
      `src/codegen/memory/arena/graph_copy.rs`, emitted by `engine/builder/mod.rs` when the set is
      non-empty; kinds = enumeration order of the set; `lower_thread_copy_function` takes `kind`
      and calls the walker. `node_copies -ncode` emits `_mfb_rt_graph_copy`,
      `_mfb_rt_graph_stack_grow` and the two shims. `deep_chain` (macOS): 50 000 / 100 000 /
      1 000 000 → `top=<n>`, exit 0 (was 139 from 70 000).
- [x] Unit test beside the emitter: the walker's edge list for `TYPE Node (kids AS List OF
      Node)` and for `json::Json` equals the pointer fields `copy_*_fields_into_existing` visits
      (a table test over `TypeModel::builtin_records()` and a hand-built model). —
      `graph_copy_edges_match_the_copy_calls` (hand-built models, see Corrections): `cargo test
      --release --bin mfb -- graph_copy` → `1 passed; 0 failed`.
- [x] Tests: the Phase 1 cases pass; add a positive pin to the same file — a copied `json::Json`
      tree (3 levels, arrays and objects) prints identically to its source, and mutating a
      `MUT` copy of the list holding it leaves the source intact. — both depth cases pass;
      `a_copied_json_tree_equals_its_source_and_is_independent_of_the_list` added (the source
      list's slot is overwritten instead of a `MUT` copy mutated — see Corrections).

Acceptance: deep copies succeed at any depth and existing copies are unchanged in value.
  Check: `cargo test --release --test rt_recursive_value_copy_depth` → all passed;
  `cargo test --release --test rt_scope_drop_leaks` and every `thread` rt test (`cargo test
  --release rt_thread`) → passed (est. 12 min; the thread tests are the only other callers of
  the copy, so nothing smaller covers them).
  Result: met. `cargo test --release --no-fail-fast --test rt_recursive_value_copy_depth` →
  `3 passed; 0 failed`. Scoped suites (Corrections) → `rt_recursive_get_alias` 2 passed,
  `rt_recursive_map_transfer` 1, `rt_recursive_thread_transfer` 1, `rt_scope_drop_leaks` 118,
  `rt_thread_accept_res_drop_closes` 1, `rt_thread_send_cross_arena` 2,
  `rt_tls_listener_thread_transfer` 1 — 0 failed anywhere. `cargo test --release --bin mfb --
  graph_copy` → 1 passed.
Commit: 4d8a199d5

### Phase 3 — goldens and Linux

- [x] `bash scripts/artifact-gate.sh target/release/mfb all`; expected diffs: only fixtures whose
      module has a recursive type (`tests/byte-identity/json`, `tests/byte-identity/regex`, any
      fixture declaring one). Localize each package that moves to the copy functions/walker by
      building its `-ncode` before and after; regenerate with `bash
      scripts/regen-native-goldens.sh target/release/mfb`. — `all` → `2013 golden(s) checked,
      10 diff(s)`: `byte-identity/json` and `byte-identity/regex`, each on all five targets, and
      nothing else. Localized per function (Corrections). Regenerated with
      `scripts/regen-native-goldens.sh <walker mfb> tests/byte-identity/json
      tests/byte-identity/regex` (exit 0); re-gated → json `7 golden(s) checked, 0 diff(s)`,
      regex `7 golden(s) checked, 0 diff(s)`.
- [x] Cross-build `deep_chain` for `linux-aarch64` and run it on box 2223 at n = 1 000 000
      (native aarch64 box — plan-134 needs no x86 behaviour here). — `mfb build -target
      linux-aarch64 tools/recursive-value-bench/programs/deep_chain`, `deep_chain-glibc.out`
      copied to 2223 (`Linux aarch64`): `1000000` → `top=1000000 exit=0`; `70000` →
      `top=70000 exit=0`.

Acceptance: the gate's diffs are confined to modules with a recursive type, each explained;
box 2223 prints `top=1000000`, exit 0.
  Check: the gate → only json/regex/recursive-type fixtures differ (est. 15 min: the gate is the
  only instrument that sees every target's emitted copy functions); box 2223 run (est. 3 min).
  Result: met — the 10 diffs are the two recursive-type fixtures × 5 targets, each explained by
  the added walker and the per-type shims; box 2223 printed `top=1000000`, exit 0.
Commit: 54f542ec2

## Validation Plan

- Tests: `rt_recursive_value_copy_depth.rs` (RED→GREEN, plus the value pin); the edge-list unit
  test.
- Runtime proof: `deep_chain` at 1 000 000 on macOS and box 2223.
- Doc sync: `mfb spec memory arenas` — the deep-copy paragraph names the walker;
  `.ai/codegen-invariants.md` — the per-type copy is non-recursive (so no one reintroduces a
  native recursion).
- Final gate: runs once, in plan-134-H.

## Open Decisions

- Walker shape — see plan-134-A Open Decisions (recommended: one module-level walker).

## Corrections

- **Prerequisite re-run** (2026-09-13): `ls planning/completed/plan-134-A-*` → one file — MET.
- **`thread::transfer` cannot carry a value.** It takes a resource only (`mfb man thread
  transfer`: `res AS Res`). The thread case copies a worker's `List OF Node` result back with
  `thread::start` / `thread::waitFor` (the path `rt_recursive_thread_transfer.rs` covers) and
  reads it with `FOR EACH`, which borrows, so the transfer copy is the only deep copy.
- **The kind set is not "2 per recursive record".** A module importing `regex` emits 12 per-type
  copy functions (`mfb build -ncode` of `tools/recursive-value-bench/programs/regex_repeat`,
  `grep -oE '"symbol": "_mfb_thread_copy_[A-Za-z0-9_]+"'`): `__regex_Node`, its six
  variants/containers, `__regex_Cont` and three variants, `__regex_Choices`, `__regex_Choice`.
  A compare chain over 12 is still fine.
- **`TypeModel::builtin_records()` holds records only**, not `json::Json` (a union that enters a
  model through the importing module's NIR). The edge-table unit test therefore uses
  hand-built models: a user `TYPE Node`, a Json-shaped recursive union, and a cycle member with a
  non-cycle field that reaches a second cycle.
- **The json value pin cannot mutate a `MUT` copy of the list yet.** Phase 2's last task asks
  that "mutating a `MUT` copy of the list holding it leaves the source intact". That is
  bug-601's shape, which this letter does not fix: `MUT ys = xs` over a recursive list still
  shares `xs`'s block until letter D (`tools/recursive-value-bench/run.sh … tree_alias` →
  `ys=6 xs=112`). The pin instead overwrites the SOURCE list's slot in place
  (`collections::set(xs, 0, json::JsonNull[NOTHING])`) after taking the copy, then reads the
  copy — the independence this letter's walker is responsible for. Letter D's
  `rt_recursive_value_copies.rs` owns the `MUT`-copy shape.
- **"Split each `copy_*_fields_into_existing` into copy-one-level + enumerate-edges" was done
  without a split.** The three edge sites (`copy_record_fields_into_existing`,
  `copy_union_fields_into_existing`'s variant-field loop, `fix_collection_transfer_payload`'s
  pointer payload) ask `graph_copy_edge_kind` before copying an edge and, inside the walker,
  push instead. The walker's per-kind body is the unchanged `emit_thread_copy_real`, which
  copies one block and reaches exactly those sites. That meets the design's own constraint
  ("generated from the same per-field predicates, never re-derived") more strictly than a split
  would: there is one enumeration, and the push replaces the call at the same site. Outside the
  walker `graph_copy_edge_kind` returns `None` without emitting anything, so non-recursive
  codegen is untouched. Checked by `graph_copy_edges_match_the_copy_calls` (pushes == calls,
  per kind, over three models).
- **Scoped regression suites instead of `cargo test --release rt_thread`.** That name filter
  compiles every integration binary to select by test-function name. The copy's only callers
  are thread send/result and `collections::get`, so Phase 2 runs the targets that exercise them
  by name: `rt_recursive_get_alias`, `rt_recursive_map_transfer`, `rt_recursive_thread_transfer`,
  `rt_thread_send_cross_arena`, plus `rt_thread_accept_res_drop_closes`,
  `rt_tls_listener_thread_transfer` and `rt_scope_drop_leaks`. `rt_canvas_graphics_thread` is
  left out: it drives the desktop (a GUI test) and contains no recursive value.
- **Doc sync lands in `mfb spec threading isolation`, not `mfb spec memory arenas`.** The arenas
  page has no deep-copy paragraph to name the walker in (`grep -n -i "deep.cop\|per-type"
  src/docs/spec/memory/04_arenas.md` → only the scope-drop "no per-type recursive drop glue"
  line, which is letter G's). The boundary deep copy is described in
  `src/docs/spec/threading/02_isolation.md`, whose claim "every non-resource value is a flat,
  pointer-free block, [so the copy] is a single allocation plus byte copy" was never true for a
  recursive value; it now names the walker. `.ai/codegen-invariants.md` gained the
  "never recurse on the native stack" section; `.ai/collections.md`'s bug-538 note names the
  shim.
- **Golden churn localized before regenerating.** The main checkout's `target/release/mfb`
  reproduces the committed macOS goldens byte-for-byte (json `1dfb046c…`, regex `fa1eb36a…`), so
  it served as the before binary. Per-function diff of `-ncode` (`/tmp/p134-ncode-diff.py`):
  json 160 → 162 functions, added `_mfb_rt_graph_copy` + `_mfb_rt_graph_stack_grow`, changed
  only its 5 `_mfb_thread_copy_*` shims; regex 187 → 189, same two added, changed only its 12
  shims; nothing removed, no other function changed.

## Summary

This letter fixes a live crash (a deep recursive value cannot be copied) and is the foundation
the copy-insertion letters stand on. The risk is a missed or extra edge; the edge list is
generated from the same predicates the current copy uses, and pinned by a table test.
