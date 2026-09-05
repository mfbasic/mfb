# bug-538: `collections::get` of a recursive-type element aliases the list's storage; a growing `append` then frees it (use-after-free, SIGSEGV)

Last updated: 2026-09-04
Effort: medium (1h–2h) to close the hole (decline or deep-copy); large if the deep copy is generalised
Severity: HIGH
Class: Memory-safety (use-after-free reachable from ordinary MFBASIC; kills the process with an uncatchable SIGSEGV)

Status: **FIXED** (2026-09-04). Fixed by making `collections::get` return the
independent value `mfb spec language memory-semantics` §14.6 has always required,
via the per-type runtime deep copy. Reproducing it also exposed a **second,
pre-existing memory-safety defect** that had to be fixed first — the thread-transfer
deep copy of a non-flat `Map`/`Set` never reserved the hash-bucket region (see
"Second defect" below).
Regression Test: `tests/rt-behavior/collections/recursive-get-then-grow-rt`
(acceptance) + `tests/rt_recursive_get_alias.rs` (cargo, negative + positive) +
`tests/rt_recursive_map_transfer.rs` (cargo, the second defect)

A value read out of a `List OF T` with `collections::get`, where `T` participates in a
type cycle (a record holding a recursive union, or the union itself — `json::Json`,
`__regex_Cont`, `canvas::DrawItem`, any user `TYPE` that mentions itself through a
`List OF`), is **not an independent copy**: its recursive field points into the list's
own data region. `.ai/collections.md` records that (plan-121's gate `G24` declines
in-place `removeAt` for exactly this class because a compaction moves the bytes a
fetched value points into). But `append` on the same list is not declined, and when
the append **grows** the list it reallocates the data region and frees the old block
(`emit_free_pre_grow_buffer`). The fetched value now points into freed memory; the
next read of its recursive field is a use-after-free. The single correct behaviour a
fix produces: **a value returned by `collections::get` stays valid whatever is done to
the list afterwards** — as it already does for every non-recursive element type.

References:

