# plan-134-E: Deep copies where recursive values are built into records, unions and collections

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-134-D

The stores that build a value out of other values write a recursive sub-pointer verbatim:
record construction, `WITH`, union variant wrap, the collection payload writer, and `STATE`
assignment (plan-134-A §2). After this letter each of them copies an aliasing source (or moves
a last-use local / claims a fresh temp), so **every owner in a program holds a distinct
graph** — the precondition for any free.

Behavioural outcome: **a recursive value stored into a record field, a union variant, a
collection (literal, `append`, `insert`, `set`, `prepend`) or a `STATE` field is independent of
the value it came from.** Measured by construction: a node stored into two parents and one list
yields three graphs that never share a block (checked through `--debug` `arena.alloc_calls`
and by mutation).

References: plan-134-A; plan-134-D (`needs_graph_copy`, move sites); `mfb spec language
memory-semantics` §14.2 (record/union/collection construction consume the expression result),
§14.6.

Prerequisites: see plan-134-A; plan-134-D complete (`ls planning/completed/plan-134-D-*`). — MET
2026-09-13 (`planning/completed/plan-134-D-copy-insertion-at-owning-stores.md`).

## 1. Goal

- `node_copies`: `Node[kids := [a], …]` and `append(xs, a)` each produce a graph distinct from
  `a` when `a` is read afterwards.
- The E test (below) passes; no store in the census remains unhandled.

### Non-goals

- No frees. Flat payloads keep their existing byte copies, byte-identical.
- No change to the in-place collection arms' growth/realloc logic.

## 2. Current State (from the plan-134-A census)

| Store | Site | Today |
|---|---|---|
| Record construction pointer field | `builder_collection_layout.rs::emit_build_inlined_record` | pointer stored as-is |
| `WITH` update | `builder_value_semantics.rs::lower_with_update` | updated values lowered with `lower_value` (no copy); rebuilt with `emit_build_inlined_record` |
| Union variant wrap | `builder_collection_layout.rs::emit_wrap_record_in_union` (2 callers) | one-level `emit_copy_bytes` of the variant; its recursive fields stay shared |
| Collection payload writer | `builder_collection_layout.rs::emit_copy_payload_to_collection` (15 references) | pointer payload stored; inline record/union payload byte-copied with shared sub-pointers |
| `STATE` assignment | `builder_control.rs` `NirOp::StateAssign` | plain `lower_value`, no copy |
| Record/collection literal operands | lowered through `lower_value` by their construct lowerings | no copy |

**Census (measured 2026-09-13, before letter E; read-only survey, rows spot-checked in source).**
22 distinct store paths; **every one lowers its operand with plain `lower_value`**, none with
`lower_value_owned` (the only `lower_value_owned` callers are the `Bind`, `Assign`, `StoreGlobal`
arms, closure capture and `FAIL`). Paths (relative to `src/codegen/`):

