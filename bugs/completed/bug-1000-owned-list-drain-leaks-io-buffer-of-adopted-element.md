# bug-1000: an owned `List OF RES` that `owning_collections` cannot prove owns its elements (adopted from a call, floated in from an inner block, iterated or read through `get`) leaks the list block and/or the elements' records and I/O buffers

Last updated: 2026-10-10
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (2026-10-10)
Regression Test: `tests/runtime/rt_resource_drop_reclaims.rs` (the `b1000_*` cases) and the `owned_list_ownership_tests` unit tests in `src/codegen/resource/cleanup/record_ownership.rs`

When a `List OF RES fs::File` whose element was **adopted from a call** (`LET xs = mkList()`), or
**floated in from an inner block** (an inner `IF` opens the file and `append`s it into an outer
`MUT xs`), is dropped, the file is closed but its buffered-output block and record are never
freed. The 4 KB buffer is only there once `fs::setBuffered(f, TRUE)` has run. A same-scope
`append` is not affected.

**Correct behavior:** dropping the list frees everything the element owned (the I/O buffer, the
record), as a direct `RES f` binding's drop does. `live_bytes` stays flat across iterations.

Suspected cause (from bs17-agent-f's reading of the code; not traced): the owned-list drain reclaims
the buffer only when `record_ownership::owning_collections` proves the list owns its elements
(`owns_elements`). Adopted and floated lists fail that proof, so the drain only closes.

## Failing Reproduction

Measured by the coordinator on 2026-10-06 with main 468ff0f94 (`/tmp/main-dbg-target/release/mfb`),
`mfb build -q --debug <dir>`, then `N=10` / `N=20 ./<dir>/build/<dir>.out`. Counts are arena 0
`alloc_calls / free_calls / live_bytes`.

| program | N=10 | N=20 | per iteration |
|---|---|---|---|
| leakA (adopted, buffered) | 101 / 71 / 43040 | 201 / 141 / 86080 | +4304 B |
| leakA3 (floated, buffered) | 111 / 81 / 44480 | 221 / 161 / 88960 | +4448 B |

leakA, adopted from a call:
```
IMPORT os
IMPORT fs
IMPORT collections
FUNC openBuffered() AS RES fs::File
  RES f AS fs::File = fs::openFile(fs::pathJoin([fs::tempDirectory(), "mfb-leakA.txt"]), "w")
  fs::setBuffered(f, TRUE)
  RETURN f
END FUNC
FUNC mkList() AS List OF RES fs::File
  RES f AS fs::File = openBuffered()
  RETURN [f]
END FUNC
FUNC body() AS Nothing
  LET xs = mkList()
  fs::writeAll(collections::get(xs, 0), "L")
END FUNC
FUNC main() AS Integer
  MUT i AS Integer = 0
  WHILE i < toInt(os::getEnvOr("N", "10"))
    body()
    i = i + 1
  END WHILE
  RETURN 0
END FUNC
```

leakA3, floated from an inner block (same `main` as leakA):
```
IMPORT os
IMPORT fs
IMPORT collections
FUNC body() AS Nothing
  MUT xs AS List OF RES fs::File = []
  IF len("x") = 1 THEN
    RES f AS fs::File = fs::openFile(fs::pathJoin([fs::tempDirectory(), "mfb-leakA3.txt"]), "w")
    fs::setBuffered(f, TRUE)
    xs = collections::append(xs, f)
  END IF
  fs::writeAll(collections::get(xs, 0), "L")
END FUNC
FUNC main() AS Integer
  MUT i AS Integer = 0
  WHILE i < toInt(os::getEnvOr("N", "10"))
    body()
    i = i + 1
  END WHILE
  RETURN 0
END FUNC
```

bs17-agent-f also measured an unbuffered leakA (`setBuffered` removed): 91 / 71 / 2080 at N=10,
181 / 141 / 4160 at N=20, so +208 B per iteration (the record plus the list's node and block).
The coordinator has not re-measured that variant.

### A second trigger: the list is iterated or read through `get` (bs17-agent-g, re-measured by the coordinator on main 468ff0f94)

A same-scope `List OF RES` local that is iterated with `FOR EACH`, or read through
`collections::get`, never frees its own list block (232 B + header per call). `owning_collections`
refuses a list that is a `FOR EACH` iterable or named by `collections::get`, so
`register_res_collection_block_free` never runs. When the element was floated into such a list, the
drain also stops reclaiming the File record and its read buffer (16,736 B per call, agent-g's
`/tmp/g933L`, MODE=0; not re-measured by the coordinator). `--debug`, arena 0:
N=10 gives 42 alloc / 32 free / 2400 live bytes; N=100 gives 402 / 302 / 24000. Without the
`FOR EACH` (no read of `xs`) it is flat. It needs an existing `/tmp/g933/data.txt`.

```
IMPORT io
IMPORT os
IMPORT fs
IMPORT collections
FUNC once() AS Integer
  MUT xs AS List OF RES fs::File = []
  RES f AS fs::File = fs::openFile("/tmp/g933/data.txt")
  xs = collections::append(xs, f)
  MUT n AS Integer = 0
  FOR EACH h IN xs
    n = n + 1
  NEXT
  RETURN n
END FUNC
FUNC main() AS Nothing
  MUT t AS Integer = 0
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    t = t + once()
  NEXT
  io::print(toString(t))
END FUNC
```

The fix should make the ownership proof accept a list that is only read, iterated or borrowed from
(that does not make it "held elsewhere"), so that both the list block and every element's record
and buffers are reclaimed.

## Related

- bug-1001 (the same buffer leak through a resource-union drop). Found by bs17-agent-f while fixing
  bug-843; reproduced by the coordinator.

## Root Cause

The report's title names the wrong trigger. Re-measured on the base compiler:
an adopted list that is only measured (`LET xs = mkList()` then `len(xs)`) and a
floated one are flat. What leaks is a list that is **read through
`collections::get`** or **iterated with `FOR EACH`**:

| program (base compiler, N=20) | `alloc / free / live` |
|---|---|
| leakA: adopted, `fs::writeAll(collections::get(xs, 0), "L")` | 201 / 141 / 86080 |
| the same with `len(xs)` instead of the `get` | 181 / 181 / 0 |
| leakA3: floated, read through `get` | 221 / 161 / 88960 |
| the same with `len(xs)` | 201 / 201 / 0 |
| same-scope list, `FOR EACH h IN xs` | 82 / 62 / 4800 |
| the same with `len(xs)` | 82 / 82 / 0 |

`record_ownership::owning_collections` decides whether an owned-list drain may free
what it closes. It refused any collection named by a borrowed-element read
(`element_readers`) and any collection named outside a call's argument position,
which a `FOR EACH` iterable is (`escaping`). Refused, the drain only closes: the
element's record, its I/O buffers and the list block stay allocated.

The refusal protects against a reference that outlives the list. Neither reading
is one by itself:

- `fs::writeAll(collections::get(xs, 0), "L")` hands the element to a call that
  reads through it and returns. That is the argument-position rule the floated
  element locals already live by.
- `FOR EACH h IN xs` binds each element for one iteration, inside `xs`'s scope.
  `h` is a second holder only if it goes somewhere itself.

## Resolution

`record_ownership.rs:CollectionUses` now tells the two readings apart from a kept
reference, and treats two things it used to trust as keeping.

**Newly owning (and so freed):**

- A borrowed-element read written straight into an argument of a call that does
  not keep that argument is not recorded as an element reader
  (`transient_borrow`). The receiving call must be a natively lowered builtin that
  is not a `collections` member storing the argument and has no consuming
  parameter there, or a function whose body is read and found not to keep the
  parameter. Through a second borrowed read (`get(get(rows, i), j)`) the receiver
  is as transient as the result.
- A `FOR EACH` over the collection no longer holds it by itself
  (`escaping_beyond_iteration`, `iterations`). It holds it when the loop variable
  is bound, returned, captured, stored as a collection element, kept by a callee,
  read through `get`, or iterated by a loop whose own variable does one of those
  (`iterated_reference_leaves`). A callee that only iterates its parameter does
  not keep it.

**Newly refused (found while proving the above; each was a use after free on the
base compiler):**

- **A callback that keeps what it is handed.**
  `collections::forEach(xs, LAMBDA(h) -> keep = collections::append(keep, h))`
  stores every element of `xs` in an outer list, but `xs` appeared only in an
  argument position, so it counted as the one owner; its drain freed the records
  and `keep` read freed memory. Measured on the base compiler: a write through
  `collections::get(keep, 0)` after the inner scope reported `7-703-0009`
  (`ErrResourceMoved`), which is a flag bit of the scrubbed block; it reports
  `7-703-0004` (`ErrResourceClosed`) now. A call handed a `LAMBDA` or a function
  reference reads that function's body and keeps its arguments when the body keeps
  a parameter that can carry a reference; a callback held in a local, and a call
  through a function value, cannot be read and count as keeping
  (`call_reaches_unread_code`). A floated element handed to such code no longer
  qualifies either (`kept_by_code`).
- **An adopted list around a record its own scope already owns.**
  `RES a = open()` then `LET xs = wrap(a)` (`wrap` returns `[f]`) listed `a`'s
  record on `xs`'s owned-list. Both are at one scope depth, so the drain passed the
  owner test, closed and freed `a`'s record, and `a`'s own drop then read the
  freed block's owner word (`mfb dbg`: `ldr x8, [sp, #0x10]` / `ldr x9, [x8,
  #0x20]` straight after the drain's 112 B free). An adopting push now lists a
  record only when it takes it: unowned, or owned by a scope inside the
  container's. One already owned at that scope or further out keeps its owner and
  is not listed (`builder_owned_cleanup.rs:emit_owned_list_push_node`,
  `adopting`).

**Not widened: a collection of resource-union boxes.** A box a callee's binding
wrapped is freed by that binding as the callee returns (bug-1012), so the drain of
an adopted `List OF RES <Union>` that frees boxes frees them twice. That already
happens on the base compiler for a list that is only measured
(`LET xs = mkStreams()` then `len(xs)`: `free_calls` exceeds `alloc_calls`,
`double_free_skips 9` at N=10; and the floated form exits 139). So a collection
that reaches a union leaf keeps the rule it had (`block_held_elsewhere_strict`),
and this fix reaches no union list it did not reach before. Both repros are
recorded in bug-1012.

**Still close-only, by design:** a list an element is `RETURN`ed out of, bound out
of (`RES h = collections::get(xs, 0)`), or `MATCH`ed on through `get`. Those keep
the bounded leak.

### Runtime proof

`mfb build -q --debug`, arena 0, `alloc_calls / free_calls / live_bytes`
(`/tmp/k/probe.sh <mfb> <name> 10 20`), base against this branch:

| program | base N=20 | fixed N=10 | fixed N=20 |
|---|---|---|---|
| leakA (adopted, `get`) | 201 / 141 / 86080 | 101 / 101 / 0 | 201 / 201 / 0 |
| leakA3 (floated, `get`) | 221 / 161 / 88960 | 111 / 111 / 0 | 221 / 221 / 0 |
| `FOR EACH` over a same-scope list | 82 / 62 / 4800 | 42 / 42 / 0 | 82 / 82 / 0 |
| same-scope list read through `get` | 122 / 102 / 4800 | 62 / 62 / 0 | 122 / 122 / 0 |
| control: adopted, `len` only | 181 / 181 / 0 | 91 / 91 / 0 | 181 / 181 / 0 |
| control: floated, `len` only | 201 / 201 / 0 | 101 / 101 / 0 | 201 / 201 / 0 |
| `LET xs = wrap(a)` beside `RES a` | 201 / 201 / 0 (reads a freed record) | 91 / 91 / 0 | 181 / 181 / 0 |

`double_free_skips 0` in every fixed run. The fixed leakA and leakA3 allocate and
free exactly what their `len`-only controls plus the element read do.

No use after free: `rt_resource_drop_reclaims.rs` uses each handle after the point
a wrong free would run — a write through the caller's binding after the inner list
died (`open`), through an element `RETURN`ed out of the list (`open`), and through
an outer list that copied an element out without a float (`closed`, the defined
`ErrResourceClosed`; any other error fails the test, so a read of freed memory
cannot pass).

## Golden impact

The ownership verdict changes which drains free and which collections get a block
free, so `.ncode`/`.ncodesum` move for fixtures that (a) read a `List OF RES` (or
other resource-reaching collection) through `collections::get`/`getOr` in an
argument position, (b) `FOR EACH` over one, or (c) bind a call result that reaches
resources (every adopting push gains the owner test). The fixtures the artifact
gate reports are listed in the commit message.
