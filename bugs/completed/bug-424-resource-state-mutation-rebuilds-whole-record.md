# bug-424: mutating a resource STATE field rebuilds the whole STATE record (O(n²) accumulation, no in-place mutation)

Last updated: 2026-08-03
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Footgun (silent super-linear performance; no wrong result, no crash — but effectively a hang / potential OOM at scale)

Status: FIXED (Layer 1 — scalar STATE field in-place store) — collection half split to bug-430
Regression Test: `tests/rt_res_state_inplace_mutation.rs` (`scalar_state_field_assign_stores_in_place` + two non-goal guards, green) and `tests/rt-behavior/resources/bug424_state_accum_inplace` (§15 visibility, green)

## STATUS: FIXED (204e4c481)

Layer 1 (scalar STATE field stored in place) landed in `204e4c481`: a scalar
`s.state.field = v` now mutates the existing STATE block at the field's offset
instead of rebuilding the whole record, so a scalar bump on a STATE record that
also holds a large buffer no longer re-copies that buffer (scalar STATE mutation
is O(1) regardless of buffer size — scalar-only repro 0.23s → 0.01s at N=16000).
`cargo test --bin mfb`: 3783 passed.

Layer 2 (out-of-line growable representation for STATE **collection** fields, so
collection accumulation stops being O(n²)) is a genuine multi-day record-layout
migration with high memory-corruption risk and was **split out to
`bug-430-state-collection-field-accumulation-quadratic.md`** at the user's
direction. The `collection_state_field_grows_in_place` regression test is present
and `#[ignore]`d, tracked by bug-430.

Every mutation of a resource's `STATE` — including a single scalar field assignment `s.state.pos = 10` and a collection append `s.state.raw = collections::append(s.state.raw, chunk)` — is lowered as a **whole-record `WITH` rebuild** that reconstructs the entire STATE record and re-copies every inlined field. Because a flat collection field (e.g. `List OF Byte`) is *inlined* in the record block, each mutation deep-copies the entire accumulated payload. Accumulating into a STATE buffer chunk-by-chunk is therefore **O(n²)** in total bytes, and even a cheap scalar bump on a STATE record that also holds a large buffer re-copies that buffer.

The single correct behavior a fix produces: a resource `STATE` behaves like a `MUT` binding for both read and write — a scalar field assignment stores in place at the field's offset, and a collection field grows in place (amortized O(1) append with capacity headroom), exactly as a `MUT` local `List OF Byte` does today. Accumulating N chunks into a STATE buffer is O(n), not O(n²). No STATE mutation should copy fields it did not change.

This is the mechanism that makes the "non-blocking HTTP" design (a `PendingState { raw AS List OF Byte, … }` carried as resource STATE and grown by a `pump` step) quadratic, and it is why the existing blocking `http` client accumulates into a plain `MUT raw AS List OF Byte` local instead of STATE.

<!-- When the fix fully lands, add a status block here:
       ## STATUS: FIXED (<commit hash>)
     then archive this file to bugs/completed/. -->

References:

- `mfb spec language resource-management` §15.5 — "s.state reads the state record. It is updated either by assigning a single field in place (`s.state.field = value`) … the former is shorthand for the latter [whole-state `WITH`]." The spec advertises in-place field update; the implementation delivers a whole-record rebuild.
- `mfb spec language bindings-and-scope` §5 — MUT collection in-place growth / capacity headroom (the semantics STATE should match).
- `mfb spec memory collections` — kind-2 fixed-width `List OF Byte` layout, capacity headroom, in-place MUT append (`lower_list_append_in_place`); shrink-to-fit on copy.
- Discovered while designing a non-blocking `http::startRead/pump/finish` API layered on `net`/`tls` (this session). Related memory: `records-inline-their-string-fields`, `collection-memory-mgmt`, `arena-transient-churn-quadratic-graphemes`, `res-is-a-pointer-not-a-borrow`.

## Failing Reproduction

Two projects, identical 1 MB accumulation (N chunks × 64 bytes). One appends into a `List OF Byte` **STATE field**; the other into a `MUT` local. Debug binary `target/debug/mfb`, macOS-aarch64.

