# bug-1012: a resource-union `{tag, record-ptr}` box has no owner outside a binding — a `RES U` record field keeps a freed box (SIGSEGV), and a wrap at an argument, list element or field leaks 16 B

Last updated: 2026-10-10
Effort: large (3h–1d)
Severity: HIGH
Class: Memory-safety

Status: Fixed (bs19-agent-b, 2026-10-10). See "Resolution" at the end.
Coordinator note (2026-10-10): the `rk` use-after-free reproduced by the coordinator on main's code (`/tmp/main-dbg-target/release/mfb`) — exit 139 at `-O 0` and `-O 2`, no output. The 16 B leak rows were measured by bs18-agent-j only.
Regression Test: `tests/rt-behavior/resources/resource-union-box-owner-rt` (the `rk` program below) and `tests/runtime/rt_resource_union_box_owner.rs` (the `--debug` balance of every shape at N=10 and N=40, a 300-iteration run under a 64-descriptor limit, and a read through every handle)

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

Because of 1, bug-1000 deliberately does not widen ownership for a collection that
reaches a resource-union box (`record_ownership.rs:block_held_elsewhere_strict`).
Once a box has an owner outside a binding, that exclusion can go.

## Resolution (bs19-agent-b, 2026-10-10)

### The ownership rule

A resource-union box is 16 bytes, `{tag@0, record-ptr@8}`. It now has exactly one
owner, and everything else holds a copy of the two words.

