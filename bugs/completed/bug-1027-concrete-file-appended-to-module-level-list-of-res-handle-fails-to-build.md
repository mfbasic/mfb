# bug-1027: a concrete `fs::File` binding appended to a module-level `List OF RES Handle` fails to build

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (internal error where the program should build, or get a diagnostic)

Status: Fixed (bs21-agent-b)
Regression Test: `tests/rt-behavior/resources/bug1027_member_into_module_union_rt`

`RES a AS fs::File = openOne()` followed by `gHs = collections::append(gHs, a)`, with `MUT gHs AS List OF RES Handle`
declared at module level, is refused with an internal-style error naming bug-806 instead of building. Binding it as
`RES a AS Handle` builds and runs.

**Correct behavior:** the program builds and runs (a `fs::File` is a member of `Handle`, so it can be stored into a
`List OF RES Handle`), or, if it is meant to be refused, it gets a source diagnostic at the line, not this message.

## Failing Reproduction

Measured on 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at 3fa35d543, macOS aarch64):
`N=10 /tmp/bs19b/bin/probe.sh /tmp/main-dbg-target/release/mfb q_global_bug` prints

```
BUILD FAIL
error: a resource of type 'fs.File' is stored into a module-level binding, but no module-level binding's type reaches that resource (bug-806) while lowering bind a AS fs.File
```

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

MUT gHs AS List OF RES Handle = []
MUT gBs AS List OF HB = []

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

FUNC once() AS Integer
  RES a AS fs::File = openOne()
  gHs = collections::append(gHs, a)
  gBs = collections::append(gBs, HB[a])
  RETURN which("g", collections::get(gHs, len(gHs) - 1)) + which("b", collections::get(gBs, len(gBs) - 1).h) - 1
END FUNC

FUNC main AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    t = t + once()
  NEXT
  FOR EACH b IN gBs
    t = t + which("z", b.h) - 1
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC
```

## What is known

The message comes from the check that a resource stored into a module-level binding must be reachable from some
module-level binding's type. The module-level types here are `List OF RES Handle` and `List OF HB`; `fs.File` is a
member of `Handle`, so the check apparently does not look through the union (guess; not traced). Changing the line to
`RES a AS Handle = openOne()` (the union typed binding) makes the program build and run.

Found by the bug-1012 agent while fixing bug-1012 (batch 19).

## Resolution (bs21-agent-b, 2026-10-11)

The intended outcome: the program builds and runs. It is sound, measured below, so no diagnostic was added.

**Root cause.** The guess above is right. A store into a module-level binding hands every resource it adds to the
module instance (bug-806), which lists the record under a `kind`: an index into the resource leaf types the
module's globals can reach (`CodeBuilder::module_resource_leaves`). The operand walk
(`emit_module_store_float` -> `named_store_operands`) raises the NAMED operands before the store, so for
`gHs = collections::append(gHs, a)` it raises `a` by its own type, `fs.File`, while the only leaf the globals reach
is `Handle`. `module_leaf_kind` looked the member up under its own name (or its own base) and failed. The message
says "while lowering bind a" because the walk runs where `a`'s owner is decided; the binding is not at fault.
Binding `a AS Handle` worked because the operand is then the union box, whose leaf is `Handle`.

**Fix.** `module_leaf_kind` (`src/codegen/resource/module_owner.rs`) resolves a concrete resource that is no leaf
itself to a resource-union leaf it is a member of, and returns the member's variant tag with it;
`emit_module_float` writes that tag into the node instead of reading one from a box (there is none yet). A union
leaf's node is "the active variant's record plus that variant's tag", and both are known here. Any union the
resource is a member of serves: variant tags are assigned per variant, not per union
(`recompute_canonical_variant_tags`), and the drain closes by the variant's close op.

### What was measured

Fixed compiler (`target/release/mfb`), macOS aarch64, `--debug` counters:

* `N=10 /tmp/bs21/bin/probe.sh target/release/mfb q_global_bug`: exit 0, prints `10`, `alloc_calls 112
  free_calls 100 live_bytes 4288 double_free_skips 0`. `N=300`: prints `300`, `3022 / 2720 / 132000 / 0`.
  (The doc's "prints `0`" was a slip: `once()` answers 1, so the program prints N.)
* The same program with `RES a AS Handle = openOne()`, which the BASE compiler accepts, built with the base
  compiler: `live_bytes 4288` at N=10 and `132000` at N=300, the same bytes. So the fix adds no block that the
  union-bound spelling does not already leave: what is live at exit is what the two module-level lists hold (the
  instance's arena is released with the instance) plus whatever bug-1028 leaks for `collections::get` on a list
  of resource unions, which this program calls twice per iteration and which is not this bug's.
* Closed once: `mfb dbg` on the `-O0 --debug` build at N=3, `bp _mfb_rt_fs_fs_close bt 2`, stops three times,
  each under `mfb_module_resource_drain`, and `_close` is entered with descriptors 5, 4 and 3, once each. Nothing
  closes a file when `once()` returns: `which("z", b.h)` in `main` reads all three.
* Sibling matrix, each against its `AS Handle` twin built with the base compiler, at N=10 and N=40
  (`/tmp/bs21-agent-b/src/m1..m5`): a list element (`2704`/`10672` live bytes, both), a map value
  (`3216`/`12720`, both), a record field through an inline `TRAP` (`1808`/`7088`, both), an early `RETURN`
  between two stores into two unions that list the member at different positions (builds and matches the right
  arm; its twin does not build on base, having a second concrete binding), and a loop body in `main`
  (`2896`/`11056`, both). `double_free_skips 0` everywhere.

## Checks run

* `scripts/test-accept.sh /tmp/main-dbg-target/release/mfb <out> 'bug1027*'` (base): 1 mismatch, the bug-806
  error. With the fix: passes.
* `scripts/test-accept.sh target/release/mfb <out> '*bug1027*' '*806*' '*module*' '*global*' '*union*'`: 61
  fixtures, passed.
* `cargo test --bin mfb -- codegen::resource::`: 32 passed.
* `scripts/artifact-gate.sh target/release/mfb all`: 2346 goldens checked, 0 diffs.

## Golden impact

None. The changed path is one that failed the build before.
