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
| plan-121-B complete and archived | `ls planning/completed/plan-121-B-*` → 1 file | NOT MET |
| `InPlaceDest` resolves the record container | plan-121-A Phase 2 landed it | NOT MET |

If plan-121-B is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- `set`, `insert`, `removeAt`, `prepend`, `add`, `remove`, `removeKey` applied to
  a collection in a record field, in the self-update `WITH` form, take the
  in-place path.
- The 16 record-container rows in §2 reach grade B or better.

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

- [ ] Refactor `try_inplace_record_field_append` into a container matcher that
      yields an `InPlaceDest` plus the call node, with the `Some("append")` check
      moved to the dispatch step.
- [ ] Record the `.ncode` goldens containing record-field collection updates.
- [ ] Tests: existing record-field append coverage must pass unchanged.

Acceptance: `cargo test --no-fail-fast` green **and** `.ncode` byte-identical to
HEAD — at this point only `append` dispatches, so nothing may move. A diff is a
bug in the refactor: objdump one fixture and localize it.
Commit: —

### Phase 2 — Dispatch `set`, `add`, `removeKey` through the record container

The three operations whose plain-local arms predate this plan.

- [ ] Wire each to the container matcher from Phase 1.
- [ ] Elide the whole-record rebuild for the single-field self-update case (§3).
- [ ] Tests: rt-behavior fixtures asserting `WITH` value semantics are unchanged
      — a record copied before the update must not observe the mutation; a
      second field updated in the same `WITH` must take the rebuild path.
- [ ] Tests: codegen-inspection, path-taken per operation.

Acceptance: the six `set`/`add`/`removeKey` record rows reach grade B or better;
`cargo test --no-fail-fast` green; golden drift confined to the Phase 1 set.
Commit: —

### Phase 3 — Dispatch `insert`, `removeAt`, `prepend`, Set `remove`

The operations plan-121-B added, now in the record container.

- [ ] Wire each to the container matcher, carrying plan-121-B's `removeAt`
      `FOR EACH` decline rule into this container.
- [ ] Tests: the same aliasing matrix as plan-121-B Phase 2, for record fields.

Acceptance: all 16 record-container rows in §2 reach grade B or better, measured
by `./benchmark/rank.py` on a fresh `./benchmark/run.sh 10`;
`./scripts/test-accept.sh` clean with the `N ran` count unchanged.
Commit: —

## Validation Plan

- **Tests:** rt-behavior fixtures under `tests/rt-behavior/collections/` for the
  `WITH` semantics cases (copy-before-update, multi-field update, iteration
  aliasing); codegen-inspection tests for path-taken and for each decline.
- **Coverage check:** confirm each new dispatch arm is actually executed by a
  test, not merely compiled.
- **Runtime proof:** spike 1 variant C re-run — per-set cost must stop rising
  with N and approach variant A's flat profile.
- **Doc sync:** `.ai/collections.md` gains the record-rebuild elision rule and
  its `updates.len() == 1` precondition.
- **Acceptance:** `cargo test --no-fail-fast`, `./scripts/test-accept.sh`, the
  artifact gate, `cargo fmt` per AGENTS.md.

## Open Decisions

- **Whether the record-rebuild elision belongs in this sub-plan or its own** —
  recommend keeping it here, because without it the rows keep a fixed ~900 ns
  floor and the acceptance criterion cannot be met. (§3)

## Corrections

<Filled in during execution.>

## Summary

Risk is the record-rebuild elision, which is only sound for a single-field
self-update of a uniquely-owned binding. Untouched: the STATE container
(plan-121-D), and the String-representation costs (F, G) which will still cap the
`Record-Dynamic` rows until those land.
