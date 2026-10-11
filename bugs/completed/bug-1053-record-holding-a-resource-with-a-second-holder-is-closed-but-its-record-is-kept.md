# bug-1053: a record holding a resource that has a second holder is closed but its 112-byte record is kept

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-c)
Regression Test: `tests/runtime/rt_resource_last_holder.rs` (`cargo test --test rt_resource_last_holder`), and the unit tests `a_callee_that_keeps_an_element_does_not_keep_the_block`, `a_nested_collection_is_not_judged_by_the_block_rule`, `a_lambda_that_assigns_a_captured_container_is_found`, `an_assignment_the_raise_does_not_cover_is_recorded` (`cargo test --bin mfb -- codegen::resource::`)

When a record's resource has a second holder, the record's drain closes the resource and does
not free the record. Five shapes were measured: the handle bound on its own (`r3`), the record
returned after a copy (`r4`), the record captured by a `LAMBDA` (`r6`), a `MATCH` case binding
stored into another list (`r7`), and a copy appended to a list (`r9`). Each keeps its record on
every call; the descriptors are closed.

These are the "close-only drain" cases of `src/docs/spec/memory/04_arenas.md`: "A handle bound
on its own (`RES f = k.h`), a field stored as an element, a copy made by assignment, and a
record returned or handed to a callee that keeps it are holders, and the record's drain then
closes without freeing", and "A collection that fails any of those keeps a close-only drain,
which leaks those blocks rather than risking a double free."

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036). Under that
decision the close-only drain is a bug: the block must be freed once the last holder is gone.

**Correct behavior:** `live_bytes 0` at exit in all five, with no double free
(`double_free_skips 0`).

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

The five programs share one prelude (lines 1 to 50 of each source, identical in all five);
each adds the declarations and `once()` shown after it. The harness is
`N=<n> /tmp/bs21/bin/probe.sh /tmp/main-dbg-target/release/mfb <name>`.

Shared prelude:

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
END TYPE

TYPE HU
  h AS RES Handle
END TYPE

FUNC openOne AS RES fs::File
  RETURN fs::openFile("/tmp/g933/data.txt")
END FUNC

FUNC wrap(h AS fs::File) AS HB
  RETURN HB[h]
END FUNC

FUNC rd(f AS fs::File) AS Integer
  RETURN len(fs::readAll(f))
END FUNC

FUNC which(h AS Handle) AS Integer
  MATCH h
    CASE fs::File(f)
      RETURN len(fs::readAll(f))
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

```

`r3` (the handle bound on its own, `RES f AS fs::File = k.h`):

```
FUNC mk() AS HB
  RETURN HB[openOne()]
END FUNC
FUNC once() AS Integer
  LET k AS HB = mk()
  LET a AS Integer = rd(k.h)
  RES f AS fs::File = k.h
  LET k2 AS HB = k
  RETURN a + rd(f) + rd(k2.h) + 1
END FUNC
```

`r4` (the record returned after a copy):

```
FUNC mk() AS HB
  RETURN HB[openOne()]
END FUNC
FUNC pass() AS HB
  LET k AS HB = mk()
  LET a AS Integer = rd(k.h)
  LET k2 AS HB = k
  IF a = 6 THEN
    RETURN k2
  END IF
  RETURN k
END FUNC
FUNC once() AS Integer
  LET q AS HB = pass()
  RETURN rd(q.h) + 1
END FUNC
```

`r6` (the record captured by a `LAMBDA`):

```
FUNC mk() AS HB
  RETURN HB[openOne()]
END FUNC
FUNC once() AS Integer
  LET k AS HB = mk()
  LET a AS Integer = rd(k.h)
  LET g = LAMBDA() -> rd(k.h)
  RETURN a + g() + 1
