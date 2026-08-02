# plan-75: thread::transfer / accept of a resource union

Last updated: 2026-08-02
Effort: medium–large (measure Phase 0 first)
Depends on: plan-74 (union STATE — landed). This plan is the transfer facet plan-74's
Phase 5 turned out to require, and which plan-74 mis-scoped as a one-function STATE deep-copy.

## Why this is its own plan

plan-74 delivered uniform STATE on a resource union at the binding, parameter, return,
`.state` access (value / parameter / MATCH), and scope-drop. Its Phase 5 assumed
`thread::transfer` of a resource union already worked and needed only a STATE deep-copy in
`copy_union_to_current_arena`. **Measured 2026-08-02: resource-union `thread::transfer` /
`thread::accept` does not compile at all** — for a *stateless* resource union as much as a
stateful one. It is an unimplemented feature spanning many layers, each a separate STATE-suffix
or resource-union-classification gap. plan-74's §2.3 "Verified" row only checked that
`copy_union_to_current_arena` raw-copies `{tag,ptr}`; it never compiled an end-to-end transfer.

The type surface works — `ThreadWorker OF RES Stream STATE Cursor TO Integer` and
`thread::accept` returning `RES Stream STATE Cursor` both compile and produce a `.mfp`. The
*consumer* side (transfer) is what fails, in the native lowering.

## Measured gaps (the cascade), in the order the compiler hits them

Reproduce with a worker package exporting
`FUNC take(t AS ThreadWorker OF RES Stream STATE Cursor TO Integer, seed AS String) AS Integer`
and a consumer that `thread::transfer`s a `RES Stream STATE Cursor`. Each gap below blocked the
next; all are keyed on a resource union being spelled `Stream STATE Cursor` (STATE suffix) and/or
being a `{tag,ptr}` pointer handle rather than a data block:

1. **Declared runtime-helpers** — `runtime/usage.rs::push_op_helpers` looks up the union
   close-helper map with the full `Stream STATE Cursor` against a bare-`Stream` key, so a
   transferred union's variant closes are never *declared*, while the validator marks them
   *used* → `NIR runtime call requires undeclared helper 'net'`. Fix: `base_resource_name`.
2. **Transfer copy dispatch** — `builder_arena_transfer.rs::emit_thread_copy_real` routes on
   `union_names.contains(other)`, which fails on the STATE suffix, so a stateful union never
   reaches `copy_union_to_current_arena`. Fix: strip base.
3. **Variant-record deep-copy** — `copy_union_to_current_arena` raw-copies the `{tag,ptr}`
   block only; the `+8` record pointer still aliases the sender's arena (a bug-257-class UAF,
   **true for a stateless resource union too**). The variant's 80-byte record (with its uniform
   STATE) must be deep-copied via `copy_resource_to_current_arena` and the copy's `+8`
   repointed. `union_is_data` / `inline_collection_payload_size` also need the base.
4. **Transfer size / Result payload** — the transfer materializes a `Result OF <resource-union>`
   whose payload classification (`result_payload_is_block`) treats a resource union as an
   inlinable data block (`union_names.contains`), so `emit_inlined_block_size_from_ptr_slot`
   fails: `native inlined field size not available for type 'Stream STATE Cursor'`. A resource
   union is a scalar *pointer* payload like a concrete resource — only a **data** union is a
   block. Fix: `union_is_data(base)`.
5. **Accept side (unexplored)** — `thread::acceptResource` materializing / extracting the
   resource union on the receiver, and the STATE-plane agreement across the boundary, are not
   yet audited; expect further resource-union / STATE-suffix gaps mirroring the send side.

## Prerequisites

| Must be true | Command | Status (measured 2026-08-02) |
|---|---|---|
| plan-74 landed (union STATE) | `ls planning/completed/plan-74-*` | MET — `planning/completed/plan-74-resource-union-state.md` present |
| A stateful **concrete**-resource transfer works (the reference path) | `tests/rt-behavior/threads/thread-transfer-state-rt` is green | MET — `test-accept.sh ./target/release/mfb <out> thread-transfer-state-rt` → "acceptance tests passed (1 test(s) ran)" |

## Phases

Dependency order: Phase 0 → Phase 1 → Phase 2 → Phase 3 (linear; each depends on the
previous). Phase 0 measured before scheduling the rest.

### Phase 0 — measure & confirm the gaps (acceptance: every gap confirmed by symbol; accept side audited)

Commit: ad5fd7bb6 (prereq gate) + this phase's commit

- [x] Write a failing worker(`union_xfer_workers`)+consumer fixture transferring
  `RES Stream STATE Cursor` (`UNION Stream { File, Socket }`). The worker package's type
  surface compiles and produces a `.mfp`; the consumer fails to build. Reproduced at
  `/tmp/p75repro` with the release `mfb`.
- [x] **Gap 1 confirmed (runtime error + symbol):** consumer build errors
  `NIR runtime call requires undeclared helper 'net'`. Root cause:
  `src/target/shared/runtime/usage.rs::push_op_helpers` looks up `resource_union_closes`
  (keyed on the bare union name `"Stream"`, see `required_helpers` line 126) with the
  bind's full type `"Stream STATE Cursor"` (line 162) → miss → `net` (Socket's close) never
  declared while the validator marks it used. Fix: strip base via `base_resource_name`.
- [x] **Gap 2 confirmed (symbol):** `builder_arena_transfer.rs::emit_thread_copy_real`
  line 359 routes the union arm on `self.type_model.union_names.contains(other)`;
  `union_names` holds only `"Stream"` (validation.rs:260), so `"Stream STATE Cursor"` misses
  and falls to the `else` "cannot copy value of type" error. Fix: strip base.
- [x] **Gap 3 confirmed (symbol):** `copy_union_to_current_arena` (line 603) sizes the
  resource union via `inline_collection_payload_size` (16-byte `{tag,ptr}`) and raw-`memcpy`s
  it, then `copy_union_fields_into_existing` (line 1020). For a *resource* union the
  non-data-union loop (line 1103) iterates `union_variant_fields[variant]` which is EMPTY for
  resource variants (File/Socket have no record fields) → the `+8` record pointer is never
  deep-copied → it aliases the sender's arena (bug-257-class UAF, **true stateless too**).
  Also `union_is_data`/`inline_collection_payload_size`/`variants_for_union` all key on the
  bare union name and miss the STATE suffix. Fix: deep-copy the variant record via
  `copy_resource_to_current_arena` and repoint `+8`; strip base at the classification calls.
- [x] **Gap 4 confirmed (symbol):** `result_payload_is_block` (builder_value_semantics.rs:949)
  classifies a union payload via `union_names.contains(payload_type)`; a resource union is a
  scalar *pointer* payload (a pointer to the `{tag,ptr}` block), like a concrete resource —
  only a **data** union is an inlinable block. `emit_inlined_block_size_from_ptr_slot`
  (builder_collection_layout.rs:685 else-arm) is the source of
  `native inlined field size not available for type '…'`. Fix: `union_is_data(base)`.
- [x] **Accept side audited (gap 5):** `thread::accept` lowers to `thread.acceptResource`
  (builder_values.rs:1911), routed to the `thread.acceptResource`/`thread.readResource`
  runtime helper by handle direction. The receiver-side deep-copy reuses the SAME
  `emit_thread_copy_real` machinery (transfer runs arena-switched to the destination, accept
  runs in the receiver's own arena — copy_resource_to_current_arena doc, line 421), so gaps
  1–4's fixes cover both directions. `thread_copy.<type>` standalone functions are generated
  only for `recursive_transfer_types` (mod.rs:1290); a resource union is copied inline via
  `copy_value_to_current_arena`. Remaining accept-specific risk (materialize/extract of the
  union on the receiver, STATE-plane agreement) is verified empirically in Phase 2 by
  building the consumer end-to-end after the Phase 1 fixes.

### Phase 1 — send-side fixes, gaps 1–4 (acceptance: the `/tmp/p75repro` consumer builds & links; a stateless-union transfer also builds)

Commit:

- [ ] Gap 1: base-strip the `resource_union_closes` lookup in `push_op_helpers` so a
  transferred stateful union declares its variant close helpers.
- [ ] Gap 2: base-strip the union dispatch in `emit_thread_copy_real` so a stateful union
  reaches `copy_union_to_current_arena`.
- [ ] Gap 3: in `copy_union_fields_into_existing`, deep-copy a **resource** union's active
  variant record (`+8`) via `copy_resource_to_current_arena` and repoint `+8`; base-strip
  `union_is_data`/`inline_collection_payload_size`/`variants_for_union`/`copy_union_*` so the
  STATE suffix resolves. This also fixes the **stateless** resource-union aliasing UAF — file
  it as a bug (write-bug) since it is a latent memory-safety defect independent of this plan.
- [ ] Gap 4: classify a resource-union `Result` payload as a scalar pointer
  (`union_is_data(base)` in `result_payload_is_block`).
- [ ] Verify: `/tmp/p75repro` consumer builds & links with the worktree release `mfb`; a
  stateless-union variant (no STATE) also builds. `cargo test --bin mfb` green.

### Phase 2 — accept-side agreement & end-to-end run (acceptance: the transfer runs and prints 99; STATE independent across the boundary)

Commit:

- [ ] Build & run the `/tmp/p75repro` end-to-end transfer; it must print `99` (STATE
  arrived intact across the boundary).
- [ ] If the receiver materialize/extract path surfaces further resource-union / STATE-suffix
  gaps (gap 5), fix them here and record each in Corrections.
- [ ] Prove sender/receiver payload independence: a 20× repeat run stays green (no
  shared-arena UAF); a stateless-union transfer also runs clean.

### Phase 3 — committed fixtures (acceptance: rt-behavior fixtures green in test-accept; artifact-gate green)

Commit:

- [ ] Add a worker package under `tools/thread-package-sources/union_xfer_workers/` and
  regenerate its `.mfp` via `scripts/sync-package-mfp.sh` with the **release** build.
- [ ] Add a stateful rt-behavior fixture
  `tests/rt-behavior/threads/thread-transfer-union-state-rt/` (transfers a stateful union,
  asserts `99`) with goldens synced via `scripts/sync-goldens.sh`.
- [ ] Add a **stateless** resource-union transfer fixture too — it was equally broken.
- [ ] `scripts/test-accept.sh`, `scripts/artifact-gate.sh` green.

## Validation

`cargo test --bin mfb`, `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, plus the on-device
thread run. Because `result_payload_is_block` and the transfer copy are hot paths, run the full
acceptance suite — a resource-union classification change can ripple to `Result` handling.

## Corrections

<!-- Filled in during execution. -->
