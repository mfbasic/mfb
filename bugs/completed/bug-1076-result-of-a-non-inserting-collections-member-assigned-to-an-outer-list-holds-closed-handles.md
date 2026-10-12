# bug-1076: `keep = collections::filter(xs, …)` from a nested block leaves closed handles in the outer list

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (use after close)

Status: Fixed (bs22-agent-g)
Regression Test: `rt_resource_model::bug_1076_filter_result_assigned_to_outer_list`

`keep` is declared at function level; in a nested block `xs` is bound from a call and `keep = collections::filter(xs, LAMBDA(h AS fs::File) -> TRUE)` stores a list of `xs`'s elements into it. The nested block's exit closes the files; reading `keep` afterwards fails. This is bug-1045's defect for a `collections` member that does not insert its argument (bug-1045 covered `append`, `prepend`, `insert`, `set`, literals, constructors and `WITH`).

**Owner decision (2026-10-11, recorded in bug-1045):** a resource put in a list takes the list's scope when that scope is higher than its own.

**Correct behavior:** the files take `keep`'s scope; the program exits 0 with `live_bytes 0` (the stdout must be measured once it runs).

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and the batch 22 staging compiler (15449e66a), the same result on both unless a line says otherwise: `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters printed to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The whole program is `bugs/repro/bug-1076-filter-result-assigned-to-outer-list.mfb`; the part after its `main` is shown.

`N=10` and `N=40`: exit 255, empty stdout, `Error: 7-703-0004` (`Resource handle is already closed.`). Main's compiler: `alloc_calls 12`, `free_calls 9`, `live_bytes 480`; staging: 12 / 10 / 240.

```
FUNC mkList() AS List OF RES fs::File
  MUT out AS List OF RES fs::File = []
  RES a AS fs::File = openOne()
  out = collections::append(out, a)
  RETURN out
END FUNC
FUNC once() AS Integer
  MUT keep AS List OF RES fs::File = []
  MUT t AS Integer = 0
  IF len("x") = 1 THEN
    LET xs AS List OF RES fs::File = mkList()
    RES g AS fs::File = collections::get(xs, 0)
    t = t + rd(g)
    keep = collections::filter(xs, LAMBDA(h AS fs::File) -> TRUE)
  END IF
  FOR EACH f IN keep
    t = t + rd(f) + 1
  NEXT
  RETURN t
END FUNC
```

## What is known

`record_ownership::store_operands` walks only the inserting members, so a value built by `filter`, `take`, `drop`, `transform` and the like out of an inner-scope collection is not raised (`StoreFloats::unraised_outward_store` names this store). See bug-1078.

Found during batch 22 (reported by bs22-agent-c, reproduced by the coordinator).

## Resolution (bs22-agent-g, 2026-10-11)

Rule 4 was not followed at one store: an assignment whose value a call computed from a shorter-lived value (`keep = collections::filter(xs, f)`). Assignments are listed by the deepest scope their value can reach (`StoreFloats`), and the store raises what the assigned value reaches.

Regression tests: `rt_resource_model::bug_1076_filter_result_assigned_to_outer_list`. Each builds the bug's repro (`bugs/repro/`) with `--debug` and requires, at N=10 and N=40, exit 0, the expected total, `double_free_skips 0` and live bytes that do not grow, and exit 0 at N=300 under `ulimit -n 64`.

## Golden impact

Not regenerated. Expect `.ncode` changes wherever a `RES` binding, a resource-union binding or an owned-list is dropped (the take and the reclaim after the exit's last cleanup), wherever a call returns a value reaching resources (the statement's take and drain), and at assignments that float.
