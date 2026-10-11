# bug-1045: a loop-scoped record's resource moved into a list declared outside the loop is closed when the loop body ends

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (use after close)

Status: Fixed (bs22-agent-b)
Regression Test: `tests/runtime/rt_resource_store_float.rs` (`cargo test --test rt_resource_store_float`)

A resource that a loop-scoped `LET k AS HB = mk()` holds is appended to a list declared
before the loop, either as the record (`xs = collections::append(xs, k)`, list of `HB`) or as
its field (`fsl = collections::append(fsl, k.h)`, `List OF RES fs::File`). The loop body's
scope exit closes the resource although the outer list now holds it. Reading it after the loop
fails with `Error: 7-703-0004`, `Resource handle is already closed.`, exit 255.

**Owner decision (2026-10-11):** a resource put in a list takes the list's scope if that
scope is higher than its own. Here the list's scope (the function) is higher than the loop
body's, so the resource must live until the function ends.

**Correct behavior:** both programs read every resource after the loop. By hand (not
measured, the compiler does not get there): `r2` computes 3 * (6 + 1) = 21 per call, so
`210` at `N=10`; `r5` computes 2 * (6 + 1) = 14 per call, so `140` at `N=10`. `live_bytes 0`
at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`r2` (the record `k` is appended to an outer `List OF HB`):
`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb r2`: exit 255, empty stdout,
stderr begins `Error: 7-703-0004` / `Resource handle is already closed.`, `alloc_calls 20`,
`free_calls 16`, `live_bytes 464`. `N=20` gives the same (it fails in the first call).

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
  h AS RES fs::File
END TYPE

TYPE HU
  h AS RES Handle
END TYPE

FUNC openOne AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC

FUNC wrap(h AS fs::File) AS HB
  RETURN HB[h]
END FUNC

FUNC rd(f AS fs::File) AS Integer
  RETURN len(fs::readAll(f))
END FUNC

FUNC which(h AS Handle) AS Integer
  MATCH h
    CASE fs::File(f)
      RETURN len(fs::readAll(f))
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

FUNC mk() AS HB
  RETURN HB[openOne()]
END FUNC
FUNC once() AS Integer
  MUT xs AS List OF HB = []
  FOR j = 1 TO 3
    LET k AS HB = mk()
    LET a AS Integer = rd(k.h)
    xs = collections::append(xs, k)
  NEXT
  MUT t AS Integer = 0
  FOR EACH x IN xs
    t = t + rd(x.h) + 1
  NEXT
  RETURN t
END FUNC
```

`r5` (the field `k.h` is appended to an outer `List OF RES fs::File`):
`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb r5`: exit 255, empty stdout,
the same `Error: 7-703-0004` message, `alloc_calls 15`, `free_calls 12`, `live_bytes 352`.
`N=20` gives the same.

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
  h AS RES fs::File
END TYPE

TYPE HU
  h AS RES Handle
END TYPE

FUNC openOne AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC

FUNC wrap(h AS fs::File) AS HB
  RETURN HB[h]
END FUNC

FUNC rd(f AS fs::File) AS Integer
  RETURN len(fs::readAll(f))
END FUNC

FUNC which(h AS Handle) AS Integer
  MATCH h
    CASE fs::File(f)
      RETURN len(fs::readAll(f))
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

FUNC mk() AS HB
  RETURN HB[openOne()]
END FUNC
FUNC once() AS Integer
  MUT fsl AS List OF RES fs::File = []
  FOR j = 1 TO 2
    LET k AS HB = mk()
    LET a AS Integer = rd(k.h)
    fsl = collections::append(fsl, k.h)
  NEXT
  MUT t AS Integer = 0
  FOR EACH f IN fsl
    t = t + rd(f) + 1
  NEXT
  RETURN t
END FUNC
```

## What is known

Both fail in the first call of `once`: the counters are identical at `N=10` and `N=20`.
Guess (inferred, not traced): the failing read is the `FOR EACH` after the loop, the first
read after the loop body's scope exit, and that scope exit closes `k`'s resource because the
append did not move it to the list's scope. Whether the same happens in a build without
`--debug` was not measured.

