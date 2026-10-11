# bug-1044: a record built from a call that returns a `List OF RES` closes its descriptors but keeps its memory

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting `live_bytes 0` at `N=10` and `N=20`, plus 300 iterations under `ulimit -n 64`

`LET b AS Box = mkBox()`, where `mkBox` returns `Box[items := mkList()]` and `Box` holds a
`List OF RES fs::File`, closes every descriptor at the end of `once` but keeps memory on every
call.

**Owner decisions (2026-10-11):** no leak is intentional (recorded in bug-1036). A list of
resources that a call returns unbound takes the scope of whatever takes it (a record, binding
or loop), else the statement's. Here the record takes it, so the record's scope frees it.

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb o_boxcall`: exit 0, stdout
`n=10`, `alloc_calls 103`, `free_calls 63`, `live_bytes 3840`. `N=20`: stdout `n=20`,
203 / 123 / 7680. `(ulimit -n 64; N=300 …)`: exit 0, stdout `n=300`, 3003 / 1803 / 115200.
The 300-iteration run stays under the 64-descriptor limit, so the descriptors are closed.

```
IMPORT fs
IMPORT io
IMPORT os
IMPORT collections

TYPE Box
  items AS List OF RES fs::File
END TYPE

FUNC mkList() AS List OF RES fs::File
  MUT xs AS List OF RES fs::File = []
  RES f AS fs::File = fs::createTempFile()
  xs = collections::append(xs, f)
  RETURN xs
END FUNC

FUNC mkBox() AS Box
  RETURN Box[items := mkList()]
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
  LET b AS Box = mkBox()
  RETURN len(b.items)
END FUNC
```

## What is known

Per call, the N=300 run makes (3003 - 3) / 300 = 10 allocations and (1803 - 3) / 300 = 6
frees (the 3 is the fixed part, read from the N=10 and N=20 runs), and keeps 115200 / 300 =
384 bytes. Which four blocks are kept was not traced. The kept memory is independent of the
descriptor, which is closed.

Guess (not traced): the list returned by `mkList()` is an aggregate returned by a call nested
in a constructor argument, which bug-999 describes as unregistered, and nothing frees its
block or records once the record `b` is dropped.

Found during batch 21.
