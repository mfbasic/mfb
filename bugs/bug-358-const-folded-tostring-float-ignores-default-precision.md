# bug-358: constant-folded `toString(Float)` ignores the documented default precision, so a literal and a runtime value of the same Float print differently

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (compile-time / runtime divergence)

Status: Open
Regression Test: tests/rt-behavior (new) — `toString(<float literal>)` and `toString(<same value at runtime>)` produce identical text

`toString(value AS Float, precision AS Byte = 2)` is documented to default to **two**
digits after the decimal point, and that is exactly what the runtime helper does.
But when the argument is a compile-time constant the fold produces the *full*
shortest representation instead, so the same value renders two different ways
depending on whether the compiler could see it.

The single correct behavior a fix produces: `toString(f)` yields the same text for a
given Float whether or not the argument was foldable — whichever default the language
settles on.

References:

- `src/docs/man/builtins/general/toString.md` — `toString(value AS Float, precision AS
  Byte = 2) AS String`, "defaults to `2`".
- `src/target/shared/code/builder_strings.rs:751` — the runtime default,
  `move_immediate(&scratch8, "Byte", "2")`.
- Found while fixing bug-304 (`json::stringify` precision loss); it is why the first
  attempt at that fix failed its own round-trip check.

## Failing Reproduction

```basic
IMPORT io

FUNC identity(x AS Float) AS Float
  RETURN x
END FUNC

FUNC main AS Integer
  io::print("literal =" & toString(3.141592653589793))
  io::print("runtime =" & toString(identity(3.141592653589793)))
  RETURN 0
END FUNC
```

- Observed:
  ```
  literal =3.141592653589793
  runtime =3.14
  ```
- Expected: both identical.

Further runtime cases confirming the default really is two places (these are correct
per the man page): `toString(identity(0.1))` → `0.10`, `toString(identity(1.0/3.0))`
→ `0.33`, `toString(identity(2.5))` → `2.50`.

## Root Cause

Two independent renderers implement `toString(Float)`:

- the runtime helper, which reads the precision slot — defaulted to `2` at
  `builder_strings.rs:751` when no precision argument is supplied;
- the constant folder, which formats the literal without consulting that default.

Nothing keeps the two in agreement, so the divergence is invisible until the same
value takes both paths.

## Goal

- One default, honored by both paths.

### Non-goals (must NOT change)

- The explicit two-argument form `toString(f, nb)`, which is unambiguous and correct
  on both paths.
- `Money`, whose 2-place default is semantically right for currency.

## Blast Radius

- The constant folder's `toString(Float)` arm, or the runtime default — whichever the
  resolution picks.
- **Deciding which way to converge is the real work here, and it is a language
  decision, not a mechanical one.** Two decimal places is the documented default but
  is lossy and surprising for a general-purpose float (`0.000000000000123` prints as
  `0.00`); shortest-round-trip is what most languages do and what `json::stringify`
  needs, but changing the default would move every `.run` golden that prints a Float
  and is a breaking change to documented behavior. A third option is to keep the
  documented default and fix only the folder, which is the smallest change and
  restores consistency without a language change.
- `Fixed` shares the same defaulting code path and should be checked for the same
  divergence.

## Fix Design

Recommend converging the folder onto the documented runtime default first (smallest
change, restores consistency, no golden churn beyond folded call sites), and treating
"should the default be shortest-round-trip?" as a separate language question. bug-304
does not depend on the answer: it now searches for the shortest round-tripping
precision explicitly rather than relying on any default.

## Phases

### Phase 1 — failing test
- [ ] rt-behavior fixture printing a literal and a runtime Float of the same value.
### Phase 2 — the fix
- [ ] Make the folder honor the documented default (or, if the default changes,
      update the man page, the runtime, and every affected golden together).
### Phase 3 — validation
- [ ] Full suite green; check `Fixed` for the same divergence.

## Validation Plan

- Regression: literal-vs-runtime equality for several Floats.
- Doc sync: `toString.md` if the default changes.

## Summary

Two renderers, one documented default, no agreement between them. Low-risk to fix in
the folder; the broader question of what the default *should* be is worth deciding
deliberately rather than by accident.
