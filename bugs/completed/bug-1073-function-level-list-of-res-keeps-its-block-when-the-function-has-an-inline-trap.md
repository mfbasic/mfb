# bug-1073: a function-level `List OF RES` keeps its 48-byte block at exit when the function has any inline `TRAP`

Last updated: 2026-10-26
Effort: small (<1h)
Severity: LOW
Class: Correctness (memory leak)

Status: Fixed between b6bf0327b (leaks) and 3745e626a (does not), before plan-183; the fixing commit was not bisected.
Regression Test: tests/runtime/rt_resource_last_holder/trapped_and_maps.rs::an_inline_trap_in_main_does_not_keep_mains_list_of_resources

`MUT keepL AS List OF RES fs::File = []` in `main`, with an unrelated inline `TRAP` elsewhere in `main` (`LET k AS Integer = toInt("7") TRAP(e) RECOVER 100 END TRAP`), leaves the list's 48-byte block live at exit. The amount is constant: it does not grow with `N`.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** `live_bytes` equals the program's without the `TRAP` (112 by bs22-agent-a's measurement, not re-measured here: the module-level list and its kept record).

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters printed to stderr at exit (`N=<n> SRC=/tmp/bs22-agent-a/src /tmp/bs22/bin/probe.sh <mfb> <name>`). The whole program is `bugs/repro/bug-1073-function-level-list-inline-trap.mfb` (bug-1056's helper declarations, a few more unused ones, and the `main` below).

`s76`, `N=10` and `N=40`: exit 0, stdout `n=70 keep=0` / `n=280 keep=0`, `alloc_calls 10`, `free_calls 6`, `live_bytes 160`. The same with the batch 22 compiler.

```
FUNC main AS Integer
  MUT n AS Integer = 0
  MUT keepL AS List OF RES fs::File = []
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    LET k AS Integer = toInt("7") TRAP(e)
      RECOVER 100
    END TRAP
    n = n + k
  NEXT
  FOR EACH k IN keepL
    fs::writeAll(k, "still open")
    n = n + 1000
  NEXT
  io::print("n=" & toString(n) & " keep=" & toString(len(gKeep)))
  FOR EACH k IN gKeep
    fs::writeAll(k, "still open")
  NEXT
  RETURN 0
END FUNC
```

## What is known

Not traced.

Found during batch 22 (reported by bs22-agent-a, reproduced by the coordinator).

## Resolution (2026-10-26 open-bug review)

Re-verified 2026-10-26 at HEAD cdecc60c1 (release build, `mfb build -q --debug`) on
`bugs/repro/bug-1073-function-level-list-inline-trap.mfb`: `N=10` prints `n=70 keep=0` with
`alloc_calls 10`, `free_calls 8`, `live_bytes 96`; `N=40` prints `n=280 keep=0` with the same
10 / 8 / 96. The same program with the inline TRAP replaced by a plain binding also ends at
10 / 8 / 96, so the TRAP no longer keeps the function's list block (the doc recorded 10 / 6 /
160). The 96 bytes left are the module-level values of bug-1068.

Correction: an earlier note credited plan-183. A compiler built at 3745e626a (2026-10-12) already
gives the fixed counts; one built at b6bf0327b reproduces the leak and fails the regression test.
