# bug-1053: a record holding a resource that has a second holder is closed but its 112-byte record is kept

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) one `--debug` alloc/free test per `once()` below asserting `live_bytes 0` at `N=10` and `N=20`, plus 300 iterations under `ulimit -n 64`

When a record's resource has a second holder, the record's drain closes the resource and does
not free the record. Five shapes were measured: the handle bound on its own (`r3`), the record
returned after a copy (`r4`), the record captured by a `LAMBDA` (`r6`), a `MATCH` case binding
stored into another list (`r7`), and a copy appended to a list (`r9`). Each keeps its record on
every call; the descriptors are closed.

These are the "close-only drain" cases of `src/docs/spec/memory/04_arenas.md`: "A handle bound
on its own (`RES f = k.h`), a field stored as an element, a copy made by assignment, and a
record returned or handed to a callee that keeps it are holders, and the record's drain then
closes without freeing", and "A collection that fails any of those keeps a close-only drain,
which leaks those blocks rather than risking a double free."

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036). Under that
decision the close-only drain is a bug: the block must be freed once the last holder is gone.

**Correct behavior:** `live_bytes 0` at exit in all five, with no double free
(`double_free_skips 0`).

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

The five programs share one prelude (lines 1 to 50 of each source, identical in all five);
each adds the declarations and `once()` shown after it. The harness is
`N=<n> /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb <name>`.

Shared prelude:

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
  h AS RES fs::File
END TYPE

TYPE HU
  h AS RES Handle
END TYPE

FUNC openOne AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC

FUNC wrap(h AS fs::File) AS HB
  RETURN HB[h]
END FUNC

FUNC rd(f AS fs::File) AS Integer
  RETURN len(fs::readAll(f))
END FUNC

FUNC which(h AS Handle) AS Integer
  MATCH h
    CASE fs::File(f)
      RETURN len(fs::readAll(f))
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

```

`r3` (the handle bound on its own, `RES f AS fs::File = k.h`):

```
FUNC mk() AS HB
  RETURN HB[openOne()]
END FUNC
FUNC once() AS Integer
  LET k AS HB = mk()
  LET a AS Integer = rd(k.h)
  RES f AS fs::File = k.h
  LET k2 AS HB = k
  RETURN a + rd(f) + rd(k2.h) + 1
END FUNC
```

`r4` (the record returned after a copy):

```
FUNC mk() AS HB
  RETURN HB[openOne()]
END FUNC
FUNC pass() AS HB
  LET k AS HB = mk()
  LET a AS Integer = rd(k.h)
  LET k2 AS HB = k
  IF a = 6 THEN
    RETURN k2
  END IF
  RETURN k
END FUNC
FUNC once() AS Integer
  LET q AS HB = pass()
  RETURN rd(q.h) + 1
END FUNC
```

`r6` (the record captured by a `LAMBDA`):

```
FUNC mk() AS HB
  RETURN HB[openOne()]
END FUNC
FUNC once() AS Integer
  LET k AS HB = mk()
  LET a AS Integer = rd(k.h)
  LET g = LAMBDA() -> rd(k.h)
  RETURN a + g() + 1
END FUNC
```

`r7` (a `MATCH` case binding stored into another list):

```
FUNC pick(hs AS List OF RES Handle) AS List OF RES fs::File
  MUT out AS List OF RES fs::File = []
  FOR EACH h IN hs
    MATCH h
      CASE fs::File(f)
        out = collections::append(out, f)
      CASE tcp::Socket(s)
        io::print("s")
    END MATCH
  NEXT
  RETURN out
END FUNC
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  RES c AS fs::File = openOne()
  LET hs AS List OF RES Handle = [a, c]
  LET fl AS List OF RES fs::File = pick(hs)
  MUT t AS Integer = 0
  FOR EACH f IN fl
    t = t + rd(f) + 1
  NEXT
  FOR EACH h IN hs
    t = t + which(h)
  NEXT
  RETURN t
END FUNC
```

`r9` (a copy appended to a list):

```
FUNC mkU() AS HU
  RES u AS fs::File = openOne()
  RETURN HU[u]
END FUNC
FUNC once() AS Integer
  LET k AS HU = mkU()
  LET k2 AS HU = k
  MUT ks AS List OF HU = []
  ks = collections::append(ks, k2)
  RETURN which(k.h) + which(collections::get(ks, 0).h)
END FUNC
```

Measured results (all exit 0, `double_free_skips 0`):

| Case | stdout at N=10 | N=10 alloc / free / live | N=20 alloc / free / live | N=300 under `ulimit -n 64` alloc / free / live |
| ---- | -------------- | ------------------------ | ------------------------ | ---------------------------------------------- |
| r3   | `70`           | 72 / 62 / 1120           | 142 / 122 / 2240         | 2102 / 1802 / 33600                            |
| r4   | `10`           | 72 / 62 / 1120           | 142 / 122 / 2240         | 2102 / 1802 / 33600                            |
| r6   | `70`           | 92 / 82 / 1120           | 182 / 162 / 2240         | 2702 / 2402 / 33600                            |
| r7   | `140`          | 132 / 122 / 1600         | 262 / 242 / 3200         | 3902 / 3602 / 48000                            |
| r9   | `60`           | 102 / 92 / 1120          | 202 / 182 / 2240         | 3002 / 2702 / 33600                            |

N=300 stdout: `2100`, `300`, `2100`, `4200`, `1800`. The N=300 runs stay under the
64-descriptor limit, so the descriptors are closed.

## What is known

r3, r4, r6 and r9 keep 1120 / 10 = 112 bytes per call, the size of an `fs::File` record. r7
keeps 1600 / 10 = 160 bytes per call; the extra 48 bytes were not itemized. Each case keeps
exactly one more allocation than it frees per call (r3 at N=10: 72 - 62 = 10).

Found during batch 21.
