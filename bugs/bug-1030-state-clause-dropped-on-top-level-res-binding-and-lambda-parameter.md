# bug-1030: the `STATE` clause of a top-level `RES` binding and of a `LAMBDA` parameter is dropped, so `.state` on it does not type

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (valid program rejected)

Status: Open
Regression Test: (proposed) both programs below build and run; `g.state.n` and `x.state.n` type as Integer

`RES g AS fs::File STATE Cur = ...` at the top level and `LAMBDA(x AS fs::File STATE Cur) -> ...` lose their `STATE`
clause, so reading `.state` through them is rejected as if the resource had no state.

**Correct behavior:** both programs build; the `STATE Cur` is part of the binding's / parameter's type.

## Failing Reproduction

Measured on 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at 3fa35d543, macOS aarch64),
`cd <dir> && /tmp/main-dbg-target/release/mfb build -ast -ir`.

Top level (`/tmp/bs19-found/a1`):

```
IMPORT io
IMPORT fs
TYPE Cur
  n AS Integer
END TYPE

RES g AS fs::File STATE Cur = fs::createTempFile()
FUNC main AS Integer
  io::print(toString(g.state.n))
  RETURN 0
END FUNC
```

Errors: `main.mfb:9 TYPE_CALL_ARGUMENT_MISMATCH` (`Call to toString has argument type(s) (Unknown), expected Integer,
...`) and `main.mfb:9 TYPE_STATE_INVALID` (`fs.File here has no STATE to read; declare the resource with STATE T. Bare
means "no state" on an owner, and "opaque" on a resource parameter — a bare parameter cannot read the state its caller
attached.`).

Lambda parameter (`/tmp/bs19-found/a2`):

```
IMPORT io
IMPORT fs
TYPE Cur
  n AS Integer
END TYPE

FUNC main AS Integer
  LET j = LAMBDA(x AS fs::File STATE Cur) -> x.state.n
  RES f AS fs::File STATE Cur = fs::createTempFile()
  io::print(toString(j(f)))
  RETURN 0
END FUNC
```

Errors: `main.mfb:8 TYPE_UNKNOWN_VALUE` (`Initializer for binding j does not have a known type.`),
`main.mfb:10 TYPE_CALL_ARGUMENT_MISMATCH`, and `main.mfb:8 TYPE_STATE_INVALID` (same message as above).

## What is known

Stated by the bug-1013 agent, not traced by the coordinator and not verified here: `lower_binding` never reads
`HirTopLevelBinding::state_type`, and a lambda parameter is lowered from its `type_` alone, so the `STATE` clause
never reaches the lowered form.

Found by the bug-1013 agent while fixing bug-1013 (batch 19).
