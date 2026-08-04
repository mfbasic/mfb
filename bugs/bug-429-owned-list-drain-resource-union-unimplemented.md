# bug-429: owned-list drain of resource-union elements is unimplemented

Last updated: 2026-08-03
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/resources/bug429_owned_list_union_drain_rt (to be added)

A `List OF RES <ResourceUnion> STATE <S>` that **owns** its elements — i.e. the
`RES` bindings are produced in an inner scope and appended to an outer-scope
list, so ownership *floats up* to the list (§15.6) — fails to compile. The
collection becomes an "owner collection" and codegen tries to install an
owned-list drain, but the drain path only knows how to close a **concrete**
resource element; a **resource-union** element resolves no single close op and
the build aborts:

```
error: owned-list element type 'Handle STATE Cursor' has no registered close op
       while lowering bind handles AS List OF RES Handle STATE Cursor
```

The single correct behavior a fix produces: a `List OF RES <Union> STATE <S>`
that owns floated-in union resources compiles, and at scope exit each element is
closed by **tag-dispatch** on the active variant's registered close op (exactly
as a single `RES u AS Union` binding is), its uniform STATE block freed, and the
close made idempotent — so a dynamically-built list of open streams/handles is a
supported, leak-free, double-free-free construct.

This blocks the `examples/browser` parallel CSS loader
(`examples/browser/fetch/src/lib.mfb:fetchStyles`), which builds a
`List OF RES http::Stream STATE PendingState` from `http::startRead` in a loop
and drives them with `http::ready`/`http::pump`/`http::finish` — the intended,
documented usage of the non-blocking HTTP streaming API.

References:

- `mfb spec language resource-management` §15.6 ("Resources in collections",
  ownership-floats-up rule) and the resource-union STATE rules — the contract a
  fix must satisfy.