Found during batch 21.

## Resolution (bs22-agent-b, 2026-10-11)

### The expected output in this document is wrong, and the fix is right

"Correct behavior" above computes `210` and `140` by hand. Both programs read each file
TWICE: `LET a AS Integer = rd(k.h)` in the loop body reads the six bytes and leaves the file
at its end, so the read after the loop returns no bytes. The correct totals are
3 * (0 + 1) = 3 per call for `r2` (`30` at N=10) and 2 * (0 + 1) = 2 for `r5` (`20`).
Measured, not argued: `/tmp/bs22-agent-b/m1045/twice.mfb` (the same prelude, `LET k AS HB =
mk()`, `LET a = rd(k.h)`, `LET b = rd(k.h)`, `RETURN a * 100 + b`, no list and no loop)
prints `600` on the BASE compiler, that is 6 then 0. The rows below that do not read before
the store (`c`, `i`, `n`, `k`, ...) give the 7 per element the hand computation expected.

### Cause

The guess is right. `LET k AS HB = mk()` adopts what the call hands back inside the record:
the binding gets an owned-list at the loop body's scope and the file record takes that
scope's token. The escape analysis (`ir::resource_escape`) raises a resource to an outer
container only for a `RES` BINDING named as the element (`ResOwner::Float`); a record that
reaches a resource, or a field read of one, is not a `RES` binding, so nothing raised the
file when `k` or `k.h` was appended to the outer list. The loop body's drain then closed it
at the end of every iteration. Debugger on the base `-O0 --debug` build of `r2` at N=1
(`printf 'bp _mfb_rt_fs_fs_close\nrun\nbt\nquit\n' | mfb dbg r2.out`): the first close is
entered from `once + 4224`, inside the loop; the fixed build's three closes are all entered
from `once + 6024`, the function's exit, with three different records in `x0`, and the
program then prints `3` and exits 0.

### Fix

The owner ruling is implemented at the store, where the escape analysis has no answer
(`record_ownership::StoreFloats`, `CodeBuilder::emit_store_float`):

* **Which stores.** A pre-pass over the function's NIR finds every assignment to a
  container local (a collection or record that reaches a resource) whose value names, as an
  operand, a local bound in a scope INSIDE the container's: the local itself, a field chain
  or union extract over it, through a constructor, a list or map literal, a `WITH`, or
  `collections::append`/`prepend`/`insert`/`set`. A `RES` binding the escape analysis
  already floated is left to it. A `FOR EACH` variable counts by its ITERABLE's scope (it is
  a reference to an element of that list), so copying a parameter's list raises nothing.
* **What the store does.** Before the store, each marked operand is lowered, every resource
  it reaches is walked (`emit_for_each_resource`) and pushed, adopting, onto the container's
  owned-list. The adopting push takes a record only when it is unowned or owned by a scope
  inside the container's, and sets its owner word to the container's scope; a handle owned
  further out is left alone. The container gets its owned-lists at its bind, like a float
  target. The inner binding's drain already skips a record its scope no longer owns
  (`emit_skip_unless_owned`, bug-755), which is the whole of "do not close it at the end of
  the loop body".
* **Who frees.** `record_ownership` decides whether a drain may free what it closes. The
  raised operand is the hand-over, not a second holder, so the walk sets it aside when it is
  rooted at a record (copied by value where stored) or a `RES` binding; an operand rooted at
  a collection is stored by pointer and is still a holder. A container that took records
  this way frees them only when each source is proved to leave no other holder
  (`store_float_close_only`): a record source must itself be an owning container, a `RES`
  binding a fresh record that does not escape, a `FOR EACH` variable's iterable an owning
  container. Otherwise the container closes what it took and frees nothing
  (`StoreFloats::close_only`), and its own block is its own either way.

### What was measured

Fixed compiler `target/release/mfb`, macOS aarch64, `--debug` counters, default level; every
row gives the same line at `-O0` (`/tmp/bs22-agent-b/m1045/run.sh <mfb> -O0`). Programs:
`/tmp/bs22-agent-b/m1045/<row>.mfb` (prelude `pre.txt`); the filed probes with
`P=/tmp/bs22-agent-b/p45 /tmp/bs22/bin/probe.sh <mfb> r2` (`r5`). Base is main's compiler,
N=10. "fd" is `(ulimit -n 64; N=300 <program>)`; `double_free_skips` is 0 in every row.

