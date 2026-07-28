# bug-07: the minimum `Fixed` value has no writable literal

## STATUS: FIXED (test-first)

Implemented as the targeted fold (option A, zero existing-golden shift):
- `src/numeric.rs`: `fixed_raw_from_decimal` promoted here as the single source of
  truth (dedups the `src/target/shared/code/type_utils.rs` copy, which now calls it).
- `src/ir/lower.rs` (`Unary` arm): fold `Unary("-", Const{Fixed, s})` →
  `Const{Fixed, "-"+s}` *only* when the positive magnitude overflows the i64 raw
  but the negated value fits — an exact guard that fires solely at raw == 2^63
  (the min), so every in-range negated literal keeps its `Unary` shape.
- `src/target/shared/code/type_utils.rs` (`native_immediate_value`): emit the
  32.32 raw as its **u64 bit pattern** (`as u64`), because the operand encoder
  parses `u64`; identical for every non-negative raw, only reaching the min case.
- Test: `tests/rt-behavior/general/fixed-min-literal` (binds `-2147483648.0F`,
  prints it, asserts `= toFixed("-2147483648.0")`).

Boundaries still rejected at syntaxcheck (`2147483648.0F` → OVERFLOW,
`-2147483649.0F` → UNDERFLOW); `-1.5F` codegen unchanged. `cargo test` +
`scripts/test-accept.sh` green with no existing golden shift.

---


## Summary / goal

`-2147483648.0F` — the minimum representable `Fixed` — fails to compile:

```
$ mfb build …
error: Fixed constant `2147483648.0` is out of range while lowering bind m AS Fixed
```

The single correct behavior a fix produces: `-2147483648.0F` (and any negated
`Fixed` literal whose *negated* value is in range but whose positive magnitude is
not) compiles and evaluates to the minimum `Fixed`, exactly as
`toFixed("-2147483648.0")` already does. This mirrors `Integer`, where
`-9223372036854775808` (i64::MIN) already compiles while the positive
`9223372036854775808` is correctly rejected.

## Failing reproduction

```
FUNC main AS Integer
  LET m AS Fixed = -2147483648.0F
  RETURN 0
END FUNC
```
→ `error: Fixed constant \`2147483648.0\` is out of range while lowering bind m AS Fixed`

Contrast (all compile today):
- `LET i AS Integer = -9223372036854775808`  ✓  (Integer folds the sign)
- `LET f AS Float = -1.7976931348623157e308` ✓  (Float is symmetric)
- `LET m AS Fixed = toFixed("-2147483648.0")` ✓  (string path folds the sign)
- `LET n AS Fixed = -1.5F`                    ✓  (ordinary negated literal, in range)

A committed fixture already depends on the string workaround:
`tests/rt-error/math/func_math_abs_fixedarray_rt` builds the min via
`toFixed("-2147483648.0")` because `-2147483648.0F` will not compile.

## Root cause

`Fixed` is a 32.32 fixed-point value stored as an `i64` raw (`SCALE = 1<<32`), so
its range is asymmetric: min `-2147483648.0` == raw `i64::MIN`, but the positive
`2147483648.0` == raw `2^63`, which overflows `i64`.

`-2147483648.0F` is represented as `Unary("-", Const{Fixed, "2147483648.0"})` — the
negation is a **separate node** over a **positive** constant. `Fixed` constants are
lowered by `fixed_raw_from_decimal` (`src/target/shared/code/type_utils.rs:291`,
duplicated at `src/binary_repr/writer.rs:~870` and called from the constant-pool
emitter `src/binary_repr/sections.rs:375`). That function *does* handle a leading
`-` correctly (it strips the sign, computes the magnitude, then re-applies the sign
and does `i64::try_from(raw)` — `type_utils.rs:296-343`), but it is only ever
handed the **positive** magnitude string `"2147483648.0"`, so `raw = 2^63`
overflows before the negation is ever applied.

`Integer` does not have this gap: the sign is folded during the front-end range
check (`src/syntaxcheck/helpers.rs:29,249,261` — the `operator == "-"` arms of
`integer_constant_value` / `integer_literal_in_range` admit `i64::MIN`), so the
minimum survives to lowering as an already-signed constant. `Fixed` literals have
no such range check in `syntaxcheck` at all; their only range gate is
`fixed_raw_from_decimal` at lowering, which never sees the sign.

## Non-goals (must NOT change)

- `Fixed` value semantics, the 32.32 raw layout, or the constant-pool binary
  format.
- Codegen for any literal that already compiles — in particular ordinary negated
  literals like `-1.5F` must keep their current lowering/goldens. The fix should
  only enable the currently-rejected boundary case, so **no existing golden shifts**.
- The positive-overflow rejection: `2147483648.0F` (no minus) must still error.

## Phased fix (test-first)

1. **Test first.** Add a runtime-behavior fixture (e.g.
   `tests/rt-behavior/general/fixed-min-literal/`) that binds `-2147483648.0F`,
   prints `toString` of it, and asserts it equals `toFixed("-2147483648.0")` /
   the expected minimum. It fails today (compile error).
2. **Fold the sign for the boundary.** When lowering `Unary("-", Const{Fixed, s})`,
   fold it to `Const{Fixed, "-" + s}` so the constant-pool emitter calls
   `fixed_raw_from_decimal("-2147483648.0")` (which succeeds) instead of the
   positive magnitude. Scope the fold so it does not perturb in-range literals'
   codegen — either fold only when the positive magnitude overflows but the
   negated string is in range, or (cleaner, if goldens permit) fold all
   `Unary("-", Fixed/Float Const)` and regenerate the affected goldens as an
   explicit, audited step. Decide by measuring the golden delta.
3. **Verify.** New test passes; `scripts/test-accept.sh` stays green (with a
   documented golden-regeneration step if step 2 chooses the broad fold);
   `2147483648.0F` (positive) still errors.

## Cross-links

Found while modernizing `toFixed("…")` test calls to `…F` literals (the reorg /
literal-suffix cleanup). Memory: [[tests-reorg-4-folders]].
