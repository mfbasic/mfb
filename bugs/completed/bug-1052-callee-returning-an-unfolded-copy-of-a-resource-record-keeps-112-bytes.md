# bug-1052: a callee that returns an unfolded copy of a record holding a resource keeps 112 bytes per call

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-c, by bug-1053's last-holder rule)
Regression Test: `tests/runtime/rt_resource_last_holder.rs`, `a_copy_of_a_record_returned_by_a_callee_is_freed_at_every_level` (`cargo test --test rt_resource_last_holder`)

`mid` binds `LET k AS HB2 = mk2()`, copies it with `LET k3 AS HB2 = k`, and returns `k3`. The
copy is a second holder of the record that holds the file, and the record is closed but not
freed. On main at the default optimization level this keeps 112 bytes per call.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit.

## Failing Reproduction

Measured 2026-10-11 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a, macOS aarch64): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.alloc_calls`, `arena.0.free_calls` and `arena.0.live_bytes` lines the program prints to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n").

`N=10 /tmp/bs21-agent-a/bin/probe.sh /tmp/main-dbg-target/release/mfb k2`: exit 0, stdout
`10`, `alloc_calls 72`, `free_calls 62`, `live_bytes 1120`. `N=20`: stdout `20`, 142 / 122 /
2240. `(ulimit -n 64; N=300 …)`: exit 0, stdout `300`, 2102 / 1802 / 33600. With `-O0`
(`N=10 … probe.sh <mfb> k2 -O0`): 82 / 72 / 1120 on main.

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
TYPE HB2
  h AS RES fs::File
END TYPE
TYPE Outer2
  n AS Integer
  inner AS HB2
END TYPE
FUNC keep2(h AS fs::File) AS HB2
  RETURN HB2[h]
END FUNC
FUNC mk2 AS HB2
  RES u AS fs::File = openOne()
  RETURN keep2(u)
END FUNC
FUNC rd(f AS fs::File) AS Integer
  RETURN len(fs::readAll(f)) - 5
END FUNC
FUNC mid AS HB2
  LET k AS HB2 = mk2()
  LET k3 AS HB2 = k
  RETURN k3
END FUNC
FUNC once() AS Integer
  LET q AS HB2 = mid()
  LET junk1 AS List OF Integer = [7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7]
  RETURN rd(q.h)
END FUNC
```

## What is known

1120 / 10 = 112 bytes per call, the size of an `fs::File` record.

With bug-1025 fixed in batch 21 the default optimization level is balanced and only `-O0`
still keeps it: 82 / 72 / 1120 with the batch-21 compiler (`probe.sh <mfb> k2 -O0`). That is
a figure reported by the coordinator, not re-measured by this document, which has only main's
compiler. The title is therefore about the unfolded copy. Guess (not traced): at the default
level the optimizer folds `LET k3 = k` away, and at `-O0` the copy remains and is not
recognized as a holder.

Found during batch 21.

## Resolution (bs22-agent-c, 2026-10-11)

### Cause

The guess is right, and the mechanism is bug-1053's. At the default level the optimizer
forwards `LET k3 AS HB2 = k`, so `mid` returns the binding that was bound from the call and
the caller's `q` is bound from a function whose every `RETURN` is a fresh block: `q` is
proved the one holder and its drain frees the file record. At `-O0` the copy stays. `mid`
then returns a local whose one store is another local, which
`CollectionUses::user_call_returns_fresh_block` does not accept as fresh, so `q` fails
`record_ownership::owning_collections` and its drain was close-only: the descriptor closed,
the 112-byte record kept. Nothing was wrong at run time; the record's owner word named
`once`'s scope, and no other reference existed.

### Fix

No change of its own. bug-1053 replaced the close-only drain with a run-time rule: a
container that is not proved the one holder of what it lists has its drain close each
record its scope owns and take it, and the exit frees the taken records after its last
cleanup (`CodeBuilder::emit_owned_list_drain`, `emit_owned_list_reclaims`; see that
document and `mfb spec memory arenas`). `q`'s drain is such a drain at `-O0`.

### Measured

Fixed compiler `target/release/mfb` against the staging compiler
`/tmp/bs22-target/release/mfb`, macOS aarch64, `--debug` counters, N=10
(`N=<n> P=/tmp/bs22-agent-c/p [SRC=/tmp/bs22-agent-c/src] /tmp/bs22/bin/probe.sh <mfb> <name> [-O0]`).
Every fixed row is also balanced at N=20, passes `(ulimit -n 64; N=300 <program>)`, and has
`double_free_skips 0`.

| row | shape | base default | base `-O0` | fixed default | fixed `-O0` |
| --- | --- | --- | --- | --- | --- |
| `k2` | as filed | 72/72/0 | 82/72/1120 | 72/72/0 | `10`, 82/82/0 (N=20: 162/162/0; N=300 under 64 descriptors: 2402/2402/0) |
| `k3` | the copy is `WITH o { n := 2 }` of an enclosing record | 82/72/1120 | 82/72/1120 | 82/82/0 | 82/82/0 |
| `k4` | a chain of copies in the callee, and a copy in the caller | 72/72/0 | 102/92/1120 | 72/72/0 | 102/102/0 |
| `k5` | a copy returned out of a loop body, and one after the loop | 282/242/4480 | 342/282/6720 | 282/282/0 | 342/342/0 |
| `k7` | a record with a `RES Handle` field | 72/72/0 | 82/72/1120 | 72/72/0 | 82/82/0 |
| `k8` | the copy returned inside a list literal (control) | 122/122/0 | 132/132/0 | unchanged | unchanged |
| `k9` | the callee fails on every other call, bound under an inline `TRAP` | 122/102/2240 | 142/122/2240 | 122/122/0 | 142/142/0 |

`k3`, `k5` and `k9` leaked at the default level too: the copy there is not one the optimizer
folds (a `WITH`, a `RETURN` from a nested block, a trapped bind).

### Checks

* RED: `MFB_TEST_EXE=/tmp/bs22-target/release/mfb cargo test --release --test rt_resource_last_holder a_copy_of_a_record`:
  1 failed (`unfolded_copy_o0 N=10: 82 allocations, 72 frees, 1120 bytes live at exit`).
  GREEN: `cargo test --release --test rt_resource_last_holder`: 11 passed.
* Every other check is bug-1053's (the same change): see its document.

### Golden impact

None of its own. bug-1053's golden impact (the tcp and udp covers) is the whole of it.

### Shared functions changed

None beyond bug-1053's.