| row | shape | base | fixed N=10: stdout, alloc/free/live | fd |
| --- | --- | --- | --- | --- |
| `r2` | as filed: record appended | exit 255, `7-703-0004` | `30`, 232/232/0 (N=40: `120`, 922/922/0) | 0 |
| `r5` | as filed: field appended | exit 255 | `20`, 162/162/0 (N=40: `80`, 642/642/0) | 0 |
| `c` | `prepend` + `insert` | exit 255 | `280`, 262/262/0 | 0 |
| `d` | `append` then `set` over the element | exit 255 | `70`, 182/182/0 | 0 |
| `e` | list literal assigned | exit 255 | `70`, 142/142/0 | 0 |
| `f` | `Map OF String TO HB`, `set` | exit 255 | `140`, 182/182/0 | 0 |
| `g` | the record assigned to an outer record | exit 255 | `60`, 172/172/0 | 0 |
| `r` | `keep = WITH keep { h := k.h }` | exit 255 | `60`, 152/152/0 | 0 |
| `h` | `RES Handle` field: record appended, and the field | exit 255 | `300`, 412/412/0 | 0 |
| `i` | `IF` body and `WHILE` body | exit 255 | `280`, 262/262/0 | 0 |
| `n` | a loop two scopes inside the list's | exit 255 | `280`, 262/262/0 | 0 |
| `j` | early `RETURN` out of the storing loop | exit 255 | `140`, 142/142/0 | 0 |
| `k` | the list `RETURN`ed and bound by the caller | exit 255 | `210`, 232/232/0 | 0 |
| `s` | the store runs on two of four iterations | exit 255 | `140`, 222/222/0 | 0 |
| `t` | two `RES` fields, one stored | exit 255 | `260`, 222/222/0 | 0 |
| `u` | a read through the record after the store | exit 255 | `210`, 232/232/0 | 0 |
| `v` | one record into two outer lists | exit 255 | `160`, 182/182/0 | 0 |
| `x` | `FOR EACH m IN mid` (inner `List OF HB`), `m` into an outer list | exit 255 | `280`, 342/342/0 | 0 |
| `y` | the same over an inner `List OF RES fs::File` | exit 255 | `280`, 262/262/0 | 0 |
| `o` | a caller's handle, wrapped in the loop and appended | `200`, 72/72/0 | unchanged; the caller reads it after the call | 0 |
| `q` | control: record and list in ONE scope | `140`, 142/122/2240 | unchanged (nothing is raised) | 0 |
| `w` | control: a module-level list (bug-806's path) | `110`, 107/86/3824 | unchanged | n/a (the list holds every file) |

Rows that run now and still keep memory, each equal to a control that has no such store, so
the bytes are another defect's:

| row | shape | fixed | control |
| --- | --- | --- | --- |
| `l` | `LET k AS HB = mk() TRAP ...`, appended | `140`, 142/122/2240 | `l2`, the same binding with no list, on BASE: `120`, 102/82/2240 (a record bound through an inline `TRAP` never frees its resource's record) |
| `m` | `RES f`, `LET k = HB[f]`, appended (`f` floats by the escape analysis) | `140`, 122/102/2240 | identical on base |
| `p` | an inner `List OF RES fs::File` appended to a `List OF List OF RES fs::File` | `240`, 222/152/11680 | identical on base |
| `z` | the outer list then passed to a callee that copies its elements out | `140`, 162/142/2240 | the existing rule: a list a callee stores an element of keeps a close-only drain |

### Checks

* RED: `MFB_TEST_EXE=/tmp/main-dbg-target/release/mfb cargo test --release --test rt_resource_store_float`:
  7 failed (`Error: 7-703-0004`), 1 passed. GREEN with the fix: 8 passed.
* `cargo test --release --no-fail-fast --test rt_resource_aggregate_temporary --test rt_resource_temporary_close
  --test rt_resource_drop_reclaims --test rt_resource_union_box_owner --test rt_resource_union_reclaim
  --test rt_resource_owner_scope --test codegen_helper_scratch_release --test rt_canvas_geo_native
  --test test_targets_registered --test rt_resource_union_member_float --test no_type_strings
  --test architecture_guards`: all passed but one test of `rt_resource_drop_reclaims`, below.
* `cargo test --release --bin mfb -- every_body_mfb_member_declares codegen::resource:: codegen::cleanup::`: 44 passed.
* `scripts/test-accept.sh target/release/mfb <out> '*resource*' '*canvas*' '*collection*' '*bug842*' '*bug1012*'
  '*bug1000*' '*bug805*' '*bug1027*' '*bug806*'`: 300 tests, passed.
* `scripts/spec-census.sh --render` / `--citations`: 0 misses.

### One existing test pinned the old behaviour, and was corrected

`rt_resource_drop_reclaims::an_element_copied_into_an_outer_list_is_closed_not_freed` failed
with the fix: it printed `openopen` five times where it expected `closedclosed`. Its program
appends the elements of an inner `List OF RES fs::File` (bound in an `IF` body) to a list
declared in the function's scope, with `FOR EACH h IN xs` / `keep = collections::append(keep,
h)`, and reads `keep` after the `IF`. The four questions (AGENTS.md):

1. Written by `2a0a8c695` (2026-10-10, "bug-1000 RED: an owned List OF RES that is read through
   get or iterated is not freed"; `git log -S an_element_copied_into_an_outer_list_is_closed_not_freed`).
2. It protects one thing: the inner list's drain must not FREE a record the outer list still
   names (bug-1000 had made the outer list read freed memory).
3. Nothing else depends on the `closed` reading: `grep -rn "copied_out\|not floated" tests src/docs
   bugs/completed/bug-1000*` finds only this test and an unrelated row of `architecture
   escape-analysis`.
4. It is wrong by the owner decision in this document (2026-10-11, a day after the test): a
   resource put in a list takes the list's scope if that scope is higher than its own, and by
   `language resource-management` §15.6, which the test's own comment cited for the opposite
   ("adding a reference to a resource to a collection migrates the resource's owning scope up
   to the collection's scope when that scope outlives the current owner"). `keep` outlives
   `xs`, so the handles must be open when `keep` is read.

Only the disproved line changed: the expected string is `openopen ...`, the comment says why,
and the test is renamed `an_element_copied_into_an_outer_list_takes_its_scope`. Its
`double_free_skips == 0` assertion and its guard (nothing freed under the outer list) stand.

### Golden impact

None. `scripts/artifact-gate.sh target/release/mfb all`: 2348 goldens checked, 4 diffs, the
four `.ir` goldens bug-1054 lists. No fixture with a golden stores an inner scope's
resource-reaching value into an outer container, so no `.ncodesum` moved; a program that does
gains the raise at that store and one owned-list head for the container.

### Shared functions changed

* `record_ownership::RecordOwnership` gains `store_floats: StoreFloats`; `owning_collections` and
  `block_collections` (private) take it; `CollectionUses::of_with` is new (`of` is unchanged);
  `CollectionUses::visit_value`, its argument walk, `visit_op` (a `WITH` over the assigned record) and
  `note_store` set a raised operand aside (`src/codegen/resource/cleanup/record_ownership.rs`).
  `record_ownership()`'s signature is unchanged.
* `CodeBuilder::setup_owned_list` (`src/codegen/cleanup/owned/builder_owned_cleanup.rs`): the block free keys on
  `owned_list_owning_collections` as before; `owns_elements` is additionally false for a
  `StoreFloats::close_only` container. New `CodeBuilder::emit_store_float`, called first in the
  `NirOp::Assign` arm of `lower_op` (`src/codegen/engine/control/builder_control.rs`).
* `CodeBuilder` gains the field `store_floats`; `owner_containers` is extended with the store-float
  containers in `lower_function` (`src/codegen/engine/function/function_lowering.rs`).

### Spec

`mfb spec language resource-management` §15.6 and `mfb spec memory arenas` (the store-time raise
and its ownership proof).