| Store | Site | Operand lowering → writer |
|---|---|---|
| `T[a, b]` record / register-native vector | `engine/value/builder_values.rs` `NirValue::Constructor` arm | `self.lower_value(arg)?` (lines ~2109, ~2138) → `emit_build_inlined_record` / `emit_construct_helper_call` / `make_vector_native` |
| `construct.T` helper body | `memory/marshal/construct_helpers.rs::lower_construct_helper` | argument registers (the caller's constructor slots) → `emit_build_inlined_record` |
| data-union variant wrap | `builder_values.rs` `NirValue::UnionWrap` arm | `self.lower_value(value)?` → `emit_wrap_record_in_union` |
| `WITH r { f := v }` | `memory/value/builder_value_semantics.rs::lower_with_update` | target and each update `self.lower_value(...)` → `emit_build_inlined_record` |
| default record / union | `builder_value_semantics.rs::lower_default_value_inner` | defaults, not user values |
| `s.state = WITH s.state {scalar := v}` in place | `engine/control/builder_control.rs::try_inplace_state_scalar_assign` | `lower_value(&update.value)` → raw `store_u64` (inline scalars only) |
| `s.state = v` whole replace | `builder_control.rs` `NirOp::StateAssign` | `self.lower_value(value)?` + `claim_pending_temp` → pointer stored in the resource record (an aliasing source is shared, not copied — check the flat case too) |
| `l = append/prepend/insert/set(l, …)`, map `set`, set `add`, bulk append (plain local, record field, STATE field) | `collection/assign/builder_inplace_assign.rs` `try_inplace_*`; `builder_control.rs::try_inplace_state_*` | item `self.lower_value(&args[..])?` (+ `materialize_value`) → `emit_copy_payload_to_collection` (`list_mutate.rs` 797/1347/2824/2995/3334/3526, `map_mutate.rs` 385/683/1018/1052) or the bulk block copy |
| `[a, b]` / `{k: v}` / set literal | `collection/layout/builder_collection_layout.rs::lower_list_literal` / `lower_map_literal` / `lower_set_literal` | `self.lower_value(value_node)?` (lines ~1258, ~1364) → `lower_collection_values` → `emit_copy_payload_to_collection` (1687/1763) |
| rebuilding `append`/`prepend`/`insert`/`set`/`add` (in-place arm declined) | `builtins/collections/gen_mutate.rs::lower_collection_end_insert`, `func_insert.rs`, `func_set.rs`, `func_add.rs` | args pre-lowered by `lower_abi_inline_args` (`self.lower_value(arg)?`, ~2750) → singleton `lower_collection_values` + `lower_list_insert_collection` / `lower_map_concat` |

**Collection-to-collection copies (measured 2026-09-13, read-only survey).** No path below fixes a
copied payload's recursive edges (`grep -n "needs_graph_copy\|copy_value_to_current_arena"` finds
nothing in `list_mutate.rs`, `map_mutate.rs`, `gen_mutate.rs`, `gen_memory.rs`, `gen_slice.rs`,
`builder_search.rs`, `builder_collection_layout.rs` or `collection_buffer.rs`; only the three G24
guards consult `type_participates_in_cycle`):

| Builtin (native path) | Copy primitive |
|---|---|
| rebuilding `append` (item and list) / `prepend` / `insert` (`gen_mutate.rs::lower_collection_end_insert`, `func_insert.rs`) | `lower_list_insert_collection` → `emit_block_copy_advance` |
| rebuilding list `set` (non-fixed-width) | `lower_list_remove_at` + singleton + `lower_list_insert_collection` |
| map `set` | `lower_map_remove_key` (`emit_copy_one_map_entry`) + `lower_map_concat` |
| set `add` (`func_add.rs`) | `copy_collection_tight` + `lower_map_set_in_place` |
| `removeAt` (`lower_list_remove_at`), `removeKey` / set `remove` (`lower_map_remove_key`) | `emit_block_copy_advance`, `emit_copy_one_map_entry` |
| `transform`, `filter` (`func_transform.rs`, `func_filter.rs`) | `lower_list_append_in_place` → `emit_copy_payload_to_collection` (filter's item is an alias into the source) |
| `keys` / `values` / `toList` (`gen_memory.rs::lower_map_projection`) | byte loop |
| list `mid` (`builder_search.rs::lower_list_mid`), list `replace` (`builder_strings.rs::lower_list_replace`), `__collections_slice` (`gen_slice.rs`) | `emit_block_copy_advance`, `emit_bulk_copy_entries_shift` |

The fast paths of `sort`, `sortBy`, `flatten`, `zip`, `chunks`, `window`, `partition`, `groupBy`,
`merge` and `mapValues` are limited to scalar/String element types. Their general forms, and
`take`/`drop`/`distinct`/`toSet`/set algebra, are source-generic `.mfb` bodies built from
`collections::get` (which copies, `materialize_owned_element`) and the store members above.

The in-place arms' item operands (added to Phase 1 by plan-134-D's audit) are the `try_inplace_*`
row. The record-field and STATE in-place collection arms cannot fire for a recursive element (G17),
but they share the lowering.

## 3. Design

The copy decision belongs to the **operand**, not the writer: a writer receives an already
lowered value and cannot tell a fresh block from an alias. So:

- Each construction lowering that turns a `NirValue` operand into a stored field/element/
  payload lowers it with `lower_value_owned` (which, after plan-134-D, copies an aliasing
  recursive source, moves a last-use local, and claims a fresh temp) instead of `lower_value`.
