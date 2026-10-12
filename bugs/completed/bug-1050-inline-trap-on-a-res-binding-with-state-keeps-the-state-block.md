# bug-1050: `RES v AS fs::File STATE Cur = openOne() TRAP …` keeps the 32-byte `STATE` block on every evaluation

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-b)
Regression Test: `tests/runtime/rt_resource_trap_state_free.rs` (`cargo test --test rt_resource_trap_state_free`)

A `RES` binding that carries a `STATE` clause and an inline `TRAP` keeps the 32-byte `STATE`
block each time it is evaluated. The descriptor is closed.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb s3`: exit 0, stdout
`10`, `alloc_calls 92`, `free_calls 82`, `live_bytes 320`. `N=20`: stdout `20`, 182 / 162 /
640. `(ulimit -n 64; N=300 …)`: exit 0, stdout `300`, 2702 / 2402 / 9600.

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
TYPE Cur
  pos AS Integer
  note AS String
END TYPE
FUNC once() AS Integer
  RES v AS fs::File STATE Cur = openOne() TRAP(e)
    RETURN 5
  END TRAP
  v.state.pos = 3
  v.state.note = "abc" & toString(v.state.pos)
  RETURN len(fs::readAll(v)) - 5 + v.state.pos - 3
END FUNC
```

## What is known

320 / 10 = 32 bytes per call; 640 / 20 and 9600 / 300 give the same. bug-1037's resolution
(completed) recorded this as a residue it did not fix: "The 32 B left per evaluation is the
`STATE` block, which the temp's drop does not free because the temp is typed without the
`STATE`." That is the reported cause and was not verified here. It also records the same
residue for `RES v AS Handle STATE Cur = mkH() TRAP`.

Found during batch 21.

## Resolution (bs22-agent-b, 2026-10-11)

The reported cause is right. `mfb build -nir` of the program shows the desugar:
`bind $trap_val5 : fs.File = null`, `assign $trap_val5 = resultValue $trap_res4`, then
`bind v : fs.File STATE Cur = local $trap_val5`. `v` is an alias of the temporary's record
(the `aliases_live_resource` arm of `NirOp::Bind` in `builder_control.rs`) and registers no
cleanup; the temporary's `ActiveCleanup::Resource` is the one that runs, and its
`state_type` was `None` because the temporary is typed by the producer. The binding's
`emit_resource_state_init` attached a `Cur` block to the record, each write to the `String`
field replaced it, and no drop freed the last one.

**Fix.** The aliasing arm now does what the resource-union wrap arm beside it already did
for `RES c AS Union STATE S = u`: when the binding declares a `STATE`, it follows the alias
chain to the record's owner (`resource_alias_root`) and records the state type on that
owner's cleanup, a `Resource` or a `ResourceUnion`, if it had none. The owner's drop then
frees the block after its close. A root with no live cleanup (a parameter) is left alone:
its state is the caller's owner's.

### What was measured

Fixed compiler `target/release/mfb`, macOS aarch64, `--debug` counters, default level; every
row gives the same line at `-O0` (`/tmp/bs22-agent-b/m1050/run.sh <mfb> -O0`). Programs:
`/tmp/bs22-agent-b/m1050/<row>.mfb` (prelude `pre.txt`). Base is main's compiler. "fd" is
`(ulimit -n 64; N=300 <program>)`; `double_free_skips` is 0 in every row.

