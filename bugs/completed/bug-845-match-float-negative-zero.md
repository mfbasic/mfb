# bug-845: MATCH on a Float compares bit patterns, so `CASE 0.0` misses `-0.0` although `-0.0 = 0.0` is TRUE

Last updated: 2026-10-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: tests/rt-behavior/control-flow/match-float-signed-zero

## Cause (coordinator, bug-staging-24)

- Rule: for values of the same type zero and negative zero are equal everywhere: `MATCH CASE 0.0` matches `-0.0`, and record/union/collection-element `=`/`<>` treat Float fields `-0.0` and `0.0` as equal (owner decision 2026-10-05); a literal arm matches exactly when scrutinee `=` pattern.
- Where: `src/codegen/memory/value/builder_value_semantics.rs:lower_match_compare` fallback compares Float bit patterns (`compare_registers` + `branch_eq`); `emit_comparable_values_match_branch` and the other bit-pattern equality paths do the same for Float fields.
- Fix: Float equality is decided by the Float compare (as scalar `=` is) in the one equality emitter every path uses: MATCH arms and structural equality route Float through it; no bit compare of a Float remains.

**Owner decision (2026-10-05):** for values of the same type, zero and negative zero are equal everywhere: `MATCH`
`CASE 0.0` matches `-0.0`, and record (and union/collection element) `=`/`<>` treats a
Float field holding `-0.0` as equal to one holding `0.0`, consistent with scalar `-0.0 = 0.0`
being TRUE. `emit_comparable_values_match_branch` and every other bit-pattern comparison
path for Float get the fix. (Integer/Fixed/Money have a single zero already.)

A `MATCH` whose scrutinee is `Float` selects a literal arm by comparing the raw
64-bit registers. `-0.0` and `0.0` have different bits, so `CASE 0.0` does not
match a negative zero (e.g. the result of `0.0 * -1.0`), and control falls to
`CASE ELSE`, while the `=` operator on the same values returns `TRUE`. Silent
wrong-branch selection; nothing reports it.

**Correct behavior:** a literal arm matches exactly when the scrutinee `=` the
pattern in the scrutinee's type, so `CASE 0.0` matches both zeros.

References:

- `src/docs/spec/language/09_pattern-matching.md`: "The arm compares values of the
  scrutinee's type." `MATCH` replaces `SELECT CASE` value dispatch.
- Precedent: bug-140 (`bugs/completed/bug-140-string-match-pointer-compare.md`) fixed
  the same register-compare shortcut for String scrutinees.

## Failing Reproduction

```
IMPORT io
FUNC fl(f AS Float) AS String
  MATCH f
    CASE 0.0 : RETURN "zero"
    CASE ELSE : RETURN "other"
  END MATCH
END FUNC
FUNC main() AS Nothing
  LET z = -0.0
  LET y = 0.0 * -1.0
  io::print(toString(z = 0.0) & " " & toString(y = 0.0))
  io::print(fl(z) & " " & fl(y) & " " & fl(0.0))
END FUNC
```
`/tmp/hardening/mfb build <proj> && <proj>/build/<name>.out`

- Observed:
  ```
  TRUE TRUE
  other other zero
  ```
- Expected:
  ```
  TRUE TRUE
  zero zero zero
  ```

The same applies to a comma list arm (`CASE 0.0, 1.0`) — same compare routine.
Contrast: Fixed/Money/Integer/Byte literal arms (two's-complement carriers, one
zero) and String/record arms (byte-compare helper) are correct.

## Root Cause

`src/codegen/memory/value/builder_value_semantics.rs:lower_match_compare`, fallback
arm: only `String` and record scrutinees route through
`emit_comparable_values_match_branch`; every other type uses
`abi::compare_registers` + `branch_eq` — an integer compare of the Float bit
patterns. IEEE equality (`-0.0 == 0.0`) differs from bit equality only for
signed zero (NaN cannot be written as a literal pattern).

## Goal

- Float literal arms use the same Float equality as the `=` operator; repro
  prints `zero zero zero`.

### Non-goals (must NOT change)