- For an **inline** record/union payload inside a collection or union (the byte-copied
  shapes), the byte copy duplicates the outer block but the recursive sub-pointers inside it
  still point at the source's children. There, after the byte copy, run the letter-B walker's
  "copy the cycle-type edges of this block" step on the new block (the same edge enumeration,
  so a copied payload and a copied standalone value are identical).
- `StateAssign`: lower with `lower_value_owned`.

Risk: double copying (a fresh constructor result copied again — wasteful, not unsafe) and,
worse, a writer that copies an operand already claimed as a move (a leak until G, then fine).
The E test checks independence; the `--debug` alloc counts check no double copy.

## Phases

### Phase 1 — census and RED

- [x] Measure the construct lowerings: `grep -rn "fn lower_.*construct\|NirValue::Constructor\|NirValue::ListLiteral\|NirValue::MapLiteral" src/codegen --include='*.rs'`
      and every caller of the writers in §2; record each operand path (file::symbol, `lower_value`
      or `lower_value_owned`) in §2 with the count. — §2 "Census": 22 distinct paths, every operand lowered with
      `lower_value` (survey of every constructor/wrap/`WITH`/literal/in-place/rebuild/STATE store,
      rows spot-checked: `builder_values.rs` Constructor arm `self.lower_value(arg)?` at ~2109 and
      ~2138, `lower_abi_inline_args` ~2750, `lower_list_literal`/`lower_map_literal` ~1258/~1364,
      `StateAssign` `lower_value` + `claim_pending_temp`).
- [x] Add to the census the in-place arms' ITEM operands, which the §2 table does not list
      (found by plan-134-D's audit, recorded in plan-134-A "Verified properties"): 
      `try_inplace_append_assign`, `try_inplace_bulk_append_assign`, `try_inplace_prepend_assign`,
      `try_inplace_insert_assign`, `try_inplace_set_assign` (List and Map) and
      `try_inplace_set_add_assign` (`collection/assign/builder_inplace_assign.rs`) lower the item
      with `lower_value` and byte-copy it into the destination, so a constructor argument that
      aliases the destination builds a self-cycle or a dangling pointer. Record each with its
      lowering call in §2. — the `try_inplace_*` row of §2's census (item
      `self.lower_value(&args[..])?`, written by `emit_copy_payload_to_collection` or the bulk
      block copy).
- [x] `tests/runtime/rt_recursive_value_construction_copies.rs` (register in `Cargo.toml`):
      one program, one line per store in §2 — a node read after being put into a record field,
      a union variant, a list literal, `append`, `insert`, `set`, `prepend`, a `Map` value, a
      `WITH` replacement, a `STATE` field — then the source's list is rebuilt and each holder
      re-read. Also the self-referencing construction: `MUT xs AS List OF Node = []`, then
      `xs = collections::append(xs, Node[kids := xs, tag := 1])` twice, then `collections::get`
      each element — must print `len xs=2`, `first.kids=0`, `second.kids=1` (today: SIGSEGV at
      the first `get`, pre-plan compiler and plan-134-B walker alike). Confirm it fails today. —
      two tests: `a_recursive_value_built_into_another_is_independent_of_its_source` (one `MUT xs`
      stored into a record field, a union variant, a list literal, `append`, `insert`, `set`,
      `prepend`, a map value, `WITH`, and a `STATE` replacement, then grown in place) and
      `a_value_built_from_the_list_it_is_appended_to_holds_the_old_list`. Both die with SIGSEGV
      on the plan-134-D compiler (a `/tmp` build of the first printed `record=96`, `union=96`
      before crashing).

- [x] Census the builtins that copy payloads from one collection into another (the rebuilding
      store members behind `lower_abi_inline_args`, and every collection transform that
      byte-copies elements), each with its copy primitive; record in §2 with the count. — §2
      "Collection-to-collection copies": 12 native paths copy inline payload bytes with no edge
      fix-up; the scalar-only fast paths cannot meet a recursive element; the source-generic
      `.mfb` bodies inherit `get` (copies) and `append`/`set`/`add`.
