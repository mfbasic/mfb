# bug-751: `Float` addition does not overflow to infinity or raise `ErrFloatOverflow` — it silently produces a wrong finite value

Last updated: 2026-09-27
Effort: unknown (needs codegen investigation)
Severity: MEDIUM
Class: Correctness (silent wrong result)

Status: Open
Regression Test: none yet — see Reproduction

Summing two `Float` values whose mathematical total exceeds `DBL_MAX`
(`1.7976931348623157e308`) should, under IEEE-754 double arithmetic, produce
`+Infinity`. `collections::sum`'s `Float` overload is documented (and, per
bug-645, was believed) to raise `ErrFloatOverflow` (`77050015`) when that
happens. Neither occurs: the program below raises nothing and prints a large
*finite* decimal number instead.

## Reproduction

```basic
IMPORT io

FUNC main AS Integer
  LET x AS Float = 1.0e308
  LET y AS Float = 1.0e308
  LET total AS Float = x + y
  io::print(toString(total))
  RETURN 0
END FUNC
```

Observed (2026-09-27, macos-aarch64 release build):

```
179769313486231590772930519078902473361797697894230657273430081157732675805500963132708477322407536021120113879871393357658789768814416622492847430639474124377767893424865485276302219601246094119453082952085005768838150682342462881473913110540827237163350510684586298239947245938479716304835356329624224137216.00
```

That value's leading digits (`1797693134862315907...`) match `DBL_MAX`
(`1.7976931348623157e308`) — not `2e308`, not `+Infinity`. The same result
comes from `collections::sum([x, y])` (the `Float` overload), so this is not
specific to the `+` operator's own path or to `collections::sum`'s.

Expected: either `total` becomes `+Infinity` (and `toString` renders it as
such, e.g. `inf`), or the operation raises a trappable overflow error
(`ErrFloatOverflow`, as `collections::sum`'s own prose and bug-645 assumed).
Silently returning a large-but-wrong finite value is the one behavior that
cannot be correct under either design.

## Root cause (partial — needs Phase 1 investigation)

`collections::sum`'s `Float` arm (`src/codegen/builtins/collections/func_sum.rs`,
`lower_sum`) does a plain `float_add_d` with no check of any kind on the
result:

```rust
ParameterType::Float => {
    builder.emit(abi::load_u64(&scratch16, &scratch15, 0));
    builder.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &scratch14));
    builder.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], &scratch16));
    builder.emit(abi::float_add_d(
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::float_move_x_from_d(&scratch14, abi::FP_SCRATCH[0]));
}
```

There is no `ErrFloatOverflow` raise site in this function at all — so
bug-645's belief that this overload raises it does not match the code as it
stands now (found while fixing bug-698/bug-645; the `sum`/`ErrFloatOverflow`
mismatch does not reproduce, so bug-645 left `sum`'s descriptor unchanged).

Two live hypotheses, neither confirmed:

1. `float_add_d`'s emitted instruction (or the surrounding ABI/register
   convention on whichever backend `d`-native floats route through) does not
   actually produce IEEE-754 `+Infinity` for an out-of-range result — a
   codegen defect in the arithmetic itself, not a missing check.
2. The value doubles correctly to `Infinity` in the FP register, but
   `toString`'s Float-formatting path mishandles a non-finite value and
   renders `DBL_MAX`'s bit pattern's decimal expansion instead of `inf`/`nan`
   — a formatting defect downstream of correct arithmetic.

Phase 1 must distinguish these: read the FP result register directly (a
minimal test harness, or `io::print(toString(isNan(total)))`-style probes if
the language exposes one, or a raw bit-pattern dump) before touching either
`float_add_d`'s codegen or `toString`'s Float formatter.

## Blast-radius audit (not yet done)

- Every `Float` arithmetic operator (`+`, `-`, `*`, `/`) and every builtin that
  documents `ErrFloatOverflow` (`collections::sum`'s `Float` overload is the
  one instance found so far — grep `ErrFloatOverflow` across
  `src/codegen/builtins/` for others).
- Whether `Float` *underflow* (a result closer to zero than the smallest
  subnormal) has the same gap.
- Whether this reproduces on `linux-x86_64`/`linux-aarch64`/`windows-x86_64`,
  or is specific to the `d`-native float convention on one backend.

## Fix

Phase 1 — confirm which hypothesis is right (arithmetic vs. formatting) with a
minimal repro per hypothesis; RED test.

Phase 2 — fix the confirmed defect; if the correct design is "raise
`ErrFloatOverflow`", update every `Float`-arithmetic member's descriptor and
prose to match (this may reopen bug-645's `sum` finding); full suite.