- bug-427 (`tests/rt-behavior/resources/bug427_list_union_state_rt`) — added the
  ability to *spell* and read `.state` on a `List OF RES <Union> STATE <S>`, but
  its resources do **not** float (they and the list share `main`'s scope), so it
  never exercises the owned-list drain. This bug is the drain that bug-427 left
  unimplemented.
- Found while implementing the browser example's parallel CSS load
  (`examples/browser/fetch/src/lib.mfb`).
- Related memory: "Collection element STATE carry + -ast -ir blind spot",
  "Resource-union STATE: {tag,ptr} layout + 3-place close wiring".

## Failing Reproduction

Minimal executable (`main.mfb`), built with a **fresh** `cargo build --release
--bin mfb` (the on-disk release binary may be stale and predate bug-427's
parser support):

```basic
IMPORT io
IMPORT fs
IMPORT collections

TYPE Cursor
  pos AS Integer
END TYPE
UNION Handle
  File
  Socket
END UNION

FUNC main AS Integer
  MUT handles AS List OF RES Handle STATE Cursor = []
  FOR i = 1 TO 2
    RES a AS Handle STATE Cursor = fs::createTempFile()   ' inner scope → floats to `handles`
    handles = collections::append(handles, a)
  NEXT
  RETURN len(handles)
END FUNC
```

- Observed: build aborts with
  `error: owned-list element type 'Handle STATE Cursor' has no registered close op while lowering bind handles AS List OF RES Handle STATE Cursor`
- Expected: compiles and runs; both temp files are closed once at `main`'s exit.

Second manifestation — across a **package boundary** (the browser app importing
the `fetch` package that contains such a list). The `fetch` package *builds*
(package IR is not fully lowered), but the importing executable's IR verifier
rejects it before the close-op error is even reached:

```
error: PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE: ResultValue is annotated
       `Stream` but its Result carries `Stream STATE PendingState`
```

reproduced by building `examples/browser/app` after wiring `fetchStyles` (see
the browser example). This is a STATE-strip reconciliation mismatch in the
lowered union-list IR, distinct from the drain gap but on the same path.

Contrast cases that work correctly today (regression guards — must stay green):

- `tests/rt-behavior/resources/bug427_list_union_state_rt` — same
  `List OF RES <Union> STATE <S>` type, but the resources share the list's scope
  so ownership does **not** float; the list holds only aliases and closes
  nothing. Compiles and runs.
- `tests/rt-behavior/resources/resource-collection-floats-runtime` and
  `tests/rt-behavior/net/net-poll-list-rt` — floated **concrete** resource
  elements (`List OF RES File` / `List OF RES Socket`); the owned-list drain
  works for a concrete element's single close op.
- **Concrete resource WITH STATE, floated** — `List OF RES File STATE Cursor`
  built in a loop (float) — **verified working today** (2026-08-03): builds,
  reads each `.state` back through `FOR EACH` (`total=30` for pos 10+20), and
  exits 0 clean. So `List OF RES <Resource> STATE <S>` is NOT broken; the
  concrete close op resolves and the drain closes once. This bounds the bug
  strictly to **union** elements.

The bug is precisely the intersection: **floated (owned) ∧ resource-union
element**. Concrete elements (with or without STATE) already work.

## Root Cause

The owned-list drain machinery was built for a single close op per element and
never taught the resource-union tag-dispatch that single-binding cleanup already
has.

- `src/target/shared/code/builder_resource_cleanup.rs:collection_resource_close_symbol`
  (~:507) resolves the element type through
  `resource_cleanup_symbol(&element)`. That helper (~:16) returns a symbol only
  for a **concrete** resource (a builtin close op, or a user
  `RESOURCE … CLOSE BY` entry in `type_model.resource_closers`). For a resource
  **union** it returns `None` — a union has no single close op; it is dropped by
  reading its tag and dispatching to the active variant. Hence the "no
  registered close op" abort at ~:512.
- `src/target/shared/code/builder_owned_cleanup.rs:setup_owned_list` (~:7)
  stores that single `close_symbol` in `OwnedListCleanup`
  (`src/target/shared/code/mod.rs:527`), and
  `emit_owned_list_drain` (~:124) walks the node list calling that one symbol on
  each node's stored pointer — no tag load, no per-variant dispatch, no STATE
  free.
- The correct per-element behavior already exists for a single binding in
  `builder_resource_cleanup.rs:emit_resource_union_cleanup_call` (~:93): it loads
  the union block's tag @0, dispatches to the matching variant's close on the
  record pointer @8, then frees the uniform STATE block
  (`emit_free_resource_state_block`). `resource_union_cleanup` (~:52) yields the
  `(tag, close_symbol)` table (STATE-stripped base name). The drain needs to run
  this same dispatch per owned-list node.

Why the contrast cases are immune: floated **concrete** elements resolve a real
single close op (so `collection_resource_close_symbol` succeeds); **union**
elements that *don't* float never make their list an owner collection, so
`setup_owned_list` is never called for them.

Second manifestation root cause (to confirm in Phase 1):
`src/ir/verify/values.rs:check_result_value_type` (~:286) compares the
STATE-stripped annotation base (`Stream`) against the carried Result element
(`Stream STATE PendingState`) and finds them incompatible. The lowering that
stamps the union-list element/result type across the package boundary is
dropping STATE on one side but not the other; the verifier is correct to flag a
genuine inconsistency. Confirm whether this is the FOR EACH loop-variable result
annotation or the `http::finish` call result, and whether it reproduces without
a package boundary.

## Goal

- A `List OF RES <Union> STATE <S>` (user union **and** builtin union such as
  `http::Stream`) that owns floated-in elements compiles.
- At scope exit each owned element is closed by tag-dispatch on its active
  variant's registered close op, its STATE block freed, exactly once (idempotent
  re-close), on every exit path (normal, RETURN, error, EXIT/CONTINUE).
- The browser example's `fetchStyles` builds and runs across the package
  boundary; `examples/browser/app` links.

