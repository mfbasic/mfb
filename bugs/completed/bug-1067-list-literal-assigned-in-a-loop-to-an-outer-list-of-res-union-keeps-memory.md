# bug-1067: `hs = [s, u]` in a loop, assigned to a `MUT List OF RES Handle` declared outside it, keeps 832 bytes per call

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-g)
Regression Test: `rt_resource_model::bug_1067_list_literal_assigned_in_a_loop`

A list literal of two concrete files assigned in a `FOR` body to a `MUT hs AS List OF RES Handle` declared before the loop keeps the replaced lists and their boxes.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit, `double_free_skips 0`, every descriptor closed once.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and with the batch 22 staging compiler (8592e4505): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters the program prints to stderr at exit (`N=<n> SRC=<dir> /tmp/bs22/bin/probe.sh <mfb> <name>`). `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The result is the same on both compilers unless a line says otherwise.

`N=10`: exit 0, stdout `120`, `alloc_calls 302`, `free_calls 232`, `live_bytes 8320`. `N=40`: stdout `480`, 1202 / 922 / 33280.

`n6_list_literal_assign`:

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
  MUT hs AS List OF RES Handle = []
  FOR j = 1 TO 3
    RES s AS Handle = openOne()
    RES u AS Handle = openOne()
    hs = [s, u]
  NEXT
  RETURN countH(hs)
END FUNC
```

## What is known

Not traced. Near bug-1057 (`hs = files` on a `MUT List OF RES Handle` keeps the old value). Re-measure this program once bug-1057 is fixed.

Found during batch 22 (reported by bs22-agent-b, reproduced by the coordinator).

## Resolution (bs22-agent-g, 2026-10-11)

The descriptors were closed and each 112-byte record kept, because a record was freed only for a holder a static proof accepted. The scope that closes a resource now frees its record, decided by the owner word alone: the cleanup takes the record (`RESOURCE_OWNER_TAKEN`) and the exit frees it after its last cleanup (owned-lists, `RES` bindings, resource-union bindings). The proofs are deleted. A `List OF RES Handle` is plain data (rule 3), so the assignment frees the block it replaces.

Regression tests: `rt_resource_model::bug_1067_list_literal_assigned_in_a_loop`. Each builds the bug's repro (`bugs/repro/`) with `--debug` and requires, at N=10 and N=40, exit 0, the expected total, `double_free_skips 0` and live bytes that do not grow, and exit 0 at N=300 under `ulimit -n 64`.

## Golden impact

Not regenerated. Expect `.ncode` changes wherever a `RES` binding, a resource-union binding or an owned-list is dropped (the take and the reclaim after the exit's last cleanup), wherever a call returns a value reaching resources (the statement's take and drain), and at assignments that float.
