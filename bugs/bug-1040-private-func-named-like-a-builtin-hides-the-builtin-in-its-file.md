# bug-1040: a `PRIVATE FUNC` named like a built-in (`len`) hides the built-in in its file, and the error prints the mangled name

Last updated: 2026-10-10
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: (proposed) an rt-behavior fixture with the program below, printing `43`

A `PUBLIC` function that overloads a built-in name fills a gap: the built-in still answers
the argument types the user function does not take. A `PRIVATE` one does not: inside its
file every call of that name goes to the file-scoped function.

**Correct behavior:** the same gap-fill rule as for a `PUBLIC` overload; the program prints
`43`. At the least, no `#<hash>$len` in a diagnostic.

## Failing Reproduction

Measured 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at
a236fdd44), `mfb build -q .` in `/tmp/b912/pb`:

```
IMPORT io
TYPE Tag
  id AS Integer
END TYPE
PRIVATE FUNC len(t AS Tag) AS Integer
  RETURN t.id
END FUNC
FUNC main AS Integer
  io::print(toString(len(Tag[4])) & toString(len("abc")))
  RETURN 0
END FUNC
```
- Observed: `./src/main.mfb:9 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: … Argument 1 for `#c2515768f1aa7dc6$len` has type String, expected Tag.`
- Expected: prints `43`.

## What is known

`src/ast/scope_privates.rs:scope_privates` rewrites every in-file reference to `len` to the
file-scoped name before types are known, so the built-in is no longer a candidate. Whether
the `PUBLIC` case was compared on the same program was not checked by the coordinator (the
bug-912 agent's statement).

Found by the bug-912 agent (batch 20).