STATE version (`/tmp/bug424/state/src/main.mfb`):
```
IMPORT io
IMPORT fs
IMPORT strings
IMPORT collections
TYPE Accum
  raw AS List OF Byte
  n   AS Integer
END TYPE
FUNC main AS Integer
  LET n AS Integer = 16000
  LET chunk AS List OF Byte = strings::toBytes("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
  RES f AS File STATE Accum = fs::openFile("/tmp/bug424/state/project.json")
  MUT i AS Integer = 0
  WHILE i < n
    f.state.raw = collections::append(f.state.raw, chunk)
    f.state.n = f.state.n + 1
    i = i + 1
  END WHILE
  io::print("state len=" & toString(len(f.state.raw)) & " n=" & toString(f.state.n))
  fs::close(f)
  RETURN 0
END FUNC
```

MUT-local baseline (`/tmp/bug424/local/src/main.mfb`): identical loop appending into `MUT raw AS List OF Byte = []`.

Build each with `mfb build`, run the emitted `.out`, timed with `/usr/bin/time -p`:

- Observed (user CPU seconds):

| N | payload | STATE (`s.state.raw = append(...)`) | MUT local (`raw = append(...)`) |
| --- | --- | --- | --- |
| 4000  | 256 KB  | 2.47s  | ~0.00s |
| 8000  | 512 KB  | 10.09s | ~0.00s |
| 16000 | 1.0 MB  | 45.81s | 0.02s  |

- Expected: STATE within a small constant factor of the MUT-local baseline, and **linear in N** (doubling N ≈ doubles time), not quadratic.

The STATE column quadruples when N doubles — textbook O(n²). The MUT-local column is linear and ~2000× faster at N=16000 for the *same result*. This proves the cost is in STATE mutation, not in `collections::append` itself.

(Note: the repro mutates both a collection field and a scalar field each iteration. Both contribute — the scalar bump `f.state.n = f.state.n + 1` also rebuilds the whole record and re-copies the inlined `raw` buffer. Isolating either line alone still reproduces super-linear growth.)

## Root Cause

`s.state.field = value` and `s.state = value` both lower to one op, `Statement::StateAssign`, whose `value` for the single-field form is a **whole-state `WithUpdate`**:

- `src/ast/stmt.rs:224-296` (`parse_*` member-target assignment): the nested `resource.state.field = value` form is desugared to `StateAssign { resource, value: Expression::WithUpdate { target: resource.state, updates: [field := value] } }`. The code comment at `:226-228` states the *intent* — "giving in-place field mutation (§4)" — but a `WithUpdate` is a whole-record reconstruction, so the intent is not realized for any record with an inlined field.
- `src/ir/lower.rs:715-733` (`Statement::StateAssign`): lowers the `WithUpdate` value as an ordinary expression and emits `IrOp::StateAssign { resource, value }`. Nothing special-cases "only field X changed."
- `src/target/shared/code/builder_control.rs:564-594` (`NirOp::StateAssign`): stores the **new record pointer** into the resource record's `FILE_OFFSET_STATE` slot. It never mutates the existing block in place — the value it stores is a freshly built record.
- Record construction is a full inlined rebuild + deep copy: a `List OF Byte` field is *inlined* into the record's trailing data region (`record_field_is_inlined` / `type_is_flat`, `src/target/shared/code/builder_collection_layout.rs:559,622`), and building/copying the record `memcpy`s the whole block including the inlined buffer (`emit_build_inlined_record` :870, `copy_flat_block` :302 — "the byte copy *is* a deep copy"). So each `WithUpdate` re-inlines and re-copies the entire current buffer.
- The read side compounds it: `s.state.raw` loads an alias pointer into the inlined block (`src/target/shared/code/builder_value_semantics.rs:186` for `.state`, field access at :204-290). Because that source is a field-alias, not a uniquely-owned `MUT` local, `collections::append` cannot take the in-place headroom path (`lower_list_append_in_place`) and materializes an owned copy of the whole buffer first. Net: ~two full-buffer copies per mutation → Σ O(k) for k=1..n → **O(n²)**.