- `.ai/collections.md` — "There is a SECOND aliasing surface, and only payload-relocating ops hit it" (G24) and "get read-only borrow" (plan-86 E).
- `bugs/bug-536-…` — the sibling recursive-type defect (values never freed). Together they say: recursive types are second-class in the collection codegen — never freed, and not independently copied.
- Found during `bugs/bug-510-text-decoder-dos-cluster.md`: the first explicit-stack matcher kept its `__regex_Repeat` choice records in a `List OF __regex_Repeat`, fetched one on backtrack, and pushed (appended) again while it was live — `(a|b)+?c` on `abc` died of "Allocation failed" (the freed block's size word, read as a copy length).

## Failing Reproduction

```
IMPORT io
IMPORT collections

TYPE Leaf
  v AS Integer
END TYPE

TYPE Node2
  left AS Tree
  right AS Tree
END TYPE

UNION Tree
  Leaf
  Node2
END UNION

TYPE Rep
  child AS Tree
  lo AS Integer
END TYPE

FUNC describe(t AS Tree) AS String
  MATCH t
    CASE Leaf(l)
      RETURN "Leaf(" & toString(l.v) & ")"
    CASE Node2(n)
      RETURN "Node2(" & describe(n.left) & "," & describe(n.right) & ")"
  END MATCH
END FUNC

SUB main()
  MUT reps AS List OF Rep = []
  reps = collections::append(reps, Rep[child := Node2[left := Leaf[v := 1], right := Leaf[v := 2]], lo := 3])
  LET back AS Rep = collections::get(reps, 0)
  io::print("before growth: " & describe(back.child))
  MUT i AS Integer = 0
  WHILE i < 50
    reps = collections::append(reps, Rep[child := Leaf[v := 100 + i], lo := i])
    i = i + 1
  END WHILE
  io::print("after 50 appends: list has " & toString(len(reps)))
  io::print("fetched-before-growth value now reads: " & describe(back.child))
END SUB
```

- Observed (`mfb build` + run, macOS-aarch64, main `aa2121518`): prints
  `before growth: Node2(Leaf(1),Leaf(2))` and `after 50 appends: list has 51`, then
  **exit 139 (SIGSEGV)** on the third line.
- Expected: the third line prints `Node2(Leaf(1),Leaf(2))` and the program exits 0.

Contrast cases that work today: the same program with `Rep` replaced by a flat record
(`child AS Integer`) — `get` copies; the same program without the growth loop —
`back.child` reads fine because the storage is still live; re-fetching element 0
after the growth reads fine. The dangling value need not be used directly: storing it
inside another value (a union constructor) and reading that later fails the same way,
which is how bug-510 met it.

## Fix (landed 2026-09-04)

**Option 2 of the Fix Design** — `get` returns a real, independent copy — for the
reason the doc gave: it is sound on its own, it does not depend on bug-536's leak,
and it is the right end state. Option 1 (decline the in-place `append`) was
rejected: it treats the symptom, it leaves the alias in place for every other
relocating arm, and its soundness rested on recursive values never being freed.

### The change

`materialize_owned_element` (`src/codegen/memory/owned.rs`) gains one arm, ahead
of the existing flat-copy arm and *after* the plan-86 E borrow early-return:

```
if !is_freeable_flat_value(t) && type_reaches_cycle(t) && !type_contains_resource(t)
    -> copy_value_to_current_arena(t, element)
```

`copy_value_to_current_arena` already routes a cycle-participating type to
bug-391's per-type runtime deep copy (`thread_copy_symbol`), which is emitted for
every recursive type in the module whether or not the program has a thread in it —
so nothing new is emitted, only called.

Two new predicates in `collection/layout/builder_collection_layout.rs`:

- **`type_reaches_cycle`** — `type_` itself participates in a cycle, or some type
  reachable from it does. Strictly wider than `type_participates_in_cycle`, and the
  width is load-bearing: in the reproduction `Tree` and `Node2` participate but
  `Rep` does not, yet a `Rep` still owns a pointer to a `Tree` graph and was
  exactly as alias-prone. (Gate `G24` still uses the narrow predicate; that was a
  second latent hole — `List OF Rep` + `get` + in-place `removeAt` — which the
  `get` copy closes at the source, because after the copy nothing refers into the
  payload at all. The positive test covers that path.)
- **`type_contains_resource`** — excluded deliberately. A handle is move-only, so
  an alias is both the existing and the correct behaviour there (§14.6's own
  carve-out for a `List` element holding a resource pointer).

### Second defect, found while reproducing this one (fixed in the same commit)

Routing `get` through the deep copy immediately reddened
`rt-behavior/json/json-behavior`: `json::get` began answering `ErrNotFound` for a
key that was present. The deep copy, not the routing, was wrong.

`copy_collection_to_current_arena` (`memory/arena/builder_arena_transfer.rs`) sized
the destination by hand as `HEADER + capacity*ENTRY + dataCapacity` and **omitted a
map's or set's hash-bucket region** — the `capacity << 4` bytes
`emit_reserve_map_buckets` adds to every other allocation path and that
`emit_inlined_block_size_from_ptr_slot`, the single authority, has always included.
It then byte-copied the whole source block over it, so the destination inherited
`BUCKETS_READY = 1` while owning no bucket region: the first probe read past the
block and the lazy `build_buckets` rebuild WROTE past it. bug-02's exact failure
mode, in the transfer copier.

It is **pre-existing and independently reproducible on main `7b0f93c08`**, with no
change of mine — only a *non-flat* map reaches that path (a flat one goes through
`copy_flat_block` → `copy_collection_tight`, which reserves the region and marks it
not-ready), so it needs a map whose value type is recursive, which nothing
transferred before:

```
$ mfb build /tmp/jt && ./build/jt.out     # thread::waitFor a json::Json
v={"u":{"n":"A"}}                          # stringify: correct
g={}                                       # json::get(v, ["u"]): WRONG, silently empty
```

and on a user type (`tests/rt_recursive_map_transfer.rs`):
`v={u:{},v:sC,}` before, `v={u:{n:sA,m:sB,},v:sC,}` after.

Fix: size through the authority (`emit_inlined_block_size_from_ptr_slot`), and
clear `BUCKETS_READY` on the destination so the index is rebuilt on first probe —
exactly what `copy_collection_tight` already does for the same reason. Audited: no
other collection allocator omits the region; every one either calls the authority
or `emit_reserve_map_buckets`.

### Evidence

**1. RED → GREEN, on the actual shape.** The reproduction program below, built and
run in the worktree:

- before: `before growth: Node2(Leaf(1),Leaf(2))` / `after 50 appends: list has 51`
  then **exit 139**;
- after: all eight lines, **exit 0**.

**2. A correct program's observable behaviour is unchanged.** The contract the fix
realizes is `mfb spec language memory-semantics` §14.6: *"`List` and `Map` own every
stored element … Reads produce owned values, not aliases into the buffer."* The
spec was never ambiguous here — the implementation simply did not honour it for one
type class, and `.ai/collections.md` had recorded the alias as a *fact* rather than
as the defect it was (that paragraph is now corrected). The fix only **adds** a
copy at the read; it changes no value's identity, no lifetime, and no user-visible
free (a recursive value is still never freed — that is bug-536, unchanged here, and
its fix will free this copy through the same owner that already owns every other
`get` result).

**3. Golden containment.** `artifact-gate all`: 1371 tests, 1900 goldens, **10
diffs**, all `.ncodesum` — `byte-identity/json` and `byte-identity/regex` on all
five targets. Those are the only two cover fixtures that `get` an element of a
cycle-reaching type (`json::Json`, `__regex_Node`). Everything else byte-identical.
Attributed at instruction level, pre vs post binaries:

| change | fixture | delta |
| --- | --- | --- |
| the `get` copy | json | **0 lines removed**, 16 added: `mov x0,x8 / bl _mfb_thread_copy_json_Json / mov x8,x0` at 4 `get`/`getOr` sites + 4 relocation rows |
| the `get` copy | regex | **0 lines removed**, 24 added: the same 3-op sequence at 6 `__regex_Node` `get` sites |
| the bucket fix | json | confined to exactly two functions: `_mfb_thread_copy_Map_OF_String_TO_json_Json_*` and `_mfb_thread_copy_List_OF_json_Json_*` |
| the bucket fix | regex | confined to exactly one: `_mfb_thread_copy_List_OF__regex_Node_*` |

No register renumbering, no stack-offset shift, no other instruction moved.

**4. Positive pin, not only the negative one.**
`rt_recursive_get_alias.rs::ordinary_recursive_type_use_is_unchanged` builds a
`dom`-shaped recursive union and asserts that construction, `get`, nested field
reads, `FOR EACH`, an iterative `removeAt` tree walk, a re-fetch after growth and a
value read out *before* a growing append all still produce exactly the values they
did before (`render=abc`, `first=t0 last=t39`, `joined=t0…t39tail`, `total=879`,
`refetch=t0`, `len=40 first-still=t0`). The acceptance fixture adds the same shape
at the golden level. `rt_recursive_map_transfer.rs` pins two independent transfers
plus a re-read of the first, so a short block whose bucket rebuild writes past it
would be observable rather than benign.

**5. Full gates.** `artifact-gate all` → 0 diffs. `scripts/test-accept.sh` →
1393/1393 pass. `cargo test --release --no-fail-fast` → green.

## Root Cause

`collections::get` for an element type where `type_participates_in_cycle` holds
(`src/codegen/collection/layout/builder_collection_layout.rs`) is lowered as a copy
of the element's *inline* payload — the record's fixed slots — while a recursive
field inside it is a **pointer** to a block that lives in (or hangs off) the list's
data region; the runtime deep copy that `thread::transfer` uses for such types
(`thread_copy_symbol`, bug-391) is not applied, so the value returned to the program
is a pointer-linked graph sharing storage with the list. `try_inplace_append_assign`
(`src/codegen/collection/assign/builder_inplace_assign.rs`) admits the append; the
grow path of `lower_list_append_in_place` (`collection/list/list_mutate.rs`)
reallocates the data region and frees the old one. Nothing declines the append for
the recursive-element case the way `G24` declines `removeAt`, and nothing keeps the
old block alive.

## Goal

- The reproduction above prints three lines and exits 0.
- No change to any non-recursive element type's codegen (byte-identity for the
  fixtures that hold no recursive element type).

