# bug-1051: a trapped call that returns its own argument keeps the 112-byte default record on every evaluation

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-c)
Regression Test: `tests/runtime/rt_resource_last_holder.rs`, `a_trapped_call_that_returns_its_argument_frees_the_default_record` (`cargo test --test rt_resource_last_holder`)

`RES v AS fs::File = sameF(a) TRAP(e) … END TRAP`, where `sameF(h)` returns its argument,
keeps 112 bytes per evaluation. The descriptor is closed.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb c9t`: exit 0, stdout
`10`, `alloc_calls 52`, `free_calls 42`, `live_bytes 1120`. `N=20`: stdout `20`, 102 / 82 /
2240. `(ulimit -n 64; N=300 …)`: exit 0, stdout `300`, 1502 / 1202 / 33600.

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
FUNC mkH AS RES Handle
  RETURN openOne()
END FUNC
FUNC mkBad(n AS Integer) AS RES Handle
  IF n > 0 THEN
    FAIL error(77, "bad")
  END IF
  RETURN openOne()
END FUNC
FUNC sameF(h AS fs::File) AS fs::File
  RETURN h
END FUNC
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  RES v AS fs::File = sameF(a) TRAP(e)
    RETURN 5
  END TRAP
  RETURN len(fs::readAll(v)) - 5
END FUNC
```

## What is known

1120 / 10 = 112 bytes per call, the size of the closed default `fs::File` record the trap's
temp is bound to. bug-1037's resolution (completed) recorded this as a residue it did not
fix: the temp's default record is not freed because `record_ownership` cannot prove the temp
owns every record stored into it, and an unproven binding closes without freeing. It states
the same for `RES v AS Handle = same(a) TRAP`, whose box was fixed and whose default record
stays (1280 to 1120 at N=10). That is the reported cause and was not verified here.

Found during batch 21.

## Resolution (bs22-agent-c, 2026-10-11)

### Cause

As bug-1037 recorded it. The inline-`TRAP` desugar binds a temporary with no initializer
(`bind $trap_valN : fs.File`), which materializes a closed default record the temporary
claims, and delivers the call's value by assignment. The assignment first runs the
temporary's own drop on the value it replaces (`NirOp::Assign`, the resource arm of
`builder_control.rs`). That drop frees a record only when the temporary is an owning local
(`record_ownership`: every store fresh). Here the stored value is `sameF(a)`'s result, which
is `a`'s record, so the temporary is rightly not an owning local, and the drop closed the
default (a no-op, it is closed) and freed nothing. The default is not `a`'s record: it is
the temporary's own, whatever is stored over it. The same on the path where the handler
leaves without storing (`CONTINUE`, `RETURN`): the scope's drop found the default in the
slot and kept it.

Debugger on the fixed `-O0 --debug` build of `t1` (the filed shape) at N=1, `bp
_mfb_arena_free`, listing `x0`/`x1` at each stop: two frees of 112 bytes under `once`, the
default record at `once + 2636` (the delivery) and `a`'s record at `once + 4884` (the exit),
at different addresses. The base build has only the second.

### Fix

