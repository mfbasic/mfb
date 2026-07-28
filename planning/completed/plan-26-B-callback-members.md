# plan-26-B: Inline-TRAP for the collection callback members

Last updated: 2026-07-06
Effort: medium (1h–2h)

Make inline `TRAP` work directly on the collection callback members —
`collections::forEach`, `transform`, `filter`, `reduce` — so a failing user
callback is trappable at the call site like any other fallible call, instead of
forcing the "wrap it in a FUNC/SUB" workaround. This is the plan-21-B raw-`Result`
capture extended from the index/range members to the callback loop members, plus
the loop-scoped cleanup those members require.

The single behavioral outcome: `out = TRAP collections::transform(list, mayFail)
BINDING e ... RECOVER []` compiles, runs the loop, and if `mayFail` fails on some
element, the partially-built output list is freed exactly once and control enters
the handler with the callback's `Error` — no leak, no double-free.

Depends on: plan-26-A (front-end no longer rejects infallible members; this
sub-plan retires the remaining `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` rejection for
the callback set).

It complements:

- `./mfb spec language error-model` (auto-propagation vs. inline TRAP; specs under
  `src/docs/spec/**`).
- `./mfb spec memory resource-management` (the scope-drop/free discipline the
  cleanup path must honor).
- `./mfb man collections transform` / `filter` / `reduce` / `forEach`.

## 1. Goal

- Inline `TRAP` on `forEach`/`transform`/`filter`/`reduce` compiles and, on a
  callback failure, routes the callback's `Error` to the inline-TRAP handler (or the
  function-level TRAP / caller when no inline handler), freeing all loop-scoped
  intermediates exactly once.
- `inline_builtin_raw_supported` (`src/builtins/mod.rs:189`) includes these four
  members.
- `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` (`2-203-0102`) no longer fires for them;
  the codegen backstop at `builder_values.rs:781` no longer rejects them.

### Non-goals (explicit constraints)

- No change to the success-path lowering of these members: the non-trapped call
  path (auto-propagate on callback failure via `return_()`) stays byte-identical.
  Only the *trapped* path is new.
- No change to value/copy/move/drop semantics: the freed intermediates are the
  members' own private accumulators/counters, never a caller-owned value.
- No language-surface change beyond removing a rejection.
- `reduce`'s seed/accumulator ownership on the failure path must match the
  success-path drop exactly — no new leak, no double-free.

## 2. Current State

- Lowerings: `src/target/shared/code/builder_collection_queries.rs` —
  `lower_collection_for_each_call` (`:764`), `lower_collection_transform_call`
  (`:895`), `lower_collection_filter_call` (`:984`), `lower_collection_reduce_call`
  (`:1077`).
- The failure seam (transform, representative): after the callback returns, the tag
  is compared and on non-`Ok` a **bare `abi::return_()`** propagates the raw Result
  (`builder_collection_queries.rs:958-960`). This consults neither
  `raw_result_capture` nor frees the partially-built `output` list
  (allocated `:932`, tracked in `output_slot` `:933`). filter/reduce/forEach have the
  same shape with their own accumulators (filter: output list; reduce: accumulator
  slot; forEach: none).
- The capture mechanism: `emit_error_register_return`
  (`src/target/shared/code/builder_codegen_primitives.rs:737`) branches to
  `raw_result_capture` when set — but the callback failure does **not** go through
  that helper; it is a plain call-boundary Result already in the standard registers.
- Raw wrapper: `lower_inline_builtin_raw` (`builder_values.rs:1363`) sets the
  capture label and expects a **single-register** success fall-through
  (`:1384-1393`). transform/filter/reduce yield a single collection/accumulator
  register (fits); **forEach yields Nothing** (does not fit — needs a no-value
  fall-through).

## 3. Design Overview

Three layered pieces:

1. **Capture-aware callback-failure exit.** Replace the bare `return_()` at each
   member's callback-failure branch with a helper that, when `raw_result_capture`
   is set, (a) runs the member's loop-scoped cleanup, then (b) branches to the
   capture label with the raw `Result` already in the standard registers; otherwise
   emits today's `return_()` (byte-identical non-trapped path).