- Integer/Byte/Fixed/Money/enum/union-tag dispatch; String content compare.

## Blast Radius

- `lower_match_compare` `Value` and `OneOf` patterns over `Float` — fixed by this.
- Record `=` with a Float field — OBSERVED same divergence:
  `TYPE P / x AS Float`, `LET z = 0.0 * -1.0`, `toString(P[z] = P[0.0])` prints
  `FALSE`, although `z = 0.0` is `TRUE`. `./mfb spec language types` §4.1 table:
  "`=` / `<>` | IEEE numeric equality after operand conversion | equal". Whether the
  `=` row covers Float *fields* of a record is not stated (UNCLEAR, owner decision);
  if it does, `emit_comparable_values_match_branch` (also used for record MATCH)
  needs the same fix.
- `collections::contains([z], 0.0)` → `FALSE` — documented bitwise in §4.1's table;
  not a bug.

## Fix Design

Route `ParameterType::Float` through the Float compare used for `=` (fcmp on FP
registers) in `lower_match_compare`.

## Validation Plan

- rt-behavior fixture with the repro (both zeros, list arm).
- Full suite.

## Summary

One-branch fix in native match dispatch; same family as bug-140.

## Fix (bs24-agent-f, 2026-10-18)

**One equality emitter, two relations.** `emit_comparable_values_match_branch` is
the VALUE equality every `=`/`<>` on records and every `MATCH` literal or record arm
uses; it now decides a `Float`, at any field depth, with the Float compare (`fmov` to
`d` registers, `fcmp`, `b.eq`), as scalar `=` does. `lower_match_compare` routes a
`Float` scrutinee through it (it used to `compare_registers` the bits). The
collection-payload matchers (`contains`, set/map key matching), which the spec
documents as bitwise for `Float`, call the new `emit_comparable_keys_match_branch`:
the same walk with `FloatEquality::Bits`, so a record element's `Float` fields still
match the key bytes a map or set hashes. No `Float` value-equality bit compare is
left in that emitter. (`emit_values_content_equal_branch`, used only by the canvas
present's "did the scene change" test, keeps its bitwise word compare; its doc
comment now says it answers "same content", not `=`.)

**Second defect found on the same path.** A `MATCH` scrutinee that lives in a Float
register (`MATCH q + 1.25`, or a local the optimizer keeps `d`-native) was spilled
with `store_u64` of the `d` register, which stored another register's bits:
`MATCH z CASE 1.5` with `z = 1.5` took `CASE ELSE` on main too (measured with
`/tmp/main-dbg-target/release/mfb`). The Match arm now spills with
`store_value_at` (`str d` for a `d`-native Float), and `lower_match_compare` puts
the pattern's bits in a GPR (`materialize_float`) before the compare stores them.

**RED → GREEN.** `bash scripts/test-accept.sh <mfb> <out>
'rt-behavior/control-flow/match-float-signed-zero'`: main's compiler prints
`other / other / zero / other`, `other / one or zero / other`, `other / other`,
`FALSE TRUE FALSE`, `FALSE FALSE`, `other` (mismatch); this branch matches the
golden (`zero / zero / zero / other` … `one and a half`). `contains([P[-0.0]],
P[0.0])` and `contains([-0.0], 0.0)` stay `FALSE` (bitwise, as documented).

**Spec/man.** `mfb spec language types` §4.1 table gains the record/`MATCH` row and
the key-matching note; `mfb spec language pattern-matching` states the arm's IEEE
equality; `mfb man types comparisons` says the same for record fields and arms.

**Goldens expected to move.** None beyond the new fixture: every
`tests/byte-identity/*` macOS `.ncode` hashes to its committed `.ncodesum` with this
compiler (no builtin emits a Float `MATCH` or a Float-field record `=`). Fixtures
with a Float literal arm (`grep -rlE 'CASE -?[0-9]+\.[0-9]+' tests`) carry only
`.ast`/`.ir`/`.run` goldens, which this change does not touch. The lowering is
target-independent, so a byte-identity fixture whose macOS hash is unchanged emits no
moved code on the other four targets either (guess: not measured per target).
