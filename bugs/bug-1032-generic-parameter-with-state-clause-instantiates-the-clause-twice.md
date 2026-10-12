# bug-1032: a generic `FUNC gen OF T(x AS T STATE Cur)` called with a `fs::File STATE Cur` argument is rejected; the instantiation carries the clause twice

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: LOW-MEDIUM
Class: Correctness (valid program rejected, if the form is legal)

Status: Open
Regression Test: (proposed) depends on the owner decision below

OPEN OWNER DECISION: whether `T STATE S` on a generic type parameter should be legal at all. If it is legal, the
program below must build and print the state's `n`. If it is not, the declaration `x AS T STATE Cur` must get a
diagnostic at line 7 and none of the three errors below.

When `T` is bound to `fs::File` from a `fs::File STATE Cur` argument, the instantiation `gen$fs$File$STATE$Cur` has a
parameter type `Cur STATE Cur`: the clause is applied once from the argument and once from the declaration.

## Failing Reproduction

Measured on 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at 3fa35d543, macOS aarch64),
`cd /tmp/bs19-found/a5 && /tmp/main-dbg-target/release/mfb build -ast -ir`:

```
IMPORT io
IMPORT fs
TYPE Cur
  n AS Integer
END TYPE

FUNC gen OF T(x AS T STATE Cur) AS Integer
  RETURN x.state.n
END FUNC
FUNC main AS Integer
  RES f AS fs::File STATE Cur = fs::createTempFile()
  io::print(toString(gen(f)))
  RETURN 0
END FUNC
```

Three errors:

- `main.mfb:8 TYPE_UNKNOWN_VALUE`: `RETURN value does not have a known type.`
- `main.mfb:12 TYPE_STATE_MISMATCH`: `Argument 1 for gen$fs$File$STATE$Cur is declared STATE Cur STATE Cur but carries
  STATE Cur; a parameter observes a resource's state, it cannot re-type it.`
- `main.mfb:7 TYPE_STATE_INVALID`: `Parameter x STATE type Cur STATE Cur must be a copyable, defaultable data type.`

## What is known

The doubled `STATE Cur STATE Cur` in the messages shows where the clause is duplicated. The place in template
instantiation that appends it was not traced (guess: substituting `T` with a type that already carries `STATE`, then
re-applying the parameter's own clause). The bug-998 agent reported the same message for a variant of this program.

Found by the bug-998 agent while fixing bug-998 (batch 19).
