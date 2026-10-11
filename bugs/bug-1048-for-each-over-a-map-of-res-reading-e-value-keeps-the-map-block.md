# bug-1048: `FOR EACH` over a `Map OF String TO RES fs::File` read through `e.value` keeps 192 bytes per call

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting `live_bytes 0` at `N=10` and `N=20`, plus 300 iterations under `ulimit -n 64`

A map literal of resources that is iterated with `FOR EACH e IN m` and read through
`e.value` keeps memory on every call. The descriptors are closed.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb c8`: exit 0, stdout
`10`, `alloc_calls 92`, `free_calls 82`, `live_bytes 1920`. `N=20`: stdout `20`, 182 / 162 /
3840. `(ulimit -n 64; N=300 …)`: exit 0, stdout `300`, 2702 / 2402 / 57600.

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
  RES c AS fs::File = openOne()
  LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { "x" := a, "y" := c }
  MUT t AS Integer = 0
  FOR EACH e IN m
    t = t + whichF("m", e.value)
  NEXT
  RETURN t - 1
END FUNC
```

## What is known

1920 / 10 = 192 bytes per call; 3840 / 20 and 57600 / 300 give the same. The kept blocks were
not itemized; the map block is the reported culprit (guess).

The batch-21 document for resource unions (bug-1028, "a `List`/`Map` of resource unions that
is iterated or read through `collections::get` keeps its block and records") records that the
same shapes over `List OF RES fs::File` are reported balanced, and did not re-measure them.
This program uses a concrete `fs::File` map value and keeps memory, so the leak is not
specific to unions.

Found during batch 21.
