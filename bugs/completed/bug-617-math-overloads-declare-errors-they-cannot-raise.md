# bug-617: `math` Errors tables list errors on overloads that cannot raise them

Last updated: 2026-09-13
Effort: small–medium
Severity: LOW
Class: Correctness (registry error declaration) / Documentation

Status: **FIXED (`0d9366a17`, `271a978f6`)** — landed on main 2026-09-19.
Regression Test:
`codegen::builtins::math::tests::each_overload_declares_only_the_errors_its_own_lowering_can_raise`
(a 28-row table pinning every corrected member/error/overload triple) and
`codegen::builtins::math::tests::no_member_lost_its_fallibility_when_the_declarations_were_narrowed`.

## STATUS: FIXED (`0d9366a17`, `271a978f6`)

Every man Errors table now matches the measured per-overload behavior. `math::asin` —
the document's own reproduction — renders `ErrInvalidArgument` on overload 3 alone and
`ErrFloatDomain` on 1 and 2, against the measured `Fixed`→`77050002`,
`Float`/`List OF Float`→`77050012`.

**Validation.** `cargo test --no-fail-fast`: **exit 0**, 199 result blocks, 0 failures
(4274 tests in the main binary). `artifact_gate_all`: 2058 goldens checked, **0 diffs** —
the declarations are registry data, so no codegen golden moves. The rendered-manual diff
is confined entirely to `math`'s own pages (hunks 13488–15118, inside math's 13277–15163).
No runtime behaviour changed and no new diagnostic appeared: the reproduction still prints
`asin Fixed raised 77050002` / `asin Float raised 77050012` and builds warning-free.

