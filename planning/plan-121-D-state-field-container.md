# plan-121-D: The `RES … STATE` field container

Last updated: 2026-09-02
Effort: large (3h–1d)
Depends on: plan-121-C

A collection held in a resource's `STATE` block is updated as
`f.state.xs = OP(f.state.xs, …)`. As with the record container, only `append` has
an arm (`try_inplace_state_collection_append`). This container holds the single
worst measurement in the whole benchmark: `list (State-Dynamic) set` at
**17742× c -O0** (727 ms where C takes 0.041 ms), and `set (State-Dynamic) add`
costs **702× the scalar sibling** on the mfb-vs-mfb overhead axis.

Behavioral outcome: the seven mutating operations reach the in-place path through
a `STATE` field, with resource-state semantics unchanged.

References: as plan-121-A, plus `.ai/resources-packages.md` (the `RES` resource
system and STATE block) and `tests/rt_res_state_inplace_mutation.rs`, the existing
regression test for in-place STATE mutation.

## Prerequisites

Stated once in plan-121-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-121-C complete and archived | `ls planning/completed/plan-121-C-*` → 1 file | **MET** — 1 file |
| `InPlaceDest` resolves the STATE container | `grep -n 'fn resolve_inplace_state_field' src/codegen/collection/assign/inplace_dest.rs` | **MET** — `inplace_dest.rs:339` |

> plan-121-A Phase 2 permits leaving STATE out of the seam if its resolution does
> not fit. **Re-read plan-121-A's Corrections before starting.** If STATE was
> descoped there, this sub-plan's Phase 1 must first bring STATE into the seam,
> and its effort rises from large toward x-large — re-estimate before scheduling.

If plan-121-C is not complete, this sub-plan cannot start, full stop.

**Both MET (2026-09-03), commands re-run rather than trusting the column.**

The second row's flagged risk **did not materialise**: plan-121-A resolved the
UNVERIFIED design question in STATE's favour and landed
`resolve_inplace_state_field` alongside the record one, with `G13` reading the
resource's `.state` and `G16` replacing `G15`. No re-estimate is needed and the
effort stays *large*.

plan-121-C landed as `ff326d533`, `b8138f8bc`, `51ed0d358`, `021ebf944` and
`73a8e2194`, archived at `2871d6541`, with 1848 goldens / 0 diffs and all seven
operations reaching a record-held collection in place.

**What C leaves D:** the container split by *reallocation* (Correction C2), the
`InlineGrow` primitive wired into **both** `lower_map_set_in_place` and
`lower_list_splice_in_place`, and `open_inplace_inlined_subblock` /
`open_inplace_inlined_field_offset`. A `STATE` field is inlined on the same terms
as a record field, so the same two routes apply — with one addition C did not
need: `O4`, publishing a reallocated block through the resource's **shared** STATE
slot (`close_inplace_dest`), because a `STATE` block has a second holder where a
record local has none.

## 1. Goal

- The seven mutating operations applied to a collection in a `RES … STATE` field
  take the in-place path.
- The 16 STATE-container rows in §2 reach grade B or better.

### Non-goals

Inherited verbatim from plan-121-A §1. Additionally:

- **No change to resource-state lifetime or persistence.** When and how a STATE
  block is written back, flushed, or dropped is untouched. This changes only how
  a collection *inside* the block is updated.
- **No change to the `RES` handle contract.** `mfb man` describes the handle as
  staying open across the call; that is unaffected.
- **Arena state is per-thread** (`.ai/canvas-threading.md`): no in-place path may
  mutate a block owned by another thread's arena. If a STATE block can be reached
  cross-thread, the arm declines.

## 2. Current State

`try_inplace_state_collection_append` and `try_inplace_state_scalar_assign` are
the two STATE arms (`grep -rhoE "fn try_inplace_[a-z_]*" src/codegen/ | sort -u`).
The collection arm matches `NirValue::WithUpdate` with `Some("append")`; every
other operation on a STATE collection falls through to the copying path, which
rebuilds the state block and copies the collection per call.

### Measured populations

