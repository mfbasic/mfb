# bug-1043: a resource returned by a call and used unbound keeps its 112-byte record on every call

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting `live_bytes 0` at `N=10` and `N=20`, plus 300 iterations under `ulimit -n 64`

`len(fs::readAll(openOne()))`, where `openOne` returns a single `RES fs::File`, closes the
descriptor but keeps the 112-byte resource record that `openOne` produced, on every call.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb t_res`: exit 0, stdout `n=60`,
`alloc_calls 33`, `free_calls 23`, `live_bytes 1120`. `N=20`: stdout `n=120`, 63 / 43 / 2240.
`(ulimit -n 64; N=300 …)`: exit 0, stdout `n=1800`, 903 / 603 / 33600. The descriptors are
closed (the 300-iteration run stays under a 64-descriptor limit); only the memory is kept.

```
IMPORT fs
IMPORT io
IMPORT os
IMPORT collections

TYPE HB
  h AS RES fs::File
END TYPE

FUNC openOne() AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC

FUNC mkHB() AS HB
  RETURN HB[openOne()]
END FUNC

FUNC mkStrs() AS List OF String
  RETURN ["a" & toString(1), "b"]
END FUNC

FUNC main AS Integer
  MUT n AS Integer = 0
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    n = n + once()
  NEXT
  io::print("n=" & toString(n))
  RETURN 0
END FUNC

FUNC once() AS Integer
  RETURN len(fs::readAll(openOne()))
END FUNC
```

## What is known

The live bytes grow by 1120 per 10 calls, 2240 per 20 and 33600 per 300: 112 bytes per call,
the size of the `fs::File` record. Ten calls leave ten more allocations than frees
(33 - 23, and 63 - 43 = 20 at N=20).

bug-999 records that bug-842 made a resource produced inline in an expression a
statement-scoped temporary that is closed at the end of its statement. Guess (not traced):
that temporary is closed but its record is not freed. This is the single-resource case;
bug-999 is about a `List OF RES` or a record of resources returned by a nested call.

Found during batch 21.
