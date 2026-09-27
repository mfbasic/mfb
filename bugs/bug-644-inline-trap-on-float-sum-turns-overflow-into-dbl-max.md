# bug-644: an inline `TRAP` on a `Float` `collections::sum` turns an overflow into a silent `1.797e308` result

Last updated: 2026-09-16
Effort: medium
Severity: HIGH
Class: Correctness (silent wrong value; error handler bypassed)

Status: Open
Regression Test: none yet — see Phase 1

A `Float` `collections::sum` whose total overflows raises `ErrFloatOverflow`
(`77050015`), exactly as ordinary `Float` arithmetic does, **unless** the call has
an inline `TRAP`. With an inline `TRAP` attached, the call neither raises nor runs
`RECOVER`: it returns `1.7976931348623157e308`, the largest finite `Float`, as if
the sum had succeeded.

Adding error handling therefore changes an overflow from a caught error into a
wrong, plausible-looking number. The `Integer` and `Fixed` overloads are not
affected: an inline `TRAP` on an overflowing `Integer` or `Fixed` sum runs
`RECOVER` correctly (`/tmp/p125-ex/colsumtrap`, `/tmp/p125-ex/colsumfixed`).

**The single correct behavior a fix produces:** an overflowing `Float`
`collections::sum` behaves the same with or without an inline `TRAP`. It raises
`ErrFloatOverflow`, and an inline `TRAP` on the call receives it and runs `RECOVER`.
No path returns a finite value for an overflowed total.

Found by plan-125-C Phase 4 while verifying the `sum` man page and its review. The
page said the `Float` overload "never raises — an out-of-range total becomes
±Inf"; the review said it raises `ErrFloatOverflow`. Both were checked by probe,
and the inline-`TRAP` case turned out to do neither. **Filed, not fixed**, by user
instruction during a documentation-only plan ("file all bugs, make no fixes").

## Reproduction

```basic
IMPORT io
IMPORT collections

FUNC bigF() AS Float
  RETURN 1.0e308
END FUNC

SUB main()
  LET viaCalls AS Float = collections::sum([bigF(), bigF()]) TRAP(e)
    RECOVER -1.0
  END TRAP
  io::print("D inline TRAP, non-literal elements = " & toString(viaCalls))
  LET viaLiterals AS Float = collections::sum([1.0e308, 1.0e308]) TRAP(e2)
    RECOVER -2.0
  END TRAP
  io::print("E inline TRAP, literal elements = " & toString(viaLiterals))
  LET noTrapLiterals AS Float = collections::sum([1.0e308, 1.0e308])
  io::print("F no TRAP, literal elements = " & toString(noTrapLiterals))
END SUB
```

Observed (macos-aarch64, `worktree-P-125`, `/tmp/p125-ex/colsumflt3`):

```
D inline TRAP, non-literal elements = 179769313486231590772930519078902473361797...216.00
E inline TRAP, literal elements = 179769313486231590772930519078902473361797...216.00
Error: 7-705-0015                      <- F: without the TRAP, the same sum raises
Floating-point arithmetic overflowed to infinity.
```

Expected: D prints `-1.00` and E prints `-2.00` (each `RECOVER` runs), and F raises
`ErrFloatOverflow`, as it does now.

Controls, all behaving correctly (`/tmp/p125-ex/colsumflt2`):

- the same overflowing `Float` sum with **no** `TRAP`, called inside a function whose
  caller traps it: `ErrFloatOverflow` reaches the caller;
- plain `Float` arithmetic `v + v` in a function called under an inline `TRAP`:
  `RECOVER` runs (`-1.00`);
- printing the overflowing sum directly: raises `77050015`.

Because D uses non-literal elements, this is not constant folding of the literal
list.

## Root cause

Hypothesis, to confirm in Phase 1. `src/codegen/builtins/float_result.rs` classifies
`collections.sum` as able to return a non-finite result that is checked at an
observation boundary, rather than at the call. The inline-`TRAP` lowering
treats the call as the guarded operation, and the observation check that would
raise is either skipped or placed outside the trapped region on that path, so the
infinite value flows into the binding. Why it then reads as `DBL_MAX` rather
than `Inf` must also be explained: look for a saturating conversion on the
inline-`TRAP` result path.

Separately, earlier builds warned `TYPE_INLINE_TRAP_DEAD_HANDLER` ("the guarded
call cannot fail") for an inline `TRAP` on `sum`. That points to an infallibility
classification for this member that disagrees with its runtime behavior. Check
whether the dead-handler classification and this silent path share a cause.

## Non-goals

- Changing `Float` overflow semantics in general, or the `Integer`/`Fixed` sum paths
  (already correct).
- Making `sum` saturate on purpose: a finite result for an overflowed total is the
  defect, not a policy to adopt.
- Removing the inline `TRAP` from programs as a workaround.

## Blast-radius audit

- Other members classified in `float_result.rs` as observation-boundary non-finite
  producers: run the same inline-`TRAP` overflow probe on each (a `Float` member
  whose result overflows, with and without an inline `TRAP`).
- `collections::reduce` with a `Float` accumulator, which folds user arithmetic
  and may take a different path.
- Any member for which the compiler emits `TYPE_INLINE_TRAP_DEAD_HANDLER` but which
  can raise at run time.

## Fix

Phase 1 — RED test from the reproduction (D and E must run `RECOVER`), plus the
audit probes. Commit:

Phase 2 — make the inline-`TRAP` path observe the `Float` result inside the trapped
region, so overflow raises into the handler (GREEN); full suite and float-result
goldens. Update the `sum` man page, which is being corrected to describe the
as-is behavior. Commit:
