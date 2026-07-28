# bug-391: thread transfer cannot deep-copy a recursive value; a naive record arm hangs codegen

Last updated: 2026-07-27
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Correctness

Status: FIXED
Regression Test: tests/rt_recursive_thread_transfer.rs

## STATUS: FIXED

The transfer copier now emits a **per-type runtime deep-copy function** for every
recursive type and routes a recursive edge to a *call* to it, so the copy
recurses at run time over the finite data rather than at compile time over the
(infinite) type. Landed as:

- `src/target/shared/code/builder_collection_layout.rs` — `type_components`,
  `type_participates_in_cycle`, `recursive_transfer_types`, `thread_copy_symbol`
  (detect recursive types + their closure; name the copy function).
- `src/target/shared/code/builder_arena_transfer.rs` — `copy_value_to_current_arena`
  routes a recursive type to `emit_thread_copy_call` (a call to `thread_copy_symbol`)
  and otherwise to `emit_thread_copy_real` (the per-shape body, now including the
  previously-missing **non-flat record** arm, `copy_record_to_current_arena`).
- `src/target/shared/code/function_lowering.rs` — `lower_thread_copy_function`
  emits each per-type copy function (source ptr in arg0 → fresh ptr in the return
  register; its body is `emit_thread_copy_real`, whose sub-edges call the peers).
- `src/target/shared/code/mod.rs` — the driver emits one copy function per
  `recursive_transfer_types(&type_model)` entry, and ensures the empty-string data
  object exists (the alloc-failure path builds an empty-message error).

Verified on macos-aarch64: `tests/rt_recursive_thread_transfer.rs` passes (a bare
`Node` **and** a record embedding one both transfer and read back correctly after
the worker arena is reclaimed — proving a real deep copy, not aliasing); full
`cargo test` green; the `examples/browser` `fetch` worker now returns the parsed
DOM `Node` across the thread and the app renders it.

Deviations from the written design: (1) implemented the **full** feature, not the
interim diagnostic; (2) no shared "needed types" registry was threaded — the
driver emits a copy function for **every** recursive type in the type model
(closed under component references) rather than only transferred ones, which is
simpler and stays byte-identical for programs with no recursive types
(`recursive_transfer_types` is empty → no functions, unchanged code path).

A `thread::start` worker whose result type is a **recursive** value — a self-referential
union like `Node = ElementNode | TextNode` where `ElementNode.children : List OF Node`,
or any record/collection embedding one — cannot have its result deep-copied out of the
worker arena. Two distinct failure modes, both rooted in the same gap:

- A **record** containing such a field fails today with `native thread transfer cannot
  copy value of type 'LoadResult'` — `copy_value_to_current_arena` has no arm for a
  non-flat record, so it hits the `else` error.
- A bare **union**/**collection** of a recursive type (or the record case, once a record
  arm is added) makes the **code generator recurse without bound over the type** and
  **hangs `mfb`** — the transfer copier inlines every level and has no cycle guard.

The single correct behavior a fix produces: a worker may return a recursive value (a DOM
`Node`, or a record/list/map containing one), and `thread::waitFor` deep-copies it into
the parent arena as an independent value (value semantics), in bounded compile time and
without heap corruption.

This is dangerous on two counts: the record case is a hard wall for a legitimate pattern
(a worker that parses structured data and returns the tree), and the union/collection case
is a **compiler hang** — the worst kind of failure to hit by accident.

References:

- `src/target/shared/code/builder_arena_transfer.rs` — `copy_value_to_current_arena` and
  the `copy_*_fields_into_existing` / `copy_collection_to_current_arena` helpers.
- `src/target/shared/code/builder_collection_layout.rs` — `type_is_flat` (why a recursive
  type is non-flat: no inline fixed point), `emit_record_block_size_to_slot`.
- `src/docs/spec/architecture/05_binary-representation.md`, `.../threads/*` — thread
  result transfer / arena isolation.
- Found building `examples/browser`: `fetch::fetch` wanted to return a `LoadResult`
  carrying a `dom::Node` document; worked around by returning the body `String` and
  parsing on the app thread. Sibling of bug-390 (also a missing copy-arm in a serializer).

## Failing Reproduction

A recursive union in a package, and a worker returning it (bare, and inside a record).

```
# dom package: a recursive Node union + constructors (see examples/browser/dom)
#   ElementNode { tag, attrs: Map OF String TO String, children: List OF Node }
#   TextNode { text }
#   UNION Node = ElementNode | TextNode
#
# worker package `w` (imports dom):
#   EXPORT ISOLATED FUNC bare(t AS ThreadWorker OF String TO Node, s AS String) AS Node
#     RETURN dom::element("p", ..., [dom::textNode(s)])
#   END FUNC
#   EXPORT TYPE Box  doc AS Node  END TYPE      # (field name avoids the DOC keyword)
#   EXPORT ISOLATED FUNC boxed(t AS ThreadWorker OF String TO Box, s AS String) AS Box
#     RETURN Box[ dom::element("p", ..., []) ]
#   END FUNC
#
# app:
#   LET t = thread::start(w::boxed, "x") ; LET r = thread::waitFor(t)    # -> error today
#   LET t = thread::start(w::bare,  "x") ; LET r = thread::waitFor(t)    # -> compiler hang
```

- Observed (record result, `Box`): build aborts with
  `error: native thread transfer cannot copy value of type 'Box'`.
- Observed (bare `Node` result, or a record arm added): `mfb build` **does not terminate**
  — the codegen recurses `Node → ElementNode → List OF Node → Node → …` emitting inline copy
  code forever.
- Expected: both build, and `waitFor` returns a deep, independent copy of the tree.

Contrast cases that work correctly today (bound the bug):

- A worker returning a **flat** record (all scalar/String/`List OF String`/flat-`Map`
  fields — e.g. the current `fetch::LoadResult` after the workaround) transfers fine: the
  whole block is one `memcpy`.
- A worker returning a **non-recursive** non-flat value — e.g. `List OF String`, a record
  with a `List OF Integer` — transfers fine: the inline recursion terminates.
- **Same-thread** use of a recursive value is completely fine: `LET b = a` over a
  `List OF Node` deep-copies (proven: mutating `b` leaves `a` unchanged), a `Node` is built
  by `dom::parse`, returned, passed to `display::render`, and traversed — all value-semantic.
  The defect is specific to the **cross-arena thread-transfer** copier.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | macos-aarch64, release `mfb` | record → error ✗; bare union/collection → hang ✗; flat/non-recursive → ✓ |

(The copier is in `src/target/shared/code` (shared), so all native targets are expected to
behave identically; confirm during the fix.)

## Root Cause

`src/target/shared/code/builder_arena_transfer.rs` — `copy_value_to_current_arena` dispatches
by shape: scalars, flat blocks (`type_is_flat` → `memcpy`), collections, resources, and
unions. **There is no arm for a non-flat record**, so `LoadResult`/`Box` fall to:

```rust
other => Err(format!("native thread transfer cannot copy value of type '{other}'")),
```

That is the record error. But the deeper defect is that every deep-copy helper —
`copy_record_fields_into_existing`, `copy_union_fields_into_existing`,
`copy_collection_to_current_arena` — deep-copies a pointer field by calling
`self.copy_value_to_current_arena(field_type, …)` **inline**, and there is **no
cycle/emitted-type guard** in the file (verified by search: no `visited`/`in_progress`/
recursion set). For a self-referential type the emitter therefore recurses over the *type*
forever:

```
copy(Node) → copy_union_fields_into_existing → variant ElementNode
           → field children: copy(List OF Node) → copy_collection (pointer payload)
           → per-element copy(Node) → copy_union_fields_into_existing → … ∞
```

`type_is_flat` correctly classifies `Node` as non-flat (a recursive type has no inline
fixed point — the self-edge must be an out-of-line pointer), so the fast `memcpy` path can
never apply; the value is a graph of allocations that must be walked. The transfer copier
walks it by **compile-time inlining**, which only terminates for non-recursive types. The
record case merely fails earlier (missing arm) instead of hanging.

Why same-thread copies are immune: they do not go through this cross-arena copier.

## Goal

- A worker result of a recursive type (a `Node`, or a record/list/map containing one) is
  deep-copied by `thread::waitFor` into the parent arena as an independent value, in
  **bounded compile time** and with no free-list corruption.
- No `mfb` build ever fails to terminate because of a recursive transfer type.

### Non-goals (must NOT change)

- The flat `memcpy` fast path, and the byte-identity of transfer codegen for every
  non-recursive type it already handles.
- Value semantics of same-thread copies (already correct) and the arena free model.
- **No masking.** Do not "fix" this by making the frontend reject a recursive worker
  result (that re-bans the legitimate pattern), by silently shallow-copying (which would
  alias across arenas — a use-after-free when the worker arena is reclaimed), or by editing
  the repro to avoid the recursive type. A clean *diagnostic* replacing the hang is an
  acceptable **interim** (see Fix Design) but not the resolution.

## Blast Radius

Searched `tests/` for a thread worker whose result type is a user union/recursive type:
none — no fixture transfers a recursive value, so nothing regresses today; this is a latent
capability gap + a latent compiler-hang.

- `copy_value_to_current_arena` (`builder_arena_transfer.rs`) — the missing record arm and
  the inline-recursion; fixed by this bug.
- `copy_record_fields_into_existing` / `copy_union_fields_into_existing` /
  `copy_collection_to_current_arena` — must route recursion through the new mechanism; in
  scope.
- `examples/browser` (this worktree) — the motivating consumer; out of scope for the fix,
  it uses the String-body workaround (and arguably should regardless — the network, not the
  parse, is what must leave the UI thread).

## Fix Design

Break the compile-time recursion by emitting a **per-type runtime deep-copy function** for
each non-flat type reachable across a transfer, and **calling** it. Maintain an
"emitting" set: when `copy_value_to_current_arena` is asked to copy a type already on the
emit stack (a cycle), it emits a `call copy_<type>` (source ptr in the arg register → new
ptr in the return register) and enqueues `copy_<type>` for one-time emission if not already
present. `copy_<type>`'s body is the current per-shape logic (alloc + `memcpy` + fixup),
except its self-recursive edges become **calls** to the worklisted functions. Non-recursive
sub-parts may stay inlined. Add the missing **non-flat record** arm as
`copy_record_to_current_arena` (size via `emit_record_block_size_to_slot`, `arena_alloc`,
`memcpy`, then `copy_record_fields_into_existing`), which becomes the body of a `copy_<record>`
function when the record participates in a cycle.

Rejected alternative — **keep inlining but cap depth**: rejected; depth is a property of the
runtime tree, not the type, so any static cap either truncates real data or still doesn't
terminate over the type.

Interim (may land first, on its own): when the emit stack would re-enter a type, and until
the function mechanism exists, emit a **precise compile error** ("thread transfer of
recursive type `Node` is not yet supported") instead of looping — strictly better than a
hang, and a safe stopgap.

Expected output shift: none for existing non-recursive transfers; new fixtures add new
codegen.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add fixtures: a package with a recursive union + constructors; a worker returning it
      bare, and a worker returning a record embedding it. Assert the record case fails with
      the current error and (guarded so CI can't hang, e.g. a build subprocess with a
      timeout) that the bare/record-arm case does not terminate today.
- [ ] Add the passing contrast fixtures (flat record result, non-recursive non-flat result).
- [ ] Record blast-radius verdicts (done above).

Acceptance: fixtures reproduce both failure modes for the documented reason; contrasts pass.
Commit: —

### Phase 2 — the fix (or interim diagnostic)

- [ ] Add an emitting-type set + a worklist of per-type copy functions to the transfer
      builder; emit `copy_<type>` bodies and route cyclic edges to `call`s.
- [ ] Add `copy_record_to_current_arena` and wire the non-flat-record arm into
      `copy_value_to_current_arena`.
- [ ] (Or interim:) emit the precise recursion diagnostic and stop.

Acceptance: Phase 1 recursive fixtures build in bounded time and `waitFor` returns a deep,
independent copy (mutating the parent's copy leaves nothing shared with the worker); the
contrast fixtures are byte-identical; nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate any transfer-codegen goldens the new fixtures add; confirm no existing
      transfer output changed (byte-identity guard).
- [ ] Full `cargo test` + artifact gate.
- [ ] Re-run the reproduction on every target in the matrix.

Acceptance: full suite green; deltas are only the new fixtures; the reproduction builds and
runs where it previously errored/hung.
Commit: —

## Validation Plan

- Regression test(s): the recursive-worker-result fixtures under `tests/`.
- Runtime proof: a worker returns a small `Node` tree; the app deep-mutates its copy and
  shows the two are independent, and traverses the transferred tree correctly.
- Doc sync: note in the threads/BR architecture spec which result shapes transfer and how a
  recursive one is deep-copied.
- Full suite: `cargo test` (workspace) + artifact gate.

## Open Decisions

- Full per-type copy functions vs. interim diagnostic — recommend interim first (kills the
  hang safely), then the functions. (§Fix Design)
- Whether to emit one copy function per participating type unconditionally, or only for
  types on a detected cycle (non-recursive stays inlined) — recommend cycle-only, to keep
  existing output byte-identical. (§Fix Design)

## Summary

The engineering risk is concentrated in a new codegen mechanism: per-type runtime deep-copy
functions with cycle detection, without disturbing the flat `memcpy` fast path or the
byte-identity of existing transfer codegen, and without mis-sizing a block (bug-371-class
free-list corruption). The root cause is precise: the transfer copier deep-copies by
compile-time inlining with no cycle guard, so a recursive value either hits a missing record
arm (error) or recurses forever (hang). Same-thread value semantics are already correct and
untouched; the interim diagnostic can land immediately to remove the hang.
