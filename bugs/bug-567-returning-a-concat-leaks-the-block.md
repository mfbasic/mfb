# bug-567: a user function that RETURNS a `&` concatenation leaks its block, 64 B per call

Last updated: 2026-09-06
Effort: small–medium
Severity: **HIGH** (unbounded leak on the commonest String-returning function shape)
Class: Memory / correctness

Status: Open
Regression Test: — (a `tests/rt_scope_drop_leaks.rs` RSS case, to be added)

Found while fixing bug-561. Reproduces unchanged on `19880284452` and after both
bug-560 and bug-561.

## The finding

```
FUNC f(n AS Integer) AS String
  RETURN "v" & toString(n MOD 10)     ' <- a concat, not a bare call
END FUNC
```

Called in a loop, bound or unbound, this leaks **64 B per call**.

| program | 200k | 400k |
| --- | --- | --- |
| `acc = acc + len(f(i))`, `RETURN "v" & toString(n MOD 10)` | **13.3 MB** | **25.6 MB** |
| `LET s AS String = f(i)`, same body | **13.3 MB** | **25.6 MB** |
| contrast: same call shapes, body `RETURN toString(n MOD 10)` | 1.0 MB | 1.0 MB |

(macOS arm64, peak RSS via `/usr/bin/time -l`.)

The ONLY difference between the leaking and the flat program is the callee's
return expression: a `Binary { & }` versus a bare `Call`.

## Why bug-536 shape B-2 does not cover it

B-2's `tests/rt_scope_drop_leaks.rs` cases all return either a bare call
(`SHAPE_B2_TRANSITIVE`: `RETURN toString(i)` / `RETURN leaf(i)`), a `MUT` local
(`SHAPE_B2_APPEND_ARGUMENT`), or a literal (`SHAPE_B2_RETURNED_LITERAL`). None
returns a concatenation, so the corpus has a hole exactly where the commonest
real body sits. `SHAPE_B_CONCAT` covers `len("x" & toString(i) & "y")` but
*inline in `main`*, where it is flat — so the concat producer itself is fine and
the defect is in what happens to that block at the RETURN/caller seam.

## Where to look

`lower_returned_value`'s four return shapes (bug-536 B-2's table): a concat
result is "a claimed pending temp" — the producer's own `arena_alloc`, marked by
`mark_fresh_string`. Check whether the claim actually matches at a `RETURN`
whose operand is a `Binary`, and whether `function_returns_fresh_string` then
licenses the caller's free (it should: `f` returns a value, is not a param
borrow, is not callback-referenced). One of those two links is not connecting.

## What a fix must produce

`RETURN <concat>` runs at constant RSS at both counts, bound and unbound, and the
existing B-2 shapes stay flat with no second free (`arena_free` on a block a
caller still owns is a use-after-free, not a leak).

Measure as peak RSS at N and 2N, never a one-shot.


## Root cause — from bug-570, which was filed for the same defect and folded in here

An agent working bug-562 reproduced this shape independently and filed it as
bug-570 before bug-567 had landed. Same defect; bug-570 is withdrawn and its
analysis is kept here, because it is sharper than what this document had.

**`clear_pending_temps_to` truncates *every* pending temp above the watermark at
a control transfer.** Its justification is two clauses, and the second is false:

* *"the returned temp is moved to the caller"* — true, but only of the ONE temp
  `claim_pending_temp` has already popped.
* *"an interior free would be unreachable"* — **false.** It is unreachable only
  because it would be emitted *after* the branch. That is a property of where the
  code is placed, not of the program.

So an INTERIOR temp — one that is not the returned value — is dropped without
being freed.

That predicts exactly which spellings leak, and the prediction was measured:

| shape | interior temp? | 200k / 400k |
| --- | --- | --- |
| `RETURN "<" & s & ">"` | yes (the inner concat) | 25.6 MB / 50.2 MB |
| `RETURN "v" & toString(n MOD 10)` | yes (`toString`'s result) | 13.3 MB / 25.6 MB |
| `RETURN s & ">"` | **no** — both operands are already-owned values | 1.0 MB / 1.0 MB |

The flat row is the control: one operator fewer, no interior temp, no leak.

**A fix here ADDS frees**, which is the double-free direction, so it needs its own
enumeration of what is and is not interior — not a relaxation of the truncation.
Neither bug-562's change nor bug-536 shape B-2 moves these numbers at all.
