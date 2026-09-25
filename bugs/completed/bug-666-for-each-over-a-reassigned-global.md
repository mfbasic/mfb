# bug-666: `FOR EACH` over a global the body reassigns reads freed memory

Last updated: 2026-09-20
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Fixed
Regression Test: tests/runtime/rt_for_each_over_reassigned_global.rs

## STATUS: FIXED (9317e6eac)

Landed together with bug-665 (the Open Decision: its generalized walker,
`engine/value/store_reach.rs`, is the one this uses). `lower_for_each` marks a
global-rooted iterable for the operand snapshot when `ops_reach_store(body,
StoreLeaf::Global(g))`; the copy is a statement-scope temporary, so the end of
the `FOR EACH` statement frees it on every exit edge — fall-through, `EXIT FOR`
and `RETURN` are all measured flat by the test's `--debug` `live_bytes` check,
which goes red (264224 → 528224 B at 500/1000) when the free is removed.

Beyond the doc's three rows, `Map` and `Set` globals (the entry-table arms of
`lower_for_each`) crashed the same way and are fixed and tested. The local-source
contrast was re-measured: a `MUT` local and a local record field already visit
the entry value. `mfb spec language control-flow` now states the guarantee.

Validation, all measured in the `worktree-B-665` worktree after merging main
(2db51876b) into it:

- `scripts/artifact-gate.sh target/release/mfb all` → `1473 tests, 1648 build(s), 2078 golden(s) checked, 0 diff(s)`. No golden moved:
  no committed fixture passes a global to a callee that writes it, or loops over a
  global its body writes, so there was nothing to regenerate.
- `cargo test --no-fail-fast -- --skip artifact_gate_all` → 202 test binaries, every one `test result: ok` — 5898 passed, 0 failed, 6 ignored.
- `scripts/test-accept.sh target/debug/mfb target/accept-actual` → `acceptance tests passed (1499 test(s) ran)`

Fallout found on the way: see bug-665's STATUS block.


`FOR EACH v IN g` over a module-level collection `g`, with a statement in the
body (or in a function the body calls) that reassigns `g`, crashes on the next
iteration (`Error: 7-701-0001 Allocation failed.`). The loop holds `g`'s block
pointer and count; the body's `g = …` lowers through `NirOp::StoreGlobal`, which
frees that block; the next step reads freed memory.

**The single correct behavior a fix produces:** the loop visits exactly the
elements `g` held at loop entry, and the body's writes to `g` take effect. The
program below prints `loop sees: one pad=3`, `loop sees: two pad=3`,
`loop sees: three pad=3`, `final len=6`.

References:

- `src/docs/spec/memory/05_collections.md:510-512` — a `FOR EACH` iterates the
  value at loop entry ("the snapshotting iterator").
- A plain local is already protected (the copying fallback leaks the old block
  instead of freeing it: `builder_control.rs:1309-1343`). A global is not.
- Found by the plan-142 research; sibling of bug-665.

## Failing Reproduction

```basic
IMPORT collections
IMPORT io

MUT g AS List OF String = ["one", "two", "three"]

SUB main()
  FOR EACH v IN g
    g = collections::append(g, v & "!")
    MUT pad AS List OF String = ["qqqqqqqqqqqqqqqqqqqqqqqq", "wwwwwwwwwwwwwwwwwwwwwwwww", "eeeeeeeeeeeeeeeeeeeeeeee"]
    io::print("loop sees: " & v & " pad=" & toString(len(pad)))
  NEXT
  io::print("final len=" & toString(len(g)))
END SUB
```

Release compiler from `b6a10efbc`, macOS aarch64:

- Observed: `loop sees: one pad=3` then `Error: 7-701-0001` / `Allocation failed.`
- Expected: the four lines above.

Contrast (works today): the same loop over a function-local `MUT g` completes
(the Assign fallback does not free a live iterable's block).

## Root Cause

- `lower_for_each` (`src/codegen/engine/control/builder_control.rs:2442`) lowers
  the iterable with `lower_value` (`:2454`) — a `Global` yields the live block
  pointer (`builder_values.rs:1764-1791`) — and stores it for the whole loop
  (`:2493-2497`), reading `count` once (`:2555-2559`).
- Only a `NirValue::Local` iterable is recorded in `for_each_iterable_locals`
  (`:2502`); there is no global equivalent.
- `NirOp::StoreGlobal` (`:1060`) frees the old block unconditionally
  (`:1097-1119`).

## Goal

- The reproduction prints the four expected lines.
- The same holds when the write to `g` happens in a function the body calls.

### Non-goals (must NOT change)

- The loop's snapshot semantics (it does not see elements appended during the loop).
- A loop over a global whose body cannot write it keeps borrowing (no new copy).
- Fixing by never freeing a global's old block is forbidden (bug-47 leak).

## Blast Radius

- `FOR EACH` over a bare global, body writes it directly — fixed here.
- Body writes it through a called function — fixed here (reachability).
- `FOR EACH v IN g.field` over a global record's field — fixed here (same shape).
- A builtin iterating `g` with a callback that writes `g` — bug-665.

## Fix Design

In `lower_for_each`, when the iterable reads a module-level global `g` and the
loop body can reach a `StoreGlobal` of `g` (the same walker bug-665 generalizes,
applied to the body's ops), lower the iterable owned: the loop walks its own copy
(one copy per loop entry), and frees it at loop exit. Rejected: a
`for_each_iterable_globals` list that makes `StoreGlobal` skip the free — that
leaks one block per write, and any write through a called function would need the
list at runtime.

## Phases

### Phase 1 — failing test + audit

- [x] Add `tests/runtime/rt_for_each_over_reassigned_global.rs` (+ `[[test]]`
      stanza): the reproduction, the called-function variant, the record-field
      variant. Confirm each fails today.

Acceptance: `cargo test --test rt_for_each_over_reassigned_global` → every case
fails as recorded (est. 3 min).
Commit: aa9f2a816

### Phase 2 — the fix

- [x] `lower_for_each`: owned iterable when the body can reach a write of the
      global; free it at loop exit (both the normal exit and `EXIT FOR`).

Acceptance: `cargo test --test rt_for_each_over_reassigned_global` → all pass (est. 3 min).
Commit: 9317e6eac

### Phase 3 — full validation

- [x] Regenerate goldens the new copy shifts (each diff must be a loop over a
      global its body writes); `cargo test`;
      `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

Acceptance: green; golden deltas are only such loops (none moved).
Commit: 8860a6bb4 (spec) — validation recorded in STATUS

## Validation Plan

- Regression test: `tests/runtime/rt_for_each_over_reassigned_global.rs`.
- Runtime proof: the reproduction prints the four lines.
- Doc sync: none expected (the spec already states snapshot semantics).
- Full suite: `cargo test`; `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- Depends on bug-665's generalized walker — land bug-665 first (recommended), or
  add the `StoreGlobal` leaf here and let bug-665 reuse it.

## Summary

Small fix, one lowering; the risk is again a reachability walk that must fail closed.
