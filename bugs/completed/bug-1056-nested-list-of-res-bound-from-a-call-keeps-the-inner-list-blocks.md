# bug-1056: a `List OF List OF RES fs::File` returned by a call and bound keeps its inner list blocks

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-g)
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting `live_bytes 0` at `N=10` and `N=40`

`LET t AS List OF List OF RES fs::File = mkNested()` followed by `len(t)` closes the
descriptors but keeps memory on every call: the outer list is freed, the inner `List OF RES
fs::File` blocks and the records they hold are not.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello
").

`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb m_b11`: exit 0, stdout
`n=10 keep=0`, `alloc_calls 168`, `free_calls 157`, `live_bytes 2448`. `N=40`: stdout
`n=40 keep=0`, 648 / 607 / 9648. The growth is 7200 bytes over 30 calls, 240 per call
(`(9648 - 2448) / 30`). The program declares many unused helpers; only `mkList`, `mkNested`
and `main` run.

```
IMPORT fs
IMPORT io
IMPORT os
IMPORT tcp
IMPORT collections

UNION Handle
  fs::File
  tcp::Socket
END UNION

TYPE Holder
  h AS RES fs::File
  n AS Integer
END TYPE

TYPE Layer
  items AS List OF RES fs::File
END TYPE

MUT gKeep AS List OF RES fs::File = []

FUNC mkList() AS List OF RES fs::File
  MUT xs AS List OF RES fs::File = []
  RES f AS fs::File = fs::createTempFile()
  xs = collections::append(xs, f)
  RES g AS fs::File = fs::createTempFile()
  xs = collections::append(xs, g)
  RETURN xs
END FUNC

FUNC mkListN(k AS Integer, tag AS String) AS List OF RES fs::File
  MUT xs AS List OF RES fs::File = []
  FOR i = 1 TO k
    RES f AS fs::File = fs::createTempFile()
    xs = collections::append(xs, f)
  NEXT
  RETURN xs
END FUNC

FUNC mkDup() AS List OF RES fs::File
  MUT xs AS List OF RES fs::File = []
  RES f AS fs::File = fs::createTempFile()
  xs = collections::append(xs, f)
  xs = collections::append(xs, f)
  RETURN xs
END FUNC

FUNC mkMap() AS Map OF String TO RES fs::File
  MUT m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File {}
  RES f AS fs::File = fs::createTempFile()
  m = collections::set(m, "a", f)
  RETURN m
END FUNC

FUNC mkHolder() AS Holder
  RES f AS fs::File = fs::createTempFile()
  RETURN Holder[f, 1]
END FUNC

FUNC mkHolders() AS List OF Holder
  MUT hs AS List OF Holder = []
  RES f AS fs::File = fs::createTempFile()
  hs = collections::append(hs, Holder[f, 1])
  RETURN hs
END FUNC

FUNC mkNested() AS List OF List OF RES fs::File
  MUT outer AS List OF List OF RES fs::File = []
  outer = collections::append(outer, mkList())
  RETURN outer
END FUNC

FUNC mkHandles() AS List OF RES Handle
  MUT xs AS List OF RES Handle = []
  RES f AS Handle = fs::createTempFile()
  xs = collections::append(xs, f)
  RETURN xs
END FUNC

FUNC mkLayer() AS Layer
  RETURN Layer[items := mkList()]
END FUNC

FUNC mkFrom(f AS fs::File) AS List OF RES fs::File
  MUT xs AS List OF RES fs::File = []
  xs = collections::append(xs, f)
  RETURN xs
END FUNC

FUNC viewG() AS List OF RES fs::File
  RETURN gKeep
END FUNC

FUNC count(xs AS List OF RES fs::File) AS Integer
  MUT t AS Integer = 0
  FOR EACH h IN xs
    fs::writeAll(h, "x")
    t = t + 1
  NEXT
  RETURN t
END FUNC

FUNC countFail(xs AS List OF RES fs::File) AS Integer
  IF len(xs) > 0 THEN
    FAIL error(77, "boom")
  END IF
  RETURN 0
END FUNC

FUNC stash(xs AS List OF RES fs::File) AS Integer
  gKeep = collections::append(gKeep, collections::get(xs, 0))
  RETURN 1
END FUNC

FUNC useH(h AS Holder) AS Integer
  RETURN h.n
END FUNC

FUNC countH(hs AS List OF Holder) AS Integer
  RETURN len(hs)
END FUNC

FUNC early(i AS Integer) AS Integer
  IF i MOD 2 = 0 THEN
    RETURN len(mkList())
  END IF
  RETURN count(mkList()) + len(mkList())
END FUNC

FUNC main AS Integer
  MUT n AS Integer = 0
  LET flag AS Boolean = len(os::getEnvOr("N", "10")) > 99
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    LET t AS List OF List OF RES fs::File = mkNested()
    n = n + len(t)
  NEXT
  io::print("n=" & toString(n) & " keep=" & toString(len(gKeep)))
  FOR EACH k IN gKeep
    fs::writeAll(k, "still open")
  NEXT
  RETURN 0
END FUNC
```

## What is known

Not traced. Guess: the scope exit of `t` frees the outer list and closes its elements, but
does not free the inner list blocks. Related by shape to bug-999 (a list of resources
returned by a call and never closed) and bug-1044 (a record built from such a list keeps
384 bytes per call); neither document names a nested list.

Found during batch 21.

## Resolution (bs22-agent-g, 2026-10-11)

The general cause: A collection of resource aliases was classified "not flat" (`flatness_walk` met the bare resource the element accessors leave after stripping the `RES` marker), so such a list was shared by pointer at every hand-off and no scope freed its block. It is plain data now (rule 3: `is_resource_alias_payload`, concrete and resource-union elements), copied at every hand-off to a longer-lived owner and freed by its one owner through the ordinary owned-value path. A nested `List OF List OF RES T` inlines its inner lists like any nested flat list, so one free releases every block; the by-pointer inner lists and their proof-gated frees (`block_collections`, `emit_inner_block_frees`) are gone.

Measured with `/tmp/bs22/bin/probe.sh` and `/tmp/bs22/sweep.sh` (alloc/free/live bytes, `--debug` arena 0), this branch's compiler:
m_b11: 208/207/48 at N=10, 808/807/48 at N=40, same at -O0, N=300 exits 0 (main: 168/157/2448, growing). The constant 48 bytes are the program's module-level `gKeep` list block, which is bug-1068 (module-level values at exit).

Tests: `tests/runtime/rt_res_list_from_call_leak.rs` (the list block counts), the 31 resource targets of the batch. No rule keyed on where the call or the value is written.

## Golden impact

Not regenerated. `scripts/artifact-gate.sh <mfb> all`: every fixture that binds, copies, reassigns or drops a collection of `RES` elements gains the ordinary copy and scope-drop of a flat list (`.ncode`), and every call to a function body that returns a value reaching resources gains the statement's take and drain.
