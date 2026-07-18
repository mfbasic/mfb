# bug-291: a returned collection declared *after* its resource silently gets no ownership float → returned closed handles + double-close

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (resource lifetime)

Status: Open
Regression Test: tests/rt-behavior (new) — returning a collection that holds a resource declared before it, in either declaration order, does not double-close

Escape analysis floats a resource's ownership to a returned collection only when
the collection is declared strictly *before* the resource (phase 1's `order >=
res_order` skip; phase 2's `depth >= res_depth` skip). So
`RES f = open(...); MUT xs = []; xs = append(xs, f); RETURN xs` yields `f → Local`:
`f` is closed at the function's exit while the returned list still carries it, and
the caller-side owned list later attempts a second close. §15.6 promises that a
collection escaping via RETURN moves ownership out to the caller — but nothing
(resolver, `ir::verify`) rejects the declaration order the analysis cannot honor, so
the program compiles and then double-closes at runtime.

The single correct behavior a fix produces: returning a collection that holds a
resource is either honored regardless of the relative declaration order of the
resource and the collection, or rejected at compile time with a clear diagnostic —
never silently miscompiled into a double-close.

References:

- `src/docs/spec/language/15_resource-management.md` §15.6 (RETURN moves ownership
  to caller).
- `src/docs/spec/architecture/23_escape-analysis.md` (phase-1 order rationale).
- Found during goal-06 review of `src/escape.rs`.

## Failing Reproduction

```
FUNC openThem() AS List OF File
  RES f = fs::openFile("/tmp/x", "w")
  MUT xs AS List OF File = []
  xs = collections::append(xs, f)
  RETURN xs
END FUNC
' caller: LET items = openThem() : io.print("opened=" & toString(collections::count(items)))
```

- Observed: at runtime `Error: 7-703-0004 Resource handle is already closed.` plus
  `Cleanup failure: 7-703-0004` (double close), exit 255.
- Expected: prints `opened=1`, exit 0.

Contrast (correct today): declaring `MUT xs` *before* `RES f` prints `opened=1` and
exits 0 — the only change needed.

## Root Cause

`src/escape.rs:314-361` (`solve`): phase 1 only floats to a returned collection
declared strictly before the resource; phase 2 requires a strictly-outer scope. A
returned collection declared after its resource matches neither, so the resource
stays `Local` and is closed at function exit despite escaping in the returned list.

## Goal

- Either: lowering creates the returned collection's owned-list early enough that
  the float is honored regardless of declaration order; or: the resolver / `ir::verify`
  emits a compile-time diagnostic when a resource is a member of a returned
  collection declared after it ("declare the collection before the resource").

### Non-goals (must NOT change)

- The correct behavior when the collection is declared before the resource.
- The §15.6 caller-ownership-transfer semantics.

## Blast Radius

- `escape.rs:solve` (+ possibly lowering or a new verify rule) — fixed here.
- Sibling: bug-290 (inline-TRAP insertion invisible) is a distinct escape root
  cause. Confirm no other "float requires ordering" assumptions elsewhere.

## Fix Design

Preferred long-term: make lowering allocate the owned-list for a returned collection
before the resource is produced, so the float is order-independent. Cheaper interim:
add a compile-time diagnostic for the unsupportable order (turns a silent
double-close into a clear error). Recommend the diagnostic first (unblocks
correctness/safety), then the lowering improvement. Rejected alternative: leaving it
silent — a double-close is a memory-safety-adjacent runtime failure.

## Phases

### Phase 1 — failing test
- [ ] rt-behavior test of the repro; confirm double-close today. Add the contrast
      (before-order) case as a guard.
### Phase 2 — the fix
- [ ] Diagnostic (interim) and/or order-independent owned-list allocation.
### Phase 3 — validation
- [ ] Full suite green; both declaration orders behave correctly (or the bad order
      is cleanly rejected).

## Validation Plan

- Regression: the rt-behavior repro + before-order contrast.
- Runtime proof: no double-close.
- Doc sync: 23_escape-analysis.md (and 15_resource-management.md if a diagnostic is
  added).

## Summary

Escape analysis honors the resource→returned-collection float only in one
declaration order and silently miscompiles the other into a double-close. Fixing the
ordering assumption (or rejecting it) closes a runtime memory-safety-adjacent
failure; the real engineering choice is diagnostic-vs-lowering.