Main was merged in before the final run (it had advanced with bug-616, bug-654 and another
session's plan-139 work).

**A first run reported exit 101 and was NOT accepted.** It surfaced two things, both
handled: a genuine dead-code warning (`preserving_unary` had no callers once every member
moved to the typed form — removed in `271a978f6`), and one failure of
`decode_time_is_linear_in_output_size`, a wall-clock RATIO assertion in
`tests/runtime/rt_compress_bounds.rs` (4.74x against a 4.4x limit). That is a
load-sensitive flake, self-inflicted by running three cargo suites concurrently: it passes
in isolation, `compress` decode reaches no `math` member, and it passed in the clean
serialised run above. Recorded rather than filed, because the contention was ours.

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
| `math::sqrt` | `ErrFloatDomain` and `ErrInvalidArgument` on overloads 1–4 | `Float`/`List OF Float` raise `ErrFloatDomain` (`sqrt([4.0, -1.0])` → `77050012`); `Fixed`/`List OF Fixed` raise `ErrInvalidArgument` (`[4, -1]` as `Fixed` → `77050002`) (`/tmp/p125-ex/mathc1d`) |
| `math::tan` | `ErrInvalidArgument` and `ErrFloatInf` on overloads 1–3 | the `Float` forms use `FloatKernel::Tan`, whose only error is `ErrFloatNaN` (`builder_simd_float_math.rs:FloatKernel::errors`); `tan(math::pi2)` returns a finite 1.6e16 |

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

**Completed 2026-09-19.** Every `math` member built through a shared overload helper was
probed per overload against the release compiler at `be60f76eb`, and every verdict was
cross-checked against the lowering's raise sites and against the 34 in-tree
`tests/rt-error/math/` fixtures (each of which proves one `(member, type, error)` pairing
reachable). The document's ten-row table is **confirmed correct in every row**. Five
findings extend it:

1. **`pow` and `atan2` have the same defect and were NOT in the table** — though `pow` is
   named in this section. `pow` declared all four of `ErrFloatInf`/`ErrFloatNaN`/
   `ErrInvalidArgument`/`ErrOverflow` on all three overloads; each form raises a strict
   subset (the `Float` path has no `ErrOverflow` at all; the `Fixed` path never leaves
   Q32.32 so it raises no float-class error). `atan2` declared `ErrFloatNaN` and
   `ErrInvalidArgument` on all three, while `emit_fixed_atan2` contains **zero** raise
   sites and the scalar forms have no length to mismatch.
2. **`tan` was over-declared in a way the table understates.** `ErrFloatInf` and
   `ErrInvalidArgument` are raisable by *no* overload — they are absent from
   `FloatKernel::Tan::errors()` (which is `[Nan]` alone) and from `emit_fixed_tan` (whose
   single raise is `ErrOverflow`). Both came off entirely rather than being re-scoped.
3. **`sin` and `cos` needed the same `Float`/`Fixed` split** (not in the table).
   `money/gen_fixed_math.rs` has exactly five raise sites in the whole file — tan-overflow,
   asin/acos-domain, scale-by-power-of-two, log-domain, pow — so the `Fixed` forms of
   `sin`, `cos`, `atan` and `atan2` are **total**.
4. **`clamp`'s uniform declaration was already correct** and is unchanged: every form
   guards `low > high` with the same bare `ErrInvalidArgument`.
5. **`rand` is fallible and untouched** — both overloads guard `min <= max`.

Other packages with type-split error behavior (bug-606 `encoding`, bug-608 `udp`) were
**not** in scope and are not touched.

## Correction to this document's premise

The introduction says a wrong list "can also mislead that analysis", citing
`mfb spec language error-model` §8.6 rule 11. That is true of the *member-level* list but
**not of the per-overload split this bug corrects**: `registry::native_member_declares_error`
is an `any` over a member's implementations, so narrowing which overloads declare an error
cannot change a member's fallibility as long as one overload still declares it. Verified:
every affected member keeps at least one declaring overload, so no fallibility verdict
moved and no `TRAP` handler changed status. `atan2` is the closest call — its `Fixed`
overload now declares nothing, and only the two `Float` forms hold the member fallible.

That distinction is load-bearing in the opposite direction, and is now pinned by
`no_member_lost_its_fallibility_when_the_declarations_were_narrowed`: because this bug
*removes* declarations, the live hazard is emptying a member entirely, which would let
`inline_builtin_is_infallible` DELETE a live `TRAP` handler — the bug-486 / bug-533 failure.

## Declaration rule adopted

**Declare what the lowering can EMIT, not what a program can currently trigger.**

This matters for `ErrFloatNaN`. NaN and ±Inf `Float` values are unconstructible in the
language today — every producing expression (`0.0/0.0`, `1.0e308*1.0e308`, `toFloat("nan")`)
traps at its own observation boundary before any `math` kernel sees it — so `ErrFloatNaN` is
unreachable everywhere. It is nevertheless kept on the `Float` forms whose kernel emits the
check, because (a) the goal statement is about what the lowering can raise, and (b) removing
a kernel-emitted error is what risks the `TRAP`-deletion hazard above if reachability ever
changes.

## Fix

Phase 1 — [x] audit table per overload (above); RED tests that each overload declares only
what its own lowering raises, and that no member lost its fallibility. Confirmed RED on the
pre-fix tree: `asin` declared `ErrInvalidArgument` and `ErrFloatDomain` on overloads 1, 2
and 3 alike, against a measured split of `Fixed`→`77050002`, `Float`/`List OF Float`→`77050012`.
Commit: `0d9366a17`

Phase 2 — [x] per-overload error lists (GREEN). The registration helpers were generalized
rather than the `&[...]` slices edited, because bug-615's `extra:
Option<(ParameterType, &[&str])>` hook is **additive and scalar-only** — it could add
`ErrOverflow` to `tan`'s `Fixed` form but could not subtract `ErrInvalidArgument` from the
`Float` forms, and never reached the `List OF` overloads. Replaced with:

* `ErrorsByType` — errors keyed on the operand type, applied to both that type's scalar
  overload and its `List OF` form (the list form runs the same kernel per lane; verified
  including the SIMD scalar tail). **An absent type declares nothing**, which is what makes
  `atan2`'s `Fixed` form expressible.
* a `list_only` channel on `preserving_binary`, for the length-mismatch
  `ErrInvalidArgument` that is a property of the arity shape rather than the operand type.
* `rounding` gained the same partition (only the `Float` family range-checks).

No golden outside `math`'s own man pages moved, and no runtime behavior changed.
Commit: `0d9366a17`

Phase 3 — [x] full suite; man-manual diff inspected. Commit: see below

## Corrected source comments

Three comments contradicted the lowering they describe and were corrected alongside the
declarations (same class of defect — documentation disagreeing with the code):

- `vector/builder_simd_math.rs` — `lower_simd_clamp`'s doc said *"Never errors."* ~33 lines
  above its `raise_error_bare("ErrInvalidArgument")`.
- `vector/builder_simd_math.rs` — `SqrtFloat` said `ErrInvalidArgument` on a negative lane;
  it raises `ErrFloatDomain` (that is the *Fixed* kernel's error).
- `math/gen_math.rs` — `lower_math_sqrt_array` carried the same wrong error name.

And one DESC: `func_round.rs` claimed "A magnitude too large for `Integer` raises
`ErrOverflow`" with no carve-out, while its `floor`/`ceil` siblings correctly say "a `Fixed`
or `Money` result always fits". Aligned to the siblings.
