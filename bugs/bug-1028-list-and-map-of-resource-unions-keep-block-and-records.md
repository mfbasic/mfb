# bug-1028: a `List`/`Map` of resource unions that is iterated or read through `collections::get` keeps its block and records

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over each of the three `once()` bodies below, asserting `alloc_calls == free_calls` and `live_bytes 0`

A `List OF RES Handle` or `Map OF String TO RES Handle` (with `UNION Handle` = `fs::File | tcp::Socket`) whose
elements are visited with `FOR EACH`, or read with `collections::get`, keeps its block and the element records after
the function returns. The file descriptors are not the issue; the memory is.

**Correct behavior:** the collection block and its records are freed once at the end of the owning scope;
`alloc_calls == free_calls`, `live_bytes 0`.

## Failing Reproduction

Measured on 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at 3fa35d543, macOS aarch64):
`N=<n> /tmp/bs19b/bin/probe.sh /tmp/main-dbg-target/release/mfb <name>` (`mfb build -q --debug`, then the binary;
counters on stderr at exit). `/tmp/g933/data.txt` is a 6-byte file (`hello` and a newline).

| probe | N=10 live_bytes | N=20 live_bytes |
|---|---|---|
| `n_foreach` | 1920 | 3840 |
| `m_append_call` | 2560 | 5120 |
| `n_map_union` | 1440 | 2880 |

Reported by the bug-1012 agent with its fix applied (not re-measured here, not on main): 1600 / 3200, 2400 / 4800,
1280 / 2560. The same shapes over `List OF RES fs::File` are reported balanced (also not re-measured here).

All three probes share this prelude; only `once()` differs.

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

```

`n_foreach`:

```
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  RES c AS fs::File = openOne()
  LET hs AS List OF RES Handle = [a, c]
  MUT t AS Integer = 0
  FOR EACH h IN hs
    t = t + which("e", h)
  NEXT
  RETURN t - 1
END FUNC
```

`m_append_call`:

```
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  MUT hs AS List OF RES Handle = []
  hs = collections::append(hs, a)
  RETURN len(hs) + which("g", collections::get(hs, 0))
END FUNC
```

`n_map_union`:

```
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  LET m AS Map OF String TO RES Handle = Map OF String TO RES Handle { "x" := a }
  RETURN which("m", collections::get(m, "x"))
END FUNC
```

## What is known

The bug-1012 agent's account (not traced by the coordinator, and not verified here): `block_held_elsewhere_strict` in
`record_ownership.rs` still applies the stricter, pre-bug-1000 ownership rule to any collection that reaches a
resource union. Bug-1012 removed the defect that rule was written around, so the rule can now be measured and probably
removed. Removing it is a guess until measured.

Found by the bug-1012 agent while fixing bug-1012 (batch 19).
