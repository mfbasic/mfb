# bug-562: a `String` callback whose body is the `toString` identity SIGSEGVs

Last updated: 2026-09-06
Effort: small-to-medium (the change is one arm; the AUDIT is the work)
Severity: **HIGH** (a crash on ordinary source; no diagnostic)
Class: Memory / callback ABI

Status: Open
Regression Test: — (an `rt-behavior` fixture, to be added)

Found while fixing bug-536 shape B-2. **Reproduces identically on the base commit
and after that change.**

## Failing reproduction — fourteen lines

```
IMPORT io
IMPORT collections

FUNC identish(s AS String) AS String
  RETURN toString(s)
END FUNC

SUB main()
  MUT xs AS List OF String = []
  MUT k AS Integer = 0
  WHILE k < 3
    xs = collections::append(xs, "n" & toString(k))
    k = k + 1
  END WHILE
  LET c AS List OF String = collections::transform(xs, identish)   ' [exit 139]
  io::print("c=" & collections::get(c, 0))
END SUB
```

Observed: `[exit 139]` (SIGSEGV). Expected: `c=n0`.

## Root cause

The `FunctionRef` ABI **owns and frees** a callback's return value. That is
precisely why plan-86 K1 excludes callback-referenced functions from the
param-borrow elision — the exclusion FORCES a copy, so the value the HOF frees is
one it owns.

`identish` escapes that force: it returns a `Call`, not a bare `Local`, so it is
**not** a param-borrow function and K1's exclusion never applies to it. Meanwhile
`toString`'s `String` arm is the identity, so it hands the HOF the caller's own
list-element block — which the HOF then frees. The list is left holding freed
memory.

So the defect is the *interaction* of two correct-looking pieces: the identity arm
and an ABI that frees.

## The fix, and why it is not a one-word change in practice

bug-536 shape B-2's `function_returns_fresh_string` currently **excludes**
callback-referenced functions. That exclusion is deliberately conservative — it
keeps callback lowering byte-identical rather than changing a second ABI in one
step — and it is recorded as the one place that predicate is knowingly weaker than
it should be.

**Dropping the `callback_referenced` arm is the fix**: it turns the exclusion from
"no obligation" into "the callee copies", which is exactly what K1's exclusion
achieves for the borrow shape.

One word of code, and then the real work: a **callback-ABI audit**. Every shape
that reaches a `FunctionRef` return has to be enumerated and shown to hand the HOF
a block it owns — the identity arm is the one that was found, not necessarily the
only one. A partial fix here produces a double free rather than a leak.

## What a fix must produce

The repro prints `c=n0` and exits 0, `collections::transform` and every other HOF
still free exactly once, and a callback that returns a genuinely fresh block is
not copied twice.

Positive pin required: an existing callback shape must be measurably unchanged —
byte-identical codegen for a callback that already worked.
