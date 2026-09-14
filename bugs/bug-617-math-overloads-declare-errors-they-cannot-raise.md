# bug-617: `math` Errors tables list errors on overloads that cannot raise them

Last updated: 2026-09-13
Effort: small–medium
Severity: LOW
Class: Correctness (registry error declaration) / Documentation

Status: Open
Regression Test: none yet — see Phase 1

Several `math` descriptors attach one shared error list to every overload.
Because a unary member's overloads share a helper
(`src/codegen/builtins/math/mod.rs:preserving_unary`, per the reviewer's
citation), the rendered Errors table promises errors some forms cannot raise, and
leaves out which form raises which:

| Page | Errors row as rendered | What the forms actually raise |
|---|---|---|
| `math::abs` | `ErrOverflow` on overloads 1–7 | only `List OF Integer`, `List OF Fixed`, `Integer`, `Fixed`, `Money` (the most-negative input). The `Float` forms have no error path (`gen_math.rs:lower_math_abs` clears the sign) |
| `math::acos` | `ErrFloatDomain` and `ErrInvalidArgument` on overloads 1–3 | `Float` and `List OF Float` raise `ErrFloatDomain` (`77050012`); `Fixed` raises `ErrInvalidArgument` (`77050002`) |
| `math::asin` | `ErrFloatDomain` and `ErrInvalidArgument` on overloads 1–3 | same split as `acos` |
| `math::atan` | `ErrFloatNaN` on overloads 1–3 | only the `Float` forms can; the `Fixed` lowering (`emit_fixed_atan2`) has no raise path |
| `math::exp` | `ErrOverflow` on overloads 1–3 | `Float`/`List OF Float` raise `ErrFloatInf` (`math::exp(710.0)` → `77050014`); only `Fixed` raises `ErrOverflow` (`math::exp(30.0F)` → `77050010`) |
| `math::floor` (and `ceil`, `round`, which share the lowering) | `ErrOverflow` on every overload | only `Float`/`List OF Float` (`emit_float_rounding_integer_range_check`); the `Fixed` and `Money` rounding paths always fit (`math::ceil(9223372036854775808.0)` → `77050010`, while a `Fixed` near its maximum rounds normally) |
| `math::log`, `math::log10` | `ErrFloatDomain` and `ErrInvalidArgument` on overloads 1–4 | `Float`/`List OF Float` raise `ErrFloatDomain`; `Fixed`/`List OF Fixed` raise `ErrInvalidArgument` (`/tmp/p125-ex/mathlist`) |
| `math::max`, `math::min` | `ErrInvalidArgument` on overloads 1–7 | only the three list forms, for mismatched lengths (`lower_simd_binary`); the scalar comparison (`lower_math_min_max`) has no raise path |

This is the declared-error list, which is compiler data, not prose. Per
`mfb spec language error-model` §8.6 rule 11 the compiler reads that list when it
decides fallibility, so a wrong list can also mislead that analysis. Same class as
bug-606 and bug-608.

**The single correct behavior a fix produces:** each overload declares exactly
the errors its own lowering can raise, and each page's Errors table assigns
every row to exactly those overloads.

Found by plan-125-C Phase 1's Codex page reviews of `abs`, `acos` and `atan`
(`planning/plan-125-findings/C-phase1/man-page-math-{abs,acos,atan}.md`), and
confirmed by release-binary probes. **Filed, not fixed**, by user instruction
during a documentation-only plan ("file all bugs, make no fixes"). The pages'
Description prose states the per-type split correctly.

## Reproduction

```basic
IMPORT io
IMPORT math

SUB main()
  LET x AS Fixed = toFixed(1.1)
  LET a = toString(math::asin(x)) TRAP(e)
    RECOVER "asin Fixed raised " & toString(e.code)
  END TRAP
  io::print(a)
  LET b = toString(math::asin(1.1)) TRAP(e)
    RECOVER "asin Float raised " & toString(e.code)
  END TRAP
  io::print(b)
END SUB
```

Observed: `asin Fixed raised 77050002`, `asin Float raised 77050012`, while
`mfb man math asin` lists both errors on all three overloads.
(`/tmp/p125-ex/asinfixed`, `/tmp/p125-ex/mathclaims`, `/tmp/p125-ex/mathlist`.)

Caution for the fixer: `1.1f` is a `Float` literal (`typeName(1.1f)` is `Float`),
so a probe written with it does not exercise the `Fixed` overload.

## Root cause

The `errors` vector is shared across overloads by the helpers that build each
unary member's implementations (`math/mod.rs`), instead of being set per
overload from that overload's lowering.

## Non-goals

- Changing any error a call raises.
- Editing man prose to paper over the table.

## Blast-radius audit

- Every `math` member built through the shared overload helpers: audit each
  overload's lowering against its declared list (`sqrt`, `log`, `log10`, `pow`,
  `exp` and the trig functions, as well as the four above).
- Other packages with type-split error behavior; bug-606 and bug-608 cover
  `encoding` and `udp`.

## Fix

Phase 1 — audit table per overload; RED test that `asin`'s `Fixed` overload
declares only `ErrInvalidArgument` and its `Float` overloads only
`ErrFloatDomain`. Commit:

Phase 2 — per-overload error lists (GREEN); goldens carrying error lists; full
suite. Commit:
