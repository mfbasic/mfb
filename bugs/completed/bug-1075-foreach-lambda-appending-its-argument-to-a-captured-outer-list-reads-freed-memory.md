# bug-1075: a `collections::forEach` lambda that appends its argument to a captured outer list leaves that list reading freed memory

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory safety)

Status: Fixed (bs22-agent-g)
Regression Test: `rt_resource_model::bug_1075_foreach_lambda_appends_to_captured_outer_list`; `rt_resource_drop_reclaims::an_element_a_callback_keeps_is_closed_not_freed` (its pinned `closed` corrected to `open`)

`keep` is declared at function level. In a nested block a `RES a` is opened, appended to a block-local list `xs`, and `collections::forEach(xs, LAMBDA(h AS fs::File) -> keep = collections::append(keep, h))` stores each element into `keep` through the lambda's by-reference capture. The nested block's exit closes and frees `a`; reading `keep` afterwards reads a freed record.

**Owner decision (2026-10-11, recorded in bug-1045):** a resource put in a list takes the list's scope when that scope is higher than its own.

**Correct behavior:** the file takes `keep`'s scope and is read after the block; the program exits 0 with `live_bytes 0` (the stdout must be measured once it runs).

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and the batch 22 staging compiler (15449e66a), the same result on both unless a line says otherwise: `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters printed to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The whole program is `bugs/repro/bug-1075-foreach-lambda-appends-to-captured-outer-list.mfb`; the part after its `main` is shown.

`N=10` and `N=40`: exit 255, empty stdout, `Error: 7-703-0009` (`Resource handle was moved to another thread by thread::transfer and is no longer usable`, the text a freed record's flag word happens to select), `alloc_calls 11`, `free_calls 9`, `live_bytes 432`. It fails in the first call.

```
FUNC mk() AS HB
  RETURN HB[openOne()]
END FUNC
FUNC once() AS Integer
  MUT keep AS List OF RES fs::File = []
  MUT t AS Integer = 0
  IF TRUE THEN
    MUT xs AS List OF RES fs::File = []
    RES a AS fs::File = openOne()
    xs = collections::append(xs, a)
    collections::forEach(xs, LAMBDA(h AS fs::File) -> keep = collections::append(keep, h))
    t = t + len(xs)
  END IF
  FOR EACH f IN keep
    t = t + rd(f) + 1
  NEXT
  RETURN t
END FUNC
```

## What is known

The store-time raise of bug-1045 (`record_ownership::StoreFloats`) sees assignments in the function's own body; an assignment made inside a `LAMBDA` through a by-reference capture is not one of them, so nothing raises the record (`record_ownership::stores_through_a_captured_reference` names this store). bug-1053's last-holder rule keeps the close-only drain in a function that contains such a store; see bug-1078.

Found during batch 22 (reported by bs22-agent-c, reproduced by the coordinator).

## Resolution (bs22-agent-g, 2026-10-11)

Rule 4 was not followed at one store: an assignment inside a `LAMBDA` through a by-reference capture carried an alias to a longer-lived binding without raising its cleanup scope. The captured binding is now a cell with its scope token and owned-list heads, and the store raises what it stores to that scope (`setup_ref_cell`, `emit_captured_store_float`).

Regression tests: `rt_resource_model::bug_1075_foreach_lambda_appends_to_captured_outer_list`; `rt_resource_drop_reclaims::an_element_a_callback_keeps_is_closed_not_freed` (its pinned `closed` corrected to `open`). Each builds the bug's repro (`bugs/repro/`) with `--debug` and requires, at N=10 and N=40, exit 0, the expected total, `double_free_skips 0` and live bytes that do not grow, and exit 0 at N=300 under `ulimit -n 64`.

## Golden impact

Not regenerated. Expect `.ncode` changes wherever a `RES` binding, a resource-union binding or an owned-list is dropped (the take and the reclaim after the exit's last cleanup), wherever a call returns a value reaching resources (the statement's take and drain), and at assignments that float.
