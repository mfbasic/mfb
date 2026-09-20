# bug-654: `vector::slerp` / `vector::angle` on a `Fixed` vector accumulate several Q32.32 units

Last updated: 2026-09-19
Effort: small–medium
Severity: LOW
Class: Correctness (accuracy, not a wrong answer)

Status: **FIXED (in two rounds — see "Round 2", which is why)** — the `Fixed` chains of
`slerp`, `angle`, `project`, `reject` and `clamp_length` now carry their intermediates in
`Float` and round to Q32.32 exactly once, at the end. Every `Fixed` `vector::` member is
within one unit with exact inputs: 0 of 34 measurements over, worst 0.800.
Regression Test: the re-derived `Fixed` expectations in `tests/acceptance/src/vector.mfb`
(the `angle`, `slerp`, `project`, `reject` and `clamp_length` TCASEs), which pin the
correctly-rounded value for all 28 components.

**Round 1 closed this bug with three members still out of spec.** If you are reading this
to learn what was done, read "Round 2" before trusting the round-1 sections below: their
"Left undone, deliberately" verdict on `project`/`reject` was wrong, and their blast-radius
audit never measured `clamp_length` at all.

## Outcome

**Every measured component is within 0.481 units — inside the one-unit target, and each is
the correctly rounded `Fixed` (|error| < 0.5).** Measured against an independent 80-digit
`Decimal` oracle that reproduces this document's own "After" column to within display
rounding, which is what validates it:

| Call | Before (units) | After (units) |
|---|---|---|
| `slerp(Fixed4…).w` | −4.345 | **−0.345** |
| `slerp(Fixed3…).z` | −5.968 | **+0.032** |
| `slerp(Fixed3…).x` | −4.476 | **−0.476** |
| `slerp(Fixed2…).y` | −4.422 | **−0.422** |
| `slerp(Fixed2…).x` | −2.948 | **+0.052** |
| `slerp(Fixed4…).x` | −2.607 | **+0.393** |
| `slerp(Fixed4…).z` | −1.738 | **+0.262** |
| `angle(Fixed2…)` | +2.320 | **+0.320** |
| `angle(Fixed4…)` | +1.519 | **−0.481** |
| `angle(Fixed3…)` | +0.962 | **−0.038** |

The document's own table is confirmed (its 4.3 / 6.0 / 2.6 / 4.5 / 2.9 / 2.3 / 1.0 / 1.52
match the measured −4.345 / −5.968 / −2.607 / −4.476 / −2.948 / +2.320 / +0.962 / +1.519).

**Correction to this document:** the raw integers in the Reproduction are mistyped. It
gives `raw 11810686616` observed and `11810686620.345` true for `slerp(Fixed4…).w`; the
correct raws are **11810071545** and **11810071549.345**. The decimal values and the
4.3-unit deficit quoted alongside them are right — only the integers are wrong
(`11810686620 / 2^32` is 2.74989…, not the 2.7497465… the same line quotes).

## Blast-radius audit — verdicts

Measured with exact inputs, against the same oracle:

| Member | Worst deviation | Verdict |
|---|---|---|
| `slerp` 2/3/4 `Fixed` | 1.7–6.0 units | **the defect** — fixed |
| `angle` 2/3/4 `Fixed` | up to 2.3 units | **the defect** — fixed |
| `project`, `reject` | 1.111 → **0.444** | over the bar, and NOT from trig: `(dot/bb) * b` rounds the quotient to Q32.32 before multiplying. **Fixed in round 2** (see below) — round 1 wrongly left this as a documented bound. |
| `clamp_length` | 1.600 → **0.400** | **MISSED ENTIRELY BY ROUND 1's AUDIT**, and the worst member of the set. Same shape: `maxLen/len` rounded before multiplying. Fixed in round 2. |
| `reflect` | **0.000 with exact inputs** | **NOT affected.** An early measurement put it at 7.2 units, but that fed it `normalize(b)` — an already-inexact normal — and `reflect` multiplies that error by `2·dot`. With an exactly representable normal (`(0,0,1)`, `(0.5,0.5,0.5)`) it is exact. The error is inherited from its argument, which is the caller's business, not this bug's. |
| `normalize` | ≤0.667 units | within one unit — not affected |
| `rotate_2d` | ≤0.799 units | within one unit — not affected |
| `length`, `lerp` | 0.000 | exact — not affected |
| `distance` | 0.048 units | within one unit — not affected |
| `Integer` forms | — | insensitive as predicted: they round the same values to whole numbers |

