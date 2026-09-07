# bug-573: every error raised through `_mfb_make_error_result` orphans the `ErrorLoc` it just built

Last updated: 2026-09-07
Effort: medium (the free is small; proving who owns `x3` on the propagate path is the work)
Severity: **HIGH** (unbounded leak on any loop whose builtin call raises)
Class: Memory / correctness

Status: Open
Regression Test: pinned as a NEGATIVE (still-leaks) case —
`tests/runtime/rt_scope_drop_leaks.rs::an_inline_builtins_own_domain_error_still_leaks_its_error_loc`.
That test asserts the RSS **grows**, so fixing this bug reds it on purpose and
forces this document to be closed with the fix.

Found while fixing bug-565, and measured to be a different defect: bug-565 is on
the TRAP side and is fixed; this one is on the RAISE side and is byte-identical
before and after it.

## Failing reproduction

```
IMPORT io
IMPORT collections
SUB main()
  LET xs AS List OF String = ["aa", "bb", "cc"]
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < N
    LET g AS String = collections::get(xs, 9) TRAP(e2)
      RECOVER "zz"
    END TRAP
    acc = acc + len(g)
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
```

Peak RSS (`/usr/bin/time -l`, macOS arm64, release), **identical before and after
bug-565's fix**:

| N | peak RSS |
| --- | --- |
| 200 000 | 39.1 MB |
| 400 000 | 77.2 MB |

~200 B per raised error. The `TRAP` is only there to keep the program running;
the leak is upstream of it.

## The decisive evidence: it scales with the FILENAME

The same program built so its recorded source path is 131 characters instead of
`src/main.mfb`'s 12:

| N | peak RSS, long path |
| --- | --- |
| 200 000 | 149.9 MB |
| 400 000 | 298.9 MB |

~750 B per raised error. Nothing in the program changed but the path recorded in
the `ErrorLoc`, whose `filename` is inlined into the block. That is what
identifies the orphan.

## Root cause

`emit_error_register_return` assembles a raised error in two steps:

1. `_mfb_make_error_result` (plan-16) allocates an **`ErrorLoc`** — filename,
   line, column — and returns it in `RESULT_ERROR_SOURCE_REGISTER` (`x3`).
2. `_mfb_rt_park_error` (`emit_park_error_block_from_registers`, plan-118-E
   phase 2) allocates the single owned flat `Error` block and INLINES copies of
   the message and that `ErrorLoc` into it, parks the block, and RESTORES the
   loose registers — including the original `x3`.

After step 2 the `ErrorLoc` from step 1 has no owner: the parked `Error` holds a
copy, not it. Nothing frees it, on any path.

## Why it is not a two-line fix

The obvious free — inside `_mfb_rt_park_error`, right after the block is built —
is wrong. The helper is a single synthesized function shared by every raise site,
and its `x3` input is not always a fresh block:

* on the `_mfb_make_error_result` path it IS fresh (this is the leak);
* on a **propagated** error the same registers carry a `source` that is an
  interior pointer INTO the caller's parked `Error`, which
  `route_current_result_to_trap`'s rebuild branch and
  `emit_trapped_error_result`'s `CalleeRegister` source both read after the park;
* on the OOM-degraded path (`building_error_block`) `x3` may be null.

So the fix needs the per-site ownership answer, not a helper-local one — either a
second entry point taking "this `ErrorLoc` is mine", or the same runtime
pointer-identity guard bug-565 and bug-571 use, comparing `x3` against the parked
block's own inlined `source` pointer. Either way it ADDS a free on the path every
error in the language takes, so it wants its own change, its own enumeration of
raise sites, and its own gate.

## What a fix must produce

The reproduction above flat at both counts, with the long-path variant flat too
(that is the sensitive form), and every error-message / `e.source` behaviour
fixture unchanged — the origin must survive, because a raised error's
`ErrorLoc` is what `mfb`'s top-level error printer reports.