- [x] Extend the RED test with one such builtin per copy primitive (e.g. `LET ys =
      collections::append(xs, item)` on a non-`MUT` source, `collections::reverse`), each read after
      the source's element graph changed through a `MUT` copy; confirm it fails. — structural, not
      behavioural (Corrections): `a_native_collection_builtin_copies_its_result_elements_graphs`
      requires copy calls in functions using a rebuilding `append`, `removeAt`, `filter` and map
      `values` (one per copy primitive family: `lower_list_insert_collection`,
      `lower_list_remove_at`, `emit_copy_payload_to_collection`, `lower_map_projection`). On the E
      build before the fix (`/tmp/p134-mfb-e build -ncode`), all four functions made **0** copy
      calls (only `main`'s three literal stores copied).

Acceptance: §2 has no UNMEASURED row; the test fails on main.
  Check: `cargo test --release --test rt_recursive_value_construction_copies` → failed (est. 2 min).
  Result: met — §2 carries the measured census; `cargo test --release --no-fail-fast --test
  rt_recursive_value_construction_copies` → `0 passed; 2 failed`, both "status
  ExitStatus(unix_wait_status(11))" (SIGSEGV).
Commit: —

### Phase 2 — copy at each store

- [x] Switch each operand path found in Phase 1 to `lower_value_owned`, including the in-place
      arms' item operands. — via `lower_value_stored` (Corrections): the record constructor
      argument loop, `UnionWrap`, `WITH` updates, list/map-literal values, set-literal items, and 19
      in-place `item`/`val`/`rhs` operands. `cargo test --release --no-fail-fast --test
      rt_recursive_value_construction_copies --test rt_recursive_value_copies --test
      rt_recursive_value_copy_depth` → 2 + 2 + 3 passed, 0 failed.
- [x] Collection-to-collection copies from the census deep-copy the recursive edges of every
      copied inline payload (the walker's edge enumeration over the new block's range, the shape
      `fix_collection_transfer_payloads` already applies to a thread transfer), and the rebuilding
      store members copy their item operand the way `lower_value_stored` does. —
      `own_collection_payload_edges` on the result of `try_abi_inline_lower`, `try_inline_slice_op`,
      list `mid` and list `replace` (Corrections). The item operand needs no separate copy: the
      rebuilt collection holds the item's payload bytes, and the result fix copies their edges with
      everything else. `-ncode` of the probe: `viaAppend`/`viaRemoveAt`/`viaFilter`/`viaValues` each
      went from 0 to 1 copy call.
- [x] ~~Inline payload edge copy after the byte copy (`emit_copy_payload_to_collection`,
      `emit_wrap_record_in_union`), reusing the walker's edge enumeration.~~ — moot: a single-operand construction store copies its aliased operand before the writer runs (`lower_value_stored`), so the byte-copied payload's edges already point at a private graph; a builtin that byte-copies payloads out of another collection has its result fixed by `own_collection_payload_edges`. A third copy inside `emit_copy_payload_to_collection` / `emit_wrap_record_in_union` would copy every edge a second time. Evidence: `rt_recursive_value_construction_copies` (record, union, literal, `append`, `insert`, `set`, `prepend`, map, `WITH`, STATE, self-reference, builtins) passes with the writers unchanged, and the copy-count pin shows `main` copying exactly three times.
- [x] `StateAssign` → `lower_value_owned`. — `lower_value_stored` (Corrections), and the STATE
      in-place append operand; the `state=1` line of the construction test passes.
- [x] Tests: Phase 1 passes; add an alloc-count pin — `node_copies` under `--debug` shows
      exactly the copies the census predicts (no double copy of a fresh constructor). — pinned by
      copy-call count rather than `--debug` alloc totals (which also count the walker's work stack
      and every other allocation): `a_construction_store_copies_an_aliased_source_once_and_a_fresh_value_never`
      asserts `main` makes exactly 3 copy calls (`LET c = a`, `a` in a list literal, `a` as the
      in-place append item) and none for a fresh constructor appended to `ys`.
      `run.sh … node_copies` → `main_copy_calls=3`.

Acceptance: every construction store yields an independent graph, with no redundant copy.
  Check: `cargo test --release --test rt_recursive_value_construction_copies --test
  rt_recursive_value_copies` → passed (est. 4 min).
  Result: met — `cargo test --release --no-fail-fast --test rt_recursive_value_construction_copies
  --test rt_recursive_value_copies --test rt_recursive_value_copy_depth` → 4 + 2 + 3 passed, 0
  failed; `rt_scope_drop_leaks` → 118 passed after the operand switch (flat stores unchanged).
Commit: —

### Phase 3 — speed and goldens

- [~] Bench medians (`tools/recursive-value-bench/run.sh … json_repeat regex_repeat`, ×5) within
      the plan-134-A budget; a miss is traced with `--debug` alloc counts to the store and fixed.
      — json within budget, regex further over (Corrections): medians of 5 with the E build,
      `json_repeat` K=1 0.25 s (budget 0.31), `regex_repeat` K=1 **0.89 s** (budget 0.34; 0.43 s
      after letter D), RSS 2.39 GB (919 MB after D). Traced to `#regex_run`'s constructor stores
      that must now copy (not an analysis miss). Covered by the owner's 2026-09-13 decision
      (accept correctness-first copies, re-measure at plan-134-H, report before merge); the H
      report task now carries E's numbers and the cause. Remaining: that re-measure.
- [x] Artifact gate; expected diffs: json, regex, recursive-user-type fixtures; regenerate. —
      `artifact-gate [all]` (E build, `/tmp/p134-mfb-e2`): `2013 golden(s) checked, 5 diff(s)`, all
      `byte-identity/regex` on the five targets; json byte-identical. Localized per function
      (Corrections). `scripts/regen-native-goldens.sh … tests/byte-identity/regex` → `5 golden(s)
      rewritten, 0 failure(s)`; re-gated → `7 golden(s) checked, 0 diff(s)`.

Acceptance: within budget; diffs confined and explained.
  Result: diffs confined (regex only) and explained per function; budget partially met — json
  within, regex over, under the owner's 2026-09-13 decision (re-measure and report at plan-134-H).
  Check: bench (est. 2 min); gate (est. 15 min).
Commit: —

## Validation Plan

- Tests: `rt_recursive_value_construction_copies.rs` plus D's file.
- Runtime proof: the test program on macOS and box 2223.
- Doc sync: `mfb spec memory arenas` "Scope-Drop Frees" — construction stores copy recursive
  sources too (the sentence "Record/union/collection construction … introduce no new aliases"
  becomes true for the class).
- Final gate: plan-134-H.

## Open Decisions

- None beyond plan-134-A's.

## Corrections

- **Prerequisite re-run** (2026-09-13): `ls planning/completed/plan-134-D-*` → one file — MET.
- **Not `lower_value_owned`: a construction-store helper, `lower_value_stored`.** Phase 2's first
  task says to switch each operand path to `lower_value_owned`. For a flat value that function
  copies an aliasing source with `copy_flat_block` and claims a fresh value's pending temp
  (`builder_values.rs::lower_value_owned`: `claim_pending_temp(&result)`) — but a construction
  writer byte-copies a flat payload itself, so the flat copy would be redundant and the claimed
  temp would never be freed (a leak), contradicting this letter's non-goal "Flat payloads keep
  their existing byte copies, byte-identical". `lower_value_stored` is `lower_value` plus only
  plan-134-D's recursive branch (`needs_graph_copy` + aliasing source, not a last use, not a
  borrowed view → `copy_value_to_current_arena`), with no temp claim; flat operands emit exactly
  what they did. Switched: the record `Constructor` argument loop (not the register-native vector
  lanes), `UnionWrap`, `WITH` updates, list/map-literal values and set-literal items, the
  `StateAssign` whole replace and the STATE in-place append operand, and every in-place arm's
  `item`/`val`/`rhs` operand in `builder_inplace_assign.rs` (keys and indices are never
  recursive and keep `lower_value`).
