# bug-1001: dropping a resource-union binding bound straight from a producer leaks the variant's I/O buffer (4,096 B per drop)

Last updated: 2026-10-10
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (2026-10-10)
Regression Test: `tests/runtime/rt_resource_drop_reclaims.rs` (the four `a_*union*` cases): one `--debug` build run at 10 and 20 iterations must report `alloc_calls == free_calls`, `live_bytes 0` and `double_free_skips 0`

`RES u AS Stream = openBuffered()`, where `Stream` is a union with an `fs::File` variant and the
file is buffered (`fs::setBuffered(f, TRUE)`), leaks exactly one 4 KB block (the output buffer)
each time `u` is dropped. The alias form (`RES f = …` then `RES u AS Stream = f`) does not leak,
because the concrete `f` binding does the closing and its drop reclaims the buffer.

**Correct behavior:** the union drop frees the variant's I/O buffer, as the concrete binding's
drop does.

Suspected cause (from bs17-agent-f's reading of the code; not traced): the resource-union drop
(`emit_union_tag_dispatch_drop`) closes the variant and frees `STATE`, but never calls the
I/O-buffer reclaim.

## Failing Reproduction

Measured by the coordinator on 2026-10-06 with main 468ff0f94, `mfb build -q --debug`, arena 0
`alloc_calls / free_calls / live_bytes`:

| program | N=10 | N=20 | per iteration |
|---|---|---|---|
| leakB3 (union bound from the producer) | 91 / 81 / 40960 | 181 / 161 / 81920 | +4096 B, 1 block |
| leakB (alias form, control) | 91 / 91 / 0 | 181 / 181 / 0 | flat |

leakB3:
```
IMPORT os
IMPORT fs
UNION Stream
  fs::File
END UNION
FUNC openBuffered() AS RES fs::File
  RES f AS fs::File = fs::openFile(fs::pathJoin([fs::tempDirectory(), "mfb-leakB3.txt"]), "w")
  fs::setBuffered(f, TRUE)
  RETURN f
END FUNC
FUNC body() AS Nothing
  RES u AS Stream = openBuffered()
  MATCH u
    CASE fs::File(f)
      fs::writeAll(f, "U")
  END MATCH
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

### The read side too (bs17-agent-g, re-measured by the coordinator on main 468ff0f94)

An unbuffered file opened for reading, bound straight into the union and read through `MATCH`,
leaks the read buffer: **16 KiB per drop**. `--debug`, arena 0: N=10 gives 52 alloc / 42 free /
163840 live bytes; N=100 gives 502 / 402 / 1638400. So both the read and the write buffers are
missed; the fix must reclaim every I/O buffer the variant's record owns, as the concrete drop does.
It needs an existing `/tmp/g933/data.txt` with at least one line.

```
IMPORT io
IMPORT os
IMPORT fs
IMPORT tcp
UNION Stream
  fs::File
  tcp::Socket
END UNION
FUNC once() AS Integer
  RES s AS Stream = fs::openFile("/tmp/g933/data.txt")
  MATCH s
    CASE fs::File(h)
      RETURN len(fs::readLine(h))
    CASE tcp::Socket(k)
      RETURN 0
  END MATCH
  RETURN 0
END FUNC
FUNC main() AS Nothing
  MUT t AS Integer = 0
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    t = t + once()
  NEXT
  io::print(toString(t))
END FUNC
```

## Related

- bug-1000 (the same buffer leak through an owned-list drain).

## Root Cause

Confirmed, and it is what the report suspected. A resource-union drop is
`builder_resource_cleanup.rs:emit_union_tag_dispatch_drop`: read the tag, call the
active variant's close op, free the uniform `STATE` block, free the record when the
binding owns it (`record_free_tags`), then the caller frees the 16-byte box. The
concrete drop's `emit_resource_block_reclaim` frees one thing more before the
`STATE` block: the two per-`File` buffers at `FILE_OFFSET_BUF_PTR` (4,096 B) and
`FILE_OFFSET_READ_PTR` (16,384 B). The dispatch had no such step, so a `File`
variant's buffers were reachable only through a record the drop then freed.

The alias form was flat because `RES u AS Stream = f` is an alias wrap: its drop
frees only the box, and the concrete binding `f` runs the full reclaim.

## Resolution

- `emit_union_tag_dispatch_drop` takes `io_buffer_tags`: the tags of the variants
  whose record carries the two buffer words (`resource_uses_io_buffers`, i.e.
  `fs.File`; `resource_union_io_buffer_tags` collects them). For such a variant it
  runs `emit_resource_block_reclaim(payload_slot, None, true, false)` after the
  close, which is the concrete drop's own buffer step: it skips a moved record,
  frees each non-null buffer and nulls its word. The order is the concrete drop's:
  close (which flushes the output buffer), buffers, `STATE`, record, box.
- The buffers are reachable only through the record's two words and are nulled as
  they are freed, so the step is once-only whoever frees the record afterwards.
- Three callers, one rule each:
  - a lone binding's drop (`emit_resource_union_cleanup_call`) passes the tags
    whenever it closes the variant (`ResourceUnionCleanup.io_buffer_tags`);
  - an owned-list drain passes them under the same `owns_elements` proof as
    `record_free_tags` (`OwnedListDrop::Union.io_buffer_tags`);
  - the close of an unowned temporary (`owner.rs:emit_close_unowned_at`) passes
    them and keeps the record, as its concrete arm already reclaims the buffers
    and keeps the tombstone.
- Sibling found and fixed: `closure_carry.rs:emit_listed_record_close` (a resource
  union a closure carries, bug-843) freed only `STATE` on its `reclaim` path. It
  frees a `File` variant's buffers too.

### Runtime proof

`mfb build -q --debug`, arena 0, `alloc_calls / free_calls / live_bytes`, base
compiler `/tmp/main-dbg-target/release/mfb` against this branch, the report's own
programs (`/tmp/k/probe.sh <mfb> <name> 10 20`):

| program | base N=20 | fixed N=10 | fixed N=20 |
|---|---|---|---|
| leakB3 (write buffer) | 181 / 161 / 81920 | 91 / 91 / 0 | 181 / 181 / 0 |
| read side | 102 / 82 / 327680 | 52 / 52 / 0 | 102 / 102 / 0 |
| leakB (alias control) | 181 / 181 / 0 | 91 / 91 / 0 | 181 / 181 / 0 |

`double_free_skips 0` in every run. The fixed leakB3 allocates and frees exactly
what the control does.

Debugger (`mfb dbg`, `bp _mfb_arena_free bt 2; list reg x0; list reg x1`) on the
fixed read-side program at `N=1`: the drop frees, in order, the 16,384 B read
buffer, the 112 B record and the 16 B box, each once.

## Golden impact

Measured with `scripts/artifact-gate.sh <mfb> all` on this branch and on its base
commit `821806278` (built in a detached worktree): the base already reports 33
diffs it inherited (goldens not regenerated in this batch), and this branch
reports 39. The per-function comparison below is `mfb build -ncode` from the two
compilers.

**No golden moves for this bug.** No fixture with a native golden drops a
resource-union binding whose union has an `fs::File` variant, or drains an owning
list of such unions: every function of every already-diffing fixture is identical
to the base commit's except the ones bug-984 and bug-1000 account for. A union
with no `File` variant has an empty `io_buffer_tags` and emits what it did.
