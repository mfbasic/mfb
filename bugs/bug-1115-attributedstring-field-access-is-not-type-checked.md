# bug-1115: field access on an `AttributedString` is not type-checked — `a.text` compiles and `a.foo` crashes the lowering

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: MEDIUM
Class: Type checker

Status: Open
Regression Test: —

`AttributedString` is opaque: `mfb man astrings` says `a.text` does not
compile. It does. `io::print(a.text)` builds and prints the visible text, and an
unknown field, `a.foo`, passes the checker and then fails in native lowering
with an internal message instead of a diagnostic.

**The correct behavior a fix produces:** every field access on an
`AttributedString` is rejected by the type checker with a located diagnostic
(the one a field access on any other non-record value gets), and nothing
reaches the lowering.

References:

- Found by plan-125-D Phase 6, finding `astrings-overview` 1.

## Failing Reproduction

```
IMPORT astrings
IMPORT io

FUNC main() AS Integer
  LET a AS AttributedString = astrings::fromString("xy")
  io::print(a.text)
  RETURN 0
END FUNC
```

- Observed: builds; prints `xy`. With `a.foo`: `error: native code record
  'AttributedString' has no field 'foo' while lowering eval call io.print`.
- Expected: a type-checker error at the field access, in both cases.