| row | shape | base N=10: alloc/free/live | fixed N=10 | fixed N=20 | fd |
| --- | --- | --- | --- | --- | --- |
| `a` | as filed (`s3`) | 92/82/320 | `10`, 92/92/0 | `20`, 182/182/0 | 0 |
| `j` | a built-in opener written directly, `fs::openFile(...) TRAP` | 72/62/320 | `10`, 72/72/0 | `20`, 142/142/0 | 0 |
| `b` | the union form, `RES v AS Handle STATE Cur = mkH() TRAP` | 112/102/320 | `10`, 112/112/0 | `20`, 222/222/0 | 0 |
| `d` | the producer fails, the handler `RECOVER`s with another file | 92/82/320 | `10`, 92/92/0 | `20`, 182/182/0 | 0 |
| `e` | the binding in a loop body (three per call) | 272/242/960 | `30`, 272/272/0 | `60`, 542/542/0 | 0 |
| `f` | an early `RETURN` after a state write | 62/52/320 | `10`, 62/62/0 | `20`, 122/122/0 | 0 |
| `m` | the union form in an `IF` body in a loop | 112/102/320 | `10`, 112/112/0 | — | 0 |
| `l` | the stateful binding handed to a `STATE` parameter that re-binds it | 52/42/320 | `10`, 52/52/0 | — | 0 |
| `k` | no `TRAP`: `RES a AS fs::File = openOne()`, `RES b AS fs::File STATE Cur = a` | 72/62/320 | `10`, 72/72/0 | — | 0 |
| `c` | control: the binding with no `TRAP` | 72/72/0 | unchanged | | 0 |
| `g` | control: a callee returns the stateful file past its trapped binding | 52/52/0 | unchanged | | 0 |
| `h` | control: the handler diverges every time | 52/52/0 | unchanged | | 0 |

Row `k` shows the defect was never the `TRAP`: any binding that attaches a `STATE` to a
record another binding owns leaked the block, and the `TRAP` desugar is the spelling that
always produces one.

One row keeps memory and is another defect: `i`, the trapped stateful binding floated into
an outer `List OF RES fs::File STATE Cur` (`30`, 202/132/6720), equals its no-`TRAP` control
`i2` on the BASE compiler (142/72/6720): a stateful list that elements float into keeps every
record and state block, with or without a `TRAP`.

### Checks

* RED: `MFB_TEST_EXE=/tmp/main-dbg-target/release/mfb cargo test --release --test rt_resource_trap_state_free`:
  3 failed (`as_filed N=10: 92 allocations, 82 frees, 320 bytes live at exit`), 1 passed (the controls).
  GREEN with the fix: 4 passed.
* `cargo test --release --no-fail-fast --test rt_resource_aggregate_temporary --test rt_resource_temporary_close
  --test rt_resource_drop_reclaims --test rt_resource_union_box_owner --test rt_resource_union_reclaim
  --test rt_resource_owner_scope --test codegen_helper_scratch_release --test rt_canvas_geo_native
  --test test_targets_registered --test rt_resource_union_member_float --test rt_resource_store_float
  --test rt_resource_trap_state_free --test rt_res_state_inplace_mutation --test no_type_strings
  --test architecture_guards`: all passed.
* `cargo test --release --bin mfb -- every_body_mfb_member_declares codegen::resource:: codegen::cleanup::`: 44 passed.
* `scripts/test-accept.sh target/release/mfb <out> '*resource*' '*canvas*' '*collection*' '*bug842*' '*bug1012*'
  '*bug1000*' '*bug805*' '*bug1027*' '*bug806*'`: 300 tests, passed.
* `scripts/spec-census.sh --render` / `--citations`: 0 misses.

### Golden impact

None. `scripts/artifact-gate.sh target/release/mfb all`: 2348 goldens checked, 4 diffs, the
four `.ir` goldens bug-1054 lists. The emitted code changes only for a function in which a
binding that declares a `STATE` aliases another binding's record, where the owner's drop
gains the state free; no fixture with a `.ncodesum` has one (`.ai/testing-gates.md`: no
byte-identity fixture contains an inline `TRAP`).

### Shared functions changed

Only the `aliases_live_resource` arm of `NirOp::Bind` in `CodeBuilder::lower_op`
(`src/codegen/engine/control/builder_control.rs`). No signature changed.

### Spec

`mfb spec memory arenas` (the scope drop of a resource: who frees the `STATE` payload).
