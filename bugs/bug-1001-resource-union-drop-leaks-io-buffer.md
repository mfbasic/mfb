# bug-1001: dropping a resource-union binding bound straight from a producer leaks the variant's I/O buffer (4,096 B per drop)

Last updated: 2026-10-06
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over leakB3 below, asserting flat `live_bytes` from N=10 to N=20

`RES u AS Stream = openBuffered()`, where `Stream` is a union with an `fs::File` variant and the
file is buffered (`fs::setBuffered(f, TRUE)`), leaks exactly one 4 KB block (the output buffer)
each time `u` is dropped. The alias form (`RES f = …` then `RES u AS Stream = f`) does not leak,
because the concrete `f` binding does the closing and its drop reclaims the buffer.

**Correct behavior:** the union drop frees the variant's I/O buffer, as the concrete binding's
drop does.

Suspected cause (from bs17-agent-f's reading of the code; not traced): the resource-union drop
(`emit_union_tag_dispatch_drop`) closes the variant and frees `STATE`, but never calls the
I/O-buffer reclaim.

## Failing Reproduction

Measured by the coordinator on 2026-10-06 with main 468ff0f94, `mfb build -q --debug`, arena 0
`alloc_calls / free_calls / live_bytes`:

| program | N=10 | N=20 | per iteration |
|---|---|---|---|
| leakB3 (union bound from the producer) | 91 / 81 / 40960 | 181 / 161 / 81920 | +4096 B, 1 block |
| leakB (alias form, control) | 91 / 91 / 0 | 181 / 181 / 0 | flat |

leakB3:
```
IMPORT os
IMPORT fs
UNION Stream
  fs::File
END UNION
FUNC openBuffered() AS RES fs::File
  RES f AS fs::File = fs::openFile(fs::pathJoin([fs::tempDirectory(), "mfb-leakB3.txt"]), "w")
  fs::setBuffered(f, TRUE)
  RETURN f
END FUNC
FUNC body() AS Nothing
  RES u AS Stream = openBuffered()
  MATCH u
    CASE fs::File(f)
      fs::writeAll(f, "U")
  END MATCH
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

## Related

- bug-1000 (the same buffer leak through an owned-list drain).
