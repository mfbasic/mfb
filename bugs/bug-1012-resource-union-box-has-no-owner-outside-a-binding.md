# bug-1012: a resource-union `{tag, record-ptr}` box has no owner outside a binding — a `RES U` record field keeps a freed box (SIGSEGV), and a wrap at an argument, list element or field leaks 16 B

Last updated: 2026-10-07
Effort: large (3h–1d)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: (proposed) tests/rt-behavior/resources/resource-union-box-owner-rt — the `rk` program below plus a `--debug` balance test over the `rw` loop

A resource-union value (`UNION Handle  fs::File  tcp::Socket`) is a 16-byte
`{tag@0, record-ptr@8}` box. A `UnionWrap` allocates a fresh box, and the only
code that frees one is the drop of the **binding** that wrapped it
(`RES u AS Handle = …`). Two consequences:

1. **Use after free (SIGSEGV).** A record field of type `RES Handle` stores the box
   **pointer**. When the binding that owns the box goes out of scope before the
   record does, the record points at a freed box. `RETURN keep(u)`, where
   `keep(h AS Handle) AS HB` returns `HB[h]`, hands the caller a record whose field is
   `u`'s box, which `u`'s drop frees as `mkKept` returns. Reading `k.h` then faults.
2. **Leak (16 B per evaluation).** A box made by a wrap that no binding owns is
   never freed: a variant at a `Handle` argument (`which("x", a0)` with
   `a0 AS fs::File`), a `List OF RES Handle` literal element, and a `RES Handle`
   record field. Before bug-990 only the identifier-at-argument form wrapped (and
   leaked); bug-990 made every expression kind wrap there and made the `RES U`
   element/field positions wrap at all (they used to misroute `MATCH`), so those
   leak the same 16 B now.

**Correct behavior:** every box has exactly one owner and is freed exactly once:
a record field that keeps a resource-union value owns (or copies) its box and the
record's drop frees it; a box used only for the duration of a statement (a call
argument, a collection element the collection copies inline) is freed at the end
of that statement. No record may hold a box another owner frees.

References:

- `src/codegen/resource/cleanup/builder_resource_cleanup.rs` (resource-union drop,
  bug-623 comment): "The box is this binding's alone: a resource-union `UnionWrap`
  and the closed default both allocate a fresh 16-byte block for the bind, and every
  other holder of it (an aliasing bind, a record `RES` field, a floated owned-list
  node, a `RETURN`) either registers no cleanup or skips". A record `RES` field is
  named as a holder that registers no cleanup, so it never owns the box, and nothing
  keeps the owning binding alive as long as the record.
- `src/codegen/resource/module_owner.rs` header: "a box belongs to the binding that
  wrapped it and is freed at that binding's scope exit whoever owns the record".
- `mfb spec language resource-management` §15 ("A resource is closed exactly once")
  and `mfb spec memory heap-values` (a value never points into storage another owner
  frees).
- Found while fixing bug-990
  (`bugs/completed/bug-990-variant-call-or-field-result-not-union-wrapped-at-argument-and-element-positions.md`,
  "Not fixed here").

## Failing Reproduction

macOS aarch64, main `f6b4099c9` (`/tmp/main-dbg-target/release/mfb`). `project.json`
as in bug-990 (`"kind": "executable"`, `"targets": ["native"]`).

`rk` (use after free):

```
IMPORT io
IMPORT fs
IMPORT tcp

UNION Handle
  fs::File
  tcp::Socket
END UNION

TYPE HB
  h AS RES Handle
END TYPE

FUNC openOne AS RES fs::File
  RES f AS fs::File = fs::createTempFile()
  RETURN f
END FUNC

FUNC keep(h AS Handle) AS HB
  RETURN HB[h]
END FUNC

FUNC mkKept AS HB
  RES u AS Handle = openOne()
  RETURN keep(u)
END FUNC

FUNC touch(label AS String, h AS Handle) AS Nothing
  MATCH h
    CASE fs::File(f)
      io::print(label & ": file")
    CASE tcp::Socket(s)
      io::print(label & ": socket")
  END MATCH
END FUNC

FUNC main AS Integer
  LET k AS HB = mkKept()
  LET junk1 AS List OF Integer = [7, 7]
  LET junk2 AS List OF Integer = [9, 9]
  touch("kept", k.h)
  RETURN 0
END FUNC
```

- Observed: no output, `Segmentation fault: 11`, exit 139. Under the debugger
  (`mfb build --debug -O0`, then `printf 'run\nbt\nlist reg gpr\nquit\n' | mfb dbg rk.out`):
  `X: a bad memory access at 0x0000000000000030` in `main+0x94`. Guess (not traced):
  a load at `+0x30` from a record pointer of 0 read out of the freed box's `+8`.
- Expected: `kept: file`, exit 0.

Leak (`rv_var`, same declarations, `which(label AS String, h AS Handle)` matching like
`touch`):

```
FUNC main AS Integer
  RES a0 AS fs::File = fs::createTempFile()
  which("var", a0)
  RETURN 0
END FUNC
```

`mfb build --debug`: `arena.0.alloc_calls 6`, `free_calls 5`, `live_bytes 16` on main
and on bug-990's fix. With bug-990's fix, a program with 300 × `touch("", openOne())`
plus three single uses (`/tmp/bs18j/rw.mfb`) ends with 1520 allocs / 915 frees /
38,656 B live, about 128 B per iteration: 112 B is the record tombstone bug-842
documents, 16 B is the box.
A `List OF RES Handle` literal `[a0]` or `[openOne()]`: 7 allocs / 6 frees / 16 B
(main: 6 / 6 / 0, but there `MATCH` fired no arm on the element — bug-990).

