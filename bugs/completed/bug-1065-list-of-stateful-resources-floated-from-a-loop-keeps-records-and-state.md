# bug-1065: a `List OF RES fs::File STATE Cur` that a loop floats stateful files into keeps every record and `STATE` block

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-g)
Regression Test: `rt_resource_model::bug_1065_stateful_list_floated_from_a_loop`

`RES v AS fs::File STATE Cur = openOne()` in a `FOR` body, appended to a `MUT xs AS List OF RES fs::File STATE Cur` declared before the loop: the descriptors are closed, and each file's record and its `STATE` block are kept.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit, `double_free_skips 0`, every descriptor closed once.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and with the batch 22 staging compiler (8592e4505): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters the program prints to stderr at exit (`N=<n> SRC=<dir> /tmp/bs22/bin/probe.sh <mfb> <name>`). `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The result is the same on both compilers unless a line says otherwise.

`N=10`: exit 0, stdout `30`, `alloc_calls 142`, `free_calls 72`, `live_bytes 6720`. `N=40`: stdout `120`, 562 / 282 / 26880. 672 bytes and 7 allocations kept per call (three files).

`n8_stateful_list_float`:

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

TYPE Cur
  pos AS Integer
  note AS String
END TYPE

FUNC openOne AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC

FUNC mkH() AS RES Handle
  RES h AS Handle = openOne()
  RETURN h
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
  MUT xs AS List OF RES fs::File STATE Cur = []
  FOR j = 1 TO 3
    RES v AS fs::File STATE Cur = openOne()
    v.state.pos = 1
    xs = collections::append(xs, v)
  NEXT
  MUT t AS Integer = 0
  FOR EACH x IN xs
    t = t + x.state.pos
  NEXT
  RETURN t
END FUNC
```

## What is known

Not traced. The same program without `STATE` is balanced with the staging compiler (bug-1046's and bug-1045's rows), so the `STATE` on the element type is what the owning-list proof or its drain does not handle (guess).

Found during batch 22 (reported by bs22-agent-b, reproduced by the coordinator).

## Resolution (bs22-agent-g, 2026-10-11)

The descriptors were closed and each 112-byte record kept, because a record was freed only for a holder a static proof accepted. The scope that closes a resource now frees its record, decided by the owner word alone: the cleanup takes the record (`RESOURCE_OWNER_TAKEN`) and the exit frees it after its last cleanup (owned-lists, `RES` bindings, resource-union bindings). The proofs are deleted.

Regression tests: `rt_resource_model::bug_1065_stateful_list_floated_from_a_loop`. Each builds the bug's repro (`bugs/repro/`) with `--debug` and requires, at N=10 and N=40, exit 0, the expected total, `double_free_skips 0` and live bytes that do not grow, and exit 0 at N=300 under `ulimit -n 64`.

## Golden impact

Not regenerated. Expect `.ncode` changes wherever a `RES` binding, a resource-union binding or an owned-list is dropped (the take and the reclaim after the exit's last cleanup), wherever a call returns a value reaching resources (the statement's take and drain), and at assignments that float.
