# bug-1078: a function that contains a store bug-1075 or bug-1076 describes keeps the close-only drain for its unproved containers: 112 bytes per call

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-g)
Regression Test: `rt_resource_model::bug_1078_close_only_drain_kept_beside_an_unraised_outward_store`

bug-1053 made a record freed by its last holder. It left one guard: a function that contains a store carrying a reference outward WITHOUT raising the owner word (a by-reference capture assignment in a lambda, bug-1075; a non-inserting `collections` member's result assigned to an outer binding, bug-1076) keeps the old close-only drain for every container it cannot prove, because freeing there would turn those bugs' closed-handle error into a read of freed memory. In such a function bug-1053's shapes still keep their record.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes 0` at exit. Needs bug-1075 and bug-1076 fixed first (every outward store raises), then the guard (`record_ownership::runtime_reclaim`, `StoreFloats::unraised_outward_store`, `stores_through_a_captured_reference`) is removed.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and the batch 22 staging compiler (15449e66a), the same result on both unless a line says otherwise: `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters printed to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The whole program is `bugs/repro/bug-1078-close-only-drain-kept-beside-an-unraised-outward-store.mfb`; the part after its `main` is shown.

`N=10`: exit 0, stdout `70`, `alloc_calls 92`, `free_calls 82`, `live_bytes 1120`. `N=40`: stdout `280`, 362 / 322 / 4480. 112 bytes per call, on both compilers. The same function without the `keep = collections::filter(…)` statement is bug-1053's `r3`, balanced on staging (72/72/0).

```
FUNC mk() AS HB
  RETURN HB[openOne()]
END FUNC
FUNC once() AS Integer
  MUT keep AS List OF RES fs::File = []
  LET k AS HB = mk()
  LET a AS Integer = rd(k.h)
  RES f AS fs::File = k.h
  IF a = 6 THEN
    MUT xs AS List OF RES fs::File = []
    keep = collections::filter(xs, LAMBDA(h AS fs::File) -> TRUE)
  END IF
  RETURN a + rd(f) + 1 + len(keep)
END FUNC
```

## What is known

See `mfb spec memory arenas`, "Two stores carry a reference to a longer-lived binding without raising the owner word".

Found during batch 22 (reported by bs22-agent-c, reproduced by the coordinator).

## Resolution (bs22-agent-g, 2026-10-11)

The close-only drain was a guard for the two stores that did not float (bug-1075, bug-1076). Both float now, so the guard (`runtime_reclaim`, `stores_through_a_captured_reference`, `unraised_outward_store`) is deleted and every scope frees the records it closes.

Regression tests: `rt_resource_model::bug_1078_close_only_drain_kept_beside_an_unraised_outward_store`. Each builds the bug's repro (`bugs/repro/`) with `--debug` and requires, at N=10 and N=40, exit 0, the expected total, `double_free_skips 0` and live bytes that do not grow, and exit 0 at N=300 under `ulimit -n 64`.

## Golden impact

Not regenerated. Expect `.ncode` changes wherever a `RES` binding, a resource-union binding or an owned-list is dropped (the take and the reclaim after the exit's last cleanup), wherever a call returns a value reaching resources (the statement's take and drain), and at assignments that float.
