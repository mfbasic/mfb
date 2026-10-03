# bug-753: a `List OF RES` bound from a call leaks its collection block on every bind

Last updated: 2026-10-03
Effort: medium
Severity: MEDIUM
Class: Memory (unbounded leak in a loop)

Status: Partially fixed (2026-10-03): `take`/`drop` (and `chunks`/`window` internal slices) fixed. Remaining: native re-packaging members (audit), and user-function results, which are blocked on bug-755.
Regression Test: `tests/runtime/rt_res_list_from_call_leak.rs` (`--debug` peak live bytes flat across N/2N, allocs = frees, list and handle intact)

Found by the bug-643 lane. A `List OF RES T` bound from a call is never freed, so
a loop that binds one leaks a whole list block per iteration. The handles are not
involved; the leak is the collection block.

## Reproduction

```basic
IMPORT io
IMPORT collections
IMPORT fs
IMPORT os

FUNC main() AS Nothing
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
END FUNC
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

## Progress (2026-10-03)

**Fixed:** call results from audited built-in bodies.
`record_ownership::collection_store_is_fresh` now counts a call whose target
starts with one of `FRESH_BUILTIN_BODIES` as a fresh block:

- `#collections_slice$`: the native `try_inline_slice_op` has one path, which
  allocates and range-copies. Its declining fallback is the source FUNC, which
  builds from `[]` by appends.
- `#collections_take$` / `#collections_drop$`: `RETURN __collections_slice(…)`, no
  fast path of their own.

A `List OF RES` bound from them now has its block freed at drop. The `piece`
the `chunks`/`window` bodies bind from `__collections_slice` is appended into the
result, so freeing it was a use after free (bug-801); with bug-801's fix it is
counted as kept and the result list owns it. Proof:
on main the repro's peak live bytes are 304 864 → 592 864 (N=1000 → 2000).
With the fix they are flat and allocs = frees. The list and the handle still
read afterwards.

**Not fixed, with the reason for each:**

1. **User-function results.** I implemented the doc's rule ("every `RETURN`
   yields a fresh block") and then withdrew it. Main already fails at cleanup
   for those shapes ("Resource handle is already closed": a binding of a user
   call adopts and closes handles the callee was passed, which is bug-755). The
   block free stacked on that adoption turned the cleanup failure into SIGSEGV
   (`/tmp/b753v`, variants `p`/`r`). This waits for bug-755's design.
2. **Native members** (`filter`, `append`/`prepend`/`insert`/`set`/`removeAt`
   bound to a different name, `values`) and the fast-path bodies (`sort*`,
   `flatten`, `chunks`/`window` results). Each needs its native lowering audited
   for an identity shortcut before it can join the allowlist. A fast path
   replaces the body at the call, so the body's `RETURN`s prove nothing.

Related checks: `rt_debug_soak`, `rt_scope_drop_leaks` and
`rt_native_write_overload_call_argument` pass. The resource/collection
acceptance mismatch set is identical to main's (the environment's relative-path
fixtures), except for bug-785's stale `ThreadWorker` packages.
