# bug-1051: a trapped call that returns its own argument keeps the 112-byte default record on every evaluation

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting `live_bytes 0` at `N=10` and `N=20`, plus 300 iterations under `ulimit -n 64`

`RES v AS fs::File = sameF(a) TRAP(e) … END TRAP`, where `sameF(h)` returns its argument,
keeps 112 bytes per evaluation. The descriptor is closed.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb c9t`: exit 0, stdout
`10`, `alloc_calls 52`, `free_calls 42`, `live_bytes 1120`. `N=20`: stdout `20`, 102 / 82 /
2240. `(ulimit -n 64; N=300 …)`: exit 0, stdout `300`, 1502 / 1202 / 33600.

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
FUNC mkH AS RES Handle
  RETURN openOne()
END FUNC
FUNC mkBad(n AS Integer) AS RES Handle
  IF n > 0 THEN
    FAIL error(77, "bad")
  END IF
  RETURN openOne()
END FUNC
FUNC sameF(h AS fs::File) AS fs::File
  RETURN h
END FUNC
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  RES v AS fs::File = sameF(a) TRAP(e)
    RETURN 5
  END TRAP
  RETURN len(fs::readAll(v)) - 5
END FUNC
```

## What is known

1120 / 10 = 112 bytes per call, the size of the closed default `fs::File` record the trap's
temp is bound to. bug-1037's resolution (completed) recorded this as a residue it did not
fix: the temp's default record is not freed because `record_ownership` cannot prove the temp
owns every record stored into it, and an unproven binding closes without freeing. It states
the same for `RES v AS Handle = same(a) TRAP`, whose box was fixed and whose default record
stays (1280 to 1120 at N=10). That is the reported cause and was not verified here.

Found during batch 21.