Why the MUT-local contrast is immune: a `MUT raw AS List OF Byte` local is a uniquely-owned collection with capacity headroom; `raw = collections::append(raw, chunk)` hits `lower_list_append_in_place` and writes at `Data + dataLength`, bumping `count`/`dataLength` — amortized O(1). There is no record rebuild and no re-inline.

## Goal

- A resource `STATE` payload is mutated in place: `s.state.scalarField = v` writes at that field's offset in the existing STATE block without touching other fields; `s.state.collField = collections::append(s.state.collField, chunk)` grows the collection in place with amortized-O(1) append, matching a `MUT` local.
- The repro's STATE column becomes linear in N and within a small constant factor of the MUT-local baseline (target: same order of magnitude, not 2000×).
- No STATE mutation copies a field it did not change.
- The `.state`-read alias and cross-alias visibility (owner sees a callee's `s.state` mutation) are preserved (§15.5).

### Non-goals (must NOT change)

- **Ordinary record `WITH` semantics stay a rebuild.** Records are immutable values; `WITH r { … }` on a normal record must keep producing a new value. This bug is scoped to resource `STATE`, the one place the language offers mutable member assignment.
- **STATE aliasing/visibility contract (§15).** A mutation through a `RES` parameter must stay visible to the owner; STATE is one payload behind the resource pointer. In-place mutation must not introduce a private copy that breaks this.
- **Drop/free correctness.** The STATE block (and any out-of-line collection buffers introduced by the fix) must still be freed exactly once at resource drop (`emit_free_resource_state_block`, `builder_resource_cleanup.rs:400`) with no leak or double-free; `fs::close` releases the handle but not the payload (§15).
- **`.mfp` STATE encoding / thread-transfer STATE copy** (`builder_arena_transfer.rs:460`) must remain correct; a layout change to STATE collection fields must update the transfer copy too, not silently diverge.
- **Tempting wrong fix, forbidden:** do not "fix" this by making the repro/tests use a MUT local instead of STATE, or by capping/【documenting-away】 the accumulation. The broken path (`s.state.field = …`) must become fast, not be routed around.

## Blast Radius

Found by searching for STATE mutation sites and STATE-field-carrying records.

- `src/ast/stmt.rs:281-290` (`state_field_assign` desugar) — **fixed by this bug**: the single-field form must stop routing through `WithUpdate` (or `StateAssign` codegen must special-case a single-field update into an in-place store).
- `src/target/shared/code/builder_control.rs:564` (`NirOp::StateAssign`) — **fixed by this bug**: needs an in-place-field path (scalar store at offset; collection in-place grow) in addition to the whole-record replace it does today.
- `src/target/shared/code/builder_collection_layout.rs` (record inlining of flat collections) — **in scope if** the chosen fix stores STATE collection fields out-of-line (pointer + growable buffer) rather than inlined. This is the layout decision (see Fix Design / Open Decisions).
- `src/target/shared/code/builder_arena_transfer.rs:460-487` (thread-transfer STATE copy) — **in scope**: must handle the new STATE collection-field representation; currently deep-copies the inlined block.
- `src/target/shared/code/builder_resource_cleanup.rs:400` (`emit_free_resource_state_block`) — **in scope**: must free any out-of-line STATE collection buffers, not just the inlined block.
- Ordinary `Expression::WithUpdate` on non-STATE records (`src/ir/lower.rs`, record construction) — **unaffected / out of scope**: records are immutable by design; their rebuild-on-WITH is correct.
- In-tree STATE fixtures (`tests/rt-behavior/resources/resource-state-*`) use **scalar-only** STATE records (`Cursor { pos, len }`), so no in-tree test currently exhibits the quadratic; they are correctness guards for the aliasing/visibility contract the fix must preserve. **Latent, not currently failing** — the first real victim is any STATE record with a collection or large field (e.g. the proposed http `PendingState`).

## Fix Design

Two layers, and the second is the real cost/decision:

1. **Single-field in-place store (scalars).** Stop desugaring `s.state.field = v` into a whole-record `WithUpdate`. Instead carry the field identity to `StateAssign` and emit an in-place store at the field's offset within the existing STATE block (mirror the record field-access offset logic in `builder_value_semantics.rs:204-290`). This alone removes the scalar-bump half of the quadratic and makes scalar STATE mutation O(1) regardless of other fields' size.

2. **In-place growth for collection STATE fields.** For a collection field to grow in place like a `MUT` local, the field must be a **pointer to a separately-allocated growable buffer with capacity headroom**, not inlined in the fixed-size record block. Options:
   - **(a) STATE collection fields are out-of-line.** Represent a collection field of a STATE record as a pointer slot; the append path reuses `lower_list_append_in_place` against that buffer. Cost: a STATE-specific record layout divergence from ordinary records, plus matching changes in build, copy, thread-transfer, and free. Highest effort, but the only shape that reaches true amortized-O(1) accumulation.
   - **(b) STATE-as-MUT-record generally.** Treat the whole STATE payload as a mutable record: scalar stores in place (layer 1) and collection fields held out-of-line and grown in place. This is the "treat STATE as MUT for read and write" model the user asked for; (a) is its collection-field mechanism.

   Rejected: keeping fields inlined and merely avoiding the *read-side* copy — the record block is fixed-size, so a grown buffer cannot be re-inlined without rebuilding/re-copying the block, which is the quadratic. Inlined + growable are mutually exclusive.

Expected generated-output shift: STATE-carrying fixtures' `.ncode` / IR goldens change (StateAssign lowering differs; possibly the STATE record layout for collection fields). Scalar-only STATE fixtures should shift only in the StateAssign lowering, not in observable output.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Deterministic, host-independent structural tests instead of a wall-clock or runtime copy-count assertion: `tests/rt_res_state_inplace_mutation.rs` builds the STATE-mutation functions cross-target (`linux-x86_64`, build-only `--ncode`) and counts (a) `state_assign_value` stack slots — one per whole-record STATE replace; an in-place mutation allocates none — and (b) `append_inplace_realloc` labels, present only when a list grows in place. `scalar_state_field_assign_stores_in_place` and `collection_state_field_grows_in_place` both failed (`state_assign_value == 1`, want `0`) for the documented reason; two non-goal guards (`whole_state_replace_still_rebuilds`, `string_state_field_still_rebuilds`) stay on the rebuild path.
- [x] Runtime correctness guard `tests/rt-behavior/resources/bug424_state_accum_inplace`: a helper mutates a scalar AND a collection STATE field through a `RES` parameter; the owner observes the accumulation (`len=20 n=5`), proving the §15 aliasing/visibility contract the fix must preserve.
- [x] Runtime timing reproduced (debug `mfb`, macOS-aarch64, `/usr/bin/time -p`, user CPU s): STATE N=8000 → 9.05s, N=16000 → 39.40s (4.35× for 2× N — O(n²)); MUT-local N=16000 → 0.01s. Isolated: collection-only N=16000 → 23.81s; scalar-bump-on-32KB-buffer N=16000 → 0.23s (grows with buffer size — the scalar bump re-copies the inlined buffer). Both halves contribute.
- [x] Blast-radius verdicts confirmed by reading each cited site; the `state.field = v` desugar to a single-field `WITH` update over `resource.state` (IR `"kind":"with"`) confirmed in the IR dump.

Acceptance: the new test(s) failed for the documented reason; audit complete.
Commit: 204e4c481 (tests + fixture landed with Layer 1).

### Phase 2 — the fix

- [x] **Layer 1: single-field in-place scalar store.** `NirOp::StateAssign` codegen (`builder_control.rs::try_inplace_state_scalar_assign`) recognizes a single-field `WITH` update over *this* resource's own `state` whose updated fields are all fixed-width inline scalars, and stores each new value in place at its field offset in the existing STATE block — no rebuild, no re-copy of any inlined field. Inlined (`String`/collection/nested) and pointer fields fall through to the whole-record replace. Pure native-codegen change (AST/IR goldens unchanged; the in-place path only triggers for scalar STATE assigns, so no unrelated codegen shifts). Scalar STATE mutation is now O(1) regardless of buffer size (scalar-only repro 0.23s → 0.01s). `cargo test --bin mfb`: 3783 passed. Commit: 204e4c481.
- [ ] **Layer 2: out-of-line growable representation for STATE collection fields** + in-place append; update build/copy/thread-transfer/free (`builder_collection_layout.rs`, `builder_arena_transfer.rs:460`, `builder_resource_cleanup.rs:400`).

  Refined scope from the audit — this is a genuine multi-day layout change in the compiler's most memory-corruption-sensitive region, and every required change is invisible to output goldens if wrong (a leak/double-free/UAF, per the bug-374/375 lessons):
  * A collection field of a STATE record must become a **pointer slot to a separately-allocated growable buffer** (with capacity headroom), so `f.state.coll = append(f.state.coll, x)` reuses `lower_list_append_in_place`. Inlined + growable are mutually exclusive.
  * `record_field_is_inlined` is keyed on the record **type**, so the layout cannot diverge only "when used as STATE" without a whole-program STATE-only-type analysis; the alternative is a STATE-specific representation with **boundary conversions** at whole-state read (`LET a = f.state`) and whole-state write (`f.state = <ordinary record>`). In-tree, whole-state read only appears in an `-invalid` fixture (never runs) and whole-state write is scalar-`WITH` (Layer 1) or a scalar state — so the collection-bearing boundary is currently unexercised, but the language allows it and a correct fix must handle it, not error.
  * `emit_free_resource_state_block` frees the STATE payload as **one** arena block sized by `emit_inlined_block_size_from_ptr_slot` (assumes inlined). Out-of-line buffers must be freed **separately** (this same size helper feeds the thread-transfer copy at `builder_arena_transfer.rs:460`, so both diverge together).
  * The uniform alternative (make collection-in-record fields out-of-line for **all** records) removes the divergence but makes every record-with-a-collection non-flat — wide behavior/golden churn against the plan-02 inlining direction.

Acceptance: Phase 1 tests pass; STATE accumulation is linear and within a small constant factor of the MUT-local baseline; scalar-only STATE fixtures unchanged in observable output; aliasing/visibility fixtures still green; **no leak / double-free / UAF** (measure RSS on the repro + thread-transfer of a collection STATE + full acceptance).
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate affected `.ncode`/IR goldens; diff and confirm the delta is only the intended StateAssign/layout change.
- [ ] `scripts/test-accept.sh` + `cargo test` (compiler tests via `--bin mfb`) + `scripts/artifact-gate.sh`.
- [ ] Re-run the repro at N=4000/8000/16000 and confirm linear scaling.

Acceptance: full suite green; golden deltas are exactly the intended change; repro is linear.
Commit: —

## Validation Plan

- Regression test(s): the STATE-accumulation copy-count test + the scalar-in-place test under `tests/rt-behavior/resources/`.
- Runtime proof: the N=4000/8000/16000 timing table goes linear and approaches the MUT-local baseline.
- Doc sync: none expected — §15.5 already promises in-place field update; this makes the implementation match the spec. If the STATE collection-field layout changes, note it in `mfb spec package resource-regions` / `mfb spec memory`.
- Full suite: `scripts/test-accept.sh target/debug/mfb target/accept-actual`, `cargo test --bin mfb`, `scripts/artifact-gate.sh`.

## Open Decisions

- STATE collection-field representation — **(a) out-of-line pointer + growable buffer (recommended)** vs. (b) keep inlined and accept that only scalar mutation becomes in-place (leaves collection accumulation quadratic). Only (a) delivers the stated goal. (§Fix Design)
- Scope — fix scalar in-place only first (small, lands independently and kills the scalar-bump copy) vs. full scalar+collection in one change. Recommended: land Layer 1 first, then Layer 2, as separate commits under this doc.

## Summary

The real engineering risk is Layer 2: giving STATE collection fields an out-of-line, growable representation and threading it through build, deep-copy, thread-transfer, and drop-free without breaking the §15 aliasing/visibility contract or leaking. Layer 1 (in-place scalar store) is contained and independently landable. Ordinary record `WITH` semantics and all currently-correct STATE behavior stay untouched; the observable change is that STATE accumulation stops being O(n²).
