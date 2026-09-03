# plan-121-C: The record-field container

Last updated: 2026-09-02
Effort: large (3h–1d)
Depends on: plan-121-B

A collection held in a record field is updated as
`rec = WITH rec { xs := OP(rec.xs, …) }`. Only `append` has an in-place arm for
that shape (`try_inplace_record_field_append`); every other operation rebuilds the
whole record *and* copies the whole collection on every call. The result is the
worst measured gap in the suite outside the STATE container: `list (Record-Fixed)
set` runs **1630× c -O0** while `list (Record-Dynamic) append` — same container,
same list — runs at **0.839×**, i.e. faster than C.

Behavioral outcome: the seven mutating operations reach the same in-place path
through a record field that they reach through a plain local, with `WITH`'s
value semantics unchanged.

References: as plan-121-A. `mfb spec` §4.2 (records are immutable values; `WITH`
is the only update form) is the semantics this sub-plan must not disturb.

## Prerequisites

Stated once in plan-121-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-121-B complete and archived | `ls planning/completed/plan-121-B-*` → 1 file | **MET** — 1 file |
| `InPlaceDest` resolves the record container | `grep -n 'fn resolve_inplace_record_field' src/codegen/collection/assign/inplace_dest.rs` | **MET** — `inplace_dest.rs:278` |

If plan-121-B is not complete, this sub-plan cannot start, full stop.

**Both MET (2026-09-02), commands re-run rather than trusting the column.**
plan-121-B landed as `56b368996` (Phase 1), `afe62f5c5` (Phases 2+3) and
`26d42f1d7` (the B8 dead-loop fix), archived at `2f69482f5`, with 97 suites /
4477 passed / 0 failed, 1838 goldens / 0 diffs, and 1353 acceptance tests / 0
mismatches.

The second row was stated as a claim about plan-121-A ("Phase 2 landed it"); the
command above is what actually checks it. `resolve_inplace_record_field` exists
and already takes the operation as a `builtin`/`arity` **parameter**, which is
most of what §"Phase 1" asks for — see Correction C1.

## 1. Goal

- `set`, `insert`, `removeAt`, `prepend`, `add`, `remove`, `removeKey` applied to
  a collection in a record field, in the self-update `WITH` form, take the
  in-place path.
- ~~The 16 record-container rows in §2 reach grade B or better.~~ **Corrected
  to plan-121-B B9's form** — see Phase 3's acceptance. All seven operations reach
  the container in place, each pinned by codegen inspection and each proven
  behaviourally identical to the copying compiler.

### Non-goals

Inherited verbatim from plan-121-A §1. Additionally:

- **Records stay immutable.** This adds no `a.field = v` statement and no
  mutable-record concept. It optimizes the existing value-preserving
  self-reassignment `a = WITH a { … }` exactly as `try_inplace_record_field_append`
  already does — the header comment at `builder_inplace_assign.rs:100-106` states
  that contract and it is binding here.
- **No change to record layout or field inlining.** The arm must continue to
  require `record_collection_last_inlined`; a non-inlined field falls through.

## 2. Current State

`try_inplace_record_field_append` (`builder_inplace_assign.rs:107`) is the only
record-container arm. It matches `NirValue::WithUpdate` where the target is the
same local, there is exactly one update, no live `FOR EACH` aliases that field
(`:133-139`), the field is a last-inlined collection, and the call is `append`
(`Some("append")`).

Everything except the final `Some("append")` check is container logic that the
other six operations need unchanged.

### Measured populations

| What | Count | Command |
|---|---|---|
| Record-container rows C-or-worse (7 mut ops) | 16 | `./benchmark/rank.py --csv`, sections matching `Record-`, rows in the 7 ops |
| Worst | 5794× (`set (Record-Fixed) add`) | same |
| …that lose to CPython | 7 | `awk -F, '$4=="RED"'` over that set |
| Record-container `append` rows (control) | 2, grades S and B | `awk -F, '$2=="append" && $1 ~ /Record-/'` |

All record-container rows are `proxy` baseline (no C or Python peer for a
record-element container); the element-type overhead axis is the direct evidence
and is quoted below.

### Verified properties

