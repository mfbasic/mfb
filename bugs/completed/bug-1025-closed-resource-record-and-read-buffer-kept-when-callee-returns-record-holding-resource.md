# bug-1025: a closed resource record, and its read buffer, are kept when a callee returns a record holding a resource that its caller's callee produced

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs21-agent-a, 2026-10-11). See "Resolution" at the end.
Regression Test: `tests/runtime/rt_resource_union_reclaim.rs` — `a_record_binding_frees_the_resource_record_it_adopted`, `a_copied_record_binding_frees_the_record_and_its_read_buffer`, `a_record_binding_whose_handle_leaves_does_not_free_the_record` (each a `--debug` build read at N=10 and N=40, plus 300 iterations under a 64-descriptor limit); unit tests `record_ownership::owned_list_ownership_tests::a_record_read_through_its_fields_and_copies_is_not_held`, `a_record_whose_handle_or_copy_leaves_is_held`, `a_function_returning_a_record_it_built_returns_a_fresh_block`

`mk2` opens a file in `openOne`, binds it (`RES u AS fs::File = openOne()`), and returns a record that holds it
(`keep2(u)` returns `HB2[h]`). The caller reads through the record and drops it. The file descriptor is closed, but
the record block is never freed; when the file was also read with `fs::readLine`, its read buffer is kept too.

**Correct behavior:** the record, and anything its resource owned (the buffer), are freed once when the caller's
binding goes out of scope; `alloc_calls == free_calls`, `live_bytes 0`.

## Failing Reproduction

`/tmp/g933/data.txt` is a 6-byte file (`hello` and a newline). Measured on 2026-10-10 with main's compiler
(`/tmp/main-dbg-target/release/mfb`, main at 3fa35d543, macOS aarch64), through
`N=<n> /tmp/bs19b/bin/probe.sh /tmp/main-dbg-target/release/mfb x_conc_keep` (`mfb build -q --debug`, then the binary
with `N` set; arena counters print on stderr at exit).

Repro A (`/tmp/bs19b/src/x_conc_keep.mfb`): N=10 gives `alloc_calls 52`, `free_calls 42`, `live_bytes 1120`; N=20
gives `alloc_calls 102`, `free_calls 82`, `live_bytes 2240`. That is 112 B per call, one record block never freed.

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

TYPE HB2
  h AS RES fs::File
END TYPE
FUNC keep2(h AS fs::File) AS HB2
  RETURN HB2[h]
END FUNC
FUNC mk2 AS HB2
  RES u AS fs::File = openOne()
  RETURN keep2(u)
END FUNC
FUNC once() AS Integer
  LET k AS HB2 = mk2()
  RETURN len(fs::readAll(k.h)) - 5
END FUNC
```

Repro B (project `/tmp/bs19-rev/e2`, built with `mfb build -q --debug`, run as `N=10 ./build/e2.out`) has the same shape
with a four-field record (`name`, `h AS RES fs::File`, `n`, `tail`) and reads the file with `fs::readLine` first:
N=10 gives `alloc_calls 142`, `free_calls 122`, `live_bytes 164960`; N=40 gives `alloc_calls 562`, `free_calls 482`,
`live_bytes 659840`. That is 16496 B per call (the record plus the file's read buffer). The descriptors ARE closed:
`(ulimit -n 64; N=300 ./build/e2.out)` exits 0.

```
IMPORT io
IMPORT os
IMPORT fs
IMPORT tcp
IMPORT collections
UNION Stream
  fs::File
  tcp::Socket
END UNION
FUNC openData() AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC
FUNC main() AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    t = t + once()
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC

TYPE HB
  name AS String
  h AS RES fs::File
  n AS Integer
  tail AS String
END TYPE
FUNC keep(nm AS String, h AS fs::File, n AS Integer) AS HB
  RETURN HB[nm, h, n, "tail" & toString(n)]
END FUNC
FUNC mk(n AS Integer) AS HB
  RES u AS fs::File = openData()
  RETURN keep("name" & toString(n), u, n)
END FUNC
FUNC readOne(f AS fs::File) AS Integer
  RETURN len(fs::readLine(f))
END FUNC
FUNC readOneOld(h AS Stream) AS Integer
  MATCH h
    CASE fs::File(f)
      RETURN len(fs::readLine(f))
    CASE tcp::Socket(s)
      RETURN 1000
  END MATCH
END FUNC
FUNC once() AS Integer
  LET k AS HB = mk(3)
  LET junk1 AS List OF Integer = [7, 7, 7, 7]
  LET k2 AS HB = WITH k { n := 9 }
  LET junk2 AS String = "abcdefghijklmnopqrstuvwxyz" & toString(k2.n)
  LET k3 AS HB = k2
  MUT r AS Integer = 0
  IF k.name = "name3" AND k.tail = "tail3" AND k.n = 3 AND k3.n = 9 AND k3.tail = "tail3" AND k2.name = "name3" THEN
    r = 1
  END IF
  RETURN r * 100 + readOne(k3.h)
