# bug-538: `collections::get` of a recursive-type element aliases the list's storage; a growing `append` then frees it (use-after-free, SIGSEGV)

Last updated: 2026-09-04
Effort: medium (1h–2h) to close the hole (decline or deep-copy); large if the deep copy is generalised
Severity: HIGH
Class: Memory-safety (use-after-free reachable from ordinary MFBASIC; kills the process with an uncatchable SIGSEGV)

Status: Open (found implementing bug-510's regex matcher; reproduced with the minimal program below on main `aa2121518`)
Regression Test: `tests/rt-behavior/collections/recursive_get_then_grow_rt` (to add) — the program below must print three lines and exit 0

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
- [ ] Add `rt-behavior/collections/recursive_get_then_grow_rt` with the program above
      (RED: SIGSEGV). Audit the arms above; record verdicts.
Commit: —

### Phase 2 — the fix
- [ ] Option 1 gate (or option 2 deep copy) in the collection assign/read lowering.
Commit: —

### Phase 3 — regenerate + validate
- [ ] `regen-ncodesum.sh` under bash; `artifact-gate all`; full `cargo test`.
Commit: —

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