2. **Loop-scoped cleanup on the failure path.** Each member frees its own private
   intermediates before joining the capture: transform/filter free the partial
   output list; reduce frees the live accumulator if it is an owned value; forEach
   frees nothing. This reuses the existing owned-value free path (the same call the
   success path uses at scope end), invoked once on the early exit.

3. **Success fall-through shape.** transform/filter/reduce already produce a single
   success register — wire them through `lower_inline_builtin_raw`'s existing
   fall-through. forEach needs a **Nothing** success fall-through added to
   `lower_inline_builtin_raw` (tag `Ok`, no value register, materialize `Result OF
   Nothing`).

Correctness risk concentrates in (2): the failure-path free must free exactly the
set the success path would have freed by that point and nothing the handler/caller
still owns. The source `collection` and the callback `action` are borrowed inputs
(not freed by the member); only the member-private `output`/accumulator is freed.

## 4. Detailed Design

### 4.1 Capture-aware exit helper (`builder_collection_queries.rs`)

- Add a small builder method, e.g. `emit_callback_failure_exit(cleanup: impl
  FnOnce(&mut Self))`, that:
  - if `self.raw_result_capture` is `Some(label)`: run `cleanup(self)` (free the
    member's intermediates), then `emit(abi::branch(&label))` — leaving the raw
    `Result` in `RESULT_TAG_REGISTER`/`RESULT_VALUE_REGISTER` as the callback left it;
  - else: `emit(abi::return_())` (today's behavior, byte-identical).
- Replace the four bare `return_()` failure branches
  (transform `:960`, and the analogous lines in filter/reduce/forEach) with a call
  to this helper, passing each member's cleanup closure.

### 4.2 Per-member cleanup closures

- **transform / filter:** free the partial output list held in `output_slot`
  (load the pointer, call the owned-list free path used at scope drop). The output is
  a private uniquely-owned buffer (`builder_collection_queries.rs:969-972` comment),
  so freeing it on early exit is sound and matches the success path's eventual drop.
- **reduce:** free the current accumulator if it is an owned type (String/collection
  seed); a scalar accumulator needs no free. Mirror the success-path accumulator
  ownership.
- **forEach:** no accumulator — empty cleanup.
- In every case the source `collection` and `action` are inputs the member does not
  own, so they are **not** freed here (freed by the caller's scope as today).

### 4.3 Success fall-through for forEach (`builder_values.rs:1363`)

- Add a `forEach` arm to `lower_inline_builtin_raw` that lowers the call and, on
  success fall-through, tags `Ok` with **no** value register and materializes
  `Result OF Nothing`. Factor the success tail so the single-register members reuse
  the existing path and forEach takes the no-value path.

### 4.4 Gate + predicate updates

- `src/builtins/mod.rs:189` `inline_builtin_raw_supported`: add
  `"forEach" | "transform" | "filter" | "reduce"` to the allowed set. This
  automatically removes them from `inline_trap_unsupported` (`:171`, defined as
  fallible native target `&& !raw_supported`).
- `src/target/shared/code/builder_values.rs:764-781`: these members already route to
  their dedicated `lower_collection_*_call`; ensure that under a trapped call they
  run with `raw_result_capture` set (the raw path sets it) and the backstop at `:781`
  no longer rejects them.
- Update the doc comments at `builtins/mod.rs:178-214` (which currently document the
  callback members as *excluded*) to reflect that they are now raw-supported and how.

## Layout / ABI Impact

None. No struct/record/`.mfp`/register-model change. The non-trapped native output
for these members is byte-identical (the failure exit is unchanged when
`raw_result_capture` is `None`). Verified by the artifact gate.

## Phases

### Phase 1 — capture-aware failure exit + predicate (no forEach value shape yet)

Land transform/filter/reduce (single-register success) end-to-end first; they share
the existing fall-through and are the higher-value members.

- [x] Add `emit_callback_failure_exit` to the builder
      (`builder_collection_queries.rs`) per §4.1.
- [x] Replace the failure `return_()` in transform (`:960`), filter, and reduce with
      the helper + each member's cleanup closure (§4.2).
- [x] `src/builtins/mod.rs:189`: add `transform`/`filter`/`reduce` to
      `inline_builtin_raw_supported`; update the doc comments (`:178-214`).
- [x] Relax the `builder_values.rs:781` backstop for these three.

Acceptance: `TRAP collections::transform(list, mayFail) ... RECOVER []` compiles and,
at runtime, on a mid-loop callback failure enters the handler with the `Error` and
leaks nothing (proof: run under the leak-count harness / arena free==alloc). Artifact
gate byte-identical except the new fixtures.
Commit: —

### Phase 2 — forEach (Nothing success shape)

- [x] Add the no-value success fall-through to `lower_inline_builtin_raw`
      (`builder_values.rs:1363`) and a `forEach` arm (§4.3).
- [x] Add `forEach` to `inline_builtin_raw_supported`; relax the backstop; empty
      cleanup closure on its failure exit.

Acceptance: `TRAP collections::forEach(list, mayFail) ... RECOVER 0` (value-less
context) compiles and traps a callback failure into the handler; artifact gate clean.
Commit: —

### Phase 3 — front-end retirement + fixtures + spec

- [x] `src/syntaxcheck/inference.rs:127-140`: the `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`
      branch no longer fires for the four callback members (they are now raw-supported,
      so `inline_trap_unsupported` returns false for them — verify the branch is
      naturally dead for them, and keep it for any remaining unsupported inline target).
- [x] `tests/func_collections_transform_valid/**` (+ filter/reduce/forEach): inline
      TRAP over a failing callback with a runtime proof of handler entry, correct
      `Error`, and no leak; plus the success path unchanged.
- [x] `tests/func_collections_transform_invalid/**` etc.: keep any genuinely-invalid
      cases; remove the now-stale "inline TRAP unsupported" invalid fixtures (or
      convert them to valid).
- [x] `src/docs/spec/language/error-model.md` + `./mfb man collections <fn>`: document
      that inline TRAP is supported on the callback members.
- [x] If no inline target remains that warrants `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`,
      coordinate its retirement with plan-26-C (which owns the diagnostics-table
      cleanup); otherwise narrow its description.

Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` green; the
four members' man pages and error-model spec updated.
Commit: —

## Validation Plan

- Function tests: `tests/func_collections_{transform,filter,reduce,forEach}_valid/**`
  and `_invalid/**`, covering success, inline-TRAP-caught callback failure, and
  function-level-TRAP/propagated callback failure.
- Runtime proof: a program whose callback fails on the k-th element; assert the
  handler runs with the callback's `Error`, the result is the RECOVER value, and
  arena free-count == alloc-count (no partial-output leak). Cross-check against a
  double-free by running the same under both arches per [[No rebuild during acceptance]].
- Doc sync: `mfb spec language error-model`, `mfb man collections transform|filter|reduce|forEach`.
- Acceptance: `scripts/test-accept.sh`; codegen non-regression via
  `scripts/artifact-gate.sh`.

## Open Decisions

- **reduce accumulator free on failure — free vs. let scope-drop handle it.**
  Recommend **free in the cleanup closure**, because the early branch to the capture
  label skips the member's normal end-of-body drop; leaving it would leak an owned
  seed. (Alternative: route the failure through the normal scope-drop walk — heavier
  and diverges from the plan-21-B pattern.) Verify with the leak-count harness which
  seed types actually own heap memory.

## Non-Goals

- Infallible-member TRAP passthrough (plan-26-A).
- `expectTrap`/`expectNTrap` parity and final diagnostics-table cleanup (plan-26-C).

## Summary

The mechanism already exists (plan-21-B raw capture); this extends it over a loop
body with a private accumulator. The real risk is the failure-path free: it must
free exactly the member-private intermediate and nothing borrowed — validated by an
alloc==free leak-count proof, not just golden output.