## Fix

The `Fixed` bodies of `__vector_slerp_fixed{2,3,4}` and `__vector_angle_fixed{2,3,4}` now
compute the whole chain — squared lengths, the dot, `cosv`, `omega`, `sin(omega)`, `w0`,
`w1` and the final combination — in `Float`, converting to `Fixed` exactly once per
returned component. `Float`'s 53-bit mantissa carries ~51 fractional bits for these
magnitudes against Q32.32's 32, so the single final rounding is the only one that costs
anything, which is why every result is now correctly rounded.

The dot products are inlined in `Float` rather than calling `#vector_dot_fixedN`: routing
through the `Fixed` dot would round early and is also what overflowed (see below).

`slerp` computes its own `omega` inline instead of calling `__vector_angle_fixedN`, so the
angle is not rounded to `Fixed` and back in the middle of the chain.

### Behavior changes, disclosed

1. **A spurious `ErrOverflow` is gone.** `vector::slerp` on a `Fixed4` of 1e8-magnitude
   components used to raise `ErrOverflow` — the all-`Fixed` chain squared each component
   (1e16) far past the `Fixed` ±2.1e9 range, so any vector longer than ~46341 failed. It
   now returns `274974656.04323304`. This is a fix, not a regression, but it converts a
   raise into a success and is recorded here rather than left to be discovered.
2. **Edge cases verified unchanged**: `angle` and `slerp` still raise `77050002` on a
   zero-length input (the `FAIL` the inlined `angle` used to raise is preserved verbatim —
   an early draft wrongly fell back to `lerp_unclamped` there), and the near-parallel
   `sin(omega) ≈ 0` fallback to `lerp_unclamped` still fires.

### Re-derived expectations — proof, not re-baselining

The 12 `Fixed` components in `tests/acceptance/src/vector.mfb` were re-derived. The
four-question bar:

1. **When/why written:** `31d40c93a`, "tests(bug-615): re-derive the Fixed trig
   expectations the new kernels moved" — themselves measured against a 336-bit oracle.
2. **Behavior protected:** the correctly-rounded `Fixed` result of each composite.
3. **Who else depends:** nobody. `grep -rn '0.179853500332683325' tests/ src/ planning/ bugs/`
   → one hit, the line itself.
4. **Proof wrong:** bug-615's own commit message records these residuals as "the `Fixed`
   composites' own intermediate rounding, not the kernels; **filed as bug-654**" — the
   author documented them as encoding this very defect. Independently, the oracle puts them
   2.6–6.0 units from the true value while the new ones are ≤0.481.

Only the ten disproved literals changed; no other line of the file moved. Acceptance:
782 pass / 0 fail.

### Goldens

18 goldens moved in total, in two passes — the second pass is worth recording, because the
first filter was wrong in a way the acceptance harness alone did not catch:

* **Pass 1 (12 `.ir`)**, found by `test-accept.sh 'vector*' '*vector*'`: the IR dump embeds
  the injected bodies. One diff was inspected in full to localize — it contains only the
  rewritten `angle`/`slerp` bodies.
