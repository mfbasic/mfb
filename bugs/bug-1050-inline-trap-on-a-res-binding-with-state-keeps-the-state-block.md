# bug-1050: `RES v AS fs::File STATE Cur = openOne() TRAP …` keeps the 32-byte `STATE` block on every evaluation

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting `live_bytes 0` at `N=10` and `N=20`, plus 300 iterations under `ulimit -n 64`

A `RES` binding that carries a `STATE` clause and an inline `TRAP` keeps the 32-byte `STATE`
block each time it is evaluated. The descriptor is closed.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb s3`: exit 0, stdout
`10`, `alloc_calls 92`, `free_calls 82`, `live_bytes 320`. `N=20`: stdout `20`, 182 / 162 /
640. `(ulimit -n 64; N=300 …)`: exit 0, stdout `300`, 2702 / 2402 / 9600.

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
TYPE Cur
  pos AS Integer
  note AS String
END TYPE
FUNC once() AS Integer
  RES v AS fs::File STATE Cur = openOne() TRAP(e)
    RETURN 5
  END TRAP
  v.state.pos = 3
  v.state.note = "abc" & toString(v.state.pos)
  RETURN len(fs::readAll(v)) - 5 + v.state.pos - 3
END FUNC
```

## What is known

320 / 10 = 32 bytes per call; 640 / 20 and 9600 / 300 give the same. bug-1037's resolution
(completed) recorded this as a residue it did not fix: "The 32 B left per evaluation is the
`STATE` block, which the temp's drop does not free because the temp is typed without the
`STATE`." That is the reported cause and was not verified here. It also records the same
residue for `RES v AS Handle STATE Cur = mkH() TRAP`.

Found during batch 21.