- **The census missed collection-to-collection payload copies.** The 22 paths are stores of ONE
  operand. Builtins that build a new collection by byte-copying payloads out of an existing one —
  the rebuilding `append`/`prepend`/`insert`/`set`/`add` when the in-place arm declines (args
  pre-lowered by `lower_abi_inline_args`, which also feeds `get`, `len` and every other inline
  builtin, so it cannot copy blindly), and transforms such as reverse/sort/slice/concat/filter —
  leave each copied inline record/union payload's recursive edges pointing at the SOURCE
  collection's children. Sharing between two owners is unobservable until letter G/H free both,
  where it is a double free. Added below as Phase 1/2 tasks (append-only).
- **That sharing cannot fail a value test before letter G/H.** Two collections sharing an element's
  child blocks print identical values: no in-place arm reaches a collection nested inside another
  collection's element (`collections::get` copies, `FOR EACH` borrows, and the record-field
  in-place arms decline a recursive field by G17), so nothing can mutate the shared block. It
  fails only when both owners free it. The RED for these builtins is therefore structural (the
  copy calls are absent before the fix); plan-134-G/H's churn tests with `double_free_skips = 0`
  are the behavioural proof. `collections::reverse` does not exist in the registry; the four
  builtins above cover the census's copy primitives.
- **One hook per native entry point, not one per byte-copy primitive.** The census found a dozen
  primitives but only three native entry points producing a collection: `try_abi_inline_lower`
  (every `abi_inline` member, including the `TRAP` raw path `lower_inline_builtin_raw`, which calls
  it), `try_inline_slice_op` (`#collections_slice$T`), and the intrinsic list `mid`
  (`builder_search.rs`) and `replace` (`builder_strings.rs`). Each result passes through
  `own_collection_payload_edges` (`builder_arena_transfer.rs`): a collection whose payload type
  `needs_graph_copy` gets the thread-transfer payload fix run in place (each edge read before its
  copy is written back), so every element owns its graph. User and source-generic `.mfb`
  functions return a graph their own stores already copied and are not hooked.
