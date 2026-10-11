# bug-1041: diagnostics on a file's `PRIVATE`+`PUBLIC` overload set: a no-match call prints `#<hash>$r`, and `PRIVATE_SHADOWS_PUBLIC` fires on a legal overload

Last updated: 2026-10-10
Effort: small (<1h)
Severity: LOW
Class: Diagnostics

Status: Open
Regression Test: (proposed) a syntax fixture for each item below

Since bug-912 a `PRIVATE FUNC r` beside a `PUBLIC FUNC r` with a different signature in the
same file is a working overload set. Two diagnostics have not caught up.

1. A call that matches neither member names the file-scoped symbol.
2. The pair still draws `PRIVATE_SHADOWS_PUBLIC` ("PRIVATE declaration shadows a PUBLIC
   declaration of the same name within its file"), although nothing is shadowed.

**Correct behavior:** (1) the message names `r`. (2) NEEDS AN OWNER DECISION: drop the
warning for a same-file `FUNC`/`FUNC` pair with different signatures, or keep it.

## Failing Reproduction

Measured 2026-10-10.

Item 1, main's compiler (`/tmp/main-dbg-target/release/mfb`), `/tmp/b912/probe/nomatch`:
```
IMPORT io
FUNC r(x AS Integer) AS String
  RETURN "int"
END FUNC
PRIVATE FUNC r(x AS String) AS String
  RETURN "str"
END FUNC
FUNC main AS Integer
  io::print(r(TRUE))
  RETURN 0
END FUNC
```
- Observed: `./src/main.mfb:9 error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: … Callable `#c2515768f1aa7dc6$r` is not a top-level function.`
- Expected: a message that names `r` (its wording is bug-836's).

Item 2, the batch-20 compiler (`/tmp/bs20-b-target/release/mfb`), a file with
`FUNC r(x AS Integer) AS String` calling `r("s")` and `PRIVATE FUNC r(x AS String) AS String`;
`main` in another file calls `r(1)`:
- Observed: builds, prints `intstr`, and warns `src/a.mfb:5 warn[2-201-0017 PRIVATE_SHADOWS_PUBLIC]`.

## What is known

Item 1 is the same family as bug-836 (no-match wording) and bug-881 (mangled name in
`SYMBOL_DUPLICATE_TOP_LEVEL`); fix them together. Item 2: the unit test
`shadow_warning_reports_the_line_of_each_decl_kind` pins the warning.

Found by the bug-912 agent and the coordinator (batch 20).
