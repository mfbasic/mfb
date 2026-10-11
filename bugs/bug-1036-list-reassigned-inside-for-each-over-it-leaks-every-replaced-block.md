# bug-1036: a list reassigned inside a `FOR EACH` over it leaks every replaced block

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting `alloc_calls == free_calls` and `live_bytes 0`

A `MUT` list that is reassigned in the body of a `FOR EACH` over that same list keeps every
block the reassignment replaced. The code does this on purpose: the loop's iterator still
reads the old block, so the reassignment skips the free, and nothing frees it later.

**Owner decision (2026-10-10):** there is no intentional memory leak. A block may outlive
its binding only where the semantics require it, and then it is freed afterwards — here,
when the loop ends.

**Correct behavior:** every replaced block is freed once the `FOR EACH` that was reading it
is done; `live_bytes` is 0 at exit.

## Failing Reproduction

Measured 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at
a236fdd44, macOS aarch64): `mfb build -q --debug`, run with `N=10`. Result: stdout `10`,
`alloc_calls 912`, `free_calls 502`, `live_bytes 501600`. Expected `live_bytes 0`.
`/tmp/g933/data.txt` is a 6-byte file.

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

FUNC once() AS Integer
  RES a AS Handle = openOne()
  RES c AS Handle = openOne()
  MUT hs AS List OF RES Handle = []
  hs = collections::append(hs, a)
  MUT t AS Integer = 0
  FOR EACH h IN hs
    RES v AS Handle = h
    FOR j = 1 TO 40
      hs = collections::append(hs, c)
    NEXT
    t = t + which("v", v)
  NEXT
  RETURN t
END FUNC
```

The same loop over a `List OF Integer` (`xs = collections::append(xs, j)` inside
`FOR EACH x IN xs`) does not leak: 82 allocations, 82 frees, `live_bytes 0`, same compiler.
So the leaking path is narrower than "any list"; which element types reach it was not
traced. Part of the 501,600 bytes may be bug-1028's leak (a list of resource unions that
is iterated keeps its block); separate the two before fixing.

## Root Cause

`src/codegen/engine/control/builder_control.rs`, the `NirOp::Assign` arm, excludes a name
in `for_each_iterable_locals` from the old-value free, with the comment "That old block
leaks, but the pattern is rare." `try_inplace_set_assign` has the mirror guard.

## Goal

- The program above ends with `live_bytes 0` and no double free.
- The comment that accepts the leak is gone.

### Non-goals

- Freeing the block while the iterator still reads it (that is a use-after-free).
