# bug-1129: vector Fixed/Integer overloads don't declare ErrOverflow; math's Constants table shows Float values for the Fixed twins

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: LOW
Class: Registry (declared errors / rendered values)

Status: Open
Regression Test: —

Two registry-data defects that make `mfb man` render the wrong table:

1. **vector.** The prose of `dot`, `scale`, `cross`, `distance`, `length`,
   `perpendicular` and `reflect` says their Fixed/Integer overloads fail with
   `ErrOverflow` when a product leaves the element type's range, but those
   overloads declare no errors, so the pages render no Errors table.
2. **math.** The Constants table renders `math::piFixed │ Fixed │
   3.141592653589793` — the Float literal — for every `*Fixed` constant, while the
   value actually held is the nearest Fixed (`3.141592653701081`). The overview
   now says the table gives the Float value each pair is derived from.

**The correct behavior a fix produces:** each vector overload's `errors` lists
`ErrOverflow` where it can raise it (and the declared-errors guard covers them);
the math Constants table shows each `*Fixed` constant's real value.

References:

- Found by the plan-125 B recheck (`vector` finding 1, `math` finding 1).
