# bug-592: an UNBOUND `collections::get`/`getOr` `String` element is never freed

Last updated: 2026-09-12
Effort: small–medium (one more producer to opt into bug-536 shape B; the audit is which collection members hand back a FRESH element and which hand back an alias)
Severity: MEDIUM (unbounded leak in any loop that reads a `String` element without binding it)
Class: Memory / correctness

Status: **FIXED** — landed on main in round 3, merge `6ca7e8b8e` (branch `bug-592-getor-element-owner`)
Regression Test: `tests/runtime/rt_scope_drop_leaks.rs` — `an_unbound_{list,hash_map,scan_map}_getor_{hit,miss}_runs_at_constant_rss`, `a_returned_getor_element_runs_at_constant_rss`, `a_borrowed_get_with_a_fresh_key_operand_runs_at_constant_rss`; positive pins `a_bound_getor_element_still_runs_at_constant_rss`, `an_unbound_get_element_still_runs_at_constant_rss`, `a_borrowed_get_element_still_runs_at_constant_rss`, `every_unbound_collection_element_position_still_produces_the_right_value`; census `every_collections_member_returning_an_element_has_an_ownership_verdict` (`src/codegen/builtins/tests/collections.rs`)

Found while fixing bug-576, and measured to be a **different defect**: it needs no
runtime helper in the program at all, and it survives bug-576's fix.

## The finding

Measured on the bug-576 branch's release binary (so, WITH bug-576 fixed),
200 000 → 400 000 iterations, peak RSS via `/usr/bin/time -l`:

| shape | per call |
| --- | --- |
| `n = n + len(collections::getOr(names, 0, ""))`, `names` from a **literal** | **64 B** |
| the same read into `LET e AS String = collections::getOr(names, 0, "")` | 0 B |
| `names = collections::append(names, os::arch())` with no `getOr` at all | 0 B |

The third row is the control that rules bug-576 out: the helper's own block is
flat now. The first row has no helper in it.

Repro (the leaking one):

```
IMPORT io
IMPORT collections
SUB main()
  MUT n AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 400000
    MUT names AS List OF String = ["aarch64"]
    n = n + len(collections::getOr(names, 0, ""))
    i = i + 1
  END WHILE
  io::print("n=" & toString(n))
END SUB
```

## Where to look

Same mechanism as bug-536 shape B and bug-576: `register_pending_temp`
(`src/codegen/engine/value/builder_values.rs`) frees a bare `String` temp only
with freshness provenance, and the collection `get`/`getOr` lowering never calls
`mark_fresh_string`. A `LET` binding is flat because the bind owns and frees the
block; an unbound read has no owner at all.

`materialize_owned_element` (`src/codegen/memory/owned.rs`) is the obvious site
and is NOT it: its copy arm explicitly excludes `String`
(`is_freeable_flat_value(..) && result.type_ != ParameterType::String`), because a
`String` element is already handed back as an owned fresh block by the `get`
lowering itself. That producer is the one to mark.

## Why it is not a one-liner

The mark is only sound for a member that returns a FRESH block, and this family
has both kinds in it:

* `borrow_get_result` (plan-86 E) makes a read-only `get` return an ALIAS into the
  container's data region — marking one of those is an `arena_free` INTO the
  container. `register_pending_temp` already early-returns while that flag is set,
  so the guard exists, but the audit has to confirm it covers every path.
* the whole surface has to be enumerated, not just `getOr`: `get`, `getOr`,
  `first`, `last`, `pop`, `keys`, `values`, `find`, map iteration — each is either
  fresh or an alias, and a wrong verdict is a wild free rather than a leak.

That audit is the work, exactly as bug-566's per-helper audit was for the runtime
helpers, which is why this is filed rather than folded into bug-576.

## Findings (fix branch `bug-592-getor-element-owner`)

### Re-measured on the base (`integ-576-590`, `d2e1b7fe3`), 200k -> 400k, `/usr/bin/time -l`

Command: `bash /tmp/b592/measure.sh <mfb> <shapes>` (one `WHILE` loop per shape,
element read unbound through `len(..)`).

| shape | base | fixed |
| --- | --- | --- |
| list `getOr` hit | 64 B | 0 B |
| list `getOr` miss | 129 B | 0 B |
| `Map OF String TO String` (hash probe) `getOr` hit / miss | 64 / 128 B | 0 / 0 B |
| `Map OF Scalar TO String` (entry scan) `getOr` hit / miss | 65 / 130 B | 0 / 0 B (after the 4th join mark) |
| unbound `get`: list / hash map / scan map | 0 / 0 / 0 B | 0 / 0 / 0 B |
| `LET`-bound `getOr` | 0 B | 0 B |
| append only, no read | 0 B | 0 B |

### The real mechanism: the mark was set, then overwritten

