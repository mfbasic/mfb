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
| plan-121-C complete and archived | `ls planning/completed/plan-121-C-*` → 1 file | NOT MET |
| `InPlaceDest` resolves the STATE container | plan-121-A Phase 2, final task | NOT MET — **may have been descoped** |

> plan-121-A Phase 2 permits leaving STATE out of the seam if its resolution does
> not fit. **Re-read plan-121-A's Corrections before starting.** If STATE was
> descoped there, this sub-plan's Phase 1 must first bring STATE into the seam,
> and its effort rises from large toward x-large — re-estimate before scheduling.

If plan-121-C is not complete, this sub-plan cannot start, full stop.

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
- **UNVERIFIED — whether a STATE block can be reached from more than one thread
  in a way that makes in-place mutation observable.** This is the sub-plan's
  design uncertainty and Phase 1 settles it before any arm is added. If it can,
  the arm gains a decline condition; it does not make the design impossible.

## 3. Design Overview

Identical in shape to plan-121-C: lift the operation check out of the container
check, resolve an `InPlaceDest` for the STATE field, dispatch to the
per-operation lowerings from plan-121-B.

Two costs, as in C: the collection copy, and the surrounding **state-block
rebuild**. The rebuild elision has the same precondition (single-field
self-update of a uniquely-owned binding) plus one more that records do not have:
the state block may be shared with the resource's runtime, so the elision is only
sound if nothing observes the block between the mutation and the write-back.

**Where correctness risk concentrates:** this sub-plan has the largest blast
radius in plan-121, which is why it is scheduled last of the container work. A
STATE block is reachable from resource internals and, potentially, from another
thread; `.ai/canvas-threading.md` records that arena state is per-thread and that
no thread frees another's block. An in-place mutation that reallocates a STATE
collection could free a block another thread holds. **Phase 1 must establish
reachability before Phase 2 writes a single arm**, and if the answer is "yes,
cross-thread", the arm declines for those resource types rather than proceeding.

**Byte-identity is NOT the gate.** Expected drift: `.ncode` fixtures containing
STATE collection updates other than `append`, plus — because STATE lives in
resource code — potentially the `_mfb_rt_fs_` kitchen-sink goldens. Phase 1
records the set; drift outside it is a bug.

### Rejected alternatives

- **Treat STATE as a record and reuse plan-121-C unchanged.** Rejected: the
  reachability question above has no analogue for a plain record local, so the
  gate set is genuinely different. Sharing the *lowering* is right; sharing the
  *gate* is not.

## Phases

### Phase 1 — Settle STATE reachability, then generalize the container arm

The uncertainty-first phase: the cheapest experiment that could change the design.

- [ ] Determine whether a `RES … STATE` block holding a collection can be
      reached by another thread, or by resource-internal code, between the
      mutation and the write-back. Read `.ai/resources-packages.md` and
      `.ai/canvas-threading.md`; write an rt fixture that attempts it.
- [ ] Record the answer in this plan. If cross-thread reachability exists,
      enumerate the resource types affected — those become decline conditions,
      not a stop.
- [ ] Refactor `try_inplace_state_collection_append` into a container matcher +
      operation dispatch, `append` still the only dispatched operation.
- [ ] Record the `.ncode` goldens containing STATE collection updates.

Acceptance: the reachability question is answered with a fixture, not an
argument, and recorded here; `cargo test --no-fail-fast` green and `.ncode`
byte-identical (only `append` dispatches, so nothing may move).
Commit: —

### Phase 2 — Dispatch `set`, `add`, `removeKey` through the STATE container

- [ ] Wire each operation, applying every decline condition from Phase 1.
- [ ] Elide the state-block rebuild for the single-field self-update case, only
      where Phase 1 showed it is unobservable.
- [ ] Tests: extend `tests/rt_res_state_inplace_mutation.rs` with a case per
      operation; add rt-behavior fixtures asserting state semantics (a state read
      taken before the update does not observe the mutation).

Acceptance: the `set`/`add`/`removeKey` STATE rows reach grade B or better;
`cargo test --no-fail-fast` green; golden drift confined to the Phase 1 set.
Commit: —

### Phase 3 — Dispatch `insert`, `removeAt`, `prepend`, Set `remove`

- [ ] Wire each, carrying plan-121-B's `removeAt` `FOR EACH` decline rule.
- [ ] Tests: the aliasing matrix from plan-121-B Phase 2, for STATE fields.

Acceptance: all 16 STATE rows in §2 reach grade B or better on a fresh
`./benchmark/run.sh 10`; `./scripts/test-accept.sh` clean with `N ran` unchanged;
`list (State-Dynamic) set` specifically is no longer the worst row in the suite.
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

<Filled in during execution.>

## Summary

The largest blast radius in plan-121 and the reason it is scheduled last: a STATE
block may be reachable in ways a plain local is not, and a reallocating in-place
mutation could free a block another thread holds. Phase 1 answers that with a
fixture before any arm exists. Untouched: the String-representation costs (F, G),
which still cap the `State-Dynamic` rows until those land.
