# bug-753: a `List OF RES` bound from a call leaks its collection block on every bind

Last updated: 2026-10-03
Effort: medium
Severity: MEDIUM
Class: Memory (unbounded leak in a loop)

Status: Fixed (2026-10-03, bug-staging-7)
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

## Progress (2026-10-03, after bug-755)

Fixed, measured with `--debug` peak live bytes at N=500/1000 (`/tmp/b753u`):

| shape | before | after |
| --- | --- | --- |
| user function returning `take(xs, 2)` / `[h, h]` / an accumulated local | 768 B/iter together | flat |
| `out = collections::append(out, collections::get(xs, i))` (also on main) | 336 B/iter | flat, allocs = frees |
| `filter` / `removeAt` / `mid` bound to a new name | 192 / 144 / 96 | 0 |
| `append` / `prepend` / `insert` / `set` bound to a new name | 336 / 336 / 336 / 432 | 0 |

- **User functions.** `record_ownership::CollectionUses::user_call_returns_fresh_block`:
  a call is a fresh block when every `RETURN` of the callee is a literal, a call
  already trusted fresh, a fresh user call, or a local whose every store is fresh,
  that no call keeps, and that is named only in `RETURN`s. A parameter never
  qualifies.
- **The in-place append declined on the `RES` marker.** G11 in
  `try_inplace_append_assign` compared the bare `fs.File` of `collections::get` with
  the `RES fs.File` element and sent the append down the copying path. It now
  compares without the marker. The copying path also frees the block it replaces
  when the collection is proved its block's sole holder.
- **Native members** (`FRESH_NATIVE_MEMBERS`): `filter`, `append`, `prepend`,
  `insert`, `set` (list and map), `removeAt`, `mid` — each audited to return a block
  it allocated on every successful path (no identity shortcut; citations on the
  constant). `free_intermediate_collection` now also frees a `RES` collection's
  intermediates (the singleton an element append builds, `set`'s removed copy).
- **`chunks` / `window`** bodies are listed in `FRESH_BUILTIN_BODIES` (their native
  fast paths serve only numeric and `String` elements).

Known residue, by design: a list a user function keeps (`same(xs) … RETURN xs`) is
not proved its block's sole holder (bug-801), so neither it nor the alias the call
returns frees that one block — bounded per scope, never per iteration.

**Remaining:** `chunks`/`window` still leak 480/528 B/iter: their result is a
`List OF List OF RES T`, which `is_res_marked_resource_collection` does not
recognize (its element is a list, not `RES`), so no path frees the outer block or
the inner pieces. Freeing them needs a nested-`RES`-collection block free
(outer block plus each inner block, no element drops).

Regression tests: `rt_res_list_from_call_leak`
(`a_res_list_bound_from_a_user_function_frees_its_block`,
`a_res_list_bound_from_a_native_member_frees_its_block`; the member test fails on
main: peak 952 864 vs 1 888 864).

## Resolution (2026-10-03, bug-staging-7)

The nested remainder. A `List OF List OF RES T` holds its inner lists BY POINTER
(bug-801's reproduction shows a freed piece corrupting the outer list), so the
outer block and each inner block are separate allocations. Measured on main
(`--debug` counters, 300 iterations of `chunks(xs, 2)` + `window(xs, 2)` over 4
handles): 2724 allocations, 621 frees, i.e. 7 blocks per iteration (two outer
blocks, the outer blocks the bodies' `result = append(result, piece)` replaced, and
the five pieces).

- `CodeBuilder::is_res_collection_block` extends the sole-holder block free to a
  list of `RES`-marked collections (the outer block).
- `record_ownership::block_collections`:
  - reassign-free: a copying reassignment frees the replaced block when the
    collection is fresh, unkept, unread, and named outside an argument only by
    `RETURN`. This is what the `chunks`/`window` bodies need: their `result` is
    returned.
  - inner-owning: a sole-holder nested binding whose every store is
    `chunks`/`window` (audited: each fresh slice is appended once) frees each inner
    block first (`emit_inner_block_frees`).

After: 2724 allocations, 2721 frees. The 3 left over come from a one-off temporary
in the program's last statement, so 0 blocks leak per iteration and
`double_free_skips` is 0.

Residue, by design (fail-safe): a nested list the program builds itself frees its
outer block only, since its inner pieces may have another holder; and a nested
result an element is read back from (`collections::get(parts, i)`) is not a sole
holder, so it keeps leaking.

Regression test: `rt_res_list_from_call_leak::a_nested_res_list_from_chunks_or_window_frees_every_block`.
With main's compiler it fails (peak live bytes 377 296 -> 737 296 from 300 to 600
iterations); it passes here.
