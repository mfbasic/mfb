# bug-1077: a function-level `TRAP` handler that reads a `List OF RES` of the function reads freed records

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory safety)

Status: Open
Regression Test: (proposed) a runtime test over the program asserting exit 0 and stdout `70`

`MUT xs AS List OF RES fs::File = []` at function level; a file opened in a nested block is appended to it; a later call fails and the function's own `TRAP(e)` handler iterates `xs` and reads each file. The error route frees the records before the handler runs.

**Correct behavior:** the handler reads the open files: by hand (not measured), `rd(h) + 1` is 7 per call, stdout `70` at `N=10`; exit 0, `live_bytes 0`.

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and an earlier batch 22 build, the same result on both unless a line says otherwise: `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters printed to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The whole program is `bugs/repro/bug-1077-function-trap-handler-reads-drained-list.mfb`; the part after its `main` is shown.

`N=10` and `N=40`: exit 255, empty stdout, `Error: 7-703-0009`, `alloc_calls 10`, `free_calls 9`, `live_bytes 192`.

```
FUNC boom(n AS Integer) AS Integer
  IF n > 0 THEN
    FAIL error(77, "boom")
  END IF
  RETURN 1
END FUNC
FUNC once() AS Integer
  MUT xs AS List OF RES fs::File = []
  MUT t AS Integer = 0
  IF len("x") = 1 THEN
    RES a AS fs::File = openOne()
    xs = collections::append(xs, a)
  END IF
  t = t + boom(len(xs))
  RETURN t
  TRAP(e)
    FOR EACH h IN xs
      t = t + rd(h) + 1
    NEXT
    RETURN t
  END TRAP
END FUNC
```

## What is known

Not traced. On the route to the function's own handler the function-level list is drained (closed and freed) although the binding stays in scope for the handler (`trap_route_cleanups`, `src/codegen/engine/control/builder_exits.rs`).

Found during batch 22 (reported by bs22-agent-c, reproduced by the coordinator).