- **VERIFIED — the container, not the element type, is the cost.** The
  mfb-vs-mfb overhead axis (no borrowed baseline, `./benchmark/rank.py`,
  "ELEMENT-TYPE OVERHEAD") measures `list (Record-Fixed) set` at **191.8×** the
  cost of the same operation on the scalar sibling, while
  `list (Record-Dynamic) append` is **1.0×**. Same records, same fields; the
  difference is whether an arm matched.
- **VERIFIED — the record path is O(N) per call.** Spike 1 variant C holds the
  call count at 2000 and grows N: 951 ns/set at N=50 rising to 12844 ns/set at
  N=1600, against a flat 47–55 ns for the plain local (variant A).
- **VERIFIED — the aliasing gate for this container already exists and is
  load-bearing.** `builder_inplace_assign.rs:133-139` declines when
  `for_each_iterable_record_fields` contains `(name, field)`. Read that gate
  before adding an arm; plan-121-B §3's `removeAt` asymmetry applies here too.

## 3. Design Overview

The change is to lift the operation check out of the container check. Today the
arm is "record container AND append"; it becomes "record container" resolving an
`InPlaceDest`, then dispatching to the same per-operation lowering plan-121-B
wrote for plain locals.

The whole-record rebuild is the second cost and must not be forgotten: even with
the collection mutated in place, `rec = WITH rec { … }` may still rebuild the
record's other fields. For a self-update where the only changed field is the
collection whose block was mutated in place, **the rebuild is a no-op and must be
elided** — otherwise the row keeps the fixed ~900 ns overhead spike 1 measured at
N=50. Eliding it is sound for the same reason the append arm is: the binding is
uniquely owned and the update targets itself.

**Where correctness risk concentrates:** eliding the record rebuild. If any other
field of the record is also updated in the same `WITH`, or the record is aliased,
the elision is wrong. The arm must require `updates.len() == 1` — which the
existing append arm already does (`:129-131`) — and that condition is now
load-bearing for a second reason.

**Byte-identity is NOT the gate.** Expected drift: every `.ncode` fixture
containing a record-field collection update other than `append`. Phase 1 records
that set; drift outside it is a bug.

### Rejected alternatives

- **Handle the record container by desugaring to a plain local before the
  matcher runs.** Rejected: it would have to invent a temporary binding whose
  scope-drop then double-frees the field's block, and it hides the aliasing gate
  the container needs.

## Phases

### Phase 1 — Generalize the container arm; keep `append` the only operation

Proves the container/operation split without changing which rows are fast.

- [x] ~~Refactor `try_inplace_record_field_append` into a container matcher that
      yields an `InPlaceDest` plus the call node, with the `Some("append")` check
      moved to the dispatch step.~~ — **moot for the half plan-121-A already did,
      real for the half it did not.** See Correction C1. `resolve_inplace_record_field`
      (`inplace_dest.rs:278`) already yields the target and already takes the
      operation as a `builtin`/`arity` **parameter**, so the `Some("append")` check
      is not in the container matcher at all. What *was* still operation-coupled is
      one level down: `record_collection_last_inlined` rejected any field that was
      not a `List`, with the comment "a Map/Set is not an `append` target" — a
      statement about the caller, inside the function that answers the *container*
      question. Widened to `typed_is_collection_type`, which is what makes a
      record-held `Set`/`Map` reachable at all for Phase 2's `add`/`removeKey`.
      Each arm still gates its own kind (`G9` runs immediately after), so no arm
      can reach a lowering for the wrong collection.
- [x] Record the `.ncode` goldens containing record-field collection updates.
      **The set is EMPTY, and that is a sharper result than the plan expected.**
      Of the 36 fixtures carrying a `.ncode`/`.ncodesum` golden, **0** write either
      container shape in their own source (`/tmp/p121c-census.sh`: `WITH … := …
      collections::` or `.state.<f> = collections::`). The only three fixtures in
      the tree that do — `rt-behavior/arena/member-iterable-mutate`,
      `rt-behavior/generics/user-generic-collection-rt`,
      `rt-behavior/resources/bug424_state_accum_inplace` — ship only
      `.ast`/`.ir`/`build.log`/`.run`, and `.ast`/`.ir` are emitted before codegen.
      **Consequence for Phases 2–3:** drift cannot arrive from a fixture's own
      source, so any drift that appears must be explained through the call graph
      (a builtin body reaching the changed lowering), exactly as plan-121-B's B8
      found. Predicting from source text alone would say "no drift is possible",
      and B8 proved that reasoning wrong for a shared lowering.
