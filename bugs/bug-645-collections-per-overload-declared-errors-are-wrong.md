# bug-645: `collections` per-overload declared errors are wrong (`set` map overload, `sum` `Float` overload)

Last updated: 2026-09-16
Effort: small
Severity: LOW
Class: Correctness (declared-error data disagrees with behavior)

Status: Open
Regression Test: none yet — see Phase 1

Two `collections` members declare the same error list on every overload, but not
every overload can raise those errors. The rendered Errors tables are therefore
wrong for some overloads:

- **`collections::set`** declares `ErrIndexOutOfRange` on both overloads
  (`src/codegen/builtins/collections/func_set.rs:119,147`). The `Map OF K TO V`
  overload has no index and never range-checks: assigning an absent key inserts
  it (`/tmp/p125-ex/colkeys`). Only the `List OF T` overload can raise
  `ErrIndexOutOfRange`.
- **`collections::sum`** lists `ErrOverflow` on all three overloads. The `Integer`
  and `Fixed` overloads raise `ErrOverflow` on a total out of range
  (`/tmp/p125-ex/colmix`, `/tmp/p125-ex/colsumfixed`). The `Float` overload raises
  `ErrFloatOverflow` (`77050015`) instead (`/tmp/p125-ex/colsumflt2`), and does not
  declare it.

This is the `collections` counterpart of bug-617 (the same defect in `math`'s
per-overload declarations) and is separate from bug-641, which covers members that
declare no errors at all.

**The single correct behavior a fix produces:** each overload's declared errors
are exactly the errors that overload can raise, so every rendered Errors table
names the right overloads:

- `set`: `ErrIndexOutOfRange` on the list overload only.
- `sum`: `ErrOverflow` on `Integer` and `Fixed`, and `ErrFloatOverflow` on `Float`.

Found by plan-125-C Phase 4 while triaging the `set` and `sum` man-page reviews.
**Filed, not fixed**, by user instruction during a documentation-only plan ("file
all bugs, make no fixes").

## Reproduction

```
$ ./target/release/mfb man collections set    # Errors: ErrIndexOutOfRange — Overloads 1, 2
$ ./target/release/mfb man collections sum    # Errors: ErrOverflow — Overloads 1, 2, 3
```

Behavior evidence:

- `collections::set(mapValue, "absentKey", v)` inserts the key and raises nothing.
- An overflowing `Float` `collections::sum` raises `77050015`
  ("Floating-point arithmetic overflowed to infinity"), not `77050010`.

## Root cause

Each descriptor's `errors` list is written per implementation by hand, and these
two were copied across overloads without being narrowed to what each overload's
lowering can raise.

## Non-goals

- Changing what any overload raises.
- The inline-`TRAP` defect on the `Float` `sum` overload (bug-644), which is separate.
- Members that declare no errors at all (bug-641).

## Blast-radius audit

Census every multi-overload `collections` member for per-overload accuracy:
`contains`, `find`, `get`, `getOr`, `append`, `set`, `sum`. Also check any
`strings`/`math` overloads sharing the same list-versus-map or numeric-type split.

## Fix

Phase 1 — RED test asserting each overload's declared errors against a probe of
what that overload raises, plus the census. Commit:

Phase 2 — narrow the declarations (GREEN); re-render the affected man pages; full
suite and man-page goldens. Commit:
