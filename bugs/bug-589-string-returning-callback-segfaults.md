# bug-589: a `String`-returning function used as a CALLBACK double-frees and SIGSEGVs

Last updated: 2026-09-12
Effort: small — the fix is believed to be ONE word; the callback-ABI audit is
the actual work
Severity: HIGH — a crash (`exit 139`) on a valid fourteen-line program
Class: Memory / correctness (double free)

Status: Open
Regression Test: an `rt` fixture driving a `String`-returning callback through
`collections::transform`.

Split out of **bug-536**, found while fixing shape B-2 and recorded rather than
filed so the numbering would not race a peer session. It reproduces identically
on that fix's base commit and after it.

## Reproduction

Fourteen lines, exits **139**:

```basic
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

## Root Cause

A chain of three individually reasonable decisions:

1. The `FunctionRef` ABI **owns and frees** the callback's return value. That is
   precisely why plan-86 K1 excludes callback-referenced functions from the
   param-borrow elision — the exclusion FORCES a copy.
2. `identish` is **not** a param-borrow function (it returns a `Call`, not a bare
   `Local`), so K1's forced copy does not apply to it.
3. `toString`'s identity arm hands the HOF the **caller's own list-element
   block** — not a fresh one. The HOF then frees it.

Result: the HOF frees a block it does not own. This is a **double free**, not a
leak, which is why it presents as a segfault rather than growth.

bug-536 shape B-2 does not fix it because `function_returns_fresh_string`
**excludes** callback-referenced functions, and that exclusion was deliberately
conservative — it kept callback lowering byte-identical rather than changing a
second ABI in the same change.

## Fix Design

**The fix is believed to be one word:** drop the `callback_referenced` arm from
`function_returns_fresh_string`. That turns the exclusion from "no obligation"
into "the callee copies" — exactly what K1's exclusion already achieves for the
borrow shape.

**But the one-word change is not the work.** It changes the callback ABI's
ownership contract, so it needs its own callback-ABI audit:

- enumerate every producer that can reach a `FunctionRef` return slot, and
  assert the enumeration is TOTAL (a wildcard-free `match`, so a new variant is
  a build error) — a default-to-safe gate here is exactly how bug-572 nearly
  shipped a use-after-free;
- `toString`'s identity arm is the known aliasing producer; find the others
  before assuming it is alone. A recogniser and its measurer are two lists.

### Non-goals (must NOT change)

- Do not remove the `FunctionRef` ABI's ownership of the return value; make the
  callee satisfy it instead.
- Do not weaken plan-86 K1's exclusion for the borrow shape.

## Memory gate (required)

1. the RED fixture stops exiting 139 and prints the expected value;
2. name the contract in `.ai/collections.md` (HOF-rewrite tradeoffs) / `mfb spec`
   §14 the fix realizes, and show it only ADDS a copy — never moves a lifetime;
3. artifact-gate delta confined to the fixtures that emit a callback, everything
   else byte-identical;
4. a POSITIVE pin: a param-borrow callback (`RETURN s`) and a genuinely-fresh
   producer callback both still work and do NOT gain a redundant copy.

A leak test cannot see this bug and a flatness assertion cannot either — the
failure is a crash. Pin it by exit status and output.
