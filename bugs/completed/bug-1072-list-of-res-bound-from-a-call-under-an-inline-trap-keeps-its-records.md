# bug-1072: `LET xs AS List OF RES fs::File = mkList() TRAP …` keeps memory on every evaluation

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below asserting `live_bytes` does not grow between `N=10` and `N=40`

A `List OF RES fs::File` bound from a call under an inline `TRAP` closes its descriptors and keeps memory on every evaluation. The same binding without the `TRAP` is balanced.

**Owner decision (2026-10-11):** no leak is intentional (recorded in bug-1036).

**Correct behavior:** the live bytes at exit do not grow with `N` (the constant 48 bytes of the program's empty module-level list aside).

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b): `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters printed to stderr at exit (`N=<n> SRC=/tmp/bs22-agent-a/src /tmp/bs22/bin/probe.sh <mfb> <name>`). The whole program is `bugs/repro/bug-1072-bound-list-of-res-inline-trap.mfb` (bug-1056's helper declarations, a few more unused ones, and the `main` below).

`s74`, `N=10`: exit 0, stdout `n=20 keep=0`, `alloc_calls 170`, `free_calls 117`, `live_bytes 7792`. `N=40`: stdout `n=80 keep=0`, 650 / 447 / 30832. 768 bytes per evaluation.

With the batch 22 compiler after bug-999's fix (bs22-agent-a, c76430e10) the callee's own block is freed and the rest stays: 170 / 127 / 5392 at `N=10`, 650 / 487 / 21232 at `N=40`, 528 bytes per evaluation.

```
FUNC main AS Integer
  MUT n AS Integer = 0
  MUT keepL AS List OF RES fs::File = []
  FOR i = 1 TO toInt(os::getEnvOr("N", "10"))
    LET xs AS List OF RES fs::File = mkList() TRAP(e)
      RECOVER []
    END TRAP
    n = n + count(xs)
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

Not traced. The inline-`TRAP` desugar binds the call's result through a wrapper temporary; what stays is the wrapper and the file records (reported by bs22-agent-a, not verified). Near bug-1064 (a record bound from a call under an inline `TRAP`) and bug-1051.

Found during batch 22 (reported by bs22-agent-a, reproduced by the coordinator).