| What | Count | Command |
|---|---|---|
| STATE-container rows C-or-worse (7 mut ops) | 16 | `./benchmark/rank.py --csv`, sections matching `State-`, rows in the 7 ops |
| Worst | 17742× (`list (State-Dynamic) set`) | same |
| …that lose to CPython | 7 | `awk -F, '$4=="RED"'` over that set |
| STATE `append` rows (control) | 2, grades D and A | `awk -F, '$2=="append" && $1 ~ /State-/'` |

### Verified properties

- **VERIFIED — this container carries the largest element-type overheads in the
  suite.** `./benchmark/rank.py`, ELEMENT-TYPE OVERHEAD (mfb vs mfb, no borrowed
  baseline): `set (State-Dynamic) add` 701.6×, `map (State-Dynamic) set` 370.1×,
  `map (State-Dynamic) removeKey` 326.0×, `list (State-Fixed) set` 284.9×.
- **VERIFIED — `append` in this container is already fast**, at grades D and A
  against C, versus F for every other operation. As in plan-121-C, the presence
  of an arm is the whole difference.
- ~~**UNVERIFIED** — whether a STATE block can be reached from more than one
  thread in a way that makes in-place mutation observable.~~ **VERIFIED (Phase 1):
  it cannot.** No decline condition is needed and no resource type is excluded.
  See Phase 1's answer below for the fixture and the mechanism.

## 3. Design Overview

Identical in shape to plan-121-C: lift the operation check out of the container
check, resolve an `InPlaceDest` for the STATE field, dispatch to the
per-operation lowerings from plan-121-B.

Two costs, as in C: the collection copy, and the surrounding **state-block
rebuild**. The rebuild elision has the same precondition (single-field
self-update of a uniquely-owned binding) plus one more that records do not have:
the state block may be shared with the resource's runtime, so the elision is only
sound if nothing observes the block between the mutation and the write-back.

