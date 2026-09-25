# bug-692: a record field can hold a `Thread`, and the record then copies — two values own one thread

Last updated: 2026-09-24
Effort: large (3h–1d) — fixed as part of plan-156
Severity: HIGH
Class: Correctness

Status: Open — to be fixed by plan-156 (Thread and ThreadWorker become RES resources)
Regression Test: none yet — `tests/rt-behavior/threads/bug692-record-thread-copy/` (plan-156-D Phase 1)

A `Thread` is not copyable (§14.1: "Threads and resource handles are not
copyable"), and a record is copyable only when all its fields are. Collections
correctly refuse a thread element (`TYPE_COLLECTION_OWNERSHIP_VIOLATION`). A
record field does not: `TYPE H / t AS Thread OF … / END TYPE` compiles, and so
does `LET copy AS H = h`. Two record values then own the same thread, and the
second `waitFor` fails at runtime. Nothing flags it at compile time.

**Correct behavior after the fix.** Under plan-156, a thread in a record is a
`RES` field (§15.6): a pointer, never an owner, and the record is not
copyable. Until then, the copy must not compile.

References:

- Spec §14.1 (copyability), §14 "Ordinary containers cannot store thread
  handles", §15.6 (resource pointers in records).
- plan-156 (the fix). bug-691 (the RES hole in the same checks).
- Found by the plan-156 design review, 2026-09-24.

## Failing Reproduction

```basic
IMPORT io
IMPORT thread

ISOLATED FUNC work(worker AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer
  RETURN n
END FUNC

TYPE H
  t AS Thread OF Integer TO Integer
END TYPE

FUNC main AS Integer
  LET h = H[t := thread::start(work, 5)]
  LET copy AS H = h
  io::print(toString(thread::waitFor(h.t)))
  io::print(toString(thread::waitFor(copy.t)))
  RETURN 0
END FUNC
```

- Observed: builds with no diagnostic; prints `5`, then
  `Error: 7-703-0004 Resource handle is already closed.` (exit 255).
- Expected: a compile error at `LET copy AS H = h` (the record is not
  copyable). Under plan-156, the field is spelled `t AS RES Thread OF …` and
  follows §15.6.

Contrast: `MUT ts AS List OF Thread OF Integer TO Integer = []` is rejected
with `TYPE_COLLECTION_OWNERSHIP_VIOLATION` (measured 2026-09-24). Without the
copy, the record works (prints 5): the hole is only visible once the value is
duplicated. Not yet measured: what dropping such a record does to a running
worker.

## Root Cause

- `src/ir/verify/types.rs:check_type_declarations` checks each field with
  `check_map_key_comparable` (which recurses into nested collections, not the
  field itself), `check_thread_sendability` and `res_axis_slot`. None of them
  rejects a bare `ThreadHandle` field. Collections go through
  `src/ir/verify/values.rs:check_collection_element_thread_free`, and records
  never call it.
- Copyability: the record's copy is not refused. `is_copyable` has a
  `ThreadHandle` arm returning false (`src/ir/verify/resources.rs:is_copyable`),
  so the missing link is the record-copy check not consulting field
  copyability for this case. UNVERIFIED which call site: Phase 1 of the fix
  pins it with the reproduction.

## Goal

- Under plan-156: a thread field must be `RES`, and copying a record that
  holds one is a compile error. The reproduction fails to compile; its
  `RES`-field rewrite runs and prints 5 once.

### Non-goals

- Records of copyable fields stay copyable. Collections' existing rejection is
  unchanged until plan-156 moves them to §15.6 pointer storage.

## Blast Radius

- `check_type_declarations`: fixed by plan-156.
- Other non-copyable field types (resources): already require `RES` fields
  (`res_axis_slot`). Unaffected.

## Fix Design

Fixed by plan-156, verifier phase (ThreadHandle becomes a resource, so
`res_axis_slot` requires `RES` on the field and the §15.6 rules apply).

## Phases

Tracked in plan-156-D (`planning/plan-156-D-*.md`); this document
records the reproduction and closes when that phase lands. plan-156-F Phase 3
archives it with a `STATUS: FIXED` block.

Commit: —

## Summary

Silent double ownership of a thread through a record copy. The fix is
plan-156.
