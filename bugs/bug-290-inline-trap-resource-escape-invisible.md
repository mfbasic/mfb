# bug-290: inline `TRAP` hides a resource insertion from escape analysis → resource closed while a collection still holds it (use-after-close)

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (resource lifetime)

Status: Open
Regression Test: tests/rt-behavior (new) — a resource inserted into a collection via a `... TRAP ... END TRAP` expression floats its ownership up correctly

Escape analysis (`scan_collection_expr`/`scan_element`) matches
`Identifier`/`ListLiteral`/`MapLiteral`/insertion `Call` and ignores everything
else. An inline `TRAP` — `Expression::Trapped { expression, handler, .. }` — is
neither unwrapped nor is its handler statement list walked, so
`xs = collections::insert(xs, 0, f) TRAP … END TRAP` produces no ownership routing.
The resource stays `ResOwner::Local` and is closed at its own (e.g. loop-body)
scope even though the outer collection retains the borrow — violating the §15.6
float-up contract. The program compiles with zero diagnostics and then fails at
runtime with a use-after-close.

The single correct behavior a fix produces: inserting a resource into a collection
through an inline-`TRAP` expression floats the resource's owning scope up to the
collection's, exactly as the non-TRAP insertion does — no premature close.

References:

- `src/docs/spec/language/15_resource-management.md` §15.6 (collection float-up).
- `src/docs/spec/architecture/23_escape-analysis.md` (documents the `_ -> ignore`
  scan — codifies the hole).
- Found during goal-06 review of `src/escape.rs`.

## Failing Reproduction

```
MUT handles AS List OF File = []
LET n AS Integer = 0
WHILE n < 3
  RES f = fs::openFile("/tmp/x", "w")
  handles = collections::insert(handles, 0, f) TRAP(e) RECOVER handles END TRAP
  n = n + 1
END WHILE
io.print("opened=" & toString(collections::count(handles)))
```

- Observed: compiles with no diagnostics; at runtime `Error: 7-703-0004 Resource
  handle is already closed.`, exit 255.
- Expected: prints `opened=3` (identical to the same program without the `TRAP`).

## Root Cause

`src/escape.rs:208-266` (`scan_collection_expr`/`scan_element`): no arm for
`Expression::Trapped`, so the inner insertion call is never seen; and
`walk_statement` never descends into `Expression::Trapped.handler` or
`Statement::Recover`, so declarations/assignments inside inline handlers are
invisible. The resource keeps `ResOwner::Local` and is closed at its own scope.

## Goal

- `scan_collection_expr`/`scan_element` unwrap `Expression::Trapped` to its inner
  expression (and scan `RECOVER` values in the handler as flowing to the same
  target).
- `walk_statement` walks `Trapped.handler` statements.

### Non-goals (must NOT change)

- The float-up semantics for non-TRAP insertions (already correct).
- Error-handling semantics of inline TRAP.

## Blast Radius

- `escape.rs` scan/walk — fixed here.
- Sibling: bug-291 (returned-collection declaration-order float gap) is a distinct
  escape-analysis root cause. The audit collector has a parallel inline-TRAP
  recursion gap (bug-283 A4) — different module, same shape.

## Fix Design

Add a `Trapped` unwrap in the collection-expression scanners and walk the handler
statements in `walk_statement`, so an insertion (and any handler-side acquisition)
routes ownership like the plain form. Rejected alternative: forbidding inline TRAP
on insertion expressions — a real usability regression.

## Phases

### Phase 1 — failing test
- [ ] rt-behavior test of the repro; confirm the use-after-close today.
### Phase 2 — the fix
- [ ] Unwrap Trapped in scanners + walk handler in `walk_statement`.
### Phase 3 — validation
- [ ] Full suite green (resource/thread/collection tests especially); repro prints
      `opened=3`.

## Validation Plan

- Regression: the rt-behavior repro + a handler-side acquisition variant.
- Runtime proof: no premature close.
- Doc sync: update 23_escape-analysis.md to describe Trapped handling.

## Summary

Escape analysis is blind to inline-TRAP-wrapped insertions, closing a still-borrowed
resource. Unwrapping Trapped in the scanners fixes it; the risk is covering the
handler-side acquisition case too.