- **Speed gate: E roughly doubles `regex_repeat` again, and the copies are inherent.** Five passes
  of `run.sh /tmp/p134-mfb-e2 json_repeat regex_repeat`: `json_repeat` K=1 0.43 / 0.25 / 0.25 /
  0.25 / 0.25 s → median 0.25 s, RSS 212 MB; `regex_repeat` K=1 0.89 / 0.88 / 0.89 / 0.88 / 0.89 s
  → **median 0.89 s**, RSS **2.39 GB** (after D: 0.43 s, 919 MB). `-ncode` copy-shim calls in
  `#regex_run`: D 8 `__regex_Node` / 1 `__regex_Repeat` / 1 `__regex_Cont` → E 12 / 8 / 5 plus 1
  `List OF __regex_Node`; `#regex_parseQuantSuffix` +3 `__regex_Node` (one `__regex_Repeat[atom,
  …]` per quantifier, parse time only). The constructor listing (`-nir`, args that are a local
  or field) names the hot ones: every `__regex_Choice[kind, root, repRec, cont, pos, caps, …,
  stack]` push copies `root` (a parameter, never movable), `repRec` and `cont` (read again after
  the push); `__regex_ContSeq[seqNode.parts, 0, cont]` copies a pattern subtree through a borrowed
  MATCH view; `__regex_ContRep[repRec, …]` copies the repeat record. Under value semantics each
  record owns its fields, so these are the copies the design requires — and the matcher's own
  comment says `root`/`repRec` are placeholders a kind does not use ("Fields a kind does not use
  carry the root node and the current repeat as placeholders", `regex/helper_run.rs`). Removing
  them means rewriting the regex helper (the option the owner was offered and deferred), not a
  copy/move analysis change. Recorded for the plan-134-H owner report.
- **Golden localization (Phase 3), against the plan-134-D final dumps.** `-ncode` per-function
  diff (`/tmp/p134-ncode-diff.py`): **json** 162 → 162 functions, **none changed** — its decoders
  append in place with a last-use move, and no native builtin in the module returns a recursive
  collection, so neither `lower_value_stored` nor the result hook emits anything there;
  **regex** 189 → 189, changed only `#regex_run` (copy-shim calls 8 `__regex_Node` / 1
  `__regex_Repeat` / 1 `__regex_Cont` → 12 / 8 / 5 + 1 `List OF __regex_Node`, the constructor
  stores listed above) and `#regex_parseQuantSuffix` (+3 `__regex_Node`, its
  `__regex_Repeat[atom, lo, hi, greedy]` constructor). Nothing added or removed.

## Summary

Completes the ownership tree: after E no two owners share a recursive block. Still frees
nothing, so still cannot corrupt memory; its failure mode is a leftover share that G would
double-free, which the per-store mutation test is there to catch.
