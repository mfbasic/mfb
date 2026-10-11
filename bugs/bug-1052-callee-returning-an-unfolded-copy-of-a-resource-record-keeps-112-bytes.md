# bug-1052: a callee that returns an unfolded copy of a record holding a resource keeps 112 bytes per call

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below at `-O0` asserting `live_bytes 0` at `N=10` and `N=20`

`mid` binds `LET k AS HB2 = mk2()`, copies it with `LET k3 AS HB2 = k`, and returns `k3`. The
copy is a second holder of the record that holds the file, and the record is closed but not
freed. On main at the default optimization level this keeps 112 bytes per call.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb k2`: exit 0, stdout
`10`, `alloc_calls 72`, `free_calls 62`, `live_bytes 1120`. `N=20`: stdout `20`, 142 / 122 /
2240. `(ulimit -n 64; N=300 …)`: exit 0, stdout `300`, 2102 / 1802 / 33600. With `-O0`
(`N=10 … probe.sh <mfb> k2 -O0`): 82 / 72 / 1120 on main.

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
TYPE HB2
  h AS RES fs::File
END TYPE
TYPE Outer2
  n AS Integer
  inner AS HB2
END TYPE
FUNC keep2(h AS fs::File) AS HB2
  RETURN HB2[h]
END FUNC
FUNC mk2 AS HB2
  RES u AS fs::File = openOne()
  RETURN keep2(u)
END FUNC
FUNC rd(f AS fs::File) AS Integer
  RETURN len(fs::readAll(f)) - 5
END FUNC
FUNC mid AS HB2
  LET k AS HB2 = mk2()
  LET k3 AS HB2 = k
  RETURN k3
END FUNC
FUNC once() AS Integer
  LET q AS HB2 = mid()
  LET junk1 AS List OF Integer = [7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7]
  RETURN rd(q.h)
END FUNC
```

## What is known

1120 / 10 = 112 bytes per call, the size of an `fs::File` record.

With bug-1025 fixed in batch 21 the default optimization level is balanced and only `-O0`
still keeps it: 82 / 72 / 1120 with the batch-21 compiler (`probe.sh <mfb> k2 -O0`). That is
a figure reported by the coordinator, not re-measured by this document, which has only main's
compiler. The title is therefore about the unfolded copy. Guess (not traced): at the default
level the optimizer folds `LET k3 = k` away, and at `-O0` the copy remains and is not
recognized as a holder.

Found during batch 21.
