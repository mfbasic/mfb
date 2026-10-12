# bug-1047: `collections::get` under an inline `TRAP` written into an argument keeps 96 bytes per call

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-c, by bug-1053's block rule)
Regression Test: `tests/runtime/rt_resource_last_holder.rs`, `a_trapped_element_read_does_not_keep_the_list_block` (`cargo test --test rt_resource_last_holder`)

`whichF("t", collections::get(hs, i)) TRAP(e) … END TRAP`, where `hs` is a
`List OF RES fs::File`, keeps 96 bytes per call of the enclosing function. The descriptors are
closed.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb c5`: exit 0, stdout
`10`, `alloc_calls 82`, `free_calls 72`, `live_bytes 960`. `N=20`: stdout `20`, 162 / 142 /
1920. `(ulimit -n 64; N=300 …)`: exit 0, stdout `300`, 2402 / 2102 / 28800.

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
  LET hs AS List OF RES fs::File = [a]
  MUT r AS Integer = 0
  r = whichF("t", collections::get(hs, 5)) TRAP(e)
    RECOVER 7
  END TRAP
  MUT q AS Integer = 0
  q = whichF("t", collections::get(hs, 0)) TRAP(e)
    RECOVER 7
  END TRAP
  RETURN r + q - 7
END FUNC
```

## What is known

960 / 10 = 96 bytes per call at `N=10`, 1920 / 20 = 96 at `N=20`, 28800 / 300 = 96 at
`N=300`. Each call evaluates two trapped `get` arguments: `get(hs, 5)` (out of range, takes
the `RECOVER`) and `get(hs, 0)` (succeeds). Which of the two keeps the bytes was not
isolated, and the kept block was not identified.

Found during batch 21.

## Resolution (bs22-agent-c, 2026-10-11)

### Which read keeps what

Either one, and what is kept is neither read's: it is the block of the list `hs`.

Isolated with three programs (`/tmp/bs22-agent-c/src/c5a.mfb`, `c5b.mfb`, `c5c.mfb`: the filed
prelude and one statement each), on the staging compiler `/tmp/bs22-target/release/mfb`, N=10:

| row | `once()` | alloc / free / live |
| --- | --- | --- |
| `c5` | as filed: both trapped reads | 82 / 72 / 960 |
| `c5a` | only `get(hs, 5)` (out of range, takes the `RECOVER`) | 62 / 52 / 960 |
| `c5b` | only `get(hs, 0)` (succeeds) | 52 / 42 / 960 |
| `c5c` | `get(hs, 0)` with no `TRAP` | 42 / 42 / 0 |
| `c5d` | the list alone (`RETURN len(hs)`) | 32 / 32 / 0 |
| `c5e` | `c5b` with three files in the list | 92 / 82 / 1920 |

One allocation per call is not freed in every leaking row, and its size follows the list:
96 bytes for one element (a 40-byte header, one 40-byte entry, 8 bytes of data, rounded to
16), 192 for three. Two reads in one function still keep one block, not two. So neither the
`Result`, nor the trapped error, nor a record is kept; the list's own block is.

### Cause

An inline `TRAP` lifts every fallible call of the trapped expression into a temporary of its
own (`mfb build -nir` of `c5b`: `bind $trap_res5 : Result OF fs.File = callResult
collections.get(hs, 0)`, then `whichF("t", resultValue($trap_res5))` on the `Ok` branch). The
untrapped `whichF("t", collections::get(hs, 0))` is a borrowed-element read written straight
into an argument, which `record_ownership` reads as a use that ends with the call. Bound to
`$trap_res5` it is an element read that is kept, so `hs` landed in
`CollectionUses::element_readers`, failed `owning_collections`, and the free of its block
(registered only for a proved collection) was never emitted. Debugger, `bp
_mfb_rt_drop_owned_collection` on the `-O0 --debug` builds of `c5b` at N=1: the fixed build
stops once, under `once + 5020`, and reports 7 allocations and 7 frees; the base build never
reaches the helper and reports 7 and 6.

### Fix

No change of its own. bug-1053 judges a `List`/`Map OF RES T` block separately from the
records it lists (`record_ownership::block_owning_collections`): an element read out of the
collection is a record pointer or a copy of a union box, never a holder of the block, so
`hs`, whose every store is a literal and which is named nowhere but in argument positions,
frees its block at its scope's end whoever read its elements.

### Measured

Fixed compiler `target/release/mfb`, N=10 (`N=<n> P=/tmp/bs22-agent-c/p SRC=/tmp/bs22-agent-c/src
/tmp/bs22/bin/probe.sh <mfb> <name> [-O0]`). Every fixed row is the same at `-O0`, balanced at
N=20, passes `(ulimit -n 64; N=300 <program>)`, and has `double_free_skips 0`.

| row | shape | base | fixed N=10 | fixed N=20 |
| --- | --- | --- | --- | --- |
| `c5` | as filed | 82/72/960 | `10`, 82/82/0 | 162/162/0 (N=300: 2402/2402/0) |
| `c5a` | the failing read alone | 62/52/960 | 62/62/0 | 122/122/0 |
| `c5b` | the succeeding read alone | 52/42/960 | 52/52/0 | 102/102/0 |
| `c5e` | three elements | 92/82/1920 | 92/92/0 | 182/182/0 |
| `c5f` | `List OF RES Handle`, both reads | 102/92/960 | 102/102/0 | 202/202/0 |
| `c5g` | `Map OF String TO RES fs::File`, a missing key and a present one | 82/72/1120 | 82/82/0 | 162/162/0 |
| `c5h` | a list floated in a loop, read under `TRAP` in a loop with an early `RETURN` | 152/122/4640 | 152/152/0 | 302/302/0 |
| `c5i` | `RES g = collections::get(hs, 0) TRAP …`, and one that `RECOVER`s another binding | 112/102/960 | 112/112/0 | 222/222/0 |

`c5h` kept the two records and their buffers as well as the block (464 bytes a call): its
list is a float target, so its drain was close-only; bug-1053's last-holder rule frees them.

### Checks

* RED: `MFB_TEST_EXE=/tmp/bs22-target/release/mfb cargo test --release --test rt_resource_last_holder a_trapped_element_read`:
  1 failed (`two_reads N=10: 82 allocations, 72 frees, 960 bytes live at exit`). GREEN:
  `cargo test --release --test rt_resource_last_holder`: 14 passed.
* Every other check is bug-1053's and bug-1051's (no source changed for this bug).

### Golden impact

None of its own.

### Shared functions changed

None beyond bug-1053's.
