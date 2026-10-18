# bug-1104: `collections::window` with a run-time stride near the Integer maximum raises an undeclared `ErrOverflow`, located inside the stdlib body

Last updated: 2026-10-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (declared errors)

Status: Open
Regression Test: none yet

Found by bs24-agent-e while fixing bugs 1007/1008, reproduced by the bug-staging-24
coordinator, and placed in a later phase.

**The single correct behavior a fix produces:** the program below prints `1` (one window,
`["a"]`), as the native fast path now does for the same constant stride, and as
`mfb man collections window` describes; no error escapes that `window` does not declare
(`ErrOutOfMemory`, `ErrInvalidArgument`).

## Reproduction (main `7bb985ed8`)

```
IMPORT collections
IMPORT io
IMPORT os

FUNC main() AS Nothing
  LET st AS Integer = toInt(os::getEnvOr("STRIDE", "9223372036854775807"))
  LET w AS List OF List OF String = collections::window(["a", "b", "c"], 1, st)
  io::print(toString(len(w)))
END FUNC
```

```
Error:    7-705-0010
Message:  Arithmetic overflow or numeric conversion outside the destination range.
Location: src/main.mfb:280:11
```

The location is a line of the assembled stdlib source, not the caller's line 7.

## Where

The generic `__collections_window` body advances with `i = i + stride`, which overflows
before the loop test sees the end of the list. bug-1008's fix made the native fast path's
loop overflow-free (`start <= n - size`, step only while `stride <= (n - size) - start`); the
generic body needs the same test.