### Non-goals (must NOT change)

- Concrete-resource owned-list drain codegen (existing `close_symbol` path) —
  must be byte-identical; its goldens must not shift.
- The float/escape decision procedure (`ir/resource_escape`) — this is a
  cleanup-emission gap, not an ownership-assignment gap.
- bug-427's spelling/`.state`-read behavior for non-floating union lists.
- Resource-union value layout (`{tag@0, record-ptr@8}`), STATE layout/offsets,
  or the 96-byte resource envelope.
- **Tempting wrong fix, forbidden:** do not make the example avoid the float
  (e.g. by not building a real owned list) to dodge the drain — the owned union
  list is the feature. Do not weaken `check_result_value_type` to stop flagging
  the STATE mismatch instead of fixing the lowering that produces it.

## Blast Radius

Found by searching callers of the owned-list drain and union cleanup:

- `builder_owned_cleanup.rs:setup_owned_list` / `emit_owned_list_drain` /
  `emit_owned_list_push` / `emit_owned_list_seed_from_collection` — **fixed by
  this bug**: taught to carry and dispatch a union table for union elements.
- `builder_resource_cleanup.rs:collection_resource_close_symbol` — **fixed**:
  must succeed for a union element (return/record the dispatch table, not error).
- `builder_control.rs` (~:252–271, the `owner_collections` / RES-marked-from-call
  bind arms) and `builder_exits.rs` / `builder_control.rs` drain sites
  (`ActiveCleanup::OwnedList`) — **on-path**, must handle the new union variant
  of the cleanup uniformly on every exit.
- `emit_owned_list_seed_from_collection` (adopting a **returned**
  `List OF RES <Union>`) — **in scope**: a function returning an owned union list
  and a caller adopting it must both drain by dispatch. Add coverage.
- `src/ir/verify/values.rs:check_result_value_type` — **investigate/fix** the
  STATE-strip inconsistency it correctly flags (second manifestation); the fix
  is in the lowering that stamps the type, not the verifier.
- Map-valued owned collections (`Map OF K TO RES <Union> STATE S`) —
  `collection_resource_close_symbol` also serves map values (§15.6); **same
  hazard**, fix and test both list and map.
- `bug427_list_union_state_rt` (non-floating union list) — **unaffected**;
  regression guard that the concrete/alias paths are untouched.

## Fix Design

Thread a resource-union dispatch through the owned-list cleanup instead of a lone
close symbol:

1. Give `OwnedListCleanup` an optional union descriptor (the
   `Vec<(tag, close_symbol)>` from `resource_union_cleanup` plus the STATE type),
   in addition to (or in place of) the single `close_symbol`. When the element
   base names a resource union, populate the union descriptor; otherwise keep the
   concrete `close_symbol` exactly as today.
2. In `collection_resource_close_symbol` (or a new sibling), return a union
   descriptor when the element is a union rather than erroring — reuse
   `resource_union_cleanup` and `resource_uses_io_buffers`/state handling.
3. In `emit_owned_list_drain`, when the cleanup is a union: for each node, load
   the stored union block pointer and run the same tag-dispatch-close + STATE-free
   sequence as `emit_resource_union_cleanup_call`. Factor that sequence out of
   `emit_resource_union_cleanup_call` into a shared helper that operates on a
   union pointer already in a register/slot, so the two call sites cannot drift.
   Keep the concrete path byte-identical (single `emit_symbol_call`).
4. Confirm what `emit_owned_list_push` stores for a floated union binding: the
   binding's slot holds the union block pointer (what
   `emit_resource_union_cleanup_call` reads), so the node's field 0 is the union
   pointer — the drain reads tag @0 / payload @8 from it. Verify against the
   actual float push site; adjust if a concrete-record pointer is stored instead.
