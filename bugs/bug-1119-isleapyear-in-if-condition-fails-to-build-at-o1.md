# bug-1119: `IF datetime::isLeapYear(year) THEN` fails to build at -O1 with an internal string-literal error

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: MED
Class: Compiler (optimizer / codegen)

Status: Open
Regression Test: —

The second example on `mfb man datetime isLeapYear` does not build at the
default optimization level. It builds and runs at `-O0`.

```
IMPORT datetime
IMPORT io

FUNC main() AS Nothing
  LET year AS Integer = 2024
  MUT days AS Integer = 28
  IF datetime::isLeapYear(year) THEN
    days = 29
  END IF
  io::print(toString(days))
END FUNC
```

`mfb build` (default `-O1`) →
`error: native code string literal 'builtins/datetime.mfb' has no data object while lowering if`.
`mfb build -O0` → builds.

**The correct behavior a fix produces:** the program builds at every `-O`
level and prints `29`; the page example stays as written.

References:

- Found by the plan-125 A–C recheck: `bash scripts/man-run-examples.sh datetime --run`
  → `failures: datetime::isLeapYear#2(build)`.
- Likely the inlined/folded `isLeapYear` body leaves a reference to the
  package's source-name string (used for error locations) whose data object was
  pruned; guess, not investigated (documentation task).
