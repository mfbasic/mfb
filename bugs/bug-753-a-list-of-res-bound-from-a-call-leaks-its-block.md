# bug-753: a `List OF RES` bound from a call leaks its collection block on every bind

Last updated: 2026-09-27
Effort: medium
Severity: MEDIUM
Class: Memory (unbounded leak in a loop)

Status: Open
Regression Test: none yet — see Fix

Found by the bug-643 lane. A `List OF RES T` bound from a call is never freed, so
a loop that binds one leaks a whole list block per iteration. The handles are not
involved; the leak is the collection block.

## Reproduction

```basic
IMPORT io
IMPORT collections
IMPORT fs
IMPORT os

SUB main()
  fs::writeText("/tmp/b753.txt", "")
  RES a AS fs::File = fs::open("/tmp/b753.txt", "read")
  LET la AS List OF RES fs::File = [a, a, a]
  LET n AS Integer = toInt(os::getEnv("N"))
  MUT i AS Integer = 0
  MUT total AS Integer = 0
  WHILE i < n
    LET p = collections::take(la, 2)
    total = total + len(p)
    i = i + 1
  END WHILE
  io::print(toString(total))
END SUB
```

`mfb build --debug`, then `N=1000` and `N=2000`: `arena.0.peak_live_bytes` is
144 304 and 288 304 — 144 B per iteration, linear. Identical at `a0082e4ae`
(144 320 / 288 320), so it predates bug-643's fix. The `chunks`/`window` bodies hit it
once per block (`LET piece AS List OF T = __collections_slice(…)` with `T` a `RES`).

## Root cause

A `List OF RES X` is not `is_freeable_flat_value` (bug-645's note in
`setup_owned_list`: `collection_payload_types` strips the marker and the walk
answers "not flat"), so no `OwnedValue` cleanup is registered for it at a bind.
bug-645/bug-651 added the block free only for collections `record_ownership`
proves fresh — a literal or a self-receiving mutator — and a call result is never
counted fresh, because it need not be: `lower_returned_value` returns a non-flat
parameter's block uncopied (`RETURN xs` hands the caller's own block back), and a
borrowed `collections::get` of a nested `List OF RES` element yields a pointer into
the container. Freeing either would be a double or wild free.

## Fix

Prove freshness per call kind before registering the free: a `collections`
re-packaging member whose native lowering always allocates a new block (audit each
one — `take`/`drop`/`mid`/`slice`, `filter`, `sort*`, `append`/`prepend`/`insert`/
`set`/`removeAt`, `flatten`, `values` — for an identity shortcut that returns the
argument's block), excluding `get`/`getOr`; and a user function only when every
`RETURN` of it yields a fresh block (not a parameter). Pin with a `--debug`
`alloc_bytes`/`peak_live_bytes` bound (`.ai/testing-gates.md`).