### Non-goals (must NOT change)

- Value semantics of `get` (an independent value) — a fix that documents the alias
  instead of removing it is not a fix.
- The in-place append fast path for flat element types.
- Forbidden wrong fixes: never freeing the pre-grow buffer for recursive lists
  (turns a UAF into a leak, and bug-536 already leaks these); "fixing" the
  reproduction by re-fetching after growth.

## Blast Radius

- Every `List OF <cycle-participating T>` read with `get` and then appended past its
  capacity while the read value is live: `json::JsonArr`'s `List OF Json`
  (a program doing `LET first = get(items, 0)` then building `items` further),
  `canvas` scene lists (`List OF DrawItem` grown after an element was read),
  `__regex_Alt.opts` / `__regex_Concat.parts` (`List OF __regex_Node`; the parser
  reads and appends in the same function — audit `helper_parse_alt.rs`), user code.
- `set` on such a list with a different-width element shifts the data tail
  (plan-121-F) — the same alias class; audit whether the recursive gate declines it.
- `insert`/`prepend` shift only entries, not payloads — the doc says these were
  measured safe; confirm the grow path of insert is not the same realloc.
- Unaffected: flat element types (copied), `Byte`/`Integer`/`String` lists.

## Fix Design

Two sound options; the first is the small one:

1. **Decline in-place `append` (and any other grow/relocate arm) for recursive
   element types**, as `G24` does for `removeAt`. The fallback is the out-of-place
   copying reassignment, which builds a new list and — because recursive values are
   never freed (bug-536) — leaves the old storage alive, so every fetched alias stays
   valid. Cost: O(n) per append for these lists (they are small in practice: parser
   node lists, scene lists), and it depends on bug-536's leak for its soundness — the
   two must be fixed together or this one must be revisited when bug-536 lands.
2. **Make `get` a real copy**: route the element read through the per-type runtime
   deep copy (`thread_copy_symbol`) for cycle-participating element types. Sound on
   its own, independent of bug-536, and the right end state; more codegen.

Expected output shift: `.ncodesum` of every fixture that appends to or reads from a
recursive-element list (json, regex, canvas byte-identity + rt fixtures).

## Phases

### Phase 1 — failing test + audit
- [x] Added `rt-behavior/collections/recursive-get-then-grow-rt` (RED: exit 139)
      plus `tests/rt_recursive_get_alias.rs`. Arm verdicts recorded below.
- [x] `set` (length-changing) and `insert`/`prepend`: after the fix, `get` is no
      longer an alias source for this class at all, so the question they raised is
      answered at the source rather than per-arm. `G24` keeps its `removeAt`
      decline (narrower predicate, harmless), and the positive test exercises an
      in-place `removeAt` on a `List OF Slot` — a reaches-cycle element type `G24`
      does NOT decline — to prove a fetched value survives it.
- [x] `borrow_get_result` (plan-86 E) is disjoint from the new arm: the borrow gate
      requires `is_freeable_flat_value`, which is false for every cycle-reaching
      type. The new arm also sits AFTER the borrow early-return, so the borrow
      always wins.

### Phase 2 — the fix
- [x] Option 2 deep copy in `materialize_owned_element`, plus the pre-existing
      bucket-region defect in the transfer copier it exposed.
Commit: (see below)

### Phase 3 — regenerate + validate
- [x] `bash scripts/regen-ncodesum.sh target/release/mfb`; `artifact-gate all` →
      0 diffs; `test-accept.sh` → 1393/1393; full `cargo test` green.
Commit: (see below)

## Validation Plan

- Regression: the rt fixture above (exit 0, three lines).
- Runtime proof: bug-510's first matcher design (a `List OF __regex_Repeat` side
  table) run against `(a|b)+?c` — recorded in that bug's notes.
- Doc sync: `.ai/collections.md` (extend the G24 paragraph: `append`'s grow path is
  a relocation too).
- Full suite: `cargo test --no-fail-fast -- --skip artifact_gate_all`.

## Summary

A `get` that does not copy plus an `append` that frees: the risk is choosing the fix
that stays sound once bug-536 frees these values. Untouched: every flat element type.