* **Pass 2 (6 more)**, found by the full `artifact_gate_all` on the first suite run. The
  `vector*` filter missed the five cross-target
  `byte-identity/vector/vector_codegen_cover_rt.<target>.ncodesum` hashes (the injected
  bodies changed, so the generated MACHINE CODE changed — and those are written by
  `regen-native-goldens.sh`, the gate's write half, not by `sync-goldens.sh`), plus
  `rt-behavior/arithmetic/float-call-boundary-finite-rt`, which embeds the vector bodies
  but whose path does not contain "vector".

**No `.run` and no `build.log` moved in either pass**, i.e. no program's observable output
changed anywhere in the tree.

## STATUS (round 1 — superseded by Round 2 below)

**Validation.** `cargo test --no-fail-fast` on the merged tree: **exit 0**, 199 result
blocks, 0 failures (4272 tests in the main binary). `artifact-gate.sh vector`: 7 goldens
checked, 0 diffs. NOTE: this ran the gate with a `vector` selector only; round 2 runs the
FULL `artifact-gate.sh all`. `mfb test tests/acceptance`: 782 pass / 0 fail. Main was merged in before
the final run (it had advanced with bug-616 and another session's plan-139 work).

## Round 2 — the first pass closed this bug early, and should not have

**This document was marked FIXED after round 1 with `project`/`reject` left at 1.111 units
and recorded as a "documented bound". That was a misreading of its own acceptance
criterion, and the close was wrong.** The criterion is not scoped to `slerp`/`angle`:

> a `Fixed` `vector::` member whose inputs are exact is within one Q32.32 unit of the true
> result … **or, if that is not reachable for a given member**, a documented bound that is
> actually measured.

`project` and `reject` are `Fixed` `vector::` members measured over the bar with exact
inputs, and one unit **was** reachable for them — round 1 had already identified how
(divide last rather than rounding the quotient first) and declined to do it. The
documented-bound alternative is only sanctioned when the bound is unreachable, so it did
not apply. Recorded here rather than quietly amended, because "measured it, wrote the
number down, closed the bug" is the failure mode this note exists to prevent.

### What a complete audit found

Round 1 probed a handful of members. Round 2 swept **every** `Fixed` `vector::` member with
exact inputs — 34 measurements — and found **8 over one unit across three members**,
including one round 1 never measured at all:

| Member | Worst (before) | Note |
|---|---|---|
| `clamp_length` | **1.600** | **never measured in round 1** — the worst of the set |
| `project` | 1.111 | |
| `reject` | 1.111 | inherits `project`'s error wholesale |

All three share one shape, and it is the same shape in different clothes: a **ratio rounded
to Q32.32 and then multiplied**, so the quotient's rounding is scaled up by every
component. `project` computes `dot(a,b)/dot(b,b)` first; `clamp_length` computes
`maxLen/len` first.

Fixed the same way `slerp`/`angle` were: carry the chain in `Float`, convert to `Fixed`
exactly once per returned component.

### After

**0 of 34 measurements over one unit; worst 0.800.** The members this change does not touch
were measured and confirmed in spec rather than assumed: `normalize` ≤0.800, `rotate_2d`
0.799, `distance` 0.096, `length`/`dot`/`lerp`/`scale` exact, `reflect` exact with exact
inputs.

Edge cases verified unchanged: `project`/`reject` still raise `77050002` on a zero-length
`b`; `clamp_length` still raises on a negative max, still returns `v` unchanged under and
at the cap, and still handles the zero vector without dividing by zero.

### Re-derived expectations, and a mistake worth recording

16 acceptance expectations across the three members' `Fixed` TCASEs. Provenance is
`b5cb72b51`, a mechanical literal-modernisation rather than an oracle derivation; sole
dependent; the old values are 1.111–1.600 units from truth against the 80-digit oracle
while the new ones are ≤0.444.

**They were edited BY LINE NUMBER after a first attempt by string replacement silently
corrupted `normalize`'s own correct expectation** — the same literal
(`0.666666666511446237F`) appears in members this change does not touch. The acceptance run
caught it. Anyone re-deriving expectations in this file should target lines, not literals.

### Round 2 validation

`cargo test --no-fail-fast`: **exit 0**, 199 result blocks, 0 failures (4274 tests in the
main binary). Full `artifact-gate.sh all`: **2058 goldens checked, 0 diffs**.
`mfb test tests/acceptance`: 782 pass / 0 fail. No `.run` and no `build.log` moved.
Commit: `2e80e12e6`

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
