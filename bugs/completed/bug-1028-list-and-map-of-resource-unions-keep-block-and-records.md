# bug-1028: a `List`/`Map` of resource unions that is iterated or read through `collections::get` keeps its block and records

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs21-agent-a, 2026-10-11). See "Resolution" at the end.
Regression Test: `tests/runtime/rt_resource_union_reclaim.rs` — `an_iterated_list_of_resource_unions_frees_its_block`, `a_collection_of_resource_unions_read_through_get_frees_its_block`, `a_list_of_floated_resource_unions_frees_its_block_and_records`, `a_case_binding_that_leaves_keeps_its_list_from_freeing_the_record` (each a `--debug` build read at N=10 and N=40, plus 300 iterations under a 64-descriptor limit); unit tests `record_ownership::owned_list_ownership_tests::a_match_that_only_reads_its_scrutinee_does_not_hold_the_list` and `a_match_whose_case_binding_leaves_holds_the_list`

A `List OF RES Handle` or `Map OF String TO RES Handle` (with `UNION Handle` = `fs::File | tcp::Socket`) whose
elements are visited with `FOR EACH`, or read with `collections::get`, keeps its block and the element records after
the function returns. The file descriptors are not the issue; the memory is.

**Correct behavior:** the collection block and its records are freed once at the end of the owning scope;
`alloc_calls == free_calls`, `live_bytes 0`.

## Failing Reproduction

Measured on 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at 3fa35d543, macOS aarch64):
`N=<n> /tmp/bs19b/bin/probe.sh /tmp/main-dbg-target/release/mfb <name>` (`mfb build -q --debug`, then the binary;
counters on stderr at exit). `/tmp/g933/data.txt` is a 6-byte file (`hello` and a newline).

| probe | N=10 live_bytes | N=20 live_bytes |
|---|---|---|
| `n_foreach` | 1920 | 3840 |
| `m_append_call` | 2560 | 5120 |
| `n_map_union` | 1440 | 2880 |

Reported by the bug-1012 agent with its fix applied (not re-measured here, not on main): 1600 / 3200, 2400 / 4800,
1280 / 2560. The same shapes over `List OF RES fs::File` are reported balanced (also not re-measured here).

All three probes share this prelude; only `once()` differs.

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

```

`n_foreach`:

```
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  RES c AS fs::File = openOne()
  LET hs AS List OF RES Handle = [a, c]
  MUT t AS Integer = 0
  FOR EACH h IN hs
    t = t + which("e", h)
  NEXT
  RETURN t - 1
END FUNC
```

`m_append_call`:

```
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  MUT hs AS List OF RES Handle = []
  hs = collections::append(hs, a)
  RETURN len(hs) + which("g", collections::get(hs, 0))
END FUNC
```

`n_map_union`:

```
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  LET m AS Map OF String TO RES Handle = Map OF String TO RES Handle { "x" := a }
  RETURN which("m", collections::get(m, "x"))