| Position | Who owns the box | When it is freed |
|---|---|---|
| `RES u AS U = <wrap / call returning U / trapped default>` | the binding | at the binding's scope exit (unchanged) |
| `RES v AS U = u` (a bare local) | `u`; `v` shares the pointer inside `u`'s scope | by `u` (unchanged) |
| `RES v AS U = rec.field` | `v` owns a copy of the field's box | at `v`'s scope exit (new: the field read is an address inside the record's block, which a reassignment of the record frees) |
| a wrap or a call returning `U` that no binding takes: a call argument, a collection element, a record field, an unbound call | the statement | at the end of the statement, and on its error, `EXIT`, `CONTINUE` and `RETURN` paths (new: a `union_box` pending temporary) |
| `RETURN` | the caller | the callee moves out the box a local binding owns, or copies the value (a parameter, a field, an element); a wrap or call result it built is claimed, not freed |
| a `RES U` record field | nobody: the record holds the two words INLINE | with the record's own block (new layout, below) |
| a `List`/`Map` element | nobody: the collection holds the two words inline | with the collection block (unchanged) |
| an owned-list node (a resource floated or adopted into a container) | the node holds a copy: `{tag@0, record@8, next@16}`, 24 B | with the node, at the drain (new: it used to hold the box's address) |
| a closure environment, the module instance's list | their own copy | unchanged (bug-843, bug-806) |
| a binding whose resource floated to a container | the binding still owns the box its initializer made | at the binding's scope exit, closing nothing (new: the owned-list used to be the only holder) |

Freeing a box closes nothing. The record it names is closed by the scope that
owns the record (the owner word, bug-755), exactly as before.

### What changed

- **Record layout.** A `RES <resource union>` record field is inlined: the slot
  holds the block-relative offset of a 16-byte `{tag, record-ptr}` box in the
  record's data region, and a read of the field is `recordBase + offset`
  (`field_is_inlined_union_box`, `record_field_is_inlined`,
  `emit_inlined_block_size_from_ptr_slot`, and the helper-tier
  `memory::marshal::record::emit_inlined_block_size`). It was a pointer to a box
  somebody else freed. A `memcpy` of the record copies the field and the record's
  one block free releases it, so the record stays ordinary flat data. This is the
  layout a `List OF RES <Union>` element already has; the collection layout is
  untouched. A concrete `RES fs::File` field is untouched (one handle slot).
- **Statement-scope boxes.** `lower_value` registers a `union_box` pending
  temporary for a resource-variant `UnionWrap` and for a call to a function of
  the program whose declared return is a resource union
  (`value_yields_fresh_union_box`, `register_union_box_temp`). The statement-end
  drop, the in-place frees on a failing call's exit, and the `RETURN`/`EXIT`
  frees all go through the existing pending-temp machinery
  (`emit_pending_temp_free` → `emit_union_box_free`, null-guarded, slot nulled,
  slot in the prologue zero-init). `lower_value_owned`'s claim hands the box to a
  binding, an assignment or a module-level store; `lower_returned_value` claims
  it for a `RETURN` (`claim_union_box_temp`). A record constructor keeps a box
  temporary to the end of the statement (`drop_arg_block_temps_to`), because the
  resource temporary of the call that returned it reads the record through it.
- **Owned-list nodes.** `emit_owned_list_push_node` copies the tag and the record
  into a 24-byte node for a resource-union leaf, and `emit_owned_list_drain`
  hands the node's own address to the tag-dispatched close and frees no box.
- **Bindings.** A union bind from a record field copies the box and registers a
  drop that frees only the box (`register_union_box_cleanup`); a `MATCH`
  scrutinee temporary is excepted (its reads are the tag test and the case binds
  at the head of each case). A floated union bind keeps a box-only drop, and the
  inline-`TRAP` source of a floated bind keeps a box-only drop instead of losing
  its cleanup (`release_floated_source_cleanup`).

### What the debugger showed (and a correction to this document)

`mfb build --debug -O0`, then `/tmp/main-dbg-target/release/mfb dbg`:

- `rk`: `bp keep`, `run`, `wp x0+8 8`, `run` stopped at
  `_mfb_arena_free+0x168` with `0x…a5138: 0x…a50c0 -> 0x10`, `bt` =
  `mfb_arena_free ← mkKept+1392 ← main+56`: `u`'s scope-exit drop frees the box
  while the returned record still points at it. The next `run` is the fault in
  `main+0x94` at address `0x30`. (A `wp x0 8` on the tag word does not fire: the
  tag is 0 and the freed block's first word stays 0.)
- Shape (c): `bp mkStreams`, `run`, `finish` gives the list block
  (`x1 = 0x…c180c0`); with `bp _mfb_arena_free`, the frees from `once` are the
  record (112 B), then **16 B at `0x…c18110`, which is list block + 80** (the
  inline element 0), then the node, then the 96-byte list block itself.

So the cause of (c) and (d) written above is not exact. The callee's binding
frees its own box once, correctly. The caller's adopting drain was handed, as
"the box", the address of the element inside the returned list's block, and freed
16 bytes out of the middle of that block. That is the `double_free_skips` in (c)
and the heap corruption behind the SIGSEGV in (d).

### The `block_held_elsewhere_strict` exclusion (bug-1000)

Left in place. Making (c) and (d) correct does not need it removed: with the
exclusion still there, `returned_literal` and `returned_floated` in
`rt_resource_union_box_owner` report `double_free_skips 0` at N=10 and N=40 and
`live_bytes 0`. Its stated reason is gone (a drain frees no box now), and its
comment says so; removing it would only widen which union collections free their
block and records, which the Goal does not ask for and which has not been
measured. What it still costs, measured on this fix (`mfb build -q --debug`,
`N=10` then `N=40`, `live_bytes`): a `List OF RES Handle` that is iterated keeps
160 B per iteration (`n_foreach`: 1600, 6400), one read through
`collections::get` 240 B (`m_append_call`: 2400, 9600), a `Map … TO RES Handle`
read through `get` 128 B (`n_map_union`: 1280, 5120). The same append-then-`get`
shape over `List OF RES fs::File` is 0 (`x_conc_app`). These are the list block
and the closed records, not boxes (each is 16 B per box lower than on main).

### Measurements

macOS aarch64, `mfb build -q --debug`, `live_bytes` and `double_free_skips` of
`arena.0`, `once()` run N times. "main" is `/tmp/main-dbg-target/release/mfb`
(3fa35d543); "fixed" is this branch. Opens and closes are counted with the
debugger (`bp _mfb_rt_fs_fs_openFile` / `bp close`, the number of `[BP]` stops at
N=4; 0 closes at N=0 for every row).

| Shape | main: N=10 / N=20 | fixed: N=10 / N=40 | slope | opens = closes (N=4) |
|---|---|---|---|---|
| `rk` (the fixture) | exit 139 | `kept: file`, exit 0, dfs 0 | — | — |
| `rk` in a loop (`kept`) | exit 139 | 1120 / 4480, dfs 0 | 112 B (the closed record, see below) | 4 = 4 |
| argument, variant local (`arg_var`) | 160 / 320 | 0 / 0 | 0 | 4 = 4 |
| argument, producer call (`arg_call`) | 1280 / 2560 | 1120 / 4480 | 112 B (bug-842 tombstone) | 4 = 4 |
| list literal `[a0]` | 160 / 320 | 0 / 0 | 0 | 4 = 4 |
| list literal `[openOne()]` | 160 / 320 | 0 / 0 | 0 | 4 = 4 |
| record field `HB[a0]` | 160 / 320 | 0 / 0 | 0 | 4 = 4 |
| (c) returned `[s]` | 0 / 0, dfs 9 / 19 | 0 / 0, dfs 0 | 0 | 4 = 4 |
| (d) returned, floated in the callee | exit 139 | 0 / 0, dfs 0 | 0 | 4 = 4 |
| contrast: `RES one AS Handle`, then use | 0 / 0 | 0 / 0 | 0 | 4 = 4 |
| contrast: bound, then `[a]` | 0 / 0 | 0 / 0 | 0 | 4 = 4 |
| contrast: bound, then `append` | 0 / 0 | 0 / 0 | 0 | 4 = 4 |

The 300-iteration `touch("", openOne())` loop of the Goal is `arg_call`: 112 B
per iteration, the documented tombstone, against 128 B on main.

The sibling matrix (`fixed`, N=10 / N=40 `live_bytes`; every row exit 0 with the
expected total, `double_free_skips 0`, the same numbers at `-O 0` and `-O 2` for
N=10 / N=20, 300 iterations under `ulimit -n 64` pass, and opens = closes):

| Shape | main N=10 | fixed N=10 / N=40 |
|---|---|---|
| `MATCH` on the field, then the field as an argument | 160 | 0 / 0 |
| `RES v AS Handle = b.h` | 160 | 0 / 0 |
| the same, then `b = HB[c]` and a read of `v` | 320 | 0 / 0 (with the bind's copy disabled it prints 0 instead of 10) |
| `WITH b { h := c }` | 320 | 0 / 0 |
| a copy of the record (`LET b2 = b`) | 160 | 0 / 0 |
| a nested record (`Outer[3, HB[a]]`), `WITH` on the outer | 160–320 | 0 / 0 |
| a list of such records, literal and appended in a loop, `FOR EACH` | 320–480 | 0 / 0 |
| a map of such records, read through `get` | 160 | 0 / 0 |
| a record returned through two frames | exit 139 | 1120 / 4480 (112 B, the closed record) |
| the field reassigned (`b = HB[c]`, `b = WITH b { h := a }`) | 480 | 0 / 0 |
| a closure capturing the record; one capturing a union binding | 160; 0 | 0 / 0 |
| a variant record of a data union holding the field | 160 | 0 / 0 |
| a call that fails with a box pending (inline `TRAP`) | 160 | 0 / 0 |
| a function unwinding past pending boxes into a caller's `TRAP` | 160 | 0 / 0 |
| `FAIL`, `EXIT FOR`, `CONTINUE FOR`, `RETURN` inside a statement holding a box | 480 | 0 / 0 |
| a loop of wraps, fields and literals | 2400 | 0 / 0 |
| `RETURN b.h` (a field returned as a union) | 320 | 0 / 0 |
| an unbound call returning a union, and one bound | 1280 | 1120 / 4480 (the bug-842 tombstone) |
| module-level `List OF RES Handle` and `List OF HB` | runs | runs, 3 opens = 3 closes at N=3 |

"The closed record" in the `kept` rows: a record a callee hands back inside a
record is closed by the caller's binding and not freed. It is the same 112 B with
a concrete `RES fs::File` field on main and on this fix (`x_conc_keep`, 1120 /
2240 on both), so it is the resource record and not a box. Not fixed here.

### Golden impact

`scripts/artifact-gate.sh target/release/mfb all`: 1658 tests, 1837 builds, 2320
goldens checked, **5 diffs**, all one fixture:

- `byte-identity/http/http_codegen_cover_rt.{macos-aarch64,linux-aarch64,linux-x86_64,linux-riscv64,windows-x86_64}.ncode`

`http` is the one cover fixture with a resource union (`http::Stream`). Compared
function by function against the base compiler's `-ncode` on each of the five
targets (`mfb build -q --ncode --target <t>` with both compilers), the same four
functions differ and nothing else, no data object, import or other section:
`http_startRead` (+2 instructions), `http_read` (+2), `http_write` (+2),
`http_startExchange` (+4). Each gains one
`union_box_temp` stack slot per union wrap (two in `startExchange`), its
prologue zero store and the spill of the box pointer; the later slots shift by 8.
No call and no label is added or removed (the same `bl` targets and label names,
counted). In all four the wrap is bound or returned, so the temporary is claimed
and no free is emitted: that is the "statement-scope boxes" part of the fix, the
registration every pending temporary makes. No golden is regenerated here.

No other golden moves: no cover fixture has a `RES <resource union>` record
field, a floated or adopted resource-union element, or an unbound union wrap.

The new fixture's goldens are its own (`golden/build.log`, `.ast`, `.ir`, an
empty `.run`).

### The test this fix changed

`tests/runtime/rt_resource_union_box_owner.rs` is this bug's own RED test. Its
two `kept` cases were written with a slope of 0 and now allow 112 B. (1) written
in the RED commit of this bug; (2) it protects "a record field does not read a
freed box, and no box leaks"; (3) nothing else depends on it; (4) the 0 was
wrong: the Goal above allows the 112 B, and the same program with a concrete
`RES fs::File` field keeps 112 B per call on main (`x_conc_keep`: 1120 at N=10,
2240 at N=20), so the bytes are the closed resource record, not a box. No other
test, count or golden was edited.

### Found, not fixed (reported to the coordinator)

- A record a callee hands back inside a record keeps its closed resource record
  (112 B per call), concrete or union (`x_conc_keep`).
- `RES v AS Handle = collections::get(hs, 0)` is an address inside the list's
  block; growing the list afterwards leaves `v` reading freed memory (the
  `MATCH` on it falls through every case, no crash). Same on main.
- A concrete `fs::File` binding appended to a module-level `List OF RES Handle`
  fails the build: "a resource of type 'fs.File' is stored into a module-level
  binding, but no module-level binding's type reaches that resource (bug-806)".
  Same on main.
- The union-collection leaks listed under the exclusion above.
- A call through a function value that returns a resource union, and a builtin
  that does, are not registered as box temporaries (`value_yields_fresh_union_box`
  answers only for a function of the program), so an unbound one keeps its 16 B
  as before.

### Checks run (all on the fix, macOS aarch64)

- `cargo test --test rt_resource_union_box_owner`: 11 passed (before the fix, 6
  of the first 7 failed).
- `scripts/test-accept.sh target/release/mfb <out> resource-union-box-owner-rt`:
  `kept: file`, `[exit 0]` (before: `[exit 139]`). All 59
  `tests/rt-behavior/resources` fixtures and the 29 other fixtures whose source
  declares a `UNION` or calls `http::` pass (88 ran).
- `cargo test --test` for `codegen_helper_scratch_release` (12),
  `rt_resource_temporary_close` (1), `rt_inline_trap_union_bind` (6),
  `rt_union_trap_bind_default` (3), `rt_resource_drop_reclaims` (16),
  `rt_resource_review_804` (7), `_805` (18), `_806` (17),
  `rt_resource_owner_scope` (8), `rt_imported_resource_scope_drop` (4),
  `rt_scope_drop_leaks` (154), `rt_owned_argument` (13), `rt_debug_soak` (49),
  `rt_http_async_stream` (2), `rt_macos_d4_union_state_tls` (1),
  `rt_inplace_self_update` (1, 3 ignored), `codegen_string_return_freshness`
  (11), `codegen_return_interior_temp_drop` (5), `cli_thread_accept_res_bind`
  (5): all pass.
- Guards: `architecture_guards` (8), `example_project_names` (1),
  `example_readmes` (1), `no_operator_strings` (8), `no_type_strings` (7),
  `inplace_self_update_census` (2), `test_targets_registered` (4): all pass.
- Unit tests, `cargo test --bin mfb -- <filter>`: `codegen::cleanup::owned::`
  (6), `codegen::resource::` (32), `codegen::collection::layout::` (19),
  `codegen::memory::marshal::` (11), `codegen::engine::value::` (6),
  `codegen::engine::control::` (3), `target::shared::validate::` (17),
  `every_body_mfb_member_declares` (1): all pass.
- `scripts/spec-census.sh --render` (no leaked marker in any package) and
  `--citations` (`MISS-PATH 0`, `MISS-LINE 0`, `MISS-SYMBOL 0`).
- `rustup run 1.96.0 cargo fmt --all --check`: clean. `cargo clippy
  --all-targets`: no warning that is not in `/tmp/bs19-clippy-baseline.log`,
  compared per file and message.
- `examples/browser` (a `List OF RES http::Stream STATE PendingState` of floated
  elements) builds, all four packages.