Contrast cases (balanced today): `RES one AS Handle = openOne()` then use (6 / 6 / 0);
`RES a AS Handle = …` then `[a]` or `collections::append(hs, a)` (7 / 7 / 0, 8 / 8 / 0).
The binding owns the box and the collection copies it inline
(`is_pointer_collection_payload_type`: "Resource *unions* carry a tag and are not
pointer payloads").

## Root Cause

Hypothesis from reading the code (the store itself has not been traced in `.ncode`):

- `src/codegen/engine/value/builder_values.rs`, the `NirValue::UnionWrap` arm: for a
  resource variant it `_mfb_arena_alloc`s a `8 * (1 + max_payload)` box (`union_result`)
  and returns its pointer. It registers no pending temp, because a resource union is not
  `is_freeable_flat_value` (`pending_temp_is_freeable`), so only a consumer that takes
  ownership frees it, and the only such consumer is a resource-union binding's
  `ActiveCleanup::ResourceUnion` drop (`builder_resource_cleanup.rs`).
- `src/codegen/collection/layout/builder_collection_layout.rs` ("DIVERGENCE 3"): a
  resource union reaches `record_field_is_pointer` and is a pointer field. The
  constructor stores the box pointer it is given (`lower_value_stored_field` copies only
  a recursive graph). From a binding's box (`HB[h]`, `h` a parameter aliasing the
  caller's `u`), the record aliases a box `u`'s drop frees. From a fresh wrap
  (`HB[a0]`), the box has no owner and leaks.
- A call argument or a list element only needs the box for the statement, but nothing
  frees it after the statement.

## Goal

- `rk` prints `kept: file`, exit 0, with `double_free_skips 0`.
- `rv_var`, the `[a0]` / `[openOne()]` literal and the 300-iteration `touch` loop leak
  no box: `live_bytes` grows by at most the documented 112 B tombstone per iteration.
- Every contrast case stays balanced and closes each resource exactly once.

### Non-goals (must NOT change)

- Resource close semantics (bug-842 temporaries, bug-755 owner words, §15.6 floating).
- The inline element layout of a resource-union collection.
- Tempting wrong fix: freeing an argument's box at the end of the statement while a
  record field can still hold it by pointer. That turns `keep(UnionWrap(a0))` into the
  same use after free as `rk`. The record-field ownership must be settled first.

## Blast Radius

- Record/`WITH` fields of type `RES U` (and any composite holding one): use after free
  when the box comes from a binding the record outlives; leak when it comes from a fresh
  wrap.
- Call arguments (user and built-in), `List`/`Map` literal elements, `append`/`insert`
  arguments: leak only (the callee and the collection copy what they keep, except the
  record-field case above).
- Closure capture: copies the box (bug-843, `ClosureEnvWord::ResourceUnionBox`).
  Unaffected.
- `RETURN`: owns or copies the box (`returned_resource_union_owns_box`). Unaffected.

## Fix Design

Not designed yet. The likely shape: a `RES U` record field owns a copy of the box (the
constructor and `WITH` copy an aliasing box, the record drop frees it), and a
statement-scope box (argument, element) is registered as a shallow pending temp that an
owning bind or field claims. Both halves need an `--debug` balance proof, and a check
that each resource still closes exactly once.

## Validation Plan

- `rk` and the leak programs above, as an rt fixture plus a `--debug` balance test.
- `tests/rt-behavior/resources/**`, `rt_resource_temporary_close`,
  `rt_inline_trap_union_bind`, and `artifact-gate all`.

## More shapes of the same defect (bs18-agent-k, 2026-10-10)

Found while fixing bug-1000 and bug-999, on the base compiler
(`/tmp/main-dbg-target/release/mfb`, `mfb build -q --debug`). Both return a
`List OF RES <Union>` from a function. They are this bug: the list's element is the
box pointer, and nothing but the wrapping binding owns the box.

Common prelude:

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
```

1. **Double free of the box.** The callee's binding frees the box as it returns;
   the caller's owning drain frees it again.

   ```
   FUNC mkStreams() AS List OF RES Stream
     RES s AS Stream = openData()
     RETURN [s]
   END FUNC
   FUNC once() AS Integer
     LET xs = mkStreams()
     RETURN len(xs)
   END FUNC
   ```

   `N=10`: `alloc_calls 52 free_calls 62 live_bytes 0 double_free_skips 9`.
   `N=20`: `102 / 122 / 0`, `double_free_skips 19`.

2. **SIGSEGV.** The same `once`, with the element floated in the callee:

   ```
   FUNC mkStreams() AS List OF RES Stream
     MUT xs AS List OF RES Stream = []
     IF len("x") = 1 THEN
       RES s AS Stream = openData()
       xs = collections::append(xs, s)
     END IF
     RETURN xs
   END FUNC
   ```

   `N=10`: exit 139. (`N=1` at `-O0` exits 0.)

Because of 1, bug-1000 and bug-999 deliberately do not widen ownership for a
collection that reaches a resource-union box
(`record_ownership.rs:block_held_elsewhere_strict`,
`builder_values.rs:adopt_exclusive_temp`). Once a box has an owner outside a
binding, both exclusions can go.