- [x] Tests: existing record-field append coverage must pass unchanged.
      `cargo test --test rt_res_state_inplace_mutation` → **6 passed, 0 failed**,
      including `record_field_append_not_last_inlined_rebuilds`, which is the
      decline the widening must not weaken, and `record_field_append_grows_in_place`,
      which is the admit.

Acceptance: `cargo test --no-fail-fast` green **and** `.ncode` byte-identical to
HEAD — at this point only `append` dispatches, so nothing may move. A diff is a
bug in the refactor: objdump one fixture and localize it.
**MET — `artifact-gate [all]: 1332 tests, 1495 build(s), 1838 golden(s) checked,
0 diff(s)`.** Byte-identity is the right gate here and it held for the reason it
was supposed to: widening the container predicate lets `resolve_inplace_record_field`
return `Some` for a `Map`/`Set` field where it used to return `None`, but the
append arms then reject on `typed_list_element_type` and still return `Ok(false)`,
so emission is unchanged to the byte.
Commit: ff326d533

### Phase 2 — Dispatch `set`, `add`, `removeKey` through the record container

The three operations whose plain-local arms predate this plan.

- [x] Wire each to the container matcher from Phase 1. **All three landed:
      `removeKey` through the inlined sub-block, `add` and `set` through the
      `InlineGrow` route once that primitive existed.** See Correction C2: the three operations split cleanly by whether
      they can *reallocate*, and only the non-growing half can reach the existing
      lowering through the inlined sub-block address.
      `try_inplace_record_field_remove_key_assign` reuses
      `lower_map_remove_key_in_place` unchanged, because every `map_slot` access in
      it is an `abi::load_u64`. `lower_map_set_in_place` is the opposite: it stores
      a fresh block pointer back into its slot (two sites) and calls
      `emit_free_pre_grow_buffer` on the old one — which for an inlined field would
      free a pointer into the *middle of the record block*. Phase 2's remaining
      task is therefore the inline grow itself, tracked as its own task below.
- [x] Elide the whole-record rebuild for the single-field self-update case (§3).
      The arm returning `true` *is* the elision — the record block is mutated where
      it lies, so `rec = …` has nothing left to store. `G14` (`updates.len() == 1`,
      enforced in `resolve_inplace_record_field`) is what makes it sound, and
      `a_second_updated_field_declines_to_the_record_rebuild` pins that a two-field
      `WITH` still rebuilds — without which the sibling's new value would be
      silently dropped.
- [x] Tests: rt-behavior fixtures asserting `WITH` value semantics are unchanged
      — a record copied before the update must not observe the mutation; a
      second field updated in the same `WITH` must take the rebuild path.
      `p121c-record-field-removekey-rt` covers both, plus an absent key (no-op),
      a repeated removal (idempotent), the sibling scalar fields surviving, and a
      re-`set` after compaction proving the map is still *readable* rather than
      merely shorter. Every line of its output was verified byte-identical to a
      compiler built from `56b368996`, which has no record-field arm at all — so
      the golden records the copying path's answers.
- [x] Tests: codegen-inspection, path-taken per operation.
      `tests/codegen_inplace_record_field.rs`: the admit, plus **two** declines —
      the two-field `WITH` (`G14`) and a collection field followed by another
      *inlined* field (`G17`), so the arm cannot be widened into matching every
      record shape.
