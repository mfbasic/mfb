# bug-1075: a `collections::forEach` lambda that appends its argument to a captured outer list leaves that list reading freed memory

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory safety)

Status: Open
Regression Test: (proposed) a runtime test over the program asserting exit 0, the measured stdout and `live_bytes 0`

`keep` is declared at function level. In a nested block a `RES a` is opened, appended to a block-local list `xs`, and `collections::forEach(xs, LAMBDA(h AS fs::File) -> keep = collections::append(keep, h))` stores each element into `keep` through the lambda's by-reference capture. The nested block's exit closes and frees `a`; reading `keep` afterwards reads a freed record.

**Owner decision (2026-10-11, recorded in bug-1045):** a resource put in a list takes the list's scope when that scope is higher than its own.

**Correct behavior:** the file takes `keep`'s scope and is read after the block; the program exits 0 with `live_bytes 0` (the stdout must be measured once it runs).

## Failing Reproduction

Measured 2026-10-11 on macOS aarch64 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at b6bf0327b) and an earlier batch 22 build, the same result on both unless a line says otherwise: `mfb build -q --debug`, run with `N=<n>`, reading the `arena.0.*` counters printed to stderr at exit. `/tmp/g933/data.txt` is a 6-byte file ("hello\n"). The whole program is `bugs/repro/bug-1075-foreach-lambda-appends-to-captured-outer-list.mfb`; the part after its `main` is shown.

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

The store-time raise of bug-1045 (`record_ownership::StoreFloats`) sees assignments in the function's own body; an assignment made inside a `LAMBDA` through a by-reference capture is not one of them, so nothing moves the Resource's cleanup scope out to `keep`'s scope (not traced further).

Found during batch 22 (reported by bs22-agent-c, reproduced by the coordinator).
