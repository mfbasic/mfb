# bug-644: an inline `TRAP` on a `Float` `collections::sum` turns an overflow into a silent `1.797e308` result

Last updated: 2026-09-27
Effort: medium
Severity: HIGH
Class: Correctness (silent wrong value; error handler bypassed)

Status: Fixed
Regression Test: `tests/rt-behavior/trap/bug644-inline-trap-float-sum-overflow-rt`;
unit tests `float_result::tests::every_unconstrained_member_is_observed_under_an_inline_trap`,
`builtins::tests::no_member_declaring_an_error_is_infallible`

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

## Root cause (confirmed 2026-09-27)

Reproduced on macos-aarch64 at `a0082e4ae` exactly as described (D and E print the
long number, F raises `7-705-0015`, and both trapped lines warn
`TYPE_INLINE_TRAP_DEAD_HANDLER`). Two causes, one per symptom:

1. **The handler never ran.** The desugar binds the trapped call as
   `$trap_res0 : Result OF Float = callResult collections.sum(...)` (`--nir`), and
   codegen lowers it through `lower_inline_builtin_raw`
   (`src/codegen/engine/value/builder_values.rs`) because `sum` declares
   `ErrOverflow`. The `Float` arm folds with a bare `float_add_d` and raises nothing,
   so the raw capture tagged `+Inf` as `Ok`. bug-590's re-check lives at the
   observation boundary (`observe_float` → `float_arith_node`), and the binding the
   user sees reads `ResultValue($trap_res0)` through `$trap_val1`, which is not a
   producer node, so no boundary ever looked at it. Without a `TRAP` the binding's
   value is the `Call` itself, which IS observed — hence F.
2. **The dead-handler warning.** `inline_builtin_is_infallible`
   (`src/codegen/builtins/mod.rs`) still named `sum` in its hand-written infallible
   list, written before plan-96 gave the member its `ErrOverflow` check. It was wrong
   for every overload: the `Integer` sum's handler runs (measured `-1`) under the same
   warning. Codegen never consulted that verdict for `sum` (the raw-supported test
   comes first), so the warning was the only effect; the two causes do not share code.

Why the value "reads as `DBL_MAX`": it does not. It is `+Inf` — `v * 0.5` on it
raises `7-705-0015` at base — and `toString` renders an infinity with `f64::MAX`'s
digits. A `Float` reaching `toString` non-finite is itself the invariant breach.

Audit:
- `FLOAT_RESULTS` has exactly one `Unconstrained` row, `collections::sum`; the new
  unit test requires every such row to take the observed raw path.
- `collections::reduce` with a `Float` accumulator under an inline `TRAP` already ran
  its handler (`77050015`): the callback's `RETURN` observes it.
- The other names on the infallible list are pinned against the registry by
  `no_member_declaring_an_error_is_infallible`; none declares an error.
- Behaviour change to note: inside a trapped expression the total is observed at the
  call even as an operand (`1.0 / collections::sum(xs) TRAP`, which base rejected as
  `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`), where the untrapped expression can recover to
  `0.0`. Recorded in spec §8.

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

- [x] Phase 1 — RED test from the reproduction (D and E must run `RECOVER`), plus the
audit probes. Commit: see Phase 2 (one commit; the fixture is RED at `a0082e4ae`).

- [x] Phase 2 — `lower_inline_builtin_raw` observes an `Unconstrained` member's
`Float` result inside the raw capture (`observe_float_call_result`,
`src/codegen/builtins/math/gen_math.rs`), and `sum` leaves the infallible list. The
`sum` man page and spec §8 updated; the declared-error list is bug-645's. Commit:
54605e3b2

STATUS: FIXED (54605e3b2). Deviation: the value was +Inf, not DBL_MAX; the dead-handler warning was a separate stale infallible-list entry, fixed too. Gates on the merged tip (82926917c + e703c61e3): `cargo test --release --no-fail-fast` 251 binaries, 6311 passed, 0 failed; `scripts/artifact-gate.sh target/release/mfb all` 0 diffs over 2176 goldens.
