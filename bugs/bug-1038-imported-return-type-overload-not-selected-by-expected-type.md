# bug-1038: an imported return-type overload set cannot be selected by the expected type

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: (proposed) an rt-behavior fixture with the package and importer below, printing `text TRUE`

A package exports two functions that differ only in return type. Inside the package an
annotated binding selects one. From an importer the same call is `TYPE_OVERLOAD_AMBIGUOUS`.

**Correct behavior:** the binding's annotation selects the overload, as it does for a
same-project call; the importer prints `text TRUE`.

## Failing Reproduction

Measured 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at
a236fdd44), `mfb build -q .` in `/tmp/b912/rt`.

Package `rp`, `src/lib.mfb`:
```
EXPORT FUNC pick(x AS Integer) AS String
  RETURN "text"
END FUNC

EXPORT FUNC pick(x AS Integer) AS Boolean
  RETURN x > 0
END FUNC
```
Importer `src/main.mfb`:
```
IMPORT io
IMPORT rp

FUNC main AS Integer
  LET s AS String = rp::pick(1)
  LET b AS Boolean = rp::pick(1)
  io::print(s & " " & toString(b))
  RETURN 0
END FUNC
```
- Observed: `./src/main.mfb:5 error[2-203-0101 TYPE_OVERLOAD_AMBIGUOUS]: … Call to `rp.pick` matches 2 imported overloads; annotate the argument types …` (and the same at line 6).
- Expected: builds; prints `text TRUE`.

## What is known

Not traced. Related: bug-870 (a return-type overload loses its expected type under an
inline TRAP) and bug-914 (imported-overload resolution keyed by alias).

Found by the bug-912 agent (batch 20).
