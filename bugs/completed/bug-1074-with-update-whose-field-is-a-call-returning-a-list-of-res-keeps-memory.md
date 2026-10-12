# bug-1074: `LET b = WITH a { items := mkList() }`, a `WITH` update whose field value is a call returning a `List OF RES`, keeps memory on every evaluation

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Fixed (bs22-agent-g)
Regression Test: `rt_resource_model::bug_1074_with_update_from_a_call`

`LET b AS Layer = WITH a { items := mkList() }`, where `Layer` holds a `List OF RES fs::File`
and `mkList()` is an unbound call: the files are closed, and memory is kept on every
evaluation. This is the `WITH` form of bug-999's constructor-argument position
(`Layer[items := mkList()]`), which bug-999 fixed; `WITH` was not one of bug-999's positions.

**Owner decisions (2026-10-11):** no leak is intentional (recorded in bug-1036). An unbound
aggregate takes the scope of whatever takes it; here the record `b` takes it.

**Correct behavior:** the live bytes at exit do not grow with `N`.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64, `mfb build -q --debug`, run with `N=<n>`, reading the
`arena.0.*` counters printed to stderr at exit. The whole program is
`bugs/repro/bug-1074-with-update-from-a-call.mfb`.

- main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b): `N=10` exit 0,
  stdout `n=20`, `alloc_calls 302`, `free_calls 223`, `live_bytes 7920`; `N=40` stdout `n=80`,
  1172 / 883 / 30480. 752 bytes per evaluation.
- batch 22 staging compiler with bug-999 fixed (3b47a25ae): `N=10` 312 / 263 / 5200; `N=40`
  1212 / 1043 / 19600. 480 bytes per evaluation. `N=300` under `ulimit -n 64` exits 0, so the
  descriptors are closed.

```
FUNC main AS Integer
  MUT n AS Integer = 0
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    LET a AS Layer = mkLayer()
    LET b AS Layer = WITH a { items := mkList() }
    n = n + len(b.items)
  NEXT
  io::print("n=" & toString(n))
  RETURN 0
END FUNC
```

## What is known

Reported by bs22-agent-a, not verified: what stays is `b`'s records, list and block. `WITH`
shares the other fields with `a`, and proving `b` the sole holder of what it reaches needs
field types the ownership walk (`record_ownership`) does not have.

Found during batch 22 (reported by bs22-agent-a, reproduced by the coordinator).

## Resolution (bs22-agent-g, 2026-10-11)

A resource a callee's `RETURN` released stayed unowned until some binding claimed it, so in every position where nothing bound the call nothing owned it. It is the caller's statement's at once (rule 5: `register_resource_temp`, `adopt_statement_temp`, however the function was reached), every longer-lived taker raises it (rule 4), and the statement closes and frees what it still owns. The `WITH` update's binding is the taker of what the call inside it returned.

Regression tests: `rt_resource_model::bug_1074_with_update_from_a_call`. Each builds the bug's repro (`bugs/repro/`) with `--debug` and requires, at N=10 and N=40, exit 0, the expected total, `double_free_skips 0` and live bytes that do not grow, and exit 0 at N=300 under `ulimit -n 64`.

## Golden impact

Not regenerated. Expect `.ncode` changes wherever a `RES` binding, a resource-union binding or an owned-list is dropped (the take and the reclaim after the exit's last cleanup), wherever a call returns a value reaching resources (the statement's take and drain), and at assignments that float.
