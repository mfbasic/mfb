# bug-572: a capturing `LAMBDA` leaks its closure environment on every call that takes it

Last updated: 2026-09-07
Effort: small
Severity: MEDIUM (bounded per call site, unbounded across repeats)
Class: Memory / closures

Status: Open
Regression Test: — (an RSS case in `tests/runtime/rt_scope_drop_leaks.rs`)

Found while auditing bug-569's callback enumeration, as the residual under a
capturing-lambda callback. It is independent of the callback's return type — it
reproduces with a `Boolean` predicate, which allocates no result block at all.

## Failing reproduction

```
IMPORT io
IMPORT collections

SUB main()
  LET cap AS String = "CAPTURED"
  LET xs AS List OF String = ["n0", "n1", "n2", "n3", "n4", "n5", "n6", "n7"]
  MUT i AS Integer = 0
  MUT acc AS Integer = 0
  WHILE i < 50000
    LET c AS List OF String = collections::filter(xs, LAMBDA(s AS String) -> len(s) < len(cap))
    acc = acc + len(collections::get(c, 0))
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
```

Peak RSS (`/usr/bin/time -l`), macos-aarch64, release:

| N (outer passes) | peak RSS |
|---|---|
| 50 000 | 13 MB |
| 100 000 | 25 MB |

The contrast is one token: dropping the capture (`len(s) < 8`, so the lambda is
capture-less and lowers to a `FunctionRef` rather than a `Closure`) is **1.0 MB
flat** at both counts.

## Why it matters beyond the megabytes

§14.4 says "a closure environment is owned by the function value. Dropping the
function value drops its captured values in reverse capture order." The function
value here is a temporary that dies at the end of the statement, and nothing drops
it — so the environment block, and the copies of the captured values inside it,
are never reclaimed.

## What a fix must produce

The loop above runs at constant RSS; a capture-less lambda stays flat; and a
closure that ESCAPES the statement (returned, stored in a collection, or passed to
`thread::start`) is not freed at the statement end. `collect_value_used_locals`
(`function_lowering.rs`) already exists to answer exactly that escape question for
a closure binding, so the shape of the gate is already in the tree.
