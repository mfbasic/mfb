# bug-1026: `RES v AS Handle = collections::get(hs, 0)` binds a view into the list block; after the list grows, `MATCH v` matches no case

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory safety; use after free, silent wrong value)

Status: Open
Regression Test: (proposed) the program below, run with `N=10`, asserting stdout `10`; plus a `--debug` test asserting no use of a freed block

A resource-union binding initialised from `collections::get` of a `List OF RES Handle` is a view into the list's
storage, not a copy. When the list then grows (`collections::append` reallocates its block), `v` still points at the
old, freed block, and `MATCH v` takes neither `fs::File` nor `tcp::Socket` and falls through to `RETURN 0`. The build
succeeds, the run exits 0, and the answer is wrong.

**Correct behavior:** `v` owns or safely shares its own copy of the union box, so `which("v", v)` returns 1 whatever
happens to the list afterwards; the program prints `10` at `N=10`.

## Failing Reproduction

Measured on 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at 3fa35d543, macOS aarch64):
`N=10 /tmp/bs19b/bin/probe.sh /tmp/main-dbg-target/release/mfb z_get_view` (`mfb build -q --debug`, then the binary).
Result: `exit=0 stdout=0|`, expected `10`. Arena counters at the same run: `alloc_calls 152`, `free_calls 142`,
`live_bytes 61920` (so there is also a leak; not analysed). `/tmp/g933/data.txt` is a 6-byte file (`hello` and a newline).

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

FUNC once() AS Integer
  RES a AS Handle = openOne()
  RES c AS Handle = openOne()
  MUT hs AS List OF RES Handle = []
  hs = collections::append(hs, a)
  RES v AS Handle = collections::get(hs, 0)
  FOR j = 1 TO 40
    hs = collections::append(hs, c)
  NEXT
  LET junk1 AS List OF Integer = [7, 7, 7, 7, 7, 7, 7, 7]
  LET junk2 AS List OF Integer = [9, 9, 9, 9, 9, 9, 9, 9]
  RETURN which("v", v)
END FUNC
```

## What is known

The bug-1012 fix made a binding initialised from a record FIELD take its own copy of the union box. This is the same
class one container over: the initialiser from `collections::get` of a list was not given the same treatment. Which
function does the aliasing and where the copy belongs were not traced (guess).

Found by the bug-1012 agent while fixing bug-1012 (batch 19).
