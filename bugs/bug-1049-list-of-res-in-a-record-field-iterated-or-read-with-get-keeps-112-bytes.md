# bug-1049: a `List OF RES` held in a record field, iterated or read with `get(b.hs, 0)`, keeps 112 bytes per call

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting `live_bytes 0` at `N=10` and `N=20`, plus 300 iterations under `ulimit -n 64`

`LET b AS Bag = Bag[1, [a]]`, where `hs AS List OF RES fs::File` is a field of `Bag`, then
`FOR EACH h IN b.hs` and `collections::get(b.hs, 0)`, keeps 112 bytes per call. The descriptor
is closed.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb c9`: exit 0, stdout
`10`, `alloc_calls 62`, `free_calls 42`, `live_bytes 1120`. `N=20`: stdout `20`, 122 / 82 /
2240. `(ulimit -n 64; N=300 …)`: exit 0, stdout `300`, 1802 / 1202 / 33600.

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
TYPE Bag
  n AS Integer
  hs AS List OF RES fs::File
END TYPE
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  LET b AS Bag = Bag[1, [a]]
  MUT t AS Integer = 0
  FOR EACH h IN b.hs
    t = t + whichF("e", h)
  NEXT
  RETURN t + whichF("g", collections::get(b.hs, 0)) - 1
END FUNC
```

## What is known

1120 / 10 = 112 bytes per call, the size of an `fs::File` record. Each call makes two more
allocations than frees at `N=10` (62 - 42 = 20 over 10 calls) and the same at `N=20`
(122 - 82 = 40). Which of the iteration and the `get` keeps the record was not isolated, and
the kept block was not identified.

Found during batch 21.
