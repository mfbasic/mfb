# bug-1000: an owned `List OF RES` that `owning_collections` cannot prove owns its elements (adopted from a call, floated in from an inner block, iterated or read through `get`) leaks the list block and/or the elements' records and I/O buffers

Last updated: 2026-10-06
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the three programs below, asserting flat `live_bytes` from N=10 to N=20

When a `List OF RES fs::File` whose element was **adopted from a call** (`LET xs = mkList()`), or
**floated in from an inner block** (an inner `IF` opens the file and `append`s it into an outer
`MUT xs`), is dropped, the file is closed but its buffered-output block and record are never
freed. The 4 KB buffer is only there once `fs::setBuffered(f, TRUE)` has run. A same-scope
`append` is not affected.

**Correct behavior:** dropping the list frees everything the element owned (the I/O buffer, the
record), as a direct `RES f` binding's drop does. `live_bytes` stays flat across iterations.

Suspected cause (from bs17-agent-f's reading of the code; not traced): the owned-list drain reclaims
the buffer only when `record_ownership::owning_collections` proves the list owns its elements
(`owns_elements`). Adopted and floated lists fail that proof, so the drain only closes.

## Failing Reproduction

Measured by the coordinator on 2026-10-06 with main 468ff0f94 (`/tmp/main-dbg-target/release/mfb`),
`mfb build -q --debug <dir>`, then `N=10` / `N=20 ./<dir>/build/<dir>.out`. Counts are arena 0
`alloc_calls / free_calls / live_bytes`.

| program | N=10 | N=20 | per iteration |
|---|---|---|---|
| leakA (adopted, buffered) | 101 / 71 / 43040 | 201 / 141 / 86080 | +4304 B |
| leakA3 (floated, buffered) | 111 / 81 / 44480 | 221 / 161 / 88960 | +4448 B |

leakA, adopted from a call:
```
IMPORT os
IMPORT fs
IMPORT collections
FUNC openBuffered() AS RES fs::File
  RES f AS fs::File = fs::openFile(fs::pathJoin([fs::tempDirectory(), "mfb-leakA.txt"]), "w")
  fs::setBuffered(f, TRUE)
  RETURN f
END FUNC
FUNC mkList() AS List OF RES fs::File
  RES f AS fs::File = openBuffered()
  RETURN [f]
END FUNC
FUNC body() AS Nothing
  LET xs = mkList()
  fs::writeAll(collections::get(xs, 0), "L")
END FUNC
FUNC main() AS Integer
  MUT i AS Integer = 0
  WHILE i < toInt(os::getEnvOr("N", "10"))
    body()
    i = i + 1
  END WHILE
  RETURN 0
END FUNC
```

leakA3, floated from an inner block (same `main` as leakA):
```
IMPORT os
IMPORT fs
IMPORT collections
FUNC body() AS Nothing
  MUT xs AS List OF RES fs::File = []
  IF len("x") = 1 THEN
    RES f AS fs::File = fs::openFile(fs::pathJoin([fs::tempDirectory(), "mfb-leakA3.txt"]), "w")
    fs::setBuffered(f, TRUE)
    xs = collections::append(xs, f)
  END IF
  fs::writeAll(collections::get(xs, 0), "L")
END FUNC
FUNC main() AS Integer
  MUT i AS Integer = 0
  WHILE i < toInt(os::getEnvOr("N", "10"))
    body()
    i = i + 1
  END WHILE
  RETURN 0
END FUNC
```

bs17-agent-f also measured an unbuffered leakA (`setBuffered` removed): 91 / 71 / 2080 at N=10,
181 / 141 / 4160 at N=20, so +208 B per iteration (the record plus the list's node and block).
The coordinator has not re-measured that variant.

### A second trigger: the list is iterated or read through `get` (bs17-agent-g, re-measured by the coordinator on main 468ff0f94)

A same-scope `List OF RES` local that is iterated with `FOR EACH`, or read through
`collections::get`, never frees its own list block (232 B + header per call). `owning_collections`
refuses a list that is a `FOR EACH` iterable or named by `collections::get`, so
`register_res_collection_block_free` never runs. When the element was floated into such a list, the
drain also stops reclaiming the File record and its read buffer (16,736 B per call, agent-g's
`/tmp/g933L`, MODE=0; not re-measured by the coordinator). `--debug`, arena 0:
N=10 gives 42 alloc / 32 free / 2400 live bytes; N=100 gives 402 / 302 / 24000. Without the
`FOR EACH` (no read of `xs`) it is flat. It needs an existing `/tmp/g933/data.txt`.

```
IMPORT io
IMPORT os
IMPORT fs
IMPORT collections
FUNC once() AS Integer
  MUT xs AS List OF RES fs::File = []
  RES f AS fs::File = fs::openFile("/tmp/g933/data.txt")
  xs = collections::append(xs, f)
  MUT n AS Integer = 0
  FOR EACH h IN xs
    n = n + 1
  NEXT
  RETURN n
END FUNC
FUNC main() AS Nothing
  MUT t AS Integer = 0
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    t = t + once()
  NEXT
  io::print(toString(t))
END FUNC
```

The fix should make the ownership proof accept a list that is only read, iterated or borrowed from
(that does not make it "held elsewhere"), so that both the list block and every element's record
and buffers are reclaimed.

## Related

- bug-1001 (the same buffer leak through a resource-union drop). Found by bs17-agent-f while fixing
  bug-843; reproduced by the coordinator.