END FUNC
```

## What is known

The bug-1012 agent's account (not traced by the coordinator, and not verified here): `block_held_elsewhere_strict` in
`record_ownership.rs` still applies the stricter, pre-bug-1000 ownership rule to any collection that reaches a
resource union. Bug-1012 removed the defect that rule was written around, so the rule can now be measured and probably
removed. Removing it is a guess until measured.

Found by the bug-1012 agent while fixing bug-1012 (batch 19).

## Resolution (bs21-agent-a, 2026-10-11)

### Root cause

Two rules in `record_ownership.rs` each kept such a collection out of
`owning_collections`, and either one alone is enough to keep the block. The doc's
hint named only the first; removing it alone changes none of the three probes
(measured: 1600 / 2400 / 1280 live bytes at N=10 before and after).

1. **`block_held_elsewhere_strict`** was applied to every collection reaching a
   resource union. It counts a `FOR EACH` over the collection and any
   `collections::get` of it as a second holder.
2. **A `MATCH` counted as keeping its scrutinee.** `MATCH h` desugars to
   `Bind $matchN = Local(h)`, `Match(Local($matchN))` and one
   `Bind f = UnionExtract(Local($matchN))` per case (the optimizer forwards an
   immutable scrutinee, leaving `Match(Local(h))` and `UnionExtract(Local(h))`).
   `CollectionUses` treats every bare local outside an argument position as
   escaping, so `which(label, h AS Handle)` answered "keeps `h`"
   (`callee_keeps`), and a `MATCH` in the loop body put the loop variable in
   `escaping_beyond_iteration`. A resource union can be read no other way, so
   under bug-1000's wider rule the loop variable still "left the loop" and a
   `get` handed to `which` was still an element reader.

Which block: debugger on the base compiler (`n_foreach` without the callee,
`-O0 --debug`, `N=1`, `bp _mfb_arena_alloc list reg x0; bt 2` and the same on
`_mfb_arena_free`). `once` makes three allocations of its own: two 16 B boxes
(`once+548`, `once+688`) and the 152 B list block (`once+928`). Both boxes and
both 112 B records are freed in `once`; the list block never is (9 allocations,
8 frees). So it is the collection's own block that is kept, plus, when the
elements floated into the list, everything its drain would free.

### The fix

`src/codegen/resource/cleanup/record_ownership.rs`:

* `owning_collections` asks `block_held_elsewhere` (bug-1000's rule) of every
  collection; the `union_collection_type` parameter of `record_ownership` and
  `owning_collections` is gone, with its closure in
  `function_lowering.rs`. `block_held_elsewhere_strict` remains for a float
  target that has no bind in the body.
* `CollectionUses::visit_op` reads a `MATCH` as what it is. The scrutinee
  temporary (`is_match_view`: a `$match…` name, which no source can spell) bound
  from a local, or from a borrowed-element read, is a reference to that local or
  to the collection the element came out of; a case binding
  (`Bind x = UnionExtract(Local(s))`) is a reference to `s`; the `MATCH` itself
  reads the tag. Each is recorded in `iterations`, so
  `iterated_reference_leaves` asks of it exactly what it asks of a `FOR EACH`
  variable: bound, returned, captured, stored, read through `get` or kept by a
  callee, it is a second holder and the collection is not freed. Anything else
  naming the temporary falls through to the ordinary "escapes" arm.

No lowering changed: a collection that now qualifies takes the block free and
the freeing drain every other owning collection already takes.

### Measurements (fixed compiler, `N=<n> /tmp/bs21/bin/probe.sh target/release/mfb <name>`)

| probe | N=10 alloc / free / live | N=20 alloc / free / live | double_free_skips |
|---|---|---|---|
| `n_foreach` | 92 / 92 / 0 | 182 / 182 / 0 | 0 |
| `m_append_call` | 62 / 62 / 0 | 122 / 122 / 0 | 0 |
| `n_map_union` | 52 / 52 / 0 | 102 / 102 / 0 | 0 |

`(ulimit -n 64; N=300 <probe>.out)` exits 0 for each (300, 600, 300 printed).

Sibling shapes, same harness, balanced at N=10 and N=40 with
`double_free_skips 0` and the 64-descriptor run passing (base compiler's live
bytes at N=10 in brackets): a `MATCH` in the loop body [1600]; a loop left by
`RETURN` [1600]; `MATCH collections::get(hs, 0)` [2400]; elements floated from a
loop body, iterated [7680] and read through `get` [7680]; a `Map` with floated
values read through `get` [14080]; a list a callee returned, iterated [2080],
and iterated and read [4640]; `collections::transform` with a `LAMBDA` that
calls `which` [4640]; the element handed on through a second function [4640]; a
`WHEN` guard on the case [4640]; the list passed to a function that iterates and
matches it [4640]; a list built in a loop body and left by `CONTINUE FOR` /
`EXIT FOR` [23040].

Unchanged, by design (the collection is not the one owner): a case binding
returned by the callee and kept in an outer list, a loop variable captured by a
`LAMBDA`, an element bound with `RES v AS Handle = collections::get(…)`. Each
runs with the same output and the same counters as before.

Not this bug (each keeps the same bytes with a `List OF RES fs::File`, so the
union is not the cause; repros are in the batch report): `collections::get`
under an inline `TRAP` (96 B), `FOR EACH` over a `Map` read through `e.value`,
a list held in a record field, a list appended to while it is iterated (96 B).

### Golden impact

None. `scripts/artifact-gate.sh target/release/mfb all`: 1674 tests, 2338
goldens checked, 0 diffs. No fixture holds a collection of resource unions that
is iterated or read through `get` in an owning position.

