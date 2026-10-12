# bug-1029: a call through a function value that returns a resource union leaks the 16-byte union box

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: LOW-MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below, asserting `live_bytes 0`

`LET f = mkH` takes `mkH` (`FUNC mkH AS RES Handle`) as a function value, and `f()` is passed straight to a
parameter. The union box that `f()` returns is never freed, and neither is the 112-byte tombstone of the closed file.
A direct call `which("x", mkH())` does not leak.

**Correct behavior:** the box is registered as a statement-scope temporary and freed at the end of the statement;
`alloc_calls == free_calls`, `live_bytes 0`.

## Failing Reproduction

Measured on 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at 3fa35d543, macOS aarch64):
`N=<n> /tmp/bs19b/bin/probe.sh /tmp/main-dbg-target/release/mfb z_fnval`. N=10: `alloc_calls 42`, `free_calls 22`,
`live_bytes 1280`. N=20: `alloc_calls 82`, `free_calls 42`, `live_bytes 2560`. That is 128 B per call (16 B box plus
the 112 B tombstone). `/tmp/g933/data.txt` is a 6-byte file (`hello` and a newline).

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

FUNC mkH AS RES Handle
  RETURN openOne()
END FUNC
FUNC once() AS Integer
  LET f = mkH
  RETURN which("x", f())
END FUNC
```

## What is known

The bug-1012 fix registers the statement-scope box temporary only for a call to a named function of the program:
`value_yields_fresh_union_box` in `src/codegen/engine/value/builder_values.rs` is fail-closed for every other kind of
call, so a call through a function value gets no temporary. A builtin that returns a resource union goes through the
same fail-closed path by that reading, but the builtin case was NOT measured; no repro was run for it.

Found by the bug-1012 agent while fixing bug-1012 (batch 19).