END FUNC
```

`r7` (a `MATCH` case binding stored into another list):

```
FUNC pick(hs AS List OF RES Handle) AS List OF RES fs::File
  MUT out AS List OF RES fs::File = []
  FOR EACH h IN hs
    MATCH h
      CASE fs::File(f)
        out = collections::append(out, f)
      CASE tcp::Socket(s)
        io::print("s")
    END MATCH
  NEXT
  RETURN out
END FUNC
FUNC once() AS Integer
  RES a AS fs::File = openOne()
  RES c AS fs::File = openOne()
  LET hs AS List OF RES Handle = [a, c]
  LET fl AS List OF RES fs::File = pick(hs)
  MUT t AS Integer = 0
  FOR EACH f IN fl
    t = t + rd(f) + 1
  NEXT
  FOR EACH h IN hs
    t = t + which(h)
  NEXT
  RETURN t
END FUNC
```

`r9` (a copy appended to a list):

```
FUNC mkU() AS HU
  RES u AS fs::File = openOne()
  RETURN HU[u]
END FUNC
FUNC once() AS Integer
  LET k AS HU = mkU()
  LET k2 AS HU = k
  MUT ks AS List OF HU = []
  ks = collections::append(ks, k2)
  RETURN which(k.h) + which(collections::get(ks, 0).h)