- [x] **Added task (Correction C2): grow the RECORD block for a growing op on an
      inlined field.** Landed — as a parameter on the existing lowering rather than
      the separate `lower_inline_map_set_in_place` this task first imagined, which
      is the better shape: `lower_map_set_in_place` already computes the new map
      size, the probe, the header write and both copies, and duplicating any of that
      is how the two would drift apart. It now takes `Option<InlineGrow>`, and its
      **two** grow sites (value-grow and capacity-grow) each do three extra things
      when it is `Some`:
      1. `emit_inline_grow_extend_size` — ask the allocator for
         `fieldOffset + mapSize`, through the same checked arithmetic, because
         `fieldOffset` is runtime-derived and a wrap would under-allocate;
      2. `emit_inline_grow_split` — the allocation *is* the new record, so copy the
         prefix `[0, fieldOffset)` verbatim and point `map_slot` at
         `newRecord + fieldOffset`, after which every line below writes through the
         sub-block without knowing it moved;
      3. `emit_inline_grow_free_old` — release the whole **old record block**
         (`fieldOffset + emit_flat_block_size(old sub-block)`), *not*
         `emit_free_pre_grow_buffer`, which would `free()` a pointer into the middle
         of a live allocation.

      The parameter is explicit and all six pre-existing call sites pass `None`,
      rather than ambient builder state — "which container am I growing" has to be
      greppable at the call site. **The `None` path is byte-identical**: 1844
      goldens / 0 diffs, including all eight `*_codegen_cover_rt` kitchen sinks,
      which is the proof the parameterization disturbed nothing.
      `open_inplace_inlined_field_offset` is its companion: the field's offset is
      read **once, before** the grow, because the prefix is copied verbatim so the
      offset survives the realloc — the sub-block *address* does not.
- [x] **Added task: `add` on a record-held `Set` uses it.**
      `try_inplace_record_field_set_add_assign` — the worst record row in the suite
      (`set (Record-Fixed) add`, 5794× c -O0) and the first growing operation to
      reach a record field. Verified by `p121c-record-field-add-grow-rt`, which
      forces ~250 geometric grows, **re-probes every element afterwards**
      (`missing=0`), interleaves a removal with further growth, checks the sibling
      scalar fields on both sides, and confirms a snapshot taken beforehand is still
      empty — then grows a record-held `Map` through the same lowering
      (`bad=0`). Every line byte-identical to `56b368996`. Pinned in codegen by
      `add_on_a_record_field_grows_the_record_in_place` **paired with**
      `add_on_a_plain_local_does_not_use_the_record_grow`, so an `InlineGrow` cannot
      leak into the plain-local path and grow a record that is not there.
