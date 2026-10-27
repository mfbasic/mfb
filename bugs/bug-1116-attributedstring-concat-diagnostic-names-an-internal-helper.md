# bug-1116: `AttributedString & String` reports a mismatch against the internal `#astrings_concat`, with no location

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: LOW
Class: Diagnostics quality

Status: Open
Regression Test: —

`LET b AS AttributedString = a & "y"` is correctly refused, but the message names
a compiler-internal helper and has no source location. The reverse order,
`"y" & a`, already gives the right diagnostic.

**The correct behavior a fix produces:** `a & "y"` reports
`TYPE_BINARY_OPERATOR_MISMATCH` at the `&`, with its location, exactly as
`"y" & a` does.

References:

- Found by plan-125-D Phase 6, finding `astrings-overview` 3.

## Failing Reproduction

```
IMPORT astrings

FUNC main() AS Integer
  LET a AS AttributedString = astrings::fromString("x")
  LET b AS AttributedString = a & "y"
  RETURN 0
END FUNC
```

- Observed: `error: TYPE_CALL_ARGUMENT_MISMATCH: Argument 2 for
  `#astrings_concat` has type String, expected AttributedString.` (no location).
- Expected: `TYPE_BINARY_OPERATOR_MISMATCH` at line 5.