END FUNC
```

Measured results (all exit 0, `double_free_skips 0`):

| Case | stdout at N=10 | N=10 alloc / free / live | N=20 alloc / free / live | N=300 under `ulimit -n 64` alloc / free / live |
| ---- | -------------- | ------------------------ | ------------------------ | ---------------------------------------------- |
| r3   | `70`           | 72 / 62 / 1120           | 142 / 122 / 2240         | 2102 / 1802 / 33600                            |
| r4   | `10`           | 72 / 62 / 1120           | 142 / 122 / 2240         | 2102 / 1802 / 33600                            |
| r6   | `70`           | 92 / 82 / 1120           | 182 / 162 / 2240         | 2702 / 2402 / 33600                            |
| r7   | `140`          | 132 / 122 / 1600         | 262 / 242 / 3200         | 3902 / 3602 / 48000                            |
| r9   | `60`           | 102 / 92 / 1120          | 202 / 182 / 2240         | 3002 / 2702 / 33600                            |

N=300 stdout: `2100`, `300`, `2100`, `4200`, `1800`. The N=300 runs stay under the
64-descriptor limit, so the descriptors are closed.

## What is known

r3, r4, r6 and r9 keep 1120 / 10 = 112 bytes per call, the size of an `fs::File` record. r7
keeps 1600 / 10 = 160 bytes per call; the extra 48 bytes were not itemized. Each case keeps
exactly one more allocation than it frees per call (r3 at N=10: 72 - 62 = 10).

Found during batch 21.

## Resolution (bs22-agent-c, 2026-10-11)

### Two causes, not one

Four of the five shapes keep a resource RECORD; the fifth keeps a list BLOCK.

* `r3`, `r4`, `r6`, `r9`: one allocation of 112 bytes per call, the `fs::File` record.
  The record is listed by one owned-list (the record `k`'s, or the caller's `q`'s), whose
  container the proof `record_ownership::owning_collections` refuses: `k` is named by
  `RES f = k.h` (r3), `q` is bound from a callee whose `RETURN` is a copy (r4), `k` is
  captured (r6), `k`'s copy is appended to a list (r9). The drain then closed and did not
  free.
* `r7`: one allocation of 160 bytes per call, and no record. The records are the two `RES`
  bindings' and were freed. What was kept is the block of `hs` (`List OF RES Handle`, two
  elements: `perf` shows one unfreed allocation per call, `arena.0.alloc_bytes` minus
  `free_bytes` = 1600 at N=10). `pick(hs)` appends a `MATCH` case binding of each element to
  the list it returns, so the callee "keeps" its parameter, and `hs` lost its block free
  together with its (irrelevant here) element free. The "extra 48 bytes" of the report is
  160 - 112: a different block, not a record plus something.

### Fix

**Records: the last holder frees, at run time.** The owner word of a record already names
the one scope that closes it, and every store that lets a reference outlive that scope
raises the word first (the float of a `RES` binding, the store-time raise of bug-1045, the
adoption of a call's result, the module-level raise, a `RETURN`'s release). So when the
scope holding the token exits, every remaining reference is in that scope or inside it. The
one thing a drain cannot know is whether another cleanup of the same exit still reads the
word, so an owned-list that is not proved the one holder splits its drain
(`CodeBuilder::emit_owned_list_drain`, `OwnedListPass`):

1. the drain closes each record its scope owns, writes `RESOURCE_OWNER_TAKEN` (all ones but
   the lowest bit: no token, above every token, not `0`, not the module's) into the owner
   word, and moves the node to a reclaim list headed by a second frame slot;
2. after the last cleanup of the exit (`emit_owned_list_reclaims`) the taken records, their
   buffers and `STATE` are freed, then the nodes.

A second list or binding naming the same record reads an owner word that is not its token
and skips, having read nothing else. Every exit is covered: the end of a block, `EXIT` /
`CONTINUE`, `RETURN`, an inline-`TRAP` route, and the error-return landing pads (the last
pad walks every such list of the function; a reclaim slot is zeroed with the frame). A
function-level list routed to the function's own `TRAP` handler is closed in place and
drained at the handler's exit, because the handler still reads the container. A proved
container keeps its direct free, so its code is unchanged.

**Two stores carry a reference outward without raising it**, and a function that can be on
either side of one keeps the close-only drain (`record_ownership::runtime_reclaim`):

* a `LAMBDA` that assigns, through a by-reference capture, a binding that reaches a
  resource. Asked of the whole program (`module_stores_through_a_captured_reference`);
* an assignment whose value names an inner scope's binding in a position the store-time
  raise does not cover (`keep = collections::filter(xs, ...)`). Asked of the function
  (`StoreFloats::unraised_outward_store`).

Both are open defects of their own (see "Not fixed here"), and both read `ErrResourceClosed`
or worse today; with the guard their behaviour is what it was.

**Blocks: a static rule of their own** (`record_ownership::block_owning_collections`). A
`List OF RES T` / `Map OF K TO RES T` block is held by whatever holds its pointer and nothing
else, so the binding frees it when every store is fresh, it is named outside an argument
list only as a `FOR EACH` iterable, and no call keeps the BLOCK (`CollectionUses::block_kept`:
a callee that keeps what it reads out of the argument is a second reader of the elements and
no holder of the block). A list of lists stays under the whole proof.

### Measured

Fixed compiler `target/release/mfb`, macOS aarch64, `--debug` counters; base is the staging
compiler `/tmp/bs22-target/release/mfb`. Harness: `N=<n> P=/tmp/bs22-agent-c/p [SRC=/tmp/bs22-agent-c/src]
/tmp/bs22/bin/probe.sh <mfb> <name> [-O0]`; "fd" is `(ulimit -n 64; N=300 <program>)`.
`double_free_skips` is 0 in every row.

| row | shape | base N=10 | fixed N=10 | fixed N=20 | fixed `-O0` N=10 / N=20 | fd |
| --- | --- | --- | --- | --- | --- | --- |
| `r3` | handle bound on its own | 72/62/1120 | `70`, 72/72/0 | `140`, 142/142/0 | 82/82/0, 162/162/0 | 0 (2402/2402/0) |
| `r4` | returned after a copy | 72/62/1120 | `10`, 72/72/0 | `20`, 142/142/0 | 82/82/0, 162/162/0 | 0 |
| `r6` | `LAMBDA` capture | 92/82/1120 | `70`, 92/92/0 | `140`, 182/182/0 | 92/92/0, 182/182/0 | 0 |
| `r7` | `MATCH` binding stored by a callee | 132/122/1600 | `140`, 132/132/0 | `280`, 262/262/0 | 132/132/0, 262/262/0 | 0 |
| `r9` | copy appended to a list | 102/92/1120 | `60`, 102/102/0 | `120`, 202/202/0 | 112/112/0, 222/222/0 | 0 |

Sibling matrix (`/tmp/bs22-agent-c/src/<row>.mfb`, the shared prelude plus the body in the
test of the same name). Each row is balanced at N=10 and N=20, at the default level and at
`-O0`, and passes the 64-descriptor run:

| row | shape | base N=10 | fixed N=10 |
| --- | --- | --- | --- |
| `m1` | `RES f AS Handle = k.h`, a resource-union field | 82/72/1120 | 82/82/0 |
| `m2` | `Map OF String TO RES fs::File`, a callee stores `e.value` elsewhere (the map's block) | 112/102/1920 | 112/112/0 |
| `m9` | floated `List OF RES Handle`, `RES g AS Handle = collections::get(xs, 0)` | 142/112/4640 | 142/142/0 |
| `m15` | `[a, c]` as `List OF RES Handle`, one element bound again | 112/102/1600 | 112/112/0 |
| `m8` | floated `List OF RES fs::File`, one element bound from `get` | 112/82/4640 | 112/112/0 |
| `m10` | that list `RETURN`ed and read the same way by the caller (bound call) | 142/112/4640 | 142/142/0 |
| `m11` | a callee copies the record into the list it returns | 82/72/1120 | 82/82/0 |
| `m3` | loop body: record, handle and a reading `LAMBDA` per iteration | 272/242/3360 | 272/272/0 |
| `m4` | loop body with an early `RETURN` and a `CONTINUE` | 102/82/2240 | 102/102/0 |
| `m12` | `WHILE` body with `CONTINUE` and `EXIT` | 192/152/4480 | 192/192/0 |
| `m5` | inline `TRAP` in the scope (both a recovering and a passing one) | 202/172/3360 | 202/202/0 |
| `a6` | an error leaves through the landing pads, the caller recovers | 72/62/1120 | 72/72/0 |
| `a3v` | function-level `TRAP`, reached from the function scope | 92/62/167360 | 92/92/0 |
| `a3w` | function-level `TRAP`, reached from a loop body | 122/102/2240 | 122/122/0 |
| `a4` / `a4b` | the record AND a list its handle floated into (either declaration order) | 82/72/1120 | 82/82/0 |
| `a5` | the same handle handed back by two calls and wrapped twice | 102/92/1120 | 102/102/0 |
| `m6` | a module-level `List OF RES fs::File` takes one handle of ten | 106/85/2480 | 106/104/352 (N=20: 206/204/352; the module list, its node and the one record it holds, constant in N) |

Canvas: `cargo test --release --test rt_canvas_geo_native`, 4 passed. No canvas helper's code
moved (`app-mouse-surface` is byte-identical): an owned-list whose leaf has nothing to reclaim
(a `canvas` image or font: no record free, no buffers, no `STATE`) gets no reclaim list.

Debugger (`mfb dbg`, the `-O0 --debug` build of `a4` at N=1, `bp _mfb_rt_fs_fs_close`): the
breakpoint stops once, under `once + 4748`, with the record in `x0`; the program then prints
`7` and reports 10 allocations, 10 frees. Two owned-lists name that record (`k`'s, and `xs`'s
through the floated `f`): the first drain closes and takes it, the second reads the owner
word and skips, and the reclaim frees it once.

### Other filed shapes the rule balances

Not targeted, measured because the rule is general (base against fixed, N=10; each is also
balanced at N=40 and at `-O0`, and passes the 64-descriptor run). Programs are the ones in
each document.

| bug | base | fixed |
| --- | --- | --- |
| bug-1064 (record bound from a call under an inline `TRAP`) | 102/82/2240 | 102/102/0 |
| bug-1065 (stateful resources floated from a loop) | 142/72/6720 | 142/142/0 |
| bug-1066 (`FOR EACH` over a `Map OF String TO RES Handle`, `MATCH e.value`) | 252/212/14080 | 252/252/0 |
| bug-1067 (list literal assigned in a loop to an outer `List OF RES Handle`) | 302/232/8320 | 302/302/0 |
| bug-1069 (`RES` binding in a constructed record, field appended to an outer list) | 122/102/2240 | 122/122/0 |
| bug-1070 (raised elements, list copied by a callee) | 162/142/2240 | 162/162/0 |
| bug-1071 (inner list appended to a list of lists) | 222/152/11680 | 222/192/7200 (its inner blocks remain) |

Unchanged (another agent's, or queued): bug-1043 33/23/1120, bug-1044 103/73/2720, bug-1049
62/42/1120, bug-1056 168/157/2448, bug-1057 23/12/720.

### Not fixed here

Two defects the guard above works around, both present on the base compiler and neither
changed by this fix. Each is a missing raise, bug-1045's class, not a missing free.

* **A `LAMBDA` that appends its argument to a captured outer list** (`/tmp/bs22-agent-c/src/cap1.mfb`):
  the list is declared in the function scope, the resource and `collections::forEach(xs,
  LAMBDA(h) -> keep = collections::append(keep, h))` are in a nested block, and `keep` is
  read after the block. Base and fixed: exit 255, `Error: 7-703-0009` ("moved to another
  thread"), a flag read out of a record the nested block's `RES` binding freed.
* **`keep = collections::filter(xs, ...)` from a nested block** (`a2.mfb`): base and fixed,
  exit 255, `Error: 7-703-0004`.

And one that has nothing to do with this fix: a function-level `TRAP` handler that reads a
list PROVED the one holder (`a3p.mfb`) reads freed records (`7-703-0009`), because the trap
route drains such a list in full. Base and fixed alike.

### Checks

* RED: `MFB_TEST_EXE=/tmp/bs22-target/release/mfb cargo test --release --test rt_resource_last_holder`
  on the base compiler: 5 failed of 5 (the five filed shapes). GREEN: 10 passed.
* `cargo test --release --no-fail-fast --test rt_resource_last_holder --test rt_resource_aggregate_temporary
  --test rt_resource_temporary_close --test rt_resource_drop_reclaims --test rt_resource_union_box_owner
  --test rt_resource_union_reclaim --test rt_resource_owner_scope --test codegen_helper_scratch_release
  --test rt_canvas_geo_native --test test_targets_registered --test rt_resource_store_float
  --test rt_resource_trap_state_free --test rt_resource_union_member_float --test no_type_strings
  --test architecture_guards`: all passed.
* `cargo test --release --no-fail-fast --test codegen_owned_drop_free_and_null --test codegen_shared_cleanup_helpers
  --test rt_closure_single_owner --test rt_owned_argument --test rt_inline_trap_borrowed_resource
  --test rt_inline_trap_union_bind --test rt_inline_trap_success_alloc --test rt_resource_review_804
  --test rt_resource_review_805 --test rt_resource_review_806 --test rt_res_rebind_alias
  --test rt_res_state_inplace_mutation --test rt_res_list_from_call_leak --test rt_res_list_kept_by_call
  --test rt_scope_drop_leaks --test rt_debug_soak`: all passed.
* `cargo test --release --bin mfb -- codegen::resource:: codegen::cleanup:: every_body_mfb_member_declares spec`: 94 passed.
* `scripts/test-accept.sh target/release/mfb <out> '*resource*' '*canvas*' '*collection*' '*bug842*' '*bug1012*'
  '*bug1000*' '*bug805*' '*bug1027*' '*bug806*' '*tcp*' '*udp*' '*thread*' '*trap*'`: 540 tests, 4 mismatches,
  the four `.ir` goldens bug-1054 lists (`rt-behavior/threads/{bug693-worker-state, thread-view-state-independent,
  thread-worker-state-send-rt, threadworker-res-binding-rt}`); none from this change.
* `scripts/spec-census.sh --render memory`, `--citations memory`, `--links memory`: 0 leaks, 0 misses, 0 unresolved.

### Golden impact

`scripts/artifact-gate.sh target/release/mfb all`: 2348 goldens checked, 14 diffs. Four are the
`.ir` goldens bug-1054 lists. Ten are this fix, all the block rule:

* `tests/byte-identity/tcp/golden/tcp_codegen_cover_rt.{linux-aarch64,linux-riscv64,linux-x86_64,macos-aarch64,windows-x86_64}.ncodesum`
* `tests/byte-identity/udp/golden/udp_codegen_cover_rt.{linux-aarch64,linux-riscv64,linux-x86_64,macos-aarch64,windows-x86_64}.ncodesum`

Only `main` changes in each (macos-aarch64: 11818 to 11848 instructions for tcp, 4464 to 4490
for udp; one more error-return pad). `LET pool AS List OF RES tcp::Socket = [peer, peerT]` is
read by `RES ready AS tcp::Socket = tcp::poll(pool, 100)`, a borrowed-element read, which
disqualified `pool` from the whole proof; `pool` is its block's one holder, so `main` now
frees that block at its exit. No golden was regenerated.

A program gains code only where it has an owned-list the proof refuses AND whose leaf has
something to reclaim: one frame slot, the take in the drain, and one reclaim walk per exit
sequence. No fixture with a golden has one.

### Shared functions changed

* `OwnedListCleanup` (`src/codegen/engine/builder/mod.rs`): new fields `reclaim_slot: Option<usize>`,
  `trap_shared: bool` (every constructor sets them: `setup_owned_list`, `setup_dynamic_owned_list`,
  `adopt_exclusive_temp`). `CodeBuilder`: new fields `owned_list_block_collections`,
  `owned_list_runtime_reclaim`, `reclaiming_owned_lists`.
* `CodeBuilder::emit_owned_list_drain` (`src/codegen/cleanup/owned/builder_owned_cleanup.rs`): signature
  unchanged; the body is `emit_owned_list_walk` with an `OwnedListPass`. New
  `emit_owned_list_reclaims`, `emit_owned_list_reclaim`, `emit_owned_list_take`. `setup_owned_list`: the
  block free keys on `owned_list_block_collections`, the element free on `owned_list_owning_collections`
  as before; a reclaim slot is allocated per leaf when the list is unproved.
* `CodeBuilder::emit_cleanups` (`src/codegen/engine/control/builder_exits.rs`): now the drops then the
  reclaims; `emit_cleanup_drops` (new, private) is the drops alone, used by the landing pads.
  `emit_cleanup_branch_to_depth` and the scope end in `lower_ops_inner` (`builder_control.rs`) emit the
  reclaims after their drops. `trap_route_cleanups` marks a function-level reclaiming list
  `trap_shared`. **`append_error_return_pads` now returns `Result<(), String>`** (its one caller,
  `run_register_allocation` in `builder_registers.rs`, propagates it).
* `record_ownership.rs`: `RecordOwnership` gains `block_owning_collections` and `runtime_reclaim`;
  `owning_collections` (private) returns a third value; `CollectionUses` gains `block_kept`,
  `CalleeParamUses` gains `known_block`; `StoreFloats` gains `unraised_outward_store` (private). New
  `block_owning_collections`, `runtime_reclaim`, `module_stores_through_a_captured_reference`,
  `CollectionUses::callee_block_keeps`. `record_ownership()`'s signature is unchanged.
* `src/codegen/resource/owner.rs`: new constant `RESOURCE_OWNER_TAKEN`.
* `builder_control.rs`, the unfloated `RES` collection branch of `NirOp::Bind`: keys on
  `owned_list_block_collections`.

### Spec

`mfb spec memory arenas` (the last-holder rule, the reclaim, the trap route, the two unraised
stores, the block rule; the three "close-only" sentences are gone) and `mfb spec memory
heap-values`, "The owner word" (the taken word).
