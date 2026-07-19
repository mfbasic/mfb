# bug-290: inline `TRAP` hides a resource insertion from escape analysis → resource closed while a collection still holds it (use-after-close)

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (resource lifetime)

Status: Fixed
Regression Test: tests/rt-behavior/resources/inline-trap-collection-escape-rt — both trap arms float ownership up correctly

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

## Resolution

`scan_collection_expr` gained an `Expression::Trapped` arm that scans **both** of
the trap's arms into the same target: the guarded expression, and each `RECOVER`
value in the handler. The report's suggested `walk_statement` change turned out to
be unnecessary — routing is recorded from the expression scanner, and reaching the
handler through the `Trapped` arm covers the handler-side case without walking
statements separately.

### The reproduction needed correcting before it proved anything

The report's repro as written does not compile today (`List OF File` is rejected:
a resource element must be `List OF RES File`), and once corrected it **passed** —
printing `opened=3` exactly as the report says a correct build should. That is a
false negative, not a fixed bug: counting a list never touches its handles. The
resources really were already closed. The repro only becomes a repro when it
*uses* a retained handle, at which point it dies with
`7-703-0004 Resource handle is already closed` as described. The fixture therefore
writes through a retained handle in each case, and the control — the identical
program with the `TRAP` removed — passes, isolating the trap as the sole cause.

### Both arms are independently load-bearing

Established by bisecting the fix against the fixture rather than by argument:

- with no `Trapped` arm at all, the guarded-expression case dies;
- with only the guarded-expression unwrap and no `RECOVER` scan, that case passes
  but the handler-acquisition case still dies;
- with both, both pass.

So the handler scan is not defensive padding — a resource acquired outside the trap
and inserted by the handler's `RECOVER` value is stranded by exactly the same hole,
and would have survived a fix that only unwrapped the guarded expression.

Spec `architecture/23_escape-analysis.md` — which codified this hole as
`_ -> ignore` — now documents the `Trapped` arm and records why covering one arm is
insufficient.

Full `cargo test` green; acceptance 1002/1002.