5. Fix the `check_result_value_type` STATE mismatch by making the union-list
   element/result lowering stamp STATE consistently (strip on both sides or carry
   on both). Determine the exact lowering site during Phase 1 via a
   package-free repro if possible.

Risk concentrates in (3)/(4): the drain runs per node and must be correct on
every exit path and idempotent. This is core, arch-shared codegen — verify on
macOS aarch64 (runnable here) and gate the full artifact-gate before finalizing.

Rejected alternative: emit a per-union synthetic single close thunk and keep the
drain single-symbol. Rejected — it duplicates the dispatch logic already in
`emit_resource_union_cleanup_call` and hides STATE-free ordering; sharing the
existing helper is safer and less code.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/rt-behavior/resources/bug429_owned_list_union_drain_rt`: a
      user union (File/Socket) built into a `List OF RES <Union> STATE <S>` via a
      loop (forces float), plus a map variant; assert it runs and closes each
      once. Confirm it fails today with the "no registered close op" abort.
- [ ] Add a package-boundary fixture (or reuse the browser example) that
      reproduces the `check_result_value_type` VERIFY_TYPE error; determine
      whether it reproduces without a package boundary.
- [ ] Confirm what `emit_owned_list_push` stores for a floated union binding
      (union block pointer vs record pointer) and record it here.
- [ ] Complete the blast-radius verdicts above (list + map, return/adopt paths).

Acceptance: new test(s) fail for the documented reasons; the push-payload
question is answered; audit complete.
Commit: —

### Phase 2 — the fix

- [ ] Extend `OwnedListCleanup` + `setup_owned_list` +
      `collection_resource_close_symbol` to carry a union dispatch descriptor.
- [ ] Factor the tag-dispatch-close + STATE-free out of
      `emit_resource_union_cleanup_call` into a shared helper; call it from
      `emit_owned_list_drain` for union elements. Keep the concrete path
      byte-identical.
- [ ] Fix the `check_result_value_type` STATE-strip inconsistency at its lowering
      source.
- [ ] Apply to the map-valued path too.

Acceptance: Phase 1 tests pass; concrete-resource owned-list goldens unchanged;
Non-goals intact.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Add `.ast`/`.ir` (and any codegen) goldens for the new fixtures; confirm no
      unrelated golden shifts (concrete owned-list goldens must be byte-identical).
- [ ] Rebuild `examples/browser` end to end (dom → fetch → display → app); the
      app links and `fetchStyles` is present.
- [ ] Run the full suite + `artifact-gate.sh` (serialized; check no concurrent
      gate) and `cargo test --bin mfb`.

Acceptance: full suite green; only the intended new goldens added; browser app
builds; repro passes.
Commit: —

## Validation Plan

- Regression test(s): `bug429_owned_list_union_drain_rt` (list + map, floated
  union, close-once) under `tests/rt-behavior/resources/`; a package-boundary
  fixture for the VERIFY_TYPE path.
- Runtime proof: the rt-behavior fixture runs and its `.run`/build.log shows the
  expected output; `examples/browser/app` builds and the parallel `fetchStyles`
  compiles.
- Doc sync: none expected (spec §15.6 already specifies the contract; this closes
  the implementation gap). Note the closure in bug-427's lineage if useful.
- Full suite: `cargo test --bin mfb` + `scripts/artifact-gate.sh` +
  rt-behavior acceptance for the new fixtures.

## Open Decisions

- Whether the `check_result_value_type` mismatch is a distinct sub-issue with its
  own RED test or a symptom that disappears once the drain lowering is correct —
  resolve in Phase 1 by attempting a package-free repro.

## Summary

The escape analysis already floats union resources into an owning list; only the
**cleanup emission** was never taught union tag-dispatch, so the build aborts.
The fix reuses the existing single-binding union-drop helper per owned-list node.
Real risk is in the per-node drain correctness/idempotency across exit paths and
the STATE-strip reconciliation across the package boundary; the concrete-resource
drain and all resource layout/ownership rules stay untouched.
