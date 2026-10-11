# bug-1045: a loop-scoped record's resource moved into a list declared outside the loop is closed when the loop body ends

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (use after close)

Status: Open
Regression Test: (proposed) a runtime test over both programs below asserting the program completes and prints the expected sum

A resource that a loop-scoped `LET k AS HB = mk()` holds is appended to a list declared
before the loop, either as the record (`xs = collections::append(xs, k)`, list of `HB`) or as
its field (`fsl = collections::append(fsl, k.h)`, `List OF RES fs::File`). The loop body's
scope exit closes the resource although the outer list now holds it. Reading it after the loop
fails with `Error: 7-703-0004`, `Resource handle is already closed.`, exit 255.

**Owner decision (2026-10-11):** a resource put in a list takes the list's scope if that
scope is higher than its own. Here the list's scope (the function) is higher than the loop
body's, so the resource must live until the function ends.

**Correct behavior:** both programs read every resource after the loop. By hand (not
measured, the compiler does not get there): `r2` computes 3 * (6 + 1) = 21 per call, so
`210` at `N=10`; `r5` computes 2 * (6 + 1) = 14 per call, so `140` at `N=10`. `live_bytes 0`
at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`r2` (the record `k` is appended to an outer `List OF HB`):
`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb r2`: exit 255, empty stdout,
stderr begins `Error: 7-703-0004` / `Resource handle is already closed.`, `alloc_calls 20`,
`free_calls 16`, `live_bytes 464`. `N=20` gives the same (it fails in the first call).

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

FUNC mk() AS HB
  RETURN HB[openOne()]
END FUNC
FUNC once() AS Integer
  MUT xs AS List OF HB = []
  FOR j = 1 TO 3
    LET k AS HB = mk()
    LET a AS Integer = rd(k.h)
    xs = collections::append(xs, k)
  NEXT
  MUT t AS Integer = 0
  FOR EACH x IN xs
    t = t + rd(x.h) + 1
  NEXT
  RETURN t
END FUNC
```

`r5` (the field `k.h` is appended to an outer `List OF RES fs::File`):
`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb r5`: exit 255, empty stdout,
the same `Error: 7-703-0004` message, `alloc_calls 15`, `free_calls 12`, `live_bytes 352`.
`N=20` gives the same.

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

FUNC mk() AS HB
  RETURN HB[openOne()]
END FUNC
FUNC once() AS Integer
  MUT fsl AS List OF RES fs::File = []
  FOR j = 1 TO 2
    LET k AS HB = mk()
    LET a AS Integer = rd(k.h)
    fsl = collections::append(fsl, k.h)
  NEXT
  MUT t AS Integer = 0
  FOR EACH f IN fsl
    t = t + rd(f) + 1
  NEXT
  RETURN t
END FUNC
```

## What is known

Both fail in the first call of `once`: the counters are identical at `N=10` and `N=20`.
Guess (inferred, not traced): the failing read is the `FOR EACH` after the loop, the first
read after the loop body's scope exit, and that scope exit closes `k`'s resource because the
append did not move it to the list's scope. Whether the same happens in a build without
`--debug` was not measured.

Found during batch 21.
