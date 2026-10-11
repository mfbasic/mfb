# bug-1047: `collections::get` under an inline `TRAP` written into an argument keeps 96 bytes per call

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting `live_bytes 0` at `N=10` and `N=20`, plus 300 iterations under `ulimit -n 64`

`whichF("t", collections::get(hs, i)) TRAP(e) … END TRAP`, where `hs` is a
`List OF RES fs::File`, keeps 96 bytes per call of the enclosing function. The descriptors are
closed.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb c5`: exit 0, stdout
`10`, `alloc_calls 82`, `free_calls 72`, `live_bytes 960`. `N=20`: stdout `20`, 162 / 142 /
1920. `(ulimit -n 64; N=300 …)`: exit 0, stdout `300`, 2402 / 2102 / 28800.

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
FUNC whichF(label AS String, f AS fs::File) AS Integer
  LET text AS String = fs::readAll(f)
  RETURN 1
END FUNC
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  LET hs AS List OF RES fs::File = [a]
  MUT r AS Integer = 0
  r = whichF("t", collections::get(hs, 5)) TRAP(e)
    RECOVER 7
  END TRAP
  MUT q AS Integer = 0
  q = whichF("t", collections::get(hs, 0)) TRAP(e)
    RECOVER 7
  END TRAP
  RETURN r + q - 7
END FUNC
```

## What is known

960 / 10 = 96 bytes per call at `N=10`, 1920 / 20 = 96 at `N=20`, 28800 / 300 = 96 at
`N=300`. Each call evaluates two trapped `get` arguments: `get(hs, 5)` (out of range, takes
the `RECOVER`) and `get(hs, 0)` (succeeds). Which of the two keeps the bytes was not
isolated, and the kept block was not identified.

Found during batch 21.
