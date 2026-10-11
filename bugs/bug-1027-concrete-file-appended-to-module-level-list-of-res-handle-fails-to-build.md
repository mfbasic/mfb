# bug-1027: a concrete `fs::File` binding appended to a module-level `List OF RES Handle` fails to build

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (internal error where the program should build, or get a diagnostic)

Status: Open
Regression Test: (proposed) the program below builds and prints `0`

`RES a AS fs::File = openOne()` followed by `gHs = collections::append(gHs, a)`, with `MUT gHs AS List OF RES Handle`
declared at module level, is refused with an internal-style error naming bug-806 instead of building. Binding it as
`RES a AS Handle` builds and runs.

**Correct behavior:** the program builds and runs (a `fs::File` is a member of `Handle`, so it can be stored into a
`List OF RES Handle`), or, if it is meant to be refused, it gets a source diagnostic at the line, not this message.

## Failing Reproduction

Measured on 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at 3fa35d543, macOS aarch64):
`N=10 /tmp/bs19b/bin/probe.sh /tmp/main-dbg-target/release/mfb q_global_bug` prints

```
BUILD FAIL
error: a resource of type 'fs.File' is stored into a module-level binding, but no module-level binding's type reaches that resource (bug-806) while lowering bind a AS fs.File
```

```
IMPORT io
IMPORT os
IMPORT fs
IMPORT tcp
IMPORT collections

UNION Handle
  fs::File
  tcp::Socket
END UNION

TYPE HB
  h AS RES Handle
END TYPE

MUT gHs AS List OF RES Handle = []
MUT gBs AS List OF HB = []

FUNC openOne AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC

FUNC which(label AS String, h AS Handle) AS Integer
  MATCH h
    CASE fs::File(f)
      LET text AS String = fs::readAll(f)
      RETURN 1
    CASE tcp::Socket(s)
      RETURN 2
  END MATCH
  RETURN 0
END FUNC

FUNC once() AS Integer
  RES a AS fs::File = openOne()
  gHs = collections::append(gHs, a)
  gBs = collections::append(gBs, HB[a])
  RETURN which("g", collections::get(gHs, len(gHs) - 1)) + which("b", collections::get(gBs, len(gBs) - 1).h) - 1
END FUNC

FUNC main AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    t = t + once()
  NEXT
  FOR EACH b IN gBs
    t = t + which("z", b.h) - 1
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC
```

## What is known

The message comes from the check that a resource stored into a module-level binding must be reachable from some
module-level binding's type. The module-level types here are `List OF RES Handle` and `List OF HB`; `fs.File` is a
member of `Handle`, so the check apparently does not look through the union (guess; not traced). Changing the line to
`RES a AS Handle = openOne()` (the union typed binding) makes the program build and run.

Found by the bug-1012 agent while fixing bug-1012 (batch 19).