The bind of such a temporary remembers the default record's address in a frame slot
(`CodeBuilder::remember_trap_default_record`, zeroed with the frame), carried on the
cleanup as `ResourceCleanup::default_record_slot` / `ResourceUnionCleanup::default_record_slot`.
A drop of the temporary that frees no record by proof compares the record it holds with
that address and frees it when they are equal, then clears the slot
(`emit_free_trap_default_record`): once only, and never a stored record. The concrete drop
does it after its reclaim; the union drop before it frees its box, for the record the
default box names. It is set only for a value-less bind of a record-freeing kind that is
not an owning local (an owning local's ordinary free already covers the default), so no
other binding's code changes.

### Measured

Fixed compiler `target/release/mfb` against the staging compiler
`/tmp/bs22-target/release/mfb`, macOS aarch64, `--debug` counters, N=10
(`N=<n> P=/tmp/bs22-agent-c/p SRC=/tmp/bs22-agent-c/src /tmp/bs22/bin/probe.sh <mfb> <name> [-O0]`).
Every fixed row is the same at `-O0`, balanced at N=20, passes `(ulimit -n 64; N=300
<program>)`, and has `double_free_skips 0`.

| row | shape | base | fixed N=10 | fixed N=20 |
| --- | --- | --- | --- | --- |
| `b1051` / `t1` | as filed: `RES v AS fs::File = sameF(a) TRAP(e) RETURN 5` | 52/42/1120 | `10`, 52/52/0 | 102/102/0 (N=300: 1502/1502/0) |
| `t2` | `RES v AS Handle = same(a) TRAP …` (the union form the document names) | 82/72/1120 | `10`, 82/82/0 | 162/162/0 |
| `t3` | in a loop; the callee fails every other time and the handler `CONTINUE`s | 92/72/2240 | `1070`, 92/92/0 | 182/182/0 |
| `t4` | the same, `RECOVER b` (another binding) | 122/102/2240 | `140`, 122/122/0 | 242/242/0 |
| `t5` | the union form, with `RECOVER b` and with `CONTINUE` | 282/242/4480 | `1150`, 282/282/0 | 562/562/0 |
| `t6` | the argument is a record field, and a `collections::get` element; beside `RES p = openOne() TRAP` (control, owned) | 192/152/5760 | `190`, 192/192/0 | 382/382/0 |

`t6` was 192/172/2240 with bug-1053 alone (its record `k` and its list `xs` are that
bug's); the two remaining records per call are the two defaults.

### Checks

* RED: `MFB_TEST_EXE=/tmp/bs22-target/release/mfb cargo test --release --test rt_resource_last_holder a_trapped_call`:
  1 failed (`same_file N=10: 52 allocations, 42 frees, 1120 bytes live at exit`). GREEN:
  `cargo test --release --test rt_resource_last_holder`: 12 passed.
* `cargo test --release --no-fail-fast` over 29 targets (the always-run set, `rt_resource_store_float`,
  `rt_resource_trap_state_free`, `rt_resource_union_member_float`, `no_type_strings`, `architecture_guards`,
  `rt_inline_trap_borrowed_resource`, `rt_inline_trap_union_bind`, `rt_inline_trap_success_alloc`,
  `rt_inline_trap_nested_call`, `rt_inline_trap_short_circuit`, `rt_inline_trap_expression_boundary`,
  `rt_trapped_call_capability_gate`, `rt_scope_drop_leaks`, `rt_debug_soak`, `codegen_owned_drop_free_and_null`,
  `rt_res_rebind_alias`, `rt_resource_review_804`/`805`/`806`): 29 `test result: ok`, 0 failed.
* `cargo test --release --bin mfb -- codegen::resource:: codegen::cleanup:: every_body_mfb_member_declares`: 48 passed.
* `scripts/test-accept.sh target/release/mfb <out> '*resource*' '*canvas*' '*collection*' '*bug842*' '*bug1012*'
  '*bug1000*' '*bug805*' '*bug1027*' '*bug806*' '*tcp*' '*udp*' '*trap*'`: passed.
* `scripts/spec-census.sh --render memory`, `--citations memory`: 0 leaks, 0 misses.

### Golden impact

None. `scripts/artifact-gate.sh target/release/mfb all`: 2348 goldens checked, 14 diffs, the
same 14 as before this change (ten are bug-1053's tcp and udp covers, four the `.ir` goldens
bug-1054 lists). No fixture with a golden traps a call into a temporary that is not an owning
local.

### Shared functions changed

* `ResourceCleanup` and `ResourceUnionCleanup` (`src/codegen/engine/builder/mod.rs`): new field
  `default_record_slot: Option<usize>` (every constructor sets it: the two bind arms and the
  assign arm of `builder_control.rs`, `register_union_box_cleanup`). `CodeBuilder`: new field
  `trap_default_records`.
* `CodeBuilder::emit_resource_cleanup_call` and `emit_resource_union_cleanup_call`
  (`src/codegen/resource/cleanup/builder_resource_cleanup.rs`): signatures unchanged; each frees the
  default record as described. New `emit_free_trap_default_record`, `remember_trap_default_record`.

### Spec

`mfb spec memory arenas` (the trapped temporary's default record).
