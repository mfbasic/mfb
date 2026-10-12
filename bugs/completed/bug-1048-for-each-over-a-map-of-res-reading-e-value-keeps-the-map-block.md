# bug-1048: `FOR EACH` over a `Map OF String TO RES fs::File` read through `e.value` keeps 192 bytes per call

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (the filed program by bug-1025 in batch 21; two siblings of the same shape by bug-1053's block rule, bs22-agent-c)
Regression Test: `tests/runtime/rt_resource_last_holder.rs`, `a_map_of_resources_iterated_through_its_values_frees_its_block` (`cargo test --test rt_resource_last_holder`)

A map literal of resources that is iterated with `FOR EACH e IN m` and read through
`e.value` keeps memory on every call. The descriptors are closed.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb c8`: exit 0, stdout
`10`, `alloc_calls 92`, `free_calls 82`, `live_bytes 1920`. `N=20`: stdout `20`, 182 / 162 /
3840. `(ulimit -n 64; N=300 …)`: exit 0, stdout `300`, 2702 / 2402 / 57600.

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
FUNC whichF(label AS String, f AS fs::File) AS Integer
  LET text AS String = fs::readAll(f)
  RETURN 1
END FUNC
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  RES c AS fs::File = openOne()
  LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { "x" := a, "y" := c }
  MUT t AS Integer = 0
  FOR EACH e IN m
    t = t + whichF("m", e.value)
  NEXT
  RETURN t - 1
END FUNC
```

## What is known

1920 / 10 = 192 bytes per call; 3840 / 20 and 57600 / 300 give the same. The kept blocks were
not itemized; the map block is the reported culprit (guess).

The batch-21 document for resource unions (bug-1028, "a `List`/`Map` of resource unions that
is iterated or read through `collections::get` keeps its block and records") records that the
same shapes over `List OF RES fs::File` are reported balanced, and did not re-measure them.
This program uses a concrete `fs::File` map value and keeps memory, so the leak is not
specific to unions.

Found during batch 21.

## Resolution (bs22-agent-c, 2026-10-11)

### The filed program no longer reproduces

This document was measured on main at dcf790e9a. The filed program (`b1048`) is balanced on
main's compiler today (`/tmp/main-dbg-target/release/mfb`) and on the batch 22 staging
compiler (`/tmp/bs22-target/release/mfb`), with
`N=<n> P=<dir> /tmp/bs22/bin/probe.sh <mfb> b1048 [-O0]`:

| compiler | N=10 | N=20 | `-O0` N=10 | N=300 under `ulimit -n 64` |
| --- | --- | --- | --- | --- |
| main | `10`, 92/92/0 | `20`, 182/182/0 | 92/92/0 | exit 0, 2702/2702/0 |
| staging | `10`, 92/92/0 | `20`, 182/182/0 | 92/92/0 | exit 0, 2702/2702/0 |

### What fixed it: bug-1025

`3b9ad545c` ("bug-1025: a record binding frees the resource record, and the read buffer, it
adopted"), which is after dcf790e9a (`git merge-base --is-ancestor 3b9ad545c dcf790e9a` is
false; `--is-ancestor dcf790e9a HEAD` is true).

The kept 192 bytes were the map's own block (a guess in this document, now measured: see
`x6` below, which keeps exactly 192 bytes a call and one allocation). `m` is a
`Map OF String TO RES fs::File` bound to a literal, so its block is freed only when
`record_ownership::owning_collections` proves the binding its one holder, and a `FOR EACH`
over it disqualifies it when the loop variable "leaves" the loop. In
`whichF("m", e.value)` the argument is a field read, not a bare local. Before bug-1025 the
argument walk (`CollectionUses::visit_value`) had one narrow arm, for a bare `Local`; any
other argument walked as an ordinary value, which named `e` outside an argument position, so
the loop variable left, the map was held, and its block was kept. bug-1025 added the arm
"a field read written straight into an argument position is a use of the record it reads
through" (`git log -S'if let (NirValue::MemberAccess { .. }, Some(root)) =' --
src/codegen/resource/cleanup/record_ownership.rs` names that one commit): written for a
record with a `RES` field, it reads a `MapEntry`'s `.value` the same way. bug-1028
(`62a3037bf`, also after dcf790e9a) does not contain that arm (`git show
62a3037bf:src/codegen/resource/cleanup/record_ownership.rs | grep MemberAccess` shows only
the `STATE` arm and its two unit tests) and is about `MATCH` over a resource union; bug-1037 is about the trapped
temporary's box. Neither touches this path. No old compiler was built to confirm it; the
attribution is from the two commits' text and from `x6`, which bypasses the arm and
reproduces the filed number on both of today's compilers.

### Siblings of the same shape

Programs `/tmp/bs22-agent-c/src/x0.mfb` to `x8.mfb` (the filed prelude and one `once()`
each), N=10. "fixed" is this branch (`target/release/mfb`); every fixed row is the same at
`-O0`, balanced at N=20, passes the 64-descriptor run, and has `double_free_skips 0`.

| row | shape | main | staging | fixed N=10 | fixed N=20 |
| --- | --- | --- | --- | --- | --- |
| `x0` | as filed | 92/92/0 | 92/92/0 | 92/92/0 | 182/182/0 |
| `x1` | the map built by a call (`collections::set` in the callee, `RETURN m`) | 142/142/0 | 142/142/0 | 142/142/0 | 282/282/0 |
| `x2` | the map built by `collections::set` in a loop (the files float into it) | 242/242/0 | 242/242/0 | 242/242/0 | 482/482/0 |
| `x3` | the map iterated in a callee | 92/92/0 | 92/92/0 | 92/92/0 | 182/182/0 |
| `x4` | `e.key` only | 72/72/0 | 72/72/0 | 72/72/0 | 142/142/0 |
| `x5` | an inline `TRAP` in the loop body (the callee fails for the second entry) | 102/102/0 | 102/102/0 | 102/102/0 | 202/202/0 |
| `x7` | two maps in a loop, an early `RETURN` out of the inner `FOR EACH` | 162/162/0 | 162/162/0 | 162/162/0 | 322/322/0 |
| `x6` | the value bound first: `RES f AS fs::File = e.value`, then `whichF("m", f)` | **92/82/1920** | **92/82/1920** | 92/92/0 | 182/182/0 |
| `x8` | as filed, then `RES g AS fs::File = collections::get(m, "y")` after the loop | **102/92/1920** | **102/92/1920** | 102/102/0 | 202/202/0 |

`x6` and `x8` are the same shape this document describes (a `Map OF String TO RES fs::File`
iterated with `FOR EACH` and read through `e.value`) and still kept the map's block on main
and on staging: binding the value, or reading an element back with `get`, is not an argument
position, so the map failed the proof again. They are fixed by bug-1053's block rule
(`record_ownership::block_owning_collections`: a second reader of an element is no holder of
the block), not by anything written for this document.

A union-valued map (`Map OF String TO RES Handle`, `MATCH e.value`) is bug-1066, not this
bug. Its filed program happens to be balanced by bug-1053's last-holder rule as well (252/252/0
at N=10; see that bug's document).

### Checks

* `MFB_TEST_EXE=/tmp/bs22-target/release/mfb cargo test --release --test rt_resource_last_holder a_map_of_resources`
  on the staging compiler: the seven cases of the filed shape (`x0`..`x5`, `x7`, each at the
  default level and at `-O0`) pass, and the eighth fails (`value_bound N=10: 92 allocations,
  82 frees, 1920 bytes live at exit`). On this branch: `cargo test --release --test
  rt_resource_last_holder`, 14 passed.
* No source changed for this bug; every other check is bug-1053's and bug-1051's.

### Golden impact

None.

### Shared functions changed

None.
