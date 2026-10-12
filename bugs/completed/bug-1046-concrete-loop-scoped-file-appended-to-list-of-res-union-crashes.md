# bug-1046: a loop-scoped concrete `RES fs::File` appended to a `List OF RES Handle` crashes with SIGSEGV

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory safety)

Status: Fixed (bs22-agent-b)
Regression Test: `tests/runtime/rt_resource_union_member_float.rs` (`cargo test --test rt_resource_union_member_float`)

`RES s AS fs::File = openOne()` declared in a `FOR` body and appended to a
`MUT hs AS List OF RES Handle` (a resource union) declared before the loop crashes the
program. It builds without a diagnostic.

**Correct behavior:** exit 0. By hand (not measured): `len(hs) - 2` is 1 per call, so stdout
`10` at `N=10`; `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb g1`: exit 139
(SIGSEGV), empty stdout, no counters (the program never reaches the exit report).
`N=20`: the same.

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
  MUT hs AS List OF RES Handle = []
  FOR j = 1 TO 3
    RES s AS fs::File = openOne()
    hs = collections::append(hs, s)
  NEXT
  RETURN len(hs) - 2
END FUNC
```

## What is known

Reported by the finder's debugger trace, not verified by the coordinator or by this
document: a bad access at `0x23` in `once+0x3ec` on `ldr x8,[x9,#0x20]` with `x9=3`. Their
reading is that the concrete `fs::File` record is read as a resource-union box (the union box
is `{tag, record}`; the concrete value is the bare record), so a field of the record is taken
for a pointer.

This is next to bug-1018 (a `List OF RES fs::File` given where a `List OF RES Handle` is
expected is accepted unconverted) and bug-1027 (a concrete `fs::File` binding appended to a
module-level `List OF RES Handle` fails to build). The append here is local to a function and
builds, then crashes. Whether the same append outside a loop crashes was not measured.

Found during batch 21.

## Additional reproduction (2026-10-11)

`i1` is the same crash with no loop. `once` calls `mkHandles`, which declares the concrete
`RES f AS fs::File = openOne()`, appends it to a `MUT xs AS List OF RES Handle` and returns
the list. `countH`, the callee, iterates the list with `FOR EACH` and `MATCH`.

`N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb i1` (main at dcf790e9a, macOS
aarch64, `mfb build -q --debug`): the build succeeds, `Segmentation fault: 11`, exit 139, empty
stdout.

Control `j1` is the same program with the binding declared `RES f AS Handle = openOne()`
(the only difference). `N=10 /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb j1`:
exit 0, stdout `60`, `alloc_calls 72`, `free_calls 52`, `live_bytes 3520`. (`j1` keeps
memory; that is a separate defect and is not what this section shows.)

This shows that the loop is not needed: the crash comes from appending a concrete
`fs::File` binding to a `List OF RES Handle` and reading the list as union elements, so the
title's "loop-scoped" is narrower than the defect. `/tmp/g933/data.txt` is a 6-byte file
("hello\n").

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
  h AS RES fs::File
  n AS Integer
END TYPE

FUNC openOne AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC

FUNC mkHandles() AS List OF RES Handle
  MUT xs AS List OF RES Handle = []
  RES f AS fs::File = openOne()
  xs = collections::append(xs, f)
  RETURN xs
END FUNC

FUNC mkHB() AS HB
  RETURN HB[openOne(), 4]
END FUNC

FUNC countH(hs AS List OF RES Handle) AS Integer
  MUT t AS Integer = 0
  FOR EACH h IN hs
    MATCH h
      CASE fs::File(f)
        t = t + len(fs::readAll(f))
      CASE tcp::Socket(s)
        t = t + 100
    END MATCH
  NEXT
  RETURN t
END FUNC

FUNC rdHB(k AS HB) AS Integer
  RETURN len(fs::readAll(k.h)) + k.n
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
  RETURN countH(mkHandles())
END FUNC
```

## Resolution (bs22-agent-b, 2026-10-11)

The finder's reading is right, and it is the float, not the append, that goes wrong. The IR
already wraps the member at the store (`{ "kind": "unionWrap", "union": "Handle", "member":
"fs.File", "value": local s }`, `mfb build -ir`), and the same store with the binding and
the list in ONE scope ran and balanced on the base compiler (rows `lb`..`le` below). The
crash needs the binding's close obligation to FLOAT to the container (`resourceOwners`:
`{ "name": "f", "owner": { "kind": "float", "scope": "xs" } }`, `mfb build -nir`), which is
what both a loop body and a `RETURN` of the list do.

**Cause 1 (the SIGSEGV).** The float is emitted at the binding (`NirOp::Bind`, the
`ResOwner::Float` arm in `builder_control.rs`), before the store wraps the value, so the
slot holds the member's RECORD. `emit_owned_list_push_as` found one owned-list for the
container, the `Handle` one, and `emit_owned_list_push_node` built a union node from the
slot as if it held a `{tag, record}` box: it loaded "the record" from offset 8 of the file
record, which is the handle word (the descriptor number, 3), and then read the owner word
through it. Debugger on the base build of `i1` (`printf 'run\nbt\nlist reg x0, x1, x8, x9,
x10\nquit\n' | N=1 /tmp/main-dbg-target/release/mfb dbg i1.out`): `a bad memory access at
0x0000000000000023` in `mkHandles+0x298`, `x9 = 3`.

**Fix 1.** `emit_owned_list_push_as` recognises a concrete leaf pushed onto a union list,
checks it is a member, and passes the member's variant tag to `emit_owned_list_push_node`
(new parameter `member_tag: Option<usize>`): the node is `{member tag, record, next}`, the
node a box of the member would have produced. This mirrors `module_leaf_kind` (bug-1027)
for a function-local container.

**Cause 2 (a leak the crash hid).** With the push right the programs ran and closed every
descriptor, and kept every record: `la` 182 allocations / 142 frees at N=10, 7680 bytes
live. The ownership proof (`record_ownership::owning_collections`) lets a list's drain free
what it closes only when no floated element "escapes", and `CollectionUses::visit_value`
treats a local named anywhere but directly in an argument list as escaping. The member is
named inside the `unionWrap` in the argument list, so every floated member escaped and the
drain was close-only.

**Fix 2.** In the argument walk a `UnionWrap` of a bare local is read as that local in that
argument position. The wrap builds a statement's box around the same record and holds
nothing itself.

### What was measured

Fixed compiler `target/release/mfb`, macOS aarch64, `--debug` counters, default level; the
`-O0` build of every row gives the same line (`/tmp/bs22-agent-b/m1046/run.sh <mfb> -O0`).
Programs: `/tmp/bs22-agent-b/m1046/<row>.mfb` (prelude `pre.txt`), run with
`/tmp/bs22-agent-b/m1046/run.sh <mfb>`; the filed probes with
`P=/tmp/bs22-agent-b/p /tmp/bs22/bin/probe.sh <mfb> g1` (`i1`, `j1`). "fd" is
`(ulimit -n 64; N=300 <program>)`.

| row | shape | base (N=10) | fixed N=10: stdout, alloc/free/live | fixed N=40 | fd |
| --- | --- | --- | --- | --- | --- |
| `g1` | as filed: loop, `append`, `len(hs) - 2` | exit 139 | `10`, 152/152/0 | `40`, 602/602/0 | 0 |
| `la` | loop, `append`, read every element | exit 139 | `180`, 182/182/0 | `720`, 722/722/0 | 0 |
| `lf` | loop, `prepend` + `insert` | exit 139 | `360`, 342/342/0 | — | 0 |
| `lg` | loop, `set` over element 0 | exit 139 | `60`, 182/182/0 | — | 0 |
| `lk` | `IF` body and `WHILE` body | exit 139 | `240`, 232/232/0 | — | 0 |
| `li` | two unions listing `fs::File` at different positions | exit 139 | `240`, 242/242/0 | — | 0 |
| `ra` | `i1`'s callee, result bound | exit 139 | `60`, 82/82/0 | `240`, 322/322/0 | 0 |
| `rb` | callee appends in a loop, result bound | exit 139 | `180`, 212/212/0 | `720`, 842/842/0 | 0 |
| `ea` | early `RETURN` out of the floating loop | exit 139 | `120`, 122/122/0 | `480`, 482/482/0 | 0 |
| `ta` | producer under an inline `TRAP` | exit 139 | `180`, 242/242/0 | `720`, 962/962/0 | 0 |
| `mg` | `Map`, `set` in a loop, read by `MATCH collections::get` | exit 139 | `120`, 212/212/0 | — | 0 |
| `rm` | `Map` returned, bound, read by `get` | exit 139 | `60`, balanced (asserted by the test at N=10 and N=40) | | 0 |
| `lb` `lc` `ld` `le` | one scope: `append`, `prepend`+`insert`, `set`, list literal | ran, balanced | unchanged (62/62, 102/102, 92/92, 92/92) | | 0 |
| `mbn` | one scope: map literal, `len` | ran, balanced | unchanged (42/42) | | 0 |
| `fa` | a `RES Handle` record field given the member | ran, balanced | unchanged (52/52) | | 0 |
| `ca`, `cb` | controls: `la` and `ra` with `RES s AS Handle` | 182/182, 82/82 | unchanged | | 0 |

`double_free_skips` is 0 in every row. A `—` under N=40 means the row was not run by hand at N=40; the test
asserts the same balance for it at N=10 and N=40. Closed once: `mfb dbg` on the `-O0 --debug` build of
`la` at N=1 with `bp _mfb_rt_fs_fs_close` stops three times, each under `once`, with three
different records in `x0`, then the program prints `18` and exits 0.

**`i1` as filed** (`countH(mkHandles())`, the list never bound): exit 139 before; now exit 0
and `60`, with `alloc_calls 72`, `free_calls 52`, `live_bytes 3520` at N=10 and the
64-descriptor run failing. Those are exactly the control `j1`'s numbers on the BASE compiler
(72/52/3520; `j1` binds `RES f AS Handle`), so the member spelling now does what the union
spelling does. What is left in both is the unbound aggregate, bug-999's position: a list a
call returns straight into an argument is closed by nobody. `ra` is `i1` with the result
bound, and it balances and closes. bug-999 is another agent's in this batch; not changed here.

Two more rows that equal their `AS Handle` control on base and fixed, and are other bugs:
a `Map OF String TO RES Handle` read with `FOR EACH e` / `MATCH e.value` keeps the same
bytes whichever way the element was bound (`ma` and control `mac`: 252/212/14080; with
`len(hs)` instead, `ma2` and `mac2`: 192/192/0), which is bug-1048's shape over a union map;
and a list or map LITERAL assigned in a loop to an outer `MUT` (`lh`/`lhc` 302/232/8320,
`mc`/`mcc` 172/132/4640), which is bug-1057's assignment.

### Checks

* RED: `MFB_TEST_EXE=/tmp/main-dbg-target/release/mfb cargo test --release --test rt_resource_union_member_float`:
  5 failed, 1 passed. GREEN with the fix: 6 passed.
* `cargo test --release --no-fail-fast --test rt_resource_aggregate_temporary --test rt_resource_temporary_close
  --test rt_resource_drop_reclaims --test rt_resource_union_box_owner --test rt_resource_union_reclaim
  --test rt_resource_owner_scope --test codegen_helper_scratch_release --test rt_canvas_geo_native
  --test test_targets_registered --test rt_resource_union_member_float --test no_type_strings
  --test architecture_guards`: all passed (9, 1, 16, 16, 10, 8, 12, 4, 4, 6, 7, 8).
* `cargo test --release --bin mfb -- every_body_mfb_member_declares codegen::resource:: codegen::cleanup::`: 44 passed.
* `scripts/test-accept.sh target/release/mfb <out> '*resource*' '*canvas*' '*collection*' '*bug842*' '*bug1012*'
  '*bug1000*' '*bug805*' '*bug1027*' '*bug806*'`: 300 tests, passed.
* `scripts/artifact-gate.sh target/release/mfb all`: 2348 goldens, 4 diffs, the four `.ir` goldens bug-1054
  lists; none from this change.
* `scripts/spec-census.sh --render` / `--citations`: 0 misses.

### Golden impact

None. The push changes only for a concrete leaf on a union list, which crashed before; the
argument walk changes a verdict only for a local wrapped at an argument, and no fixture's
`.ncodesum` moved.

### Shared functions changed

* `CodeBuilder::emit_owned_list_push_node` (`src/codegen/cleanup/owned/builder_owned_cleanup.rs`): new last
  parameter `member_tag: Option<usize>`; its other caller, `adopt_exclusive_temp`
  (`src/codegen/engine/value/builder_values.rs`), passes `None`.
* `CodeBuilder::emit_owned_list_push_as` (same file): computes that tag.
* `CollectionUses::visit_value` (`src/codegen/resource/cleanup/record_ownership.rs`): the argument walk peels a
  `UnionWrap` of a bare local.

### Spec

`mfb spec memory arenas` (the owned-list node of a member) and `mfb spec language
resource-management` §15.6 (a member added to a collection of its union).
