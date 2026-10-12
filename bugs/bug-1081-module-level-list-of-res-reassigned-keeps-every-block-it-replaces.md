# bug-1081: a module-level `List OF RES` that is reassigned keeps every block it replaces

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting `live_bytes` does not grow between `N=10` and `N=40`

`MUT gF AS List OF RES fs::File = []` at module level, assigned a fresh list on every
iteration (`gF = copyF(files)`, where `copyF` builds and returns a new list of its argument's
elements): the list block the binding held before each assignment is never freed. The
resources are `files`' own and stay open; only blocks are kept. A function-level `MUT` list
assigned the same way is balanced.

The same happens to a module-level `List OF RES Handle` assigned a `List OF RES fs::File`
(`gH = files`), once bug-1018 converts that list: each assignment builds a fresh converted
block and the replaced one is kept (measured on a batch 22 build).

**Owner decisions (2026-10-11):** no leak is intentional (recorded in bug-1036). A module-level
binding holds its VALUE until exit (bug-1068); a value it no longer holds is not covered by
that.

**Correct behavior:** the live bytes at exit do not grow with `N`.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`,
main at b6bf0327b) and the batch 22 staging compiler (15449e66a):
`mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters printed to stderr at
exit. The whole program is `bugs/repro/bug-1081-module-level-list-of-res-reassigned.mfb`; the
part after its shared declarations is shown.

Staging: `N=10` exit 0, stdout `n=20`, `alloc_calls 37`, `free_calls 23`, `live_bytes 2720`;
`N=40` stdout `n=80`, 97 / 53 / 9920. Main: 37 / 22 / 2960 and 97 / 52 / 10160. 240 bytes and
one block per assignment on both.

```
MUT gH AS List OF RES Handle = []
MUT gF AS List OF RES fs::File = []
FUNC convU(files AS List OF RES fs::File) AS List OF RES Handle
  MUT out AS List OF RES Handle = []
  FOR EACH f IN files
    out = collections::append(out, f)
  NEXT
  RETURN out
END FUNC
FUNC copyF(files AS List OF RES fs::File) AS List OF RES fs::File
  MUT out AS List OF RES fs::File = []
  FOR EACH f IN files
    out = collections::append(out, f)
  NEXT
  RETURN out
END FUNC
FUNC idH(h AS Handle) AS Handle
  RETURN h
END FUNC
UNION Shape
  Sq
  Ci
END UNION
TYPE Sq
  s AS Integer
END TYPE
TYPE Ci
  r AS Integer
END TYPE
FUNC main AS Integer
  MUT n AS Integer = 0
  MUT files AS List OF RES fs::File = []
  RES f1 AS fs::File = fs::createTempFile()
  files = collections::append(files, f1)
  RES f2 AS fs::File = fs::createTempFile()
  files = collections::append(files, f2)
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    gF = copyF(files)
    n = n + len(gF)
  NEXT
  fs::writeAll(f1, "still open")
  fs::writeAll(collections::get(files, 1), "still open")
  io::print("n=" & toString(n))
  RETURN 0
END FUNC
```

## What is known

Not traced. Other module-level stores of a fresh aggregate each iteration grow too, and those
also keep the RESOURCES the replaced value reached (language spec 15.6: a resource stored into
a global floats to the module instance and is closed at teardown, "removing a view closes
nothing"); with a fresh file per iteration they run out of descriptors (`x_glob_A` and its
siblings in `/tmp/bs22-agent-a/src`, exit 255 at `N=300` under `ulimit -n 64`). Whether a
replaced global's resources should be released at the assignment is a language question for the
owner; this document is only about the blocks.

Found during batch 22 (reported by bs22-agent-d, reproduced by the coordinator).
