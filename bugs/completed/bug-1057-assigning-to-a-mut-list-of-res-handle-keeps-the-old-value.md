# bug-1057: assigning a list to a `MUT List OF RES Handle` keeps 48 bytes per assignment

Last updated: 2026-10-11
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-g)
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting the live bytes do not grow between `N=10` and `N=40`

`MUT hs AS List OF RES Handle = []` followed by `hs = files` inside a loop keeps 48 bytes
on every assignment. The old value of `hs` is not freed.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes` does not grow with `N`.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello
").

`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb bb_v03`: exit 0, stdout
`n=200000`, `alloc_calls 23`, `free_calls 12`, `live_bytes 720`. `N=40`: stdout `n=800000`,
53 / 12 / 2160. The growth is 1440 bytes over 30 iterations, 48 per iteration
(`(2160 - 720) / 30`); `free_calls` stays 12.

The stdout is the `CASE ELSE` arm taken for both elements on every iteration
(2 * 10000 * 10 = 200000), which is bug-1018 (a `List OF RES fs::File` assigned to a `List
OF RES Handle` is not converted). That is a separate defect; this document is about the
bytes.

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

TYPE Box
  hs AS List OF RES Handle
END TYPE

FUNC describe(hs AS List OF RES Handle) AS Integer
  MUT t AS Integer = 0
  FOR EACH h IN hs
    MATCH h
      CASE fs::File(f)
        fs::writeAll(f, "x")
        t = t + 1
      CASE tcp::Socket(s)
        t = t + 100
      CASE ELSE
        t = t + 10000
    END MATCH
  NEXT
  RETURN t
END FUNC

FUNC describeMap(hs AS Map OF String TO RES Handle) AS Integer
  MUT t AS Integer = 0
  FOR EACH e IN hs
    MATCH e.value
      CASE fs::File(f)
        fs::writeAll(f, "x")
        t = t + 1
      CASE tcp::Socket(s)
        t = t + 100
      CASE ELSE
        t = t + 10000
    END MATCH
  NEXT
  RETURN t
END FUNC

FUNC give(files AS List OF RES fs::File) AS List OF RES Handle
  RETURN files
END FUNC

FUNC main AS Integer
  MUT n AS Integer = 0
  MUT files AS List OF RES fs::File = []
  RES f1 AS fs::File = fs::createTempFile()
  files = collections::append(files, f1)
  RES f2 AS fs::File = fs::createTempFile()
  files = collections::append(files, f2)
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    MUT hs AS List OF RES Handle = []
    hs = files
    n = n + describe(hs)
  NEXT
  fs::writeAll(f1, "still open")
  fs::writeAll(collections::get(files, 1), "still open")
  io::print("n=" & toString(n))
  RETURN 0
END FUNC
```

## What is known

bug-1018's section "Attempt in bug-staging-21" lists this position (`MUT hs = []` then
`hs = files`) in its table with the same base counters, 23/12/720 at `N=10` and 53/12/2160
at `N=40`, and does not call them out as a leak of their own. Fixing bug-1018's conversion
will change this program's behavior, so this document's check must be re-run after it.
Whether the 48 bytes is the old empty list block (guess; not traced) was not measured.

Found during batch 21.

## Resolution (bs22-agent-g, 2026-10-11)

The general cause: A collection of resource aliases was classified "not flat" (`flatness_walk` met the bare resource the element accessors leave after stripping the `RES` marker), so such a list was shared by pointer at every hand-off and no scope freed its block. It is plain data now (rule 3: `is_resource_alias_payload`, concrete and resource-union elements), copied at every hand-off to a longer-lived owner and freed by its one owner through the ordinary owned-value path. A `List OF RES Handle` (resource-union elements, each a `{tag, record}` box held inline) is such a block, so the assignment frees the value it replaces.

Measured with `/tmp/bs22/bin/probe.sh` and `/tmp/bs22/sweep.sh` (alloc/free/live bytes, `--debug` arena 0), this branch's compiler:
b1057 and f1057_0: 63/63/0 at N=10, 213/213/0 at N=40, same at -O0, N=300 exits 0 (main: 23/12/720, 53/12/2160).

Tests: `tests/runtime/rt_res_list_from_call_leak.rs` (the list block counts), the 31 resource targets of the batch. No rule keyed on where the call or the value is written.

## Golden impact

Not regenerated. `scripts/artifact-gate.sh <mfb> all`: every fixture that binds, copies, reassigns or drops a collection of `RES` elements gains the ordinary copy and scope-drop of a flat list (`.ncode`), and every call to a function body that returns a value reaching resources gains the statement's take and drain.