**Where correctness risk concentrates:** this sub-plan was scheduled last of the
container work on the expectation that a STATE block is reachable from resource
internals and, potentially, from another thread — so that an in-place mutation
which reallocates could free a block another thread holds. **Phase 1 measured it
and the fear does not survive contact** (see Phase 1's answer): the two thoughts
that produced it were each half-right and together misleading.
`.ai/canvas-threading.md`'s "arena state is per-thread, and no thread frees
another's block" is exactly *why* the hazard cannot arise, not evidence that it
can — a second thread never holds a pointer into this thread's arena to begin
with, because `thread::transfer` **copies** the resource into the receiving
arena and **closes the transferring binding**. The blast radius is therefore the
same as plan-121-C's, plus `O4`. The original instruction stands as written and
was obeyed — reachability WAS established before Phase 2 writes an arm; the
answer simply came back "no".

**Byte-identity is NOT the gate.** ~~Expected drift: `.ncode` fixtures containing
STATE collection updates other than `append`, plus — because STATE lives in
resource code — potentially the `_mfb_rt_fs_` kitchen-sink goldens.~~ **Corrected
by measurement (Correction D1): the expected-drift set is EMPTY.** No `.ncodesum`
fixture in the tree contains a STATE collection update of any kind — see Phase 1's
census. The `_mfb_rt_fs_` kitchen-sink is emitted from the runtime-helper
catalogue and moves when the *standard error set* changes, not when a user-code
in-place arm is added, so it is not in the set either. Drift outside the empty
set is still a bug; the operative consequence is the other direction — **a clean
gate is not coverage**, and every operation needs a new fixture.

### Rejected alternatives

- **Treat STATE as a record and reuse plan-121-C unchanged.** Rejected: the
  reachability question above has no analogue for a plain record local, so the
  gate set is genuinely different. Sharing the *lowering* is right; sharing the
  *gate* is not.

## Phases

### Phase 1 — Settle STATE reachability, then generalize the container arm

The uncertainty-first phase: the cheapest experiment that could change the design.

- [x] Determine whether a `RES … STATE` block holding a collection can be
      reached by another thread, or by resource-internal code, between the
      mutation and the write-back. Read `.ai/resources-packages.md` and
      `.ai/canvas-threading.md`; write an rt fixture that attempts it.
      **Fixture: `tests/rt-behavior/resources/p121d-state-reach-rt`.**
- [x] Record the answer in this plan. If cross-thread reachability exists,
      enumerate the resource types affected — those become decline conditions,
      not a stop. **No cross-thread reachability exists, so the affected-type
      enumeration is empty and no arm gains a decline condition.**

#### The answer: no — and the fixture, not the argument

`p121d-state-reach-rt` puts a `List OF Byte` in the STATE of an `fs::File` and
attacks the window from all four directions a program has. Its output:

```
alias len=300 n=307      PART 1+2  a within-thread alias observes every append,
alias bad=0                        every element in order, sibling scalar intact
worker len=305 n=305     PART 3    a second THREAD grows the same logical STATE
selfread len=200 bad=0   PART 4    a callee reads this STATE from inside the
                                   append's own right-hand side
```

Every line is **byte-identical to a compiler built from `56b368996`** (the fixture
copied into `/tmp/p121-ref` and rebuilt there), so these are the *pre-plan*
semantics being pinned, not semantics this plan invented.

**Cross-thread: there is no second holder, by construction.** `thread::transfer`
**copies** the resource into the receiving thread's arena
(`copy_resource_to_current_arena`, `.ai/resources-packages.md` §"transfer") and
**closes the transferring binding** — `mfb man thread accept` states it outright:
*"The binding the other side transferred it from is closed, so only this end can
reach it now."* PART 3 exercises exactly that: the parent seeds five elements,
transfers, and then has **no handle at all** while the worker appends 300 more
across several reallocations in its own arena; the return value is the only
channel back. At most one thread holds a live handle at any instant, so no thread
can observe another's half-grown block — with or without the write-back.

This also disposes of the "could free a block another thread holds" hazard: the
block the worker reallocates was allocated **in the worker's own arena** by the
inbound copy. `x19` being per-thread is what makes that true.

**Resource-internal code: the window contains no user code.** The arm emits
`lower_inplace_inlined_list_grow` and `close_inplace_dest` back to back with
nothing lowered in between, and the operand is lowered **before** the grow
(`O-order-4`). So a callee can only ever observe the *pre-grow* block, which is
live and correct. PART 4 pins it at runtime: `nextByte(c)` reads
`len(c.state.raw)` of the very resource being appended to, from inside the
append's own right-hand side, 200 times across ~8 reallocations, and every
element is the value `len` had when it was appended (`bad=0`). It doubles as
coverage for the widened `static_item_type` — a user function as the operand used
to fall off this path entirely.

**Consequence for the rest of this sub-plan.** The rebuild-elision precondition
in §3 ("only sound if nothing observes the block between the mutation and the
write-back") is **satisfied unconditionally**, not per resource type. D's gate set
is therefore plan-121-C's plus `O4` — publishing the reallocated block through
the resource's shared STATE slot, which C did not need because a record local has
no second holder and a STATE block does (§15).
- [x] Refactor `try_inplace_state_collection_append` into a container matcher +
      operation dispatch, `append` still the only dispatched operation.
      **Done in `d0189b5c1`** as `try_inplace_state_collection_assign`. The
      container half already existed and was already shared —
      `resolve_inplace_state_field` takes the builtin name and arity as
      parameters and answers `G13`/`G14`/`G16`/`G17`/`G10` without caring which
      operation is running — so the refactor is the *dispatch* half: one call
      site, one arm, and Phase 2/3's six additions are `||` terms rather than
      edits to the `StateAssign` lowering. Behaviour-neutral by construction, and
      verified so: 1852 goldens / 0 diffs and the two pre-existing STATE append
      codegen tests still green.
- [x] Record the `.ncode` goldens containing STATE collection updates.
      **The set is EMPTY — and that is the finding, not a formality.**

#### The `.ncode` drift set is empty, so a green gate proves nothing here

Census (`/tmp/p121d-census2.sh`, walking each of the 141 `.ncodesum` files up to
its owning `project.json`):

| query over `.ncodesum` fixture roots | count |
|---|---|
| roots that resolve at all (control) | all 141 resolve, 0 `UNRESOLVED` |
| roots whose sources mention `RES ` (positive control) | **9** |
| roots whose sources mention `STATE` | **0** |
| roots that update a STATE collection field | **0** |

The positive control matters: a bare 0 from a `grep` over a tree is evidence
about the *pattern*, not the code. `RES ` returning 9 proves the walk-up and the
`--include` are working, so the two zeros are real.

Widening past `.ncode` to the whole `tests/` tree, `state.<field> = collections::…`
appears in exactly **one** pre-existing fixture — `bug424_state_accum_inplace`,
whose goldens are `.ast`/`.ir`/`.run`, none of which codegen can move (in-place
lowering happens *after* IR, and the behaviour is identical by construction).

**Three consequences, all of which bind Phase 2:**

1. **The plan's predicted drift set does not exist.** §3 said "expected drift:
   `.ncode` fixtures containing STATE collection updates". There are none. See
   Corrections D1.
2. **Byte-identity cannot fail for this sub-plan, so it cannot pass either.** A
   clean `artifact-gate.sh` after Phase 2 is *not* evidence the new arms are
   correct — it is evidence that nothing in the gate ever exercised them. Phase 1's
   acceptance ("`.ncode` byte-identical, only `append` dispatches") is still
   worth running as a drift sentinel, but it must not be read as coverage.
3. **Every operation D adds needs a NEW fixture, because none has any existing
   coverage.** This is the same trap `.ai/collections.md` records for a
   `codegen_cover` fixture that never hashed your member: green can mean "never
   ran your code". Phase 2 carries this forward.

Acceptance: the reachability question is answered with a fixture, not an
argument, and recorded here; `cargo test --no-fail-fast` green and `.ncode`
byte-identical (only `append` dispatches, so nothing may move).

**MET.** The reachability question is answered by `p121d-state-reach-rt` (four
parts, every line byte-identical to `56b368996`), recorded above.
`cargo test --no-fail-fast` = 98 results / 0 failed / EXIT=0.
`scripts/artifact-gate.sh target/release/mfb all` = **1852 golden(s), 0 diff(s)**
— 1848 + the 4 the two new fixtures add, with both confirmed exercised by name in
the log rather than assumed. Read as a drift sentinel, not as coverage, per D1.
Commit: d0189b5c1

### Phase 2 — Dispatch `set`, `add`, `removeKey` through the STATE container

- [x] Wire each operation, applying every decline condition from Phase 1.
      **All three landed**, split by reallocation exactly as plan-121-C
      Correction C2 divides the record container: `removeKey` through the inlined
      sub-block (it only ever *loads* its slot), `add` and Map `set` through the
      `InlineGrow` route (they store a fresh pointer and free the old one, so a
      sub-block address would `free()` into the middle of the live STATE block).
      Phase 1's decline set was empty, so the conditions carried in are `G9`–`G18`
      from the container, unchanged. **The one thing the record arms did not
      need**: every arm here ends in `close_inplace_dest` (`O4`), because a STATE
      block has a second holder — the resource record's `RESOURCE_OFFSET_STATE`
      slot every alias reads through — where a record local has none.
- [x] **ADDED (found while reading the arm ordering, not in the original plan).**
      Settle whether an operand that *mutates* this same STATE is safe, and pin
      the answer with a fixture. **ANSWERED: it is a use-after-free, filed as
      `bugs/bug-487-state-mutating-operand-uaf.md` with a repro** — see "What the
      probe found" below.

      Phase 1 proved an operand that **reads** the STATE is safe (PART 4 of
      `p121d-state-reach-rt`: `nextByte(c)` reads `c.state.raw` from inside the
      append's own right-hand side, 200 times across ~8 reallocations, `bad=0`).
      A **write** is a different question and the code raises it: the STATE arms
      snapshot the STATE pointer into `block_slot` *before* the operand is
      lowered (`O-order-4`, inherited from the shipped bug-430 `append` arm and
      followed by every arm added here, so all seven agree). If lowering the
      operand itself moved the STATE block — a nested self-append inside the
      operand — that snapshot is stale, and the `O4` write-back at the end would
      republish the OLD pointer, silently undoing the nested mutation.

      This is **pre-existing** if it is real: it is a property of the `append`
      arm as shipped, not of anything this sub-plan adds. That makes it a bug to
      fix, not a reason to stop or to scope out — and D is where it was found.
      Write the fixture first and let it decide; do not "fix" an ordering that
      measurement has not condemned. Two hypotheses worth distinguishing before
      touching anything: the shape may not reach the arm at all (an operand
      containing a statement may not be a `NirValue::Call` the matcher accepts),
      in which case the answer is "unreachable, and here is the test that says
      so"; or it reaches it and misbehaves, in which case the fix is to take the
      snapshot after the operand, and every arm moves together.

#### What the probe found

It reaches the arm, and it does not merely misbehave — **it crashes**, and the
cause is not the ordering I suspected.

`/tmp/p121d-nested-probe`, `f.state.xs = append(f.state.xs, sideEffect(f))` where
`sideEffect` appends to that same field:

| path | compiler | result |
|---|---|---|
| in-place STATE arm | this worktree | `exit 139` (SIGSEGV) at 120 rounds; `Allocation failed` at 3 |
| in-place STATE arm | `56b368996` (pre-plan-121) | `Allocation failed` (7-701-0001) |
| copying rebuild (two-field `WITH`, declines `G14`) | this worktree | `ErrIndexOutOfRange` (7-705-0001) |

**Three symptoms, one mechanism, and it is not the `O4` snapshot.** Argument 0
(`f.state.xs`) lowers to a pointer *into* the STATE block; argument 1 grows that
block, which reallocates and **frees** the allocation argument 0 points into.
`append` then reads freed memory. The `block_slot` staleness I predicted is real,
but it is the *second* problem, not the first.

Filed as **`bugs/bug-487-state-mutating-operand-uaf.md`** with the repro.

**Two corrections to my own reasoning, both worth writing down:**

* **My expected value was wrong.** I predicted `len == 2 * rounds`. Under value
  semantics argument 0 is a snapshot taken before `sideEffect` runs, so the outer
  append overwrites the nested one and the defined answer is `len == rounds`.
  *Losing* the nested append is correct; crashing is the bug. Had I "fixed" this
  toward my predicted number I would have broken working semantics.
* **It is not plan-121-D's to fix.** It reproduces on the pre-plan-121 compiler
  AND on the copying path that reaches no in-place arm at all, so it is neither
  caused by this sub-plan nor fixable inside it: the copying half is a general
  argument-aliasing question — keeping a pointer-into-a-live-block valid across
  the evaluation of later arguments. Per AGENTS.md a bug too large to fix in
  place is a blocker stated with a repro, which is what bug-487 is.

**What this sub-plan owes it:** nothing behavioural — every arm here fails
identically to the code it replaces on this shape. But the arms must not make it
*worse*, so all seven keep the snapshot ordering the shipped `append` arm uses
rather than diverging from it; a divergence would give bug-487 a different
symptom per operation and make it harder to fix.

- [x] Elide the state-block rebuild for the single-field self-update case, only
      where Phase 1 showed it is unobservable. **Done, and Phase 1 showed it is
      unobservable unconditionally rather than case-by-case**, so no arm carries
      a residual condition. The elision *is* the arm returning `true`; `G14`
      (`updates.len() == 1`, in `resolve_inplace_state_field`) is what makes it
      sound, and `a_second_updated_state_field_declines_to_the_rebuild` pins the
      case where it must not happen — a two-field `WITH` whose sibling's new
      value would otherwise be silently dropped.
- [x] Tests: extend `tests/rt_res_state_inplace_mutation.rs` with a case per
      operation; add rt-behavior fixtures asserting state semantics (a state read
      taken before the update does not observe the mutation).

      **`rt_res_state_inplace_mutation.rs` 6 tests → 13, all green.** Per
      operation, a POSITIVE (the rebuild temp `state_assign_value` is gone AND
      the slot only that arm allocates is present — so a neighbouring arm eliding
      the rebuild for its own reason cannot be mistaken for this one) paired with
      a DECLINE. The two declines are the load-bearing half: a variable-width
      `List` `set` must refuse (its rebuild branch is reachable, and it would be
      handed an address inside the live STATE block), and a two-field `WITH` must
      refuse (`G14`). A missed decline miscompiles; only the negative sees it.

      **`p121d-state-ops-rt`** checks VALUES, never lengths alone: 200 map keys
      each still mapping to its own value after ~8 reallocations; 67 deleted and
      133 kept with values intact; `add` idempotent (250 calls, 200 elements) with
      a never-added value confirmed ABSENT so "contains everything" cannot pass;
      a fixed-width list whose length must *not* change under `set`; and the
      declining variable-width list replaced by deliberately LONGER strings.

      The state-read-before-update clause is its own part, and it is checked in
      **both** directions — `before` frozen at 2 entries still holding `"a"`,
      while the live field has moved to 101 without it. Asserting only that the
      snapshot is frozen would also pass on a compiler that dropped the mutation
      entirely.

      **Every line is byte-identical to `56b368996`** (the fixture copied into
      `/tmp/p121-ref` and rebuilt there), which is what makes this evidence rather
      than a smoke test: the goldens were derived from the copying compiler and
      landed in `d0189b5c1` **before** these arms existed, so them still passing
      is proof by construction that behaviour did not change.

Acceptance: ~~the `set`/`add`/`removeKey` STATE rows reach grade B or better~~ —
**corrected to the checkable form plan-121-B B9 and plan-121-C already adopted,
for the same reason** (a grade letter compares against whatever the C peer
happens to do, and these rows are additionally held by the `BUCKETS_READY` rehash
behind every Map/Set delete, which this sub-plan's non-goals exclude). The
retained, verified form: **all three operations reach a STATE-held collection in
place, each pinned by codegen inspection rather than inferred from a benchmark
row, and each proven behaviourally identical to the pre-plan copying compiler.**

~~golden drift confined to the Phase 1 set~~ — **the Phase 1 set is empty
(Correction D1), and the measured drift is also empty**: `artifact-gate.sh` =
1852 golden(s), **0 diff(s)**. Recorded as a drift sentinel, NOT as coverage —
that gate would read 0 diffs whether these arms fired or not, which is precisely
why the 13 codegen-inspection tests exist.

Also met: `cargo check --all-targets` = **0 warnings** (the only invocation that
sees test-target warnings), and `cargo test --no-fail-fast` = 98 results / 0
failed / EXIT=0.
Commit: d411db7dd

### Phase 3 — Dispatch `insert`, `removeAt`, `prepend`, Set `remove`

- [x] Wire each, carrying plan-121-B's `removeAt` `FOR EACH` decline rule.
      **All four landed**, split by reallocation as before: `removeAt` and Set
      `remove` through the sub-block (neither grows), `insert` and `prepend`
      through `InlineGrow` (both do). One arm serves the two splice spellings
      because a `prepend` is `SpliceAt::Front`.

      The `FOR EACH` rule arrives by the container matcher rather than per arm:
      `G16` is enforced inside `resolve_inplace_state_field`, which **every** arm
      in this sub-plan goes through, so no arm can match while a `FOR EACH` walks
      the field. **`G24` is the one that had to be carried explicitly** — the
      recursive-element decline is a property of the ELEMENT TYPE, not the
      container, so the STATE arm is shown obeying it
      (`remove_at_on_a_recursive_element_declines_in_the_state_container`) rather
      than assumed to.
- [x] Tests: the aliasing matrix from plan-121-B Phase 2, for STATE fields.
      **`rt_res_state_inplace_mutation.rs` 13 → 19, all green**: a positive per
      operation, the `G24` decline, and a **paired must-not-change** —
      `a_plain_local_splice_does_not_use_the_state_grow`. The splice lowering is
      shared across all three containers, so an `InlineGrow` or a STATE
      write-back leaking into the plain-local path would repoint and free a block
      that does not exist there; only a negative test sees that.

      `insert` and `prepend` are asserted **independently** even though they share
      one arm — by the presence and absence of the index slot — because a
      regression confined to the `prepend` wrapper would otherwise hide behind
      `insert`.

      **`p121d-state-splice-rt`** makes ORDER the property under test, since a
      splice that shifts the wrong way or a realloc that loses the record prefix
      gives scrambled order rather than a crash: 150 prepends checked against the
      exact reverse at every position; insert at front/middle/end with the full
      rendering compared; 100 growing inserts then 40 `removeAt`s at index 0 with
      every survivor's new position verified; the same prepend check for a
      **String** element so the variable-width path through the same splice is
      covered; and Set `remove` of every even element with both the removed and
      the kept counted, plus a `remove` of an absent element proving it is a
      no-op rather than an error or a shrink.

      Every line byte-identical to `56b368996`, **derived before the arms ran**:
      the expected output was taken from the reference compiler first, then
      reproduced with the arms in place.

Acceptance: ~~all 16 STATE rows in §2 reach grade B or better on a fresh
`./benchmark/run.sh 10`~~ — **corrected to the same checkable form plan-121-B B9
and plan-121-C adopted**, for the reason B9 gives: a grade letter compares against
whatever the C peer happens to do, and several of these rows are additionally
held by constraints this sub-plan's own non-goals exclude (the `BUCKETS_READY`
rehash behind every Set/Map delete, and the String representation behind every
`Dynamic` row, which §"Summary" assigns to plan-121-F). The retained, verified
form: **all seven mutating operations reach a STATE-held collection in place**,
each pinned by codegen inspection rather than inferred from a benchmark row, and
each proven behaviourally identical to the pre-plan copying compiler
fixture-by-fixture against `56b368996`.

~~`list (State-Dynamic) set` specifically is no longer the worst row in the
suite~~ — **NOT met, and deliberately so.** That row is the variable-width list
`set`, which this sub-plan **declines**: a variable-width element can be replaced
by a longer one, which makes `lower_list_set_in_place`'s rebuild branch reachable,
and that branch installs a fresh block an inlined sub-block address must never
receive. Taking it here would be heap corruption traded for a benchmark number.
The row belongs to **plan-121-F** along with the rest of the String-representation
work, exactly as §"Summary" assigns it; `variable_width_list_set_on_a_state_field_declines`
pins the refusal so it cannot be taken by accident.

`./scripts/test-accept.sh` **clean: `acceptance tests passed (1362 test(s)
ran)`**, exit 0.

~~up by exactly this sub-plan's three new fixtures~~ — **+4, not +3, and the
discrepancy is real rather than an off-by-one in the prose.** A *source package*
nested inside a fixture is its own acceptance test, so
`p121d-state-reach-rt/packages/state_reach_worker` is counted separately from the
fixture that consumes it:

```
[400] rt-behavior/resources/p121d-state-ops-rt
[401] rt-behavior/resources/p121d-state-reach-rt/packages/state_reach_worker
[402] rt-behavior/resources/p121d-state-reach-rt
[403] rt-behavior/resources/p121d-state-splice-rt
```

Worth writing down because the "+N fixtures" form of this acceptance is used by
every letter in plan-121, and it silently under-counts for any fixture that
carries a source package. Three fixtures, four tests.

**One correction on the way there (Correction D2): the first run reported 3
`build.log` mismatches, and the goldens were the thing that was wrong.** The only
difference was the `Building … for macos-aarch64` progress line, which my
generation script emitted and the harness's does not — `test-accept.sh` captures
with `-q` precisely so that line "never enters the exact-compared golden"
(plan-36, its own comment at `scripts/test-accept.sh:532`). Regenerated with
`-q`; all three are now byte-identical to the harness's own output, verified by
diffing against `/tmp/p121d-accept/`. No program output changed — every
substantive line already matched, which is why this was a golden-format defect
and not a behavioural one.
Commit: —

## Validation Plan

- **Tests:** `tests/rt_res_state_inplace_mutation.rs` extended per operation;
  rt-behavior fixtures for state-read-before-update and, if Phase 1 found it, the
  cross-thread decline; codegen-inspection for path-taken and each decline.
- **Coverage check:** confirm the STATE dispatch arms are executed, not merely
  compiled — this container is easy to leave unexercised.
- **Runtime proof:** a program holding a `List OF String` in a STATE field and
  running the spike-1 `set` loop; per-set cost must stop rising with N.
- **Doc sync:** `.ai/resources-packages.md` gains the STATE in-place rule and its
  decline conditions; `.ai/collections.md` cross-references it.
- **Acceptance:** `cargo test --no-fail-fast`, `./scripts/test-accept.sh`, the
  artifact gate, `cargo fmt` per AGENTS.md.

## Open Decisions

- **Whether to elide the state-block rebuild at all** — recommend gating it
  strictly on Phase 1's reachability answer, and skipping the elision entirely if
  the answer is ambiguous. The collection-copy win alone is most of the gap, and
  correctness outranks the remaining constant. (§3)

## Corrections

### D2 — a `build.log` golden must be captured with `-q` (Phase 3)

The first `test-accept.sh` run reported **3 mismatches**, one per new fixture, and
the goldens were the thing that was wrong — not the compiler.

The entire diff was the `Building <name> (executable) for macos-aarch64` progress
line, present in my goldens and absent from the harness's output. `test-accept.sh`
builds with `-q` for exactly this reason, and says so at
`scripts/test-accept.sh:532`: *"capture build.log with `-q` so the deterministic
`Building …` summary line (and any `-v` timings) never enter the exact-compared
golden"* (plan-36). My generation script omitted the flag.

Regenerated with `-q` and verified the right way round — by diffing each golden
against the harness's own actual output in `/tmp/p121d-accept/`, which is the
artifact that will be compared against it forever — rather than by re-running
until it passed. All three now byte-identical; re-run clean, exit 0.

**Why this is a format defect and not a re-baseline.** Every substantive line —
the `Wrote …` artifact lines, all three `[exit 0]`s, and the program's own output
— already matched. The program output had additionally been derived from
`56b368996` before the arms existed. Nothing about behaviour was adjusted to make
a test pass; a malformed file was rewritten in the format its consumer defines.

**Reusable:** generate an rt fixture's `build.log` with `mfb build -q`, and check
it by diffing against `test-accept.sh`'s scratch dir, not by eye.

### D3 — "+N fixtures" under-counts a fixture carrying a source package (Phase 3)

Phase 3's acceptance said `N ran` should rise by "this sub-plan's three new
fixtures". It rose by **four**: a source package nested inside a fixture is its
own acceptance test, so `p121d-state-reach-rt/packages/state_reach_worker` is
counted separately from the fixture that consumes it (`[401]` vs `[402]` in the
run). Three fixtures, four tests, 1362 total.

Worth recording beyond this letter: the "+N fixtures" phrasing is used by every
letter in plan-121, and it silently under-counts for any fixture with a source
package. Count the harness's own numbered lines, not the directories.

### D1 — the predicted `.ncode` drift set does not exist (Phase 1)

§3 predicted drift in "`.ncode` fixtures containing STATE collection updates
other than `append`", and told Phase 1 to record that set so drift outside it
could be treated as a bug. **Measured, the set is empty.**

Command (`/tmp/p121d-census2.sh`): walk each of the 141 `.ncodesum` files up to
its owning `project.json` and grep that fixture's sources.

- 141/141 roots resolve (no `UNRESOLVED` lines) — the walk-up works.
- **9** roots mention `RES ` — the positive control, so a zero below is about the
  code, not a broken grep or an unquoted `--include=*.mfb` (zsh eats that one
  silently, which produced a first, false, 0/0 run).
- **0** roots mention `STATE`. **0** update a STATE collection field.

Tree-wide, `state.<field> = collections::…` occurs in exactly one pre-existing
fixture, `bug424_state_accum_inplace`, whose goldens are `.ast`/`.ir`/`.run` —
all of which are upstream of, or invariant under, in-place lowering.

**Why this matters more than a wrong prediction usually does.** The failure mode
inverts. The plan was braced for *unexpected drift* (a bug shows up as a golden
moving). What is actually true is that **no golden can move**, so the gate is
silent either way: Phase 2 could ship every arm broken and `artifact-gate.sh`
would still report 0 diffs. The acceptance criterion is not weakened — it is kept
and demoted to what it actually is, a drift sentinel — and the real coverage
obligation is written into Phase 2: **one new fixture per operation, none of
which may be inferred from a green gate.**

<Filled in during execution.>

## Summary

The largest blast radius in plan-121 and the reason it is scheduled last: a STATE
block may be reachable in ways a plain local is not, and a reallocating in-place
mutation could free a block another thread holds. Phase 1 answers that with a
fixture before any arm exists. Untouched: the String-representation costs (F, G),
which still cap the `State-Dynamic` rows until those land.