`emit_load_payload_with_stride`'s `String` arm already materializes the found
element through `emit_materialize_string_from_bytes`, which DOES call
`mark_fresh_string`. `get` was flat because of that: its miss path raises, so the
last mark set is the found block, which is also `result`. `getOr`'s miss path runs
`emit_copy_owned_string` (emitted after the found path) and moves the copy into
`result`. The copy's own materializer overwrote the mark with the COPY's register,
so `lower_value`'s identity test (`block == result.location`) failed and nothing
freed the `String`, on either path.

Fix: `mark_fresh_element_result` marks `result` at the JOIN label, in all six
get/getOr emission paths: `lower_list_get_common` (list `get` and `getOr`), plus
`lower_map_get` and `lower_map_get_or`, each with a hash arm and a scan arm. A
non-`String` element is never marked.

### Enumeration: every member that can hand back a bare element

From the registry (`every_collections_member_returning_an_element_has_an_ownership_verdict`,
which computes the set and fails on any member without a verdict):

| member | returns | verdict |
| --- | --- | --- |
| `get` (list, map) | `T` / `V` | FRESH on every path: found = the `String` arm's own allocation; miss raises |
| `getOr` (list, map) | `T` / `V` | FRESH on every path: found = own allocation; miss = `emit_copy_owned_string` of the default |
| `reduce`, `reduceRight` | `Arg(1)` (accumulator) | not an element read: the callback's return, owned by the callee's `function_returns_fresh_string` promise (bug-536 B-2) |

Named in the report but not element producers: `first`/`last`/`pop` do not exist
in `collections`. `find`/`findIndex`/`findLastIndex` return `Integer`.
`keys`/`values` return a whole `List` (freeable-flat, no `String` provenance
needed). `FOR EACH` binds a loop variable, and bug-571 frees it per iteration only
when its pointer differs from the container's alias base (a runtime compare). The
interior `String` loads in `sort`/`sortBy`/`groupBy`/`forEach` are stored or freed
by their own lowering and never become a `get` result.

### Alias side: every path that can alias

* **The payload arms that alias** (inline record/union slot, flat nested
  collection) are non-`String`. They are never marked, and `register_pending_temp`
  ignores the mark for non-`String` types.
* **plan-86 E `borrow_get_result`.** `is_borrow_get` requires a freeable-flat,
  NON-`String` element, so a `String` get is never the borrowed node. While the flag
  is set, BOTH registration entry points early-return: `register_pending_temp`,
  which `register_raw_member_result_temp` also routes through, and
  `register_call_result_payload_temp`. No other consumer of `fresh_string_block`
  exists (`grep -rn fresh_string_block src`: `builder_values.rs` plus the struct
  initializers). So a `String` `getOr` nested inside a borrowed initializer (a map
  key) is not registered: it leaks rather than being freed.
* **The caller's default** is never the result: the miss path frees its copy. The
  value probe reads `dflt` back after 5000 x 5 misses.

### Audit finding: the borrow guard OVER-covered (a second leak, fixed here)

`borrow_get_result` was set for the whole initializer lowering, so it suppressed
statement-scope registration for every OPERAND of the borrowed call too. A borrowed
`LET k AS Shape = collections::get(byShape, collections::getOr(names, 0, "none"))`
(used only as a `MATCH` scrutinee) leaked the key `String` at 64 B per call on the
join-point fix alone (`borrow_nested` shape, 200k -> 400k: 13.9 MB -> 26.8 MB). The
plain borrowed read (`borrow_plain`) was flat at 0 B. It failed closed (a leak, not
a wild free), but it was a leak.

Fix: the `Bind` arm now ARMS `borrow_get_armed`, and `lower_value` moves the armed
bit into `borrow_get_result` for its own frame only, clearing it for every nested
frame and restoring it on exit. Both readers (`materialize_owned_element` and
`register_pending_temp` / `register_call_result_payload_temp`) see the same
narrowed value, so a nested `get` operand is COPIED exactly when it is FREED. The
unsafe split would be alias-but-freed. Soundness rests on the borrowed node's
`materialize_owned_element` running in that node's OWN frame. Traced:
`lower_value_inner`'s `Call` arm -> `try_abi_inline_lower` lowers each argument in
its own `lower_value` frame (`lower_abi_inline_args`), then calls `lower_get` in
the `Call`'s frame. No path re-lowers the `Call` node through `lower_value`.
`collect_borrow_get_locals` admits only `NirValue::Call`, so the `CallResult`/TRAP
raw path never carries the flag.

### Contract

`mfb spec language memory-semantics` §14.6: "Reads produce owned values, not
aliases into the buffer." The fix only ADDS a statement-scope free for a block
that already had exactly zero owners. A bound, reassigned, returned or appended
element CLAIMS the pending temp (moved, not copied), so no lifetime moves.

## What a fix must produce

The first row above flat at N and 2N; the `LET`-bound and append-only rows still
flat (no double free); and a value probe that reads every element shape back in a
churning loop — a block freed while the container still points into it shows up as
a wrong value or a later allocation failure, never as a failing free.