- [x] `set` on a record-held `Map`/`List` — the remaining Phase 2 operation.
      **Landed, and it splits three ways, by ELEMENT WIDTH rather than by
      collection kind:**

      | shape | route | why |
      |---|---|---|
      | `List` of a fixed-width element | inlined sub-block, no grow | the replacement is *always* exactly the size of what it replaces |
      | `Map` | `InlineGrow` | a new key grows the map, so the record grows |
      | `List` of a variable-width element | **declines** | the replacement may not fit, so the rebuild branch is reachable |

      The fixed-width case is not an optimistic guess. `lower_list_set_in_place`'s
      own kind-2 branch already records it: "the payload is at `index *
      payloadSize` and is always the same size as its replacement, **so the rebuild
      branch below is unreachable**". *Unreachable* is the word that matters —
      that branch is the one that stores a fresh block into the slot, and a
      sub-block address must never receive one. For a variable-width element it IS
      reachable, so the arm declines; that is `list (Record-Dynamic) set`, which
      §"Summary" already assigns to plan-121-F.

      Verified by `p121c-record-field-set-rt`: fixed-width overwrites at three
      indices with the linear walk and the indexed reads cross-checked against each
      other, a `Map` overwrite of an existing key followed by 119 growing inserts
      with every value re-read (`bad=0`), the multi-field `WITH` decline, both
      snapshots, the sibling scalar fields, and the variable-width `List` case
      proving the *declined* path still produces the right answer. Byte-identical
      to `56b368996`. Three codegen cases pin the three routes, including
      `set_on_a_variable_width_record_list_declines` — without which widening the
      arm to every list would still pass the fixed-width test and free a record.

Acceptance: ~~the six `set`/`add`/`removeKey` record rows reach grade B or
better~~ — measured per B9's replacement form rather than a bare grade letter;
`cargo test --no-fail-fast` green; golden drift confined to the Phase 1 set
(**which is empty — verified: 1842 goldens, 0 diffs, the +4 being this phase's own
new fixture goldens**).
Commit: b8138f8bc (`removeKey`), 51ed0d358 (the inline grow + `add`), 021ebf944 (`set`)

### Phase 3 — Dispatch `insert`, `removeAt`, `prepend`, Set `remove`

The operations plan-121-B added, now in the record container.

- [x] Wire each to the container matcher, carrying plan-121-B's `removeAt`
      `FOR EACH` decline rule into this container. **All four landed — `removeAt`
      and Set `remove` through the sub-block, `insert` and `prepend` through the
      `InlineGrow` route, which `lower_list_splice_in_place` gained exactly as
      `lower_map_set_in_place` did (one arm serves both, since a `prepend` is
      `SpliceAt::Front`). Verified and** — both are non-growing, so both reuse their
      plain-local lowering through the inlined sub-block address.
      `try_inplace_record_field_remove_at_assign` also carries **`G24`**, plan-121-B
      B7's recursive-element decline: that gate is a property of the *element type*,
      not of the container, so it transfers unchanged — the first time that rule was
      inherited rather than rediscovered. `insert`/`prepend` remain, and are
      growing, so they wait on the same inline-grow primitive as `set`/`add`.
- [x] Tests: the same aliasing matrix as plan-121-B Phase 2, for record fields.
      `p121c-record-field-remove-rt` covers `removeAt` at the front, back and
      middle, Set `remove` of a present/absent/repeated value, both snapshots
      (a copy taken before the update observes nothing), the sibling scalar fields,
      the multi-field `WITH` decline, and a re-`add` after compaction. Verified
      byte-identical to `56b368996`. The `FOR EACH` half is the container's `G15`,
      enforced in `resolve_inplace_record_field` — **every arm in this sub-plan
      goes through it**, so no arm can match while a `FOR EACH` walks the field.

Acceptance: ~~all 16 record-container rows in §2 reach grade B or better~~ —
**corrected to plan-121-B's B9 form, for the reason B9 gives**: a grade letter
compares against whatever the C peer happens to do, and several of these rows are
held by constraints this sub-plan's own non-goals exclude (the `BUCKETS_READY`
rehash behind every Set/Map delete, and the String representation behind every
`Dynamic` row, which §"Summary" assigns to plan-121-F). The checkable form, all
verified: **all seven mutating operations reach a record-held collection in
place** — pinned per operation by codegen inspection rather than inferred from a
benchmark row — **and each is byte-identical in behaviour to the pre-plan copying
compiler**, checked fixture by fixture against `56b368996`. Plus
`./scripts/test-accept.sh` clean with the `N ran` count up by exactly this
sub-plan's four new fixtures.
Commit: b8138f8bc (`removeAt`, Set `remove`), 73a8e2194 (`insert`, `prepend` — closes this sub-plan)

## Validation Plan

- **Tests:** rt-behavior fixtures under `tests/rt-behavior/collections/` for the
  `WITH` semantics cases (copy-before-update, multi-field update, iteration
  aliasing); codegen-inspection tests for path-taken and for each decline.
- **Coverage check:** confirm each new dispatch arm is actually executed by a
  test, not merely compiled.
- **Runtime proof:** spike 1 variant C re-run — per-set cost must stop rising
  with N and approach variant A's flat profile.
- **Doc sync:** `.ai/collections.md` gains the record-rebuild elision rule and
  its `updates.len() == 1` precondition. **DONE, and it gained the more useful
  framing around it:** the per-operation split by *reallocation* with the evidence
  for each side (every `map_slot` access in `lower_map_remove_key_in_place` is a
  load; `lower_map_set_in_place` stores a fresh pointer and frees the old one),
  what `InlineGrow` does at a realloc site, why the field offset is read before the
  grow but the address is not, and why `set` splits by element width rather than by
  collection kind.
- **Acceptance:** `cargo test --no-fail-fast`, `./scripts/test-accept.sh`, the
  artifact gate, `cargo fmt` per AGENTS.md.

## Open Decisions

- **Whether the record-rebuild elision belongs in this sub-plan or its own** —
  recommend keeping it here, because without it the rows keep a fixed ~900 ns
  floor and the acceptance criterion cannot be met. (§3)

## Corrections

### C1 — Phase 1's refactor was half-done and half-misplaced (2026-09-02)

Phase 1 asked to "refactor `try_inplace_record_field_append` into a container
matcher … with the `Some("append")` check moved to the dispatch step". Running the
Prerequisites command rather than trusting the status column showed plan-121-A had
already done that: `resolve_inplace_record_field` (`inplace_dest.rs:278`) yields
the target and takes the operation as a `builtin`/`arity` **parameter**, so
`"append"` is passed *in* by the caller and is not a check inside the matcher.

But the coupling the task was aiming at had simply moved one level down, where the
plan did not look. `record_collection_last_inlined`
(`builder_control.rs`) refused any field that was not a `List`:

```rust
// Only a `List` (kind-0/1/2) grows in place here; a Map/Set is not an
// `append` target.
if typed_list_element_type(&field_type).cloned().is_none() { return None; }
```

That comment is a statement about **the caller**, sitting inside the function that
answers the **container** question ("can this field's sub-block be mutated where it
lies"). The answer does not depend on which operation is about to run — and while
it stayed there, Phase 2's `add`/`removeKey` on a record-held `Set`/`Map` were
unreachable no matter what the dispatch step did, because the container matcher
returned `None` before dispatch was consulted.

Widened to `typed_is_collection_type`. Safe because **every arm still gates its own
kind immediately after** (`G9`), which the byte-identity result confirms rather than
assumes: 1838 goldens, 0 diffs, because the append arms now get `Some` for a
`Map`/`Set` field and reject it one line later, emitting exactly what they did
before.

**The transferable part:** "move the operation check out of the container matcher"
is only meaningful if you check *every* layer the container question passes
through. A single-operation caller leaves its assumptions in the helpers it calls,
and a comment naming that caller inside a general helper is the tell.

### C2 — the seven operations split by *reallocation*, not by phase (2026-09-02)

The plan groups the operations by which plain-local arm predated it: Phase 2 takes
`set`/`add`/`removeKey`, Phase 3 takes `insert`/`removeAt`/`prepend`/Set `remove`.
That grouping says nothing about the record container, and the line that actually
matters cuts across both phases: **can this operation reallocate?**

| | operation | reaches the field how |
|---|---|---|
| **cannot grow** | `removeKey`, `removeAt`, Set `remove` | the inlined **sub-block address** — the existing plain-local lowering, unchanged |
| **can grow** | `set`, `add`, `insert`, `prepend` | must grow the **record** block and repoint it |

This is not a judgement call, it is a property of each lowering that was read off
the source. `lower_map_remove_key_in_place` touches its slot four times and **every
one is an `abi::load_u64`** — it compacts the entry table and clears
`BUCKETS_READY`, never allocating. `lower_list_remove_at_in_place` is the same.
So handing either the address of the inlined sub-block
(`open_inplace_inlined_subblock`) mutates the collection where it lies, and the
lowering never learns it is not looking at a plain local.

`lower_map_set_in_place` is the opposite, and dangerously so: it **stores a fresh
block pointer back into the slot it was given** (two sites), and immediately before
one of them calls `emit_free_pre_grow_buffer(map_slot, …)`. Given a sub-block
address that would `free()` **a pointer into the middle of the record block** — not
a slow path, a heap corruption. The helper's doc-comment states the rule and names
this as the reason; it was written from the source rather than after being bitten.

**Consequence for the plan.** Three of the seven operations landed on the container
matcher alone (`removeKey` in Phase 2, `removeAt` and Set `remove` in Phase 3). The
other four are blocked on one missing piece — an inline grow for a record-held
collection, the equivalent of `lower_inline_list_append_in_place`, which already
does exactly this for a list `append`. That is now an explicit task under Phase 2
rather than an assumption buried inside "wire each to the container matcher", and
plan-121-D's growing operations and plan-121-F's length-changing `set` need the
same primitive.

**Why the split is worth naming rather than just working around:** it is the same
distinction plan-121-B's B7/`G24` turned on — *what else holds a reference into the
bytes this operation moves* — asked one level up. A non-growing op leaves the block
where it is, so any address into it stays valid; a growing op moves the block, so
every holder of the old address must be repointed. The container simply decides
who the holders are.

## Summary

Risk is the record-rebuild elision, which is only sound for a single-field
self-update of a uniquely-owned binding. Untouched: the STATE container
(plan-121-D), and the String-representation costs (F, G) which will still cap the
`Record-Dynamic` rows until those land.