END FUNC
```

## What is known

Both numbers are the same before and after the bug-1012 fix (measured by the batch 19 agent that fixed bug-1012), so
this is pre-existing and not caused by it. The cause was not traced (guess: the record that `keep2`/`keep` builds from
a parameter resource is not registered as owned by the caller's binding of the call result, so the free of the record
block is skipped while the close of the file is done by the binding `u`).

Found by the bug-1012 agent while fixing bug-1012 (batch 19).

## Resolution (bs21-agent-a, 2026-10-11)

### Root cause

The guess in "What is known" is half right. The record IS registered with the
caller's binding: `LET k AS HB2 = mk2()` adopts the file record (`adopts_from_call`
→ `setup_owned_list` → `emit_owned_list_seed_from_collection`), and `k`'s
owned-list drain closes it. What the drain does not do is free it. A drain frees
what it closes only for a container in `owning_collections`
(`OwnedListCleanup::owns_elements`), and a record binding could never be in that
set, for four reasons, each sufficient:

1. `record_ownership`'s `res_collection_type` admitted a resource-reaching
   aggregate only when it was a collection
   (`typed_is_collection_type(type_) && is_resource_owning_container(type_)`).
2. Its stores were never fresh: `user_call_returns_fresh_block` accepted a
   returned list, set or map literal, not a record constructor (`RETURN HB2[h]`).
3. A record is read through its fields, and `CollectionUses` counted every
   `k.h` as naming `k` outside an argument position, so `fs::readAll(k.h)` made
   `k` "held elsewhere". Repro B also reads fields in comparisons.
4. A copy (`LET k2 = WITH k { … }`, `LET k3 = k2`) named `k` in a bind.

Debugger proof on the base compiler (repro A, `-O0 --debug`, `N=1`,
`bp _mfb_arena_alloc list reg x0; bt 2`, the same on `_mfb_arena_free` with
`x0 x1`, and `bp _mfb_rt_fs_fs_close bt 3`): `fs::openFile` allocates the 112 B
record, `keep2` the 8 B `HB2` block, `once` a 16 B owned-list node (`once+200`).
`fs::close` is called from `once+884`, the drain. `once` then frees the 8 B block
and the 16 B node; nothing frees the 112 B record (7 allocations, 6 frees). With
the fix the same trace shows a 112 B free at `once+1172`, between the two.

Repro B's extra 16,384 B is the file's read buffer, which the same drain frees
with the record once it owns its elements (bug-1001).

### The fix

`src/codegen/resource/cleanup/record_ownership.rs` (and the closure in
`function_lowering.rs`), all in the ownership proof; no lowering changed, so a
record binding that qualifies takes the freeing drain an owning collection
already takes.

* `res_collection_type` admits every resource-reaching aggregate. A record's
  own block is still freed by its `OwnedValue` cleanup; the collection block
  free stays gated on a collection type.
* `user_call_returns_fresh_block`: a returned `Constructor` is a fresh block.
* `CollectionUses::visit_value`: a pure field read (`field_read_root`) written
  straight into an argument is a use of the record it reads through, kept when
  the call keeps or consumes that argument (`collections::append(out, k.h)`, a
  callee that keeps its parameter, a call into unread code) and transient
  otherwise; a borrowed-element read over a field names the record as its
  reader. A field read that is an operand of a comparison is a use (the result
  is a `Boolean`).
* `CollectionUses::visit_op`: a field read bound to a plain scalar is a use; a
  bind of a resource-reaching record from another local or from a `WITH` over
  one is a copy that lives inside its source's scope, recorded in `iterations`
  so `iterated_reference_leaves` asks of it what it asks of a `FOR EACH`
  variable. `CalleeParamUses::record_container` carries the type test; an
  assignment is never a copy in this sense, and a collection bound to a second
  name is never one (the two names share a block).

Anything else that names the record (the handle bound on its own, a `RETURN`, a
capture, a `FOR EACH` over a field) still answers "held", and the drain then
closes without freeing, as before.

### Measurements (fixed compiler, `N=<n> /tmp/bs21/bin/probe.sh target/release/mfb <name>`)

| probe | N=10 alloc / free / live | N=40 alloc / free / live | double_free_skips |
|---|---|---|---|
| `x_conc_keep` | 52 / 52 / 0 | 202 / 202 / 0 | 0 |
| `e2` | 142 / 142 / 0 | 562 / 562 / 0 | 0 |

`(ulimit -n 64; N=300 <probe>.out)` exits 0 for each (300 and 31500 printed).

Sibling shapes, balanced at N=10 and N=40 with `double_free_skips 0` and the
64-descriptor run passing (base compiler's live bytes at N=10 in brackets): an
early `RETURN` with a copy bound after it [1120]; a loop body left by
`CONTINUE FOR` [3360]; an explicit `fs::close(k.h)` before the scope ends
[1120]; a record with two `RES fs::File` fields [2240]; the record passed to a
function that reads a field [1120]; a record whose field is a `RES <Union>`
[1120]; field reads under inline `TRAP`s, before and after an explicit close
[1120].

Unchanged, by design (same output and counters as the base compiler): the record
handed to an outer binding through a callee, the record assigned to an outer
binding (`7-703-0004` on both), the handle bound on its own, the handle stored
in an outer list (`7-703-0004` on both), a `LAMBDA` that captures the record, a
nested record read through a bound inner record, the record bound under an
inline `TRAP`, a list of such records read through `FOR EACH` or `get`.

### Golden impact

None. `scripts/artifact-gate.sh target/release/mfb all`: 1674 tests, 2338
goldens checked, 0 diffs. `scripts/test-accept.sh` over `rt-behavior/resources/*`
and the native resource fixtures: 71 ran, all pass.

