# bug-1071: a loop-scoped `List OF RES fs::File` appended to a `List OF List OF RES fs::File` declared outside the loop keeps 1168 bytes per call

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-g)
Regression Test: `rt_resource_model::bug_1071_inner_list_appended_to_a_list_of_lists`

An inner list built in a loop body and appended to an outer list of lists declared before the loop: the files are readable after the loop and are closed, and the inner list blocks and the file records are kept.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit, `double_free_skips 0`, every descriptor closed once.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and with the batch 22 staging compiler (8592e4505): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters the program prints to stderr at exit (`N=<n> SRC=<dir> /tmp/bs22/bin/probe.sh <mfb> <name>`). `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The result is the same on both compilers unless a line says otherwise.

`N=10 SRC=/tmp/bs22-agent-b/m1045 probe.sh <mfb> p`: exit 0, stdout `240`, `alloc_calls 222`, `free_calls 152`, `live_bytes 11680`, on both compilers. `N=40` (staging compiler): stdout `960`, 882 / 602 / 46720.

`p (the prelude is bug-1045's `r2` prelude, `/tmp/bs22-agent-b/m1045/pre.txt`)`:

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
  h AS RES fs::File
END TYPE

TYPE HU
  h AS RES Handle
END TYPE

FUNC openOne AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC

FUNC wrap(h AS fs::File) AS HB
  RETURN HB[h]
END FUNC

FUNC rd(f AS fs::File) AS Integer
  RETURN len(fs::readAll(f))
END FUNC

FUNC which(h AS Handle) AS Integer
  MATCH h
    CASE fs::File(f)
      RETURN len(fs::readAll(f))
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

FUNC mk() AS HB
  RETURN HB[openOne()]
END FUNC

FUNC mkU() AS HU
  RETURN HU[openOne()]
END FUNC

FUNC sumHB(xs AS List OF HB) AS Integer
  MUT t AS Integer = 0
  FOR EACH x IN xs
    t = t + rd(x.h) + 1
  NEXT
  RETURN t
END FUNC
FUNC once() AS Integer
  MUT outer AS List OF List OF RES fs::File = []
  FOR j = 1 TO 2
    MUT inner AS List OF RES fs::File = []
    FOR q = 1 TO 2
      RES f AS fs::File = openOne()
      inner = collections::append(inner, f)
    NEXT
    outer = collections::append(outer, inner)
  NEXT
  MUT t AS Integer = 0
  FOR EACH l IN outer
    FOR EACH f IN l
      t = t + rd(f)
    NEXT
  NEXT
  RETURN t
END FUNC
```

## What is known

Not traced. A collection is held by pointer where it is stored, so `StoreFloatProof` proves nothing for it and the outer list's drain is close-only. Near bug-1056 (a nested list bound from a call keeps its inner list blocks); re-measure once bug-1056 is fixed.

Found during batch 22 (reported by bs22-agent-b, reproduced by the coordinator).

## Resolution (bs22-agent-g, 2026-10-11)

The descriptors were closed and each 112-byte record kept, because a record was freed only for a holder a static proof accepted. The scope that closes a resource now frees its record, decided by the owner word alone: the cleanup takes the record (`RESOURCE_OWNER_TAKEN`) and the exit frees it after its last cleanup (owned-lists, `RES` bindings, resource-union bindings). The proofs are deleted. A `List OF List OF RES T` inlines its inner lists (rule 3), so no inner block is held by pointer.

Regression tests: `rt_resource_model::bug_1071_inner_list_appended_to_a_list_of_lists`. Each builds the bug's repro (`bugs/repro/`) with `--debug` and requires, at N=10 and N=40, exit 0, the expected total, `double_free_skips 0` and live bytes that do not grow, and exit 0 at N=300 under `ulimit -n 64`.

## Golden impact

Not regenerated. Expect `.ncode` changes wherever a `RES` binding, a resource-union binding or an owned-list is dropped (the take and the reclaim after the exit's last cleanup), wherever a call returns a value reaching resources (the statement's take and drain), and at assignments that float.
