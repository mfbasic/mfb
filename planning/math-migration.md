# `math` package migration plan (evidence-backed, 2026-08-16)

Serial, judgment-heavy. NOT a clean lift like bits/money — 3 of 5 lowering files must be
**surgically split** (precise keep/move boundaries below). Execute on the main thread with
per-slice byte-identity gating; the split-file surgery needs human review of the keep/move line.

## Decision (2026-08-16, user): ENUMERATED concrete-type overloads — NO NumericVar
The `NumericVar` idea was REJECTED after the migration agent proved (against `func_math_*_invalid`
goldens + `math.rs::resolve_call`) that a single 4-type `NumericVar` (`Int/Float/Fixed/Money`) is WRONG:
math has FIVE distinct per-member numeric classes (transcendentals/pow/atan2 accept `Float`/`Fixed`
ONLY — `sqrt(1)` must error `expected Float | Fixed`; abs/min/max/clamp accept all 4; floor/ceil/round
scalar accept `Float|Fixed|Money`; the list-element sets differ again). And `NumericVar` only ever
collapses the SCALAR overload — list/SIMD overloads must be enumerated concretely regardless.

**Chosen: enumerate every member as plain concrete-type `Implementation`s** — the MOST uniform with the
existing migrated packages (tls/thread/process/datetime all enumerate; none uses a numeric type-class),
ZERO matcher change (safest — no bug-443 surgery), byte-identical (reproduces legacy `resolve_call`
per-member acceptance EXACTLY, so `func_math_*_invalid` goldens stay green). Cost ~90–100 small
`Implementation` struct literals (mechanical). Each type-preserving overload returns `ParameterType::Arg(0)`;
floor/ceil/round return `Integer`; rand `(Integer,Integer)→Integer` + `(Money,Money)→Money`; seed → Nothing.
The exact per-member type sets MUST mirror legacy `math.rs::resolve_call` (`one_float_or_fixed`,
`two_same_float_or_fixed`, `is_numeric_list`, the abs/min/max/clamp all-4, floor/ceil/round FloatishMoney).

## Members (21 callables) — all pure-move at the call-site level
Only external caller of the math dispatcher is `builder_values.rs:784 self.lower_math_call(...)`.
Two members call a STAYS-core helper: `pow` → `emit_pow_scalar`/`lower_pow_array` (builder_pow.rs,
shared with the Float `^` operator, `builder_numeric.rs:1776`); `rand`/`seed` → `RNG_NEXT_SYMBOL`
(routine `lower_rng_next` in rng_pcg64.rs stays core, referenced by symbol).

## 14 constants — pure data-relocate via `add_constant`
`pi,piFixed,twoOverPi,twoOverPiFixed,pi2,pi2Fixed,pi4,pi4Fixed,e,eFixed,ln2,ln2Fixed,ln10,ln10Fixed`
(7 Float, 7 Fixed). `RegistryConstant{name,type_name,value:Some(..),components:None}`. No gap.

## Helper split (Deliverable 2) — the surgical boundaries
- **builder_math.rs (1339 LOC) SPLIT**: STAYS-core = `observe_float` (pub(crate), ~30 callers),
  `observe_promoted_float`, `emit_float_result_check`(+`_fp`,`float_arith_node`),
  `emit_float_exponent_classify` (pub(super); money_math + simd_math callers), `FloatInfinityError`
  (builder_pow imports it). MOVES → `math/common/`: all `lower_math_*`, `lower_fixed_external_math`,
  `emit_rng_next_call`, `emit_float_rounding_integer_range_check`, `is_list_argument`,
  `numeric_element_type_code`, `list_element_type`. Keep the file as a core float-observation module.
- **builder_fixed_math.rs (1132) MOVES ENTIRELY** → `math/common/` (all callers are math surface;
  the `builder_numeric.rs:1509-1517` hits are doc-comments on the *different* `emit_fixed_pow` which stays).
- **builder_simd_math.rs (1027) SPLIT**: STAYS = `emit_alloc_result_list` (pub(super); builder_pow:422).
  MOVES: `lower_simd_unary/binary/clamp` + kernels + `Simd*Kernel`/`SimdError` types.
- **builder_simd_float_math.rs (2272) MOVES (kernels)** + repoint: `math_const_pool_words`,
  `math_const_pool_data_value`, `MATH_CONST_POOL_SYMBOL` are data producers — the EMISSION site stays
  core at `code/mod.rs:1827-1842` (scans relocations for the symbol, calls the producers); repoint those
  3 refs to `crate::codegen::builtins::math::…` ("core emits by symbol, package produces" — like strings tables).
- **builder_simd_fixed_math.rs (355) MOVES ENTIRELY** → `math/common/`.
- **STAY (not math surface)**: builder_pow.rs (shared `^`), builder_money_math.rs (Money operators),
  rng_pcg64.rs + `lower_rng_next` + `RNG_NEXT_SYMBOL`.
- Per-arch SIMD dispatch is `abi::`/`mir::active_backend()`-internal to each emit fn — moving fns wholesale
  preserves it; registry needs NO per-arch `Body` selection.

## Rewiring (Deliverable 3)
Scaffold `src/codegen/builtins/math/{mod.rs, func_<member>.rs, common/}`; register at `registry/mod.rs:~1229`;
`pub(crate) mod math;` in `codegen/builtins/mod.rs`. Each `func_*.rs`: `Body::native(None,None,Some(lower_math_<m>))`
wrapping `super::common::lower_math_call("<m>", args)`. DELETE the `strip_prefix("math.")` arm at
`builder_values.rs:783-785` (routes through the existing `try_native_lower`@721 → `registry::native_lower`).
Repoint `is_math_call` (runtime/mod.rs:166, ir/lower.rs:2104, compat.rs:314) → `registry().owning_package(name)==Some("math")`
(keep `RuntimeHelper::Math`). Remove math arms from `builtins/mod.rs` dual-paths (expected_arguments:487,536;
constants:661,668,675; call_param_names:796 — alias data moves to `Parameter.aliases`). Remove `REGISTRY` `MATH`
entry (target/shared/registry.rs:1056). Delete `src/builtins/math.rs`.

## By-name guarantee (Deliverable 4)
`math.sqrt`/`math.clamp` stay callable-by-name (builder_vector_inline.rs:267/306/446) because `try_native_lower`
matches the full `"math.sqrt"` before the deleted strip arm. vector/audio unaffected by name.

## Gates (Deliverable 5)
Full `cargo test --bin mfb`; `artifact-gate math` = 0 diffs (SIMD byte-identity — HARD, any diff is a bug);
ripple gates **vector** (mandatory — inline `math.sqrt`/`clamp` + vector_package.mfb) + **audio** (audio_mml/render.mfb
call math.); emptiness grep `__math_|is_math_call|builtins::math|MATH\b` returns only intentional STAYS.

## Execution order (gated slices)
1. `NumericVar` infra (serial) — land + full test gate.
2. Scaffold + descriptor + constants + rewire (no lowering moved yet) — gate green.
3. Move `common/` lowering group-by-group: fixed → simd → simd_float → simd_fixed → scalar; `artifact-gate math` after each.
4. Delete legacy files + emptiness grep.
