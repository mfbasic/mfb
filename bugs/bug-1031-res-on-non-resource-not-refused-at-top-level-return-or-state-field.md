# bug-1031: `RES` on a non-resource type is not refused at the top level, on a return type, or on a record field that also carries `STATE`

Last updated: 2026-10-10
Effort: small (<1h)
Severity: LOW
Class: Missing diagnostic

Status: Open
Regression Test: (proposed) a diagnostics test per program below, expecting `TYPE_RES_REQUIRES_RESOURCE` at the `RES` line

A local binding `RES h AS Integer = 1` is refused with `TYPE_RES_REQUIRES_RESOURCE`. The same `RES` on a non-resource
type builds without a diagnostic at the top level, on a function return (`FUNC mk() AS RES Integer`), and on a record
field that also carries `STATE`.

**Correct behavior:** each of these gets `TYPE_RES_REQUIRES_RESOURCE`, as a local binding already does.

## Failing Reproduction

Measured on 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at 3fa35d543, macOS aarch64),
`cd <dir> && /tmp/main-dbg-target/release/mfb build -ast -ir`. Both build and write the `.ast` and `.ir` files with no
diagnostic.

Top level and return (`/tmp/bs19-found/a3`):

```
IMPORT io
IMPORT fs
TYPE Cur
  n AS Integer
END TYPE

RES h AS Integer = 1
FUNC mk() AS RES Integer
  RETURN 1
END FUNC
FUNC main AS Integer
  io::print(toString(h + mk()))
  RETURN 0
END FUNC
```

Record field with `STATE` (`/tmp/bs19-found/a4`):

```
IMPORT io
IMPORT fs
TYPE Cur
  n AS Integer
END TYPE

TYPE Box
  field AS RES Integer STATE Cur
END TYPE
FUNC main AS Integer
  RETURN 0
END FUNC
```

## What is known

Not traced. After bug-998 lands, the `a4` line gets `TYPE_STATE_REQUIRES_RESOURCE` (reported by the bug-998 agent, not
verified here), which would mask the missing `TYPE_RES_REQUIRES_RESOURCE` on it; the field still needs the `RES`
check on its own.

Found by the bug-998 and bug-1013 agents while fixing those bugs (batch 19).
