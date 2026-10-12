# bug-1026: `RES v AS Handle = collections::get(hs, 0)` binds a view into the list block; after the list grows, `MATCH v` matches no case

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory safety; use after free, silent wrong value)

Status: Fixed (bs20-agent-a, 2026-10-10). See "Resolution" at the end.
Regression Test: `tests/runtime/rt_resource_union_box_owner.rs` — `a_binding_of_a_list_element_outlives_the_block_it_was_read_from`, `a_binding_of_a_map_value_outlives_the_block_it_was_read_from`, `a_binding_of_a_getor_default_outlives_the_statement_that_wrapped_it`, `a_trapped_binding_of_an_element_outlives_the_block_it_was_read_from`, `a_trapped_binding_recovered_with_an_alias_owns_its_box` (each a `--debug` build read at N=10 and N=40, plus 300 iterations under a 64-descriptor limit; a freed block is refilled in a `--debug` build, so a read of one fails the printed total)

A resource-union binding initialised from `collections::get` of a `List OF RES Handle` is a view into the list's
storage, not a copy. When the list then grows (`collections::append` reallocates its block), `v` still points at the
old, freed block, and `MATCH v` takes neither `fs::File` nor `tcp::Socket` and falls through to `RETURN 0`. The build
succeeds, the run exits 0, and the answer is wrong.

**Correct behavior:** `v` owns or safely shares its own copy of the union box, so `which("v", v)` returns 1 whatever
happens to the list afterwards; the program prints `10` at `N=10`.

## Failing Reproduction

Measured on 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at 3fa35d543, macOS aarch64):
`N=10 /tmp/bs19b/bin/probe.sh /tmp/main-dbg-target/release/mfb z_get_view` (`mfb build -q --debug`, then the binary).
Result: `exit=0 stdout=0|`, expected `10`. Arena counters at the same run: `alloc_calls 152`, `free_calls 142`,
`live_bytes 61920` (so there is also a leak; not analysed). `/tmp/g933/data.txt` is a 6-byte file (`hello` and a newline).

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

FUNC once() AS Integer
  RES a AS Handle = openOne()
  RES c AS Handle = openOne()
  MUT hs AS List OF RES Handle = []
  hs = collections::append(hs, a)
  RES v AS Handle = collections::get(hs, 0)
  FOR j = 1 TO 40
    hs = collections::append(hs, c)
  NEXT
  LET junk1 AS List OF Integer = [7, 7, 7, 7, 7, 7, 7, 7]
  LET junk2 AS List OF Integer = [9, 9, 9, 9, 9, 9, 9, 9]
  RETURN which("v", v)
END FUNC
```

## What is known

The bug-1012 fix made a binding initialised from a record FIELD take its own copy of the union box. This is the same
class one container over: the initialiser from `collections::get` of a list was not given the same treatment. Which
function does the aliasing and where the copy belongs were not traced (guess).

Found by the bug-1012 agent while fixing bug-1012 (batch 19).

## Resolution (bs20-agent-a, 2026-10-10)

### Root cause

A `List`/`Map` of a resource union holds each element's 16-byte `{tag, record-ptr}`
box INLINE, so `collections::get`/`getOr` yields an address inside the collection's
block. `CodeBuilder::value_aliases_live_resource` classifies that call as an alias,
and the `NirOp::Bind` arm of `builder_control.rs` stored the address as it was: the
bug-1012 copy (`copies_field_box`) matched `NirValue::MemberAccess` only. The
binding therefore named a span of a block `collections::append` frees when it
reallocates.

Debugger proof on the base compiler (`-O0 --debug`, `N=1`, a breakpoint on
`_mfb_arena_free` printing `x0, x1`, then one on `which`): the fifth free is
`x0=…c170 x1=232`, and `which` receives `x1=…c238`, which is 200 bytes into that
freed block and holds the fill pattern. After the fix `which` receives `…4260`, a
block of its own holding `{0, record}`, and it is freed once, size 16, after
`which` returns.

Two more routes to the same store were found and are the same defect:

* **`getOr`'s default.** A variant written as the default is a `UnionWrap`, whose
  box is a `union_box` pending temporary the statement frees at its end
  (bug-1012). The binding kept that address: `RES v AS Handle =
  collections::getOr(hs, 7, f)` exited 139 inside `fs::readAll`, and so did
  `MATCH collections::getOr(hs, 7, f)`.
* **The inline-`TRAP` desugar.** `RES v AS Handle = collections::get(hs, 0) TRAP …`
  binds `$trap_resN = callResult get(…)`, assigns `$trap_valN =
  resultValue($trap_resN)` and binds `v = local $trap_valN`. The store into the
  temp is "lent" (bug-648, `trap_ownership.rs`), and the lent pointer was the
  element's address, a `RECOVER b.h`'s field address, or a freed default box.

### The fix

* `builder_control.rs`, `NirOp::Bind`: `copies_borrowed_box` (was
  `copies_field_box`) also matches a `Call`/`CallResult`/`RuntimeCall` whose target
  is `target_returns_borrowed_resource`. The binding copies the two words
  (`materialize_inline_value_in_arena`) and registers `register_union_box_cleanup`,
  which frees the copy and closes nothing. A `MATCH` scrutinee temporary of `get`
  is excepted, as the field read already was (its only reads are at the head of
  each case); one of `getOr` is not.
* `builder_arena_transfer.rs`, `materialize_current_result_boxed`: a
  `RawSuccessBlock::BorrowedResource` success of a resource-UNION type is put in
  the `Result` as a copy of its box, inside the statement that made it. A concrete
  resource is still carried by pointer.
* `builder_control.rs`, `NirOp::Assign`: an aliasing `RECOVER` stored into a
  resource-union `TRAP` temp (not `store_is_owned`, and not a `resultValue`) copies
  the box.
* `builder_resource_cleanup.rs`, `emit_resource_union_cleanup_call`: a clear
  bug-648 owner flag now skips the variant close and still frees the box (it used
  to skip the whole drop). The flag test moved below the `RETURN` identity skip.
  The flag slot exists only for a lent `TRAP` temp, so no other union binding's
  code changes.

### Measurements (fixed compiler, `mfb build -q --debug`, `/tmp/bs20a/probe.sh`)

Every program prints `N` at N=10 and N=40 with `double_free_skips 0`, and
`live_bytes` is the figure the base compiler reports for the same program (the fix
adds one allocation and one free per evaluation):

| program | base stdout (N=10) | fixed stdout | live_bytes N=10 (base = fixed) |
|---|---|---|---|
| list `get` + 40 appends (this doc) | 0 | 10 | 61,920 |
| list `getOr` (element) + 40 appends | 0 | 10 | 61,920 |
| map `get` + 40 sets | 0 | 10 | 51,680 |
| `getOr` default variant, bound | exit 139 | 10 | 2,400 (fixed) |
| `getOr` default variant, `MATCH` | exit 139 | 10 | 2,400 (fixed) |
| `get … TRAP` + 40 appends | 0 | 10 | 63,200 |
| `getOr` default `… TRAP` | exit 255 (`7-703-0004`) | 10 | 3,680 (fixed) |
| `get … TRAP RECOVER b.h`, then `b = HB[c]` | 0 | 10 | 1,760 |

### Goldens expected to change

None. `bash scripts/artifact-gate.sh target/release/mfb all`: 1665 tests, 2322
goldens checked, 0 diffs. No committed fixture binds a resource-union from
`collections::get`/`getOr`, and none has a lent resource-union `TRAP` temp.

### Not fixed here (reported, different causes)

* The leak this doc noted. A `List OF RES <Union>` read through `collections::get`
  keeps its final block: `record_ownership.rs:block_held_elsewhere_strict` answers
  "held" for any collection that reaches a resource union and is named by a
  borrowed-element read. 240 B per call for a one-element list, 6,192 B for the
  41-element list above. The same program without the `get` is balanced
  (`alloc_calls 142 free_calls 142 live_bytes 0`).
* A resource-union inline-`TRAP` bind keeps 128 B per evaluation, with no
  collection involved (`RES v AS Handle = mkH() TRAP(e) … END TRAP`:
  `alloc_calls 72 free_calls 52 live_bytes 1280` at N=10): the temp's closed
  default, a 16 B box and its 112 B record, is not released by the store that
  replaces it.
