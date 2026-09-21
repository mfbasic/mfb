# bug-666: `FOR EACH` over a global the body reassigns reads freed memory

Last updated: 2026-09-20
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: tests/runtime/rt_for_each_over_reassigned_global.rs (to be added, Phase 1)

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

- [ ] Add `tests/runtime/rt_for_each_over_reassigned_global.rs` (+ `[[test]]`
      stanza): the reproduction, the called-function variant, the record-field
      variant. Confirm each fails today.

Acceptance: `cargo test --test rt_for_each_over_reassigned_global` → every case
fails as recorded (est. 3 min).
Commit: —

### Phase 2 — the fix

- [ ] `lower_for_each`: owned iterable when the body can reach a write of the
      global; free it at loop exit (both the normal exit and `EXIT FOR`).

Acceptance: `cargo test --test rt_for_each_over_reassigned_global` → all pass (est. 3 min).
Commit: —

### Phase 3 — full validation

- [ ] Regenerate goldens the new copy shifts (each diff must be a loop over a
      global its body writes); `cargo test`;
      `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

Acceptance: green; golden deltas are only such loops.
Commit: —

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
