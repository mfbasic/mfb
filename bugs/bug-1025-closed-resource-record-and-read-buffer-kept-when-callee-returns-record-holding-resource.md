# bug-1025: a closed resource record, and its read buffer, are kept when a callee returns a record holding a resource that its caller's callee produced

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over each program below, asserting `alloc_calls == free_calls` and `live_bytes 0` after the loop

`mk2` opens a file in `openOne`, binds it (`RES u AS fs::File = openOne()`), and returns a record that holds it
(`keep2(u)` returns `HB2[h]`). The caller reads through the record and drops it. The file descriptor is closed, but
the record block is never freed; when the file was also read with `fs::readLine`, its read buffer is kept too.

**Correct behavior:** the record, and anything its resource owned (the buffer), are freed once when the caller's
binding goes out of scope; `alloc_calls == free_calls`, `live_bytes 0`.

## Failing Reproduction

`/tmp/g933/data.txt` is a 6-byte file (`hello` and a newline). Measured on 2026-10-10 with main's compiler
(`/tmp/main-dbg-target/release/mfb`, main at 3fa35d543, macOS aarch64), through
`N=<n> /tmp/bs19b/bin/probe.sh /tmp/main-dbg-target/release/mfb x_conc_keep` (`mfb build -q --debug`, then the binary
with `N` set; arena counters print on stderr at exit).

Repro A (`/tmp/bs19b/src/x_conc_keep.mfb`): N=10 gives `alloc_calls 52`, `free_calls 42`, `live_bytes 1120`; N=20
gives `alloc_calls 102`, `free_calls 82`, `live_bytes 2240`. That is 112 B per call, one record block never freed.

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
FUNC keep2(h AS fs::File) AS HB2
  RETURN HB2[h]
END FUNC
FUNC mk2 AS HB2
  RES u AS fs::File = openOne()
  RETURN keep2(u)
END FUNC
FUNC once() AS Integer
  LET k AS HB2 = mk2()
  RETURN len(fs::readAll(k.h)) - 5
END FUNC
```

Repro B (project `/tmp/bs19-rev/e2`, built with `mfb build -q --debug`, run as `N=10 ./build/e2.out`) has the same shape
with a four-field record (`name`, `h AS RES fs::File`, `n`, `tail`) and reads the file with `fs::readLine` first:
N=10 gives `alloc_calls 142`, `free_calls 122`, `live_bytes 164960`; N=40 gives `alloc_calls 562`, `free_calls 482`,
`live_bytes 659840`. That is 16496 B per call (the record plus the file's read buffer). The descriptors ARE closed:
`(ulimit -n 64; N=300 ./build/e2.out)` exits 0.

```
IMPORT io
IMPORT os
IMPORT fs
IMPORT tcp
IMPORT collections
UNION Stream
  fs::File
  tcp::Socket
END UNION
FUNC openData() AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC
FUNC main() AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    t = t + once()
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC

TYPE HB
  name AS String
  h AS RES fs::File
  n AS Integer
  tail AS String
END TYPE
FUNC keep(nm AS String, h AS fs::File, n AS Integer) AS HB
  RETURN HB[nm, h, n, "tail" & toString(n)]
END FUNC
FUNC mk(n AS Integer) AS HB
  RES u AS fs::File = openData()
  RETURN keep("name" & toString(n), u, n)
END FUNC
FUNC readOne(f AS fs::File) AS Integer
  RETURN len(fs::readLine(f))
END FUNC
FUNC readOneOld(h AS Stream) AS Integer
  MATCH h
    CASE fs::File(f)
      RETURN len(fs::readLine(f))
    CASE tcp::Socket(s)
      RETURN 1000
  END MATCH
END FUNC
FUNC once() AS Integer
  LET k AS HB = mk(3)
  LET junk1 AS List OF Integer = [7, 7, 7, 7]
  LET k2 AS HB = WITH k { n := 9 }
  LET junk2 AS String = "abcdefghijklmnopqrstuvwxyz" & toString(k2.n)
  LET k3 AS HB = k2
  MUT r AS Integer = 0
  IF k.name = "name3" AND k.tail = "tail3" AND k.n = 3 AND k3.n = 9 AND k3.tail = "tail3" AND k2.name = "name3" THEN
    r = 1
  END IF
  RETURN r * 100 + readOne(k3.h)
END FUNC
```

## What is known

Both numbers are the same before and after the bug-1012 fix (measured by the batch 19 agent that fixed bug-1012), so
this is pre-existing and not caused by it. The cause was not traced (guess: the record that `keep2`/`keep` builds from
a parameter resource is not registered as owned by the caller's binding of the call result, so the free of the record
block is skipped while the close of the file is done by the binding `u`).

Found by the bug-1012 agent while fixing bug-1012 (batch 19).
