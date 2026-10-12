# bug-1064: `LET k AS HB = mk() TRAP …`, a record holding a resource bound from a call under an inline `TRAP`, keeps 112 bytes per evaluation

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-g)
Regression Test: `rt_resource_model::bug_1064_record_from_a_call_under_an_inline_trap`

`LET k AS HB = mk() TRAP(e) … END TRAP` in a loop body, where `HB` has a `RES fs::File` field, closes the descriptor and keeps the file's record on every evaluation. No list is involved.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit, `double_free_skips 0`, every descriptor closed once.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and with the batch 22 staging compiler (8592e4505): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters the program prints to stderr at exit (`N=<n> SRC=<dir> /tmp/bs22/bin/probe.sh <mfb> <name>`). `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The result is the same on both compilers unless a line says otherwise.

`N=10`: exit 0, stdout `120`, `alloc_calls 102`, `free_calls 82`, `live_bytes 2240`. `N=40`: stdout `480`, 402 / 322 / 8960. 224 bytes per call of `once`, which evaluates the binding twice: 112 per evaluation.

`n7_trap_record_container`:

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
  MUT t AS Integer = 0
  FOR j = 1 TO 2
    LET k AS HB = mk() TRAP(e)
      RETURN -1
    END TRAP
    t = t + rd(k.h)
  NEXT
  RETURN t
END FUNC
```

## What is known

Not traced. Near bug-1051 (a trapped call's temp keeps its default record) and bug-999's position 7 (the inline-`TRAP` desugar binds the result through a temporary).

Found during batch 22 (reported by bs22-agent-b, reproduced by the coordinator).

## Resolution (bs22-agent-g, 2026-10-11)

The descriptors were closed and each 112-byte record kept, because a record was freed only for a holder a static proof accepted. The scope that closes a resource now frees its record, decided by the owner word alone: the cleanup takes the record (`RESOURCE_OWNER_TAKEN`) and the exit frees it after its last cleanup (owned-lists, `RES` bindings, resource-union bindings). The proofs are deleted. The inline-`TRAP` delivery binding is the trapped call's taker.

Regression tests: `rt_resource_model::bug_1064_record_from_a_call_under_an_inline_trap`. Each builds the bug's repro (`bugs/repro/`) with `--debug` and requires, at N=10 and N=40, exit 0, the expected total, `double_free_skips 0` and live bytes that do not grow, and exit 0 at N=300 under `ulimit -n 64`.

## Golden impact

Not regenerated. Expect `.ncode` changes wherever a `RES` binding, a resource-union binding or an owned-list is dropped (the take and the reclaim after the exit's last cleanup), wherever a call returns a value reaching resources (the statement's take and drain), and at assignments that float.
