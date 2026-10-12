# bug-1068: the heap values of module-level bindings are not freed at exit

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory not released at exit)

Status: Open
Regression Test: (proposed) a `--debug` test over the six programs below asserting `live_bytes 0` and `alloc_calls == free_calls` at exit

A module-level (global) binding holds its value until the program exits. At exit the value is
not freed: its blocks are still live when the process ends. Every kind measured does it: a
`List OF RES`, a `List OF Integer`, a `String`, a `RES fs::File`, and a `RES fs::File STATE
Cur` (the form this document was first filed for, 128 bytes).

**Owner ruling (2026-10-11):** a module-level binding holds its value until exit, and it must
still be freed during the exit cleanup, not left in use when the process ends. The free
overwrites the memory, which is a security property.

**Owner ruling on the fix (2026-10-11):** when the instance's arena is removed, the teardown
first frees what is marked with the module's own scope: the values of the module-level
bindings, and the records owned by the module (`MODULE_OWNER_TOKEN`). It must NOT free
everything that is still live. A blanket free at teardown would make every leak invisible
(`live_bytes` would always end at 0); only what the module scope owns is released, so a block
leaked by a function still shows as live at exit.

**Correct behavior:** `live_bytes 0` and `alloc_calls == free_calls` at exit in all six
programs; every descriptor a module-level resource holds is closed once (as it is today).

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`,
main at b6bf0327b) and the batch 22 staging compiler, the same result on both:
`mfb build -q --debug`, run, reading the `arena.0.*` counters printed to stderr at exit
(`SRC=bugs/repro/bug-1068 /tmp/bs22/bin/probe.sh <mfb> <name>`). All exit 0.

| program | module-level binding | alloc / free / live at exit |
| --- | --- | --- |
| `g1` | `MUT gKeep AS List OF RES fs::File = []`, never appended to | 2 / 1 / 48 |
| `g2` | `MUT gInts AS List OF Integer = []`, one append | 3 / 2 / 80 |
| `g3` | `MUT gS AS String = "a"`, one concatenation | 3 / 2 / 16 |
| `g4` | `MUT gKeep AS List OF RES fs::File = []`, one file appended | 8 / 6 / 352 |
| `g5` | `RES g AS fs::File STATE Cur = fs::createTempFile()` | 7 / 5 / 128 |
| `g6` | `RES g AS fs::File = fs::createTempFile()` | 5 / 4 / 112 |

A program with no module-level binding (`io::print("x")` only) ends 0 / 0 / 0.

The programs are in `bugs/repro/bug-1068/`. `g4`:

```
IMPORT io
IMPORT fs
IMPORT collections
MUT gKeep AS List OF RES fs::File = []
FUNC main AS Integer
  RES f AS fs::File = fs::createTempFile()
  gKeep = collections::append(gKeep, f)
  io::print(toString(len(gKeep)))
  RETURN 0
END FUNC
```

`g5`:

```
IMPORT io
IMPORT fs
IMPORT collections
TYPE Cur
  n AS Integer
END TYPE
RES g AS fs::File STATE Cur = fs::createTempFile()
FUNC main AS Integer
  io::print(toString(g.state.n))
  RETURN 0
END FUNC
```

## What is known

How scopes are tracked today (read from the source, not traced):

- A local's scope is compile-time only: binding a heap-owning `LET`/`MUT` pushes an
  `ActiveCleanup::OwnedValue` onto `CodeBuilder::active_cleanups`
  (`src/codegen/engine/builder/mod.rs`), a block remembers the vector's length when it
  starts, and every exit of the block emits the frees for the entries above that length.
  Nothing is stored at run time.
- A module-level binding is never lowered inside a function body, so it has no entry in any
  `active_cleanups` and no free is emitted for it.
- Only resource records carry a run-time owner word. One stored into a module-level binding
  gets `MODULE_OWNER_TOKEN` (all ones) and a node on the per-instance `$module_resources`
  list (`src/codegen/resource/module_owner.rs`). The teardown `_mfb_module_resource_drain`
  closes each listed resource and, by its own module doc, "reclaims nothing".

So the module scope's contents are known: the globals and their types statically, the
module-owned records through the list. The teardown has to free exactly those. The module-level drain at exit (`mfb_module_resource_drain`) closes the resources
a module-level binding holds; nothing frees the bindings' blocks, records or `STATE` payloads
afterwards (guess, from the counters). The constant 48 live bytes in the tables of bug-999,
bug-1043 and bug-1056 (`bugs/completed/`) is this defect: those programs declare one empty
module-level list.

Found during batch 22 (reported by bs22-agent-b for the stateful form; the other forms
measured by the coordinator after the owner's ruling).
