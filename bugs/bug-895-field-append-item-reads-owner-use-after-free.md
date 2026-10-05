# bug-895: `r = WITH r { xs := collections::append(r.xs, r.s) }` copies the item from the freed owner block (garbage / segfault)

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: tests/rt-behavior/memory/field-grow-item-reads-owner-rt (proposed)

A growing in-place field self-update whose item operand is a sibling field of the
same owner record reads that item out of the owner's OLD block after the grow has
freed it. `r = WITH r { xs := collections::append(r.xs, r.s) }` (and `prepend`,
`insert`, a `Map` field's `set` with a key or value read from the owner, and the
same with an inlined record item, and a `Set` field's `add`) stores poisoned/garbage bytes as the new
element, or segfaults. It reproduces at `-O0`, `-O1`, `-O2`; on a local, a
module-level global, and through a `LET alias = r.s` copy that the `-O1` "place
copy forwarding" row deletes. The program prints garbage silently — no error.

**Correct behavior:** the new element equals the value of `r.s` at the time of the
statement (`"short"` below), exactly what the copying form
`LET nx = collections::append(r.xs, r.s)` / `r = WITH r { xs := nx }` produces.

References:

- `src/docs/spec/memory/05_collections.md`, *Self-updates*: "The operation's result
  and every other value are exactly what the copying form would give."
- Same file, field bullet "Growth needs the last field": "An operation that can
  reallocate the collection (a growing insert, a repack) grows the owner's block,
  so it runs in place only when the field … is its owner's last inlined field";
  and the `set` bullet: "an item that could still point into the owner
  (`get(r.xs, k)` itself) rebuilds, because the repack frees the old owner before
  the item is copied." The same hazard is unguarded for `append`/`insert`/
  `prepend`/map `set`.
- `repeat` already has the right guard: "runs in place only at the owner's last
  inlined field with no operand reading the owner (`field_realloc_admitted`)".
- Found during hardening batch B07 (memory spec review).

## Failing Reproduction

```
IMPORT io
IMPORT collections

TYPE R2
  n AS Integer
  s AS String
  xs AS List OF String
END TYPE

FUNC main() AS Nothing
  MUT w AS R2 = R2[1, "short", ["a", "b", "c"]]
  w = WITH w { xs := collections::append(w.xs, w.s) }
  io::print("[" & collections::get(w.xs, 3) & "]")
END FUNC
```

`/tmp/hardening/mfb build && ./build/<name>.out` (also with `-O0`)

- Observed: `[>���V]` (`-O1`), `[$+��i]` (`-O0`) — random bytes each run.
- Expected: `[short]`

Other observed shapes (all from `/tmp/hardening/B07/fld3`, `fld5`, `fld7`):

| statement (owner record `r`, last inlined field `xs`/`m`/`ps`) | observed |
| --- | --- |
| `r = WITH r { xs := collections::prepend(r.xs, r.s) }` | garbage element |
| `r = WITH r { xs := collections::insert(r.xs, 1, r.s) }` | garbage element |
| `g = WITH g { xs := collections::append(g.xs, g.s) }` (module global) | garbage element |
| `LET alias = r.s` / `r = WITH r { xs := collections::append(r.xs, alias) }` | garbage at `-O1`/`-O2`, correct at `-O0` (placefwd forwards `alias` to `r.s`) |
| `r = WITH r { m := collections::set(r.m, r.s, r.s) }` (`Map OF String TO String`) | key not found, `len` grows — garbage key stored |
| `r = WITH r { m := collections::set(r.m, 2, r.s) }` (`Map OF Integer TO String`) | garbage value |
| `c = WITH c { ps := collections::append(c.ps, c.p) }` (`p AS Pt`, `ps AS List OF Pt`) | `n=5906504886769538728`, then **Segmentation fault** reading `.name` |
| `RETURN WITH r { xs := collections::append(r.xs, r.s) }` in `FUNC addSelf(r AS R2) AS R2`, called `r = addSelf(r)` (`/tmp/hardening/B07/al3`) | garbage element |
| same `RETURN WITH` over a local `MUT r` built in the callee | garbage element |
| nested path `o = WITH o { inner := WITH o.inner { xs := collections::append(o.inner.xs, o.inner.s) } }` (`/tmp/hardening/B07/nest2`) | garbage element |
| nested path, item from the OUTER owner: `… collections::append(o.inner.xs, o.tag) …` | garbage element |

Contrast cases that are correct today (bound the bug):

- `LET nr = WITH r { xs := collections::append(r.xs, r.s) }` (not a self-update) → `notlast`.
- `LET l2 = collections::append(r.xs, r.s)` → correct.
- Item that builds a new value: `collections::append(r.xs, r.s & "!")` → correct.
- `collections::set(r.xs, i, r.s)` on a `List OF String` field → correct (has its own read-by check).
- `collections::repeat(r.xs, r.s, 2, 1)` → correct (uses `field_realloc_admitted`).
- `collections::append(r.xs, collections::get(r.xs, 0))` → correct (`get` materializes a copy).
- Literal item `collections::append(r.xs, "lit")` → correct.

## Root Cause

The field arms guard only the whole-field self-alias (`G12`: `site.is_self(arg)`,
i.e. `append(r.xs, r.xs)`) and `G17` (`field_is_last_inlined`). They never ask
whether the item operand *reads the owner* (`SelfUpdateSite::read_by`). The item
is lowered with `lower_value_stored`, which does not copy a flat value, so `r.s`
(or an inlined record field) is an interior pointer into the owner's block. The
grow then goes through `InlineGrow` (`lower_inplace_inlined_list_grow` /
`lower_list_splice_in_place` / `lower_map_set_in_place`), which reallocates the
owner and `arena_free`s the old block — the free entropy-scrubs it — before the
item bytes are copied in.

- `src/codegen/collection/assign/builder_inplace_assign.rs:lower_field_append` —
  checks `site.is_self(&target.args[1])` and `field_is_last_inlined`, not `read_by`.
- `src/codegen/collection/assign/builder_inplace_assign.rs:lower_field_splice`
  (`insert`/`prepend`) — same.
- `src/codegen/collection/assign/builder_inplace_assign.rs:lower_field_set`, map
  arm (after the `G17` check for a new key) — key and value are lowered with no
  `read_by` check.
- The `RETURN WITH r { f := op(r.f, …) }` site reaches the same arms:
  `src/codegen/engine/control/builder_exits.rs:try_returned_field_self_update`
  builds a `SelfUpdateSite` and calls `try_inplace_self_update`, so it inherits
  the missing check.
- The correct guard exists: `src/codegen/collection/assign/inplace_dest.rs:field_realloc_admitted`
  (`field_is_last_inlined(site) && !operands.iter().any(|v| site.read_by(v))`),
  used by the `repeat`, union/merge and rewrite arms; the `List` arm of
  `lower_field_set` has an equivalent hand-written check.

## Goal

- Every growing field arm declines (rebuilds) when any non-receiver operand
  `site.read_by` the owner, or copies such an operand before the grow; all rows of
  the table above print the copying-form value; no segfault.

### Non-goals (must NOT change)

- The in-place path for items that do not read the owner (`"row" & toString(i)`,
  literals, other locals) must stay in place (`inline_append` label family) —
  `src/codegen/builtins/tests/inplace.rs` pins it.

## Blast Radius

- `lower_field_append`, `lower_field_splice`, `lower_field_set` (map arm) — fixed by this bug.
- `lower_field_set_add` (`Set` field `add`) — same missing check, observed: with `s = strings::repeat("E", 100)`, `a = WITH a { st := collections::add(a.st, a.s) }` leaves `contains(a.st, s) = FALSE` and a 74-byte garbage element (`/tmp/hardening/B07/fld8`); with a 4-byte `s` it happened to print correctly. Fixed by this bug.
- `STATE` fields: `f.state.xs = collections::append(f.state.xs, f.state.s)` six times with a growing `f.state.s` printed every element correctly (`/tmp/hardening/B07/st1`) — not observed to fail; why the STATE arm escapes was not investigated.
