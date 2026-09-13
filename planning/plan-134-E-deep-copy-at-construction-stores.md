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

Prerequisites: see plan-134-A; plan-134-D complete (`ls planning/completed/plan-134-D-*`).

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

UNMEASURED: the exact list of construct lowerings that feed these writers with a
`NirValue` operand (the construct ops' lowering entry points). Phase 1's first task measures it.

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

- [ ] Measure the construct lowerings: `grep -rn "fn lower_.*construct\|NirValue::Constructor\|NirValue::ListLiteral\|NirValue::MapLiteral" src/codegen --include='*.rs'`
      and every caller of the writers in §2; record each operand path (file::symbol, `lower_value`
      or `lower_value_owned`) in §2 with the count.
- [ ] `tests/runtime/rt_recursive_value_construction_copies.rs` (register in `Cargo.toml`):
      one program, one line per store in §2 — a node read after being put into a record field,
      a union variant, a list literal, `append`, `insert`, `set`, `prepend`, a `Map` value, a
      `WITH` replacement, a `STATE` field — then the source's list is rebuilt and each holder
      re-read. Confirm it fails today.

Acceptance: §2 has no UNMEASURED row; the test fails on main.
  Check: `cargo test --release --test rt_recursive_value_construction_copies` → failed (est. 2 min).
Commit: —

### Phase 2 — copy at each store

- [ ] Switch each operand path found in Phase 1 to `lower_value_owned`.
- [ ] Inline payload edge copy after the byte copy (`emit_copy_payload_to_collection`,
      `emit_wrap_record_in_union`), reusing the walker's edge enumeration.
- [ ] `StateAssign` → `lower_value_owned`.
- [ ] Tests: Phase 1 passes; add an alloc-count pin — `node_copies` under `--debug` shows
      exactly the copies the census predicts (no double copy of a fresh constructor).

Acceptance: every construction store yields an independent graph, with no redundant copy.
  Check: `cargo test --release --test rt_recursive_value_construction_copies --test
  rt_recursive_value_copies` → passed (est. 4 min).
Commit: —

### Phase 3 — speed and goldens

- [ ] Bench medians (`tools/recursive-value-bench/run.sh … json_repeat regex_repeat`, ×5) within
      the plan-134-A budget; a miss is traced with `--debug` alloc counts to the store and fixed.
- [ ] Artifact gate; expected diffs: json, regex, recursive-user-type fixtures; regenerate.

Acceptance: within budget; diffs confined and explained.
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

(Filled in during execution.)

## Summary

Completes the ownership tree: after E no two owners share a recursive block. Still frees
nothing, so still cannot corrupt memory; its failure mode is a leftover share that G would
double-free, which the per-store mutation test is there to catch.
