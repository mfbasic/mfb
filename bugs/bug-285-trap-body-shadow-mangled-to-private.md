# bug-285: function-level TRAP body reference to a body local is mis-mangled to a file `PRIVATE` when names collide (silent wrong value)

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/ (new) — a function-level TRAP returning a body local returns the local, not a same-named file PRIVATE

`rewrite_item_refs` rewrites a function-level TRAP body with `trap_locals` = params
+ trap binding only; body-declared `LET`/`MUT` names are omitted. But a
function-level TRAP body *does* see function locals. So when a body local shadows a
same-named file `PRIVATE`, the trap-body reference is wrongly rewritten to the
mangled private name — the trap returns the file-private's value instead of the
local's. Adding an unrelated file-scope `PRIVATE` silently changes an existing
function's behavior.

The single correct behavior a fix produces: a function-level TRAP body reference to
a body local resolves to that local, regardless of any same-named file `PRIVATE`.

References:

- Found during goal-06 review of `src/scope_privates.rs`.

## Failing Reproduction

```
PRIVATE LET x AS Integer = 42
FUNC main() AS Integer
  LET x AS Integer = 7
  ' ... something that can trap ...
  TRAP(e)
    RETURN x
  END TRAP
END FUNC
```

- Observed: returns **42** (the file private). Deleting the `PRIVATE LET x` line
  makes the identical function return **7**.
- Expected: returns **7** (the body local shadows the private).

Contrast: inline `TRAP` expressions are unaffected (they receive the accumulated
statement scope).

## Root Cause

`src/scope_privates.rs:174-178` (`rewrite_item_refs`, Function trap arm):
`trap_locals` is built from params + the trap binding but not from the function
body's declared `LET`/`MUT` names, so a body local that shadows a file private is
not shielded from the private-name rewrite.

## Goal

- Collect the function body's declared binding names (per resolver visibility
  rules) into `trap_locals` before rewriting the trap block.

### Non-goals (must NOT change)

- The file-private mangling scheme.
- Inline-TRAP handling (already correct).

## Blast Radius

- `rewrite_item_refs` Function-trap arm — fixed here.
- The function-level `parse_trap` body handling and inline-trap paths — verify they
  already see body locals (inline does; confirm during fix).

## Fix Design

Walk the function body's statements collecting declared binding names (matching the
resolver's visibility rules for what a trap body can see), add them to
`trap_locals`. Rejected alternative: unconditionally skipping private rewrite inside
trap bodies — wrong, a trap body legitimately references file privates that aren't
shadowed.

## Phases

### Phase 1 — failing test
- [ ] Test the repro; confirm it returns 42 today.
### Phase 2 — the fix
- [ ] Extend `trap_locals` with body-declared names.
### Phase 3 — validation
- [ ] Full suite green; no golden drift except the intended.

## Validation Plan

- Regression: the repro as a runtime test.
- Runtime proof: repro returns 7.
- Doc sync: none.

## Summary

A scoping omission lets an unrelated file-private silently hijack a shadowed trap
local. Extending `trap_locals` with body-declared names fixes it; risk is matching
the resolver's exact visibility rules.
