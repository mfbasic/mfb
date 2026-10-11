# bug-1037: a resource-union binding with an inline `TRAP` keeps 128 bytes every time it is evaluated

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs21-agent-a, 2026-10-11). See "Resolution" at the end.
Regression Test: `tests/runtime/rt_resource_union_reclaim.rs` — `a_trapped_resource_union_binding_releases_its_default`, `a_trapped_resource_union_binding_recovered_releases_its_default`, `a_trapped_resource_union_binding_in_a_loop_releases_its_default` (each a `--debug` build read at N=10 and N=40, plus 300 iterations under a 64-descriptor limit); the five inline-`TRAP` cases of `tests/runtime/rt_resource_union_box_owner.rs`, whose slopes lost their 128 B allowance

`RES v AS Handle = mkH() TRAP(e) … END TRAP`, where `Handle` is a resource union and no
collection is involved, leaves 128 bytes live per evaluation.

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at
a236fdd44): `mfb build -q --debug`, run with `N=10`. Result: stdout `10`, `alloc_calls 72`,
`free_calls 52`, `live_bytes 1280`. The batch-20 compiler gives the same numbers. The
bug-1026 agent measured 5,120 at `N=40` (not re-run).

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

FUNC mkH AS RES Handle
  RETURN openOne()
END FUNC

FUNC once() AS Integer
  RES v AS Handle = mkH() TRAP(e)
    RETURN 5
  END TRAP
  RETURN which("v", v)
END FUNC
```

## What is known

128 bytes is a 16-byte union box plus a 112-byte record. Guess (the bug-1026 agent's, not
traced): the inline-`TRAP` temp is bound to a closed default first, and no `Assign` path
drops that default when the call's result replaces it.

Found by the bug-1026 agent (batch 20).

## Resolution (bs21-agent-a, 2026-10-11)

### Root cause

The guess in "What is known" is right. The desugar gives codegen

```
bind   $trap_res4 : Result OF Handle = callResult mkH()
bind   $trap_val5 : Handle = <no initializer>
if resultIsOk($trap_res4) then assign $trap_val5 = resultValue($trap_res4) else <handler>
bind   v : Handle = local $trap_val5
```

The temp's bind materializes a closed default (`lower_default_value`: a 16 B
`{tag, record}` box and a 112 B record) and registers the temp's tag-dispatched
drop (`ActiveCleanup::ResourceUnion`). The `NirOp::Assign` arm of
`builder_control.rs` drops the old value only when the assigned type has a
concrete close (`resource_cleanup_symbol`), is freeable-flat, or owns a graph. A
resource union is none of those, so the store overwrote the slot and nothing
named the default again. A concrete `RES f AS fs::File = … TRAP` takes the first
of those branches, which is why it was balanced.

Debugger proof on the base compiler (`-O0 --debug`, `N=1`,
`bp _mfb_arena_alloc list reg x0; bt 2`, then the same on `_mfb_arena_free` with
`x0 x1`): `once` allocates 24 B (`once+168`, the `Result`), 112 B (`once+1672`)
and 16 B (`once+1828`); `mkH` allocates its 16 B box and `fs::openFile` the 112 B
record. `once` frees one 112 B block (`…c050`, the record `fs::openFile` made),
one 16 B block (`mkH`'s box) and the 24 B `Result`. The 112 B and 16 B blocks
made at `once+1672` / `once+1828` are never freed: 9 allocations, 7 frees.

### The fix

`src/codegen/engine/control/builder_control.rs`, `NirOp::Assign`: a binding of a
resource-union type that has a registered drop (`resource_union_binding_cleanup`,
new in `builder_resource_cleanup.rs`) runs that drop on the value it holds before
the new value is stored, with the new value spilled across it. It is the same
`emit_resource_union_cleanup_call` the scope exit runs, so it answers for the old
value the same way: the bug-648 owner flag still describes that value (it is
rewritten after the store), a lent value loses only the temp's copy of its box,
the record is freed only when `record_ownership` proved the temp owns its
records, and the slot is nulled before the store. The only binding that reaches
it is the desugar's temp: a `RES` binding is not assignable
(`TYPE_ASSIGN_REQUIRES_MUT`).

### Measurements (fixed compiler, `N=<n> /tmp/bs21/bin/probe.sh target/release/mfb b1037`)

N=10: `alloc_calls 72`, `free_calls 72`, `live_bytes 0`, `double_free_skips 0`.
N=20: 142 / 142 / 0 / 0. `(ulimit -n 64; N=300 b1037.out)` prints 300, exit 0.

Sibling shapes, balanced at N=10 and N=40 with `double_free_skips 0` and the
64-descriptor run passing (base compiler's live bytes at N=10 in brackets):
`RECOVER a` not taken [1280] and taken [1280]; `RECOVER openOne()` taken [1280];
a handler that `FAIL`s through the caller's `TRAP` [1280]; a loop body whose
handler leaves by `CONTINUE FOR` [2560]; the same binding floated into a list
that outlives the loop, then iterated [7200]. A builtin producer
(`RES v AS Handle = fs::openFile(…) TRAP`), succeeding and failing, was already
balanced and still is.

Two residues that are not this bug, each the same with a concrete
`RES v AS fs::File`:

* A trapped call that hands back its own argument (`RES v AS Handle = same(a)
  TRAP`): the box is now freed, the 112 B default record is not (1280 → 1120 at
  N=10), because `record_ownership` cannot prove the temp owns every record
  stored into it, and an unproven binding closes without freeing.
  `RES v AS fs::File = sameF(a) TRAP` keeps the same 1120.
* A `STATE` on the trapped binding (`RES v AS Handle STATE Cur = mkH() TRAP`):
  1600 → 320 at N=10. The 32 B left per evaluation is the `STATE` block, which
  the temp's drop does not free because the temp is typed without the `STATE`.
  `RES v AS fs::File STATE Cur = openOne() TRAP` keeps the same 320.

### Golden impact

None. `scripts/artifact-gate.sh target/release/mfb all`: 1674 tests, 2338
goldens checked, 0 diffs. No fixture assigns into a resource-union temp.

