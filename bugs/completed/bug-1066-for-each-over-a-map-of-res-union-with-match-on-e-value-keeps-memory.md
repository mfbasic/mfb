# bug-1066: `FOR EACH e` over a `Map OF String TO RES Handle` with `MATCH e.value` keeps 1408 bytes per call

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-g)
Regression Test: `rt_resource_model::bug_1066_union_map_for_each_match`

bug-1048's shape over a map whose values are a resource UNION: the map is iterated with `FOR EACH e IN m` and each `e.value` is matched. The descriptors are closed; memory is kept on every call.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit, `double_free_skips 0`, every descriptor closed once.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and with the batch 22 staging compiler (8592e4505): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters the program prints to stderr at exit (`N=<n> SRC=<dir> /tmp/bs22/bin/probe.sh <mfb> <name>`). `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The result is the same on both compilers unless a line says otherwise.

`N=10`: exit 0, stdout `180`, `alloc_calls 252`, `free_calls 212`, `live_bytes 14080`. `N=40`: stdout `720`, 1002 / 842 / 56320.

`n5_union_map_foreach`:

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
  n AS Integer
END TYPE

FUNC openOne AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC

FUNC countH(hs AS List OF RES Handle) AS Integer
  MUT t AS Integer = 0
  FOR EACH h IN hs
    MATCH h
      CASE fs::File(f)
        t = t + len(fs::readAll(f))
      CASE tcp::Socket(s)
        t = t + 100
    END MATCH
  NEXT
  RETURN t
END FUNC

FUNC countM(hs AS Map OF String TO RES Handle) AS Integer
  MUT t AS Integer = 0
  FOR EACH e IN hs
    MATCH e.value
      CASE fs::File(f)
        t = t + len(fs::readAll(f))
      CASE tcp::Socket(s)
        t = t + 100
    END MATCH
  NEXT
  RETURN t
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
  MUT hs AS Map OF String TO RES Handle = Map OF String TO RES Handle {}
  FOR j = 1 TO 3
    RES s AS Handle = openOne()
    hs = collections::set(hs, toString(j), s)
  NEXT
  RETURN countM(hs)
END FUNC
```

## What is known

Not traced. bug-1048 is the concrete `fs::File` form (192 bytes per call). Re-measure this program once bug-1048 is fixed: its fix may or may not cover the union form.

Found during batch 22 (reported by bs22-agent-b, reproduced by the coordinator).

## Resolution (bs22-agent-g, 2026-10-11)

The descriptors were closed and each 112-byte record kept, because a record was freed only for a holder a static proof accepted. The scope that closes a resource now frees its record, decided by the owner word alone: the cleanup takes the record (`RESOURCE_OWNER_TAKEN`) and the exit frees it after its last cleanup (owned-lists, `RES` bindings, resource-union bindings). The proofs are deleted. A `Map OF K TO RES Handle` is plain data (rule 3).

Regression tests: `rt_resource_model::bug_1066_union_map_for_each_match`. Each builds the bug's repro (`bugs/repro/`) with `--debug` and requires, at N=10 and N=40, exit 0, the expected total, `double_free_skips 0` and live bytes that do not grow, and exit 0 at N=300 under `ulimit -n 64`.

## Golden impact

Not regenerated. Expect `.ncode` changes wherever a `RES` binding, a resource-union binding or an owned-list is dropped (the take and the reclaim after the exit's last cleanup), wherever a call returns a value reaching resources (the statement's take and drain), and at assignments that float.
