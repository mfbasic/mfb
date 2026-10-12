# bug-1047: `collections::get` under an inline `TRAP` written into an argument keeps 96 bytes per call

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-g)
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting `live_bytes 0` at `N=10` and `N=20`, plus 300 iterations under `ulimit -n 64`

`whichF("t", collections::get(hs, i)) TRAP(e) … END TRAP`, where `hs` is a
`List OF RES fs::File`, keeps 96 bytes per call of the enclosing function. The descriptors are
closed.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb c5`: exit 0, stdout
`10`, `alloc_calls 82`, `free_calls 72`, `live_bytes 960`. `N=20`: stdout `20`, 162 / 142 /
1920. `(ulimit -n 64; N=300 …)`: exit 0, stdout `300`, 2402 / 2102 / 28800.

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
  LET hs AS List OF RES fs::File = [a]
  MUT r AS Integer = 0
  r = whichF("t", collections::get(hs, 5)) TRAP(e)
    RECOVER 7
  END TRAP
  MUT q AS Integer = 0
  q = whichF("t", collections::get(hs, 0)) TRAP(e)
    RECOVER 7
  END TRAP
  RETURN r + q - 7
END FUNC
```

## What is known

960 / 10 = 96 bytes per call at `N=10`, 1920 / 20 = 96 at `N=20`, 28800 / 300 = 96 at
`N=300`. Each call evaluates two trapped `get` arguments: `get(hs, 5)` (out of range, takes
the `RECOVER`) and `get(hs, 0)` (succeeds). Which of the two keeps the bytes was not
isolated, and the kept block was not identified.

Found during batch 21.

## Resolution (bs22-agent-g, 2026-10-11)

The general cause: A collection of resource aliases was classified "not flat" (`flatness_walk` met the bare resource the element accessors leave after stripping the `RES` marker), so such a list was shared by pointer at every hand-off and no scope freed its block. It is plain data now (rule 3: `is_resource_alias_payload`, concrete and resource-union elements), copied at every hand-off to a longer-lived owner and freed by its one owner through the ordinary owned-value path. The kept 96 bytes were the list's own block.

Measured with `/tmp/bs22/bin/probe.sh` and `/tmp/bs22/sweep.sh` (alloc/free/live bytes, `--debug` arena 0), this branch's compiler:
c5: 82/82/0 at N=10, 322/322/0 at N=40, same at -O0, N=300 exits 0 (main: 82/72/960).

Tests: `tests/runtime/rt_res_list_from_call_leak.rs` (the list block counts), the 31 resource targets of the batch. No rule keyed on where the call or the value is written.

## Golden impact

Not regenerated. `scripts/artifact-gate.sh <mfb> all`: every fixture that binds, copies, reassigns or drops a collection of `RES` elements gains the ordinary copy and scope-drop of a flat list (`.ncode`), and every call to a function body that returns a value reaching resources gains the statement's take and drain.
