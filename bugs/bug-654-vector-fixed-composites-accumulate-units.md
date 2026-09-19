# bug-654: `vector::slerp` / `vector::angle` on a `Fixed` vector accumulate several Q32.32 units

Last updated: 2026-09-19
Effort: small–medium
Severity: LOW
Class: Correctness (accuracy, not a wrong answer)

Status: Open
Regression Test: none yet — see Phase 1

`Fixed` `sin`, `cos`, `tan` (bug-615), the inverse family (bug-615-C) and the
`Float` trig kernels (bug-618) are now within **one** Q32.32 unit (2^-32) of the
true value. The `vector::` members built on them are not: their intermediate
chain — normalize, dot, `acos`, `sin`, two divides, two multiply-adds — rounds to
Q32.32 at every step, and the error compounds.

Measured on the merged bug-615/618 tree against an exact reference (the same
oracle `tests/runtime/rt_math_fixed_trig_accuracy.rs` uses, Python-side for the
composite):

| Call | Before (units) | After (units) |
|---|---|---|
| `slerp(Fixed4[1,2,2,4], Fixed4[2,1,0,1], 0.5).w` | 24.3 | **4.3** |
| `slerp(Fixed3[1,2,2], Fixed3[2,1,2], 0.5).z` | 18.0 | **6.0** |
| `slerp(Fixed4…).x` | 14.6 | **2.6** |
| `slerp(Fixed3…).x` | 13.5 | **4.5** |
| `slerp(Fixed2[3,4], Fixed2[1,2], 0.5).x` | 2.9 | **2.9** |
| `angle(Fixed2[3,4], Fixed2[1,2])` | 3.3 | **2.3** |
| `angle(Fixed3[1,2,2], Fixed3[2,1,2])` | 2.0 | **1.0** |
| `angle(Fixed4…)` | 0.52 | **1.52** |

Every row improved except the last, which moved by one unit inside the same band:
these numbers are dominated by the composite's own rounding, so a more accurate
`acos` shifts them without controlling them.

**The single correct behavior a fix produces:** a `Fixed` `vector::` member whose
inputs are exact is within one Q32.32 unit of the true result, like the scalar
kernels it calls — or, if that is not reachable for a given member, a documented
bound that is actually measured.

Found while landing bug-615/bug-618 (the acceptance expectations in
`tests/acceptance/src/vector.mfb` had to be re-derived, which is what exposed the
spread).

## Reproduction

```basic
IMPORT io
IMPORT vector

SUB main()
  LET v AS vector::Fixed4 = vector::slerp(vector::Fixed4[toFixed(1.0), toFixed(2.0), toFixed(2.0), toFixed(4.0)], vector::Fixed4[toFixed(2.0), toFixed(1.0), toFixed(0.0), toFixed(1.0)], 0.5)
  io::print(toString(v.w, toByte(32)))
END SUB
```

Observed (macos-aarch64, merged bug-615/618 tree): `2.74974655942060053348541259765625`
(raw 11810686616). True: `2.7497465604323299…` = raw 11810686620.345, so the
result is 4.3 units low. Expected: within one unit (raw 11810686620).

## Root cause

Hypothesis, to confirm in Phase 1. The generated `Fixed` bodies
(`src/codegen/builtins/vector/func_slerp.rs:BODY_FIXED2..4`,
`func_angle.rs`) mirror the `Float` bodies statement for statement, so every
intermediate — `omega`, `sin(omega)`, `w0`, `w1`, each product — is a `Fixed`
rounded to 2^-32. `w0`/`w1` are quotients by `sin(omega)`, which is small for a
small angle, so an intermediate unit becomes several in the result.

## Non-goals

- Changing the `Float` bodies, which are ≤1 ULP.
- Changing the scalar `Fixed` kernels — bug-615 and bug-615-C already hold them
  to one unit, and they are not the source of this spread.

## Blast-radius audit

- Every `Fixed` member that divides by or multiplies a trig result:
  `slerp` (2/3/4), `angle` (2/3/4), `rotate_2d`; audit `normalize`, `project`,
  `reflect`, `reject`, `lerp` for the same compounding.
- The `Integer` forms round the same values to whole numbers, so they are
  insensitive; confirm.

## Fix

Phase 1 — RED test: a `Fixed` composite corpus against the exact oracle, with the
one-unit bound; confirm the chain is the source (instrument one call). Commit:

Phase 2 — carry the chain in higher precision (the scalar kernels' Q1.63 / Q3.61
route is the precedent: compute `omega`, `w0`, `w1` and the final combination
without rounding to Q32.32 in between), or document a measured bound per member.
Goldens + full suite. Commit:
