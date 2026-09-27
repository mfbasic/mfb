# bug-645: `collections` per-overload declared errors are wrong (`set` map overload, `sum` `Float` overload)

Last updated: 2026-09-16
Effort: small
Severity: LOW
Class: Correctness (declared-error data disagrees with behavior)

Status: PARTIALLY FIXED (2026-09-27) — `set` fixed; `sum`'s Float claim does not
reproduce at this HEAD, see Fix section
Regression Test: `codegen::engine::tests::builtins_codegen::declared_errors_all::
every_collections_member_declares_exactly_the_errors_its_lowering_raises`
(covers `set`'s per-overload split; does not check `sum`, a `Body::abi_inline`
member this sweep does lower but which produced no mismatch either way — see
below)

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

## Fix (landed via bug-698, partial)

**`set`: fixed.** bug-698's generalized lowering sweep measures `set`'s two
overloads separately. The `Map OF K TO V` overload declared `ErrIndexOutOfRange`
alongside the `List OF T` overload's, though only the list overload's lowering
range-checks (the map overload inserts a missing key, per its own already-
accurate prose: "The map overload has no failure path at all"). Narrowed the
map overload to `errors: vec!["ErrOutOfMemory"]` (both overloads can allocate;
bug-641 covers that half) and added one shared prose clause. Verified in the
rendered page: `mfb man collections set` now shows `ErrIndexOutOfRange` on
Overload 1 only.

**`sum`: does NOT reproduce at this HEAD.** Repro attempted 2026-09-27: a
program summing two `Float` values of `1.0e308` (`x + y`, mathematically
`2e308`, over `DBL_MAX ≈ 1.7976931348623157e308` — IEEE-754 defines this as
overflow to `+Infinity`) through `collections::sum` raised **nothing** and
printed a large finite decimal (matching `DBL_MAX`'s digits) rather than
raising `ErrFloatOverflow` or reporting infinity. The same result came from a
bare `Float + Float` add outside `collections::sum` entirely. `lower_sum`'s own
`ParameterType::Float` arm (`func_sum.rs`) does a plain `float_add_d` with no
overflow check of any kind — there is no `ErrFloatOverflow` raise site in this
function, contradicting this doc's original claim that one exists. This reads
as a genuine Float-arithmetic-overflow gap (either the check was never wired
for `+`, or `float_add_d`/its surrounding codegen does not produce IEEE
infinity the way expected), not a declared-errors documentation mismatch —
bug-698's sweep is silent on `sum` because there is nothing for it to
disagree with (declared and measured are both empty for `ErrFloatOverflow`).
Left `sum`'s descriptor unchanged. This is a **false-premise finding** for the
`sum`/`ErrFloatOverflow` half of this bug specifically, not proof the class
of per-overload defect is wrong; filing the Float-overflow gap as its own bug
is follow-up work, not done here.

Commit: (this worktree, bug-698 branch)
