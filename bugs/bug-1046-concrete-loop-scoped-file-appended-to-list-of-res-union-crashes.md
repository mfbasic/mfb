# bug-1046: a loop-scoped concrete `RES fs::File` appended to a `List OF RES Handle` crashes with SIGSEGV

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory safety)

Status: Open
Regression Test: (proposed) a runtime test over the program below asserting exit 0 and stdout `10` at `N=10`

`RES s AS fs::File = openOne()` declared in a `FOR` body and appended to a
`MUT hs AS List OF RES Handle` (a resource union) declared before the loop crashes the
program. It builds without a diagnostic.

**Correct behavior:** exit 0. By hand (not measured): `len(hs) - 2` is 1 per call, so stdout
`10` at `N=10`; `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb g1`: exit 139
(SIGSEGV), empty stdout, no counters (the program never reaches the exit report).
`N=20`: the same.

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
  MUT hs AS List OF RES Handle = []
  FOR j = 1 TO 3
    RES s AS fs::File = openOne()
    hs = collections::append(hs, s)
  NEXT
  RETURN len(hs) - 2
END FUNC
```

## What is known

Reported by the finder's debugger trace, not verified by the coordinator or by this
document: a bad access at `0x23` in `once+0x3ec` on `ldr x8,[x9,#0x20]` with `x9=3`. Their
reading is that the concrete `fs::File` record is read as a resource-union box (the union box
is `{tag, record}`; the concrete value is the bare record), so a field of the record is taken
for a pointer.

This is next to bug-1018 (a `List OF RES fs::File` given where a `List OF RES Handle` is
expected is accepted unconverted) and bug-1027 (a concrete `fs::File` binding appended to a
module-level `List OF RES Handle` fails to build). The append here is local to a function and
builds, then crashes. Whether the same append outside a loop crashes was not measured.

Found during batch 21.
