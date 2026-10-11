# bug-1037: a resource-union binding with an inline `TRAP` keeps 128 bytes every time it is evaluated

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting `live_bytes 0`

`RES v AS Handle = mkH() TRAP(e) … END TRAP`, where `Handle` is a resource union and no
collection is involved, leaves 128 bytes live per evaluation.

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at
a236fdd44): `mfb build -q --debug`, run with `N=10`. Result: stdout `10`, `alloc_calls 72`,
`free_calls 52`, `live_bytes 1280`. The batch-20 compiler gives the same numbers. The
bug-1026 agent measured 5,120 at `N=40` (not re-run).

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

FUNC main AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    t = t + once()
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC

FUNC mkH AS RES Handle
  RETURN openOne()
END FUNC

FUNC once() AS Integer
  RES v AS Handle = mkH() TRAP(e)
    RETURN 5
  END TRAP
  RETURN which("v", v)
END FUNC
```

## What is known

128 bytes is a 16-byte union box plus a 112-byte record. Guess (the bug-1026 agent's, not
traced): the inline-`TRAP` temp is bound to a closed default first, and no `Assign` path
drops that default when the call's result replaces it.

Found by the bug-1026 agent (batch 20).
