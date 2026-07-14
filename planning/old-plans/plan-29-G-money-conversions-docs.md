# plan-29-G: Money type — full conversion matrix, toString, docs & acceptance

Last updated: 2026-07-07
Effort: medium (1h–2h)

The finishing sub-plan: it makes `Money` observable and freely convertible in **both
directions to every other type**, registers the builtins, and lands every doc, man
page, spec row, acceptance program, and function test the whole plan-29 feature
requires. Because MFB has no implicit casts, crossing into or out of `Money` is always
an explicit conversion call — so the user must be able to convert *from any type into
Money* and *from Money into any type*. It depends on plan-29-A..F. After this sub-plan
`Money` is a complete, documented, fully-tested scalar type, and all seven `plan-29-*`
docs are removed in the final commit.

It complements:

- `./mfb spec language types` (§4.1/§4.10/§4.11/§4.12 gain Money rows)
- `./mfb spec memory scalar-storage` / `./mfb spec package` (Money 8-byte payload, `TYPE_MONEY` id)
- `./mfb man general` (new `toMoney`; updated `toString`/`toInt`/`toFloat`/`toFixed`/`toByte`)

## 1. Goal — the full conversion matrix

Since conversion is the only (deliberate, explicit) way to cross the Money boundary,
**every** reasonable direction exists:

- **Into Money:** `toMoney(String)`, `toMoney(Integer)`, `toMoney(Float)`,
  `toMoney(Fixed)`, `toMoney(Byte)`.
- **Out of Money:** `toString(Money [, precision])`, `toInt(Money)`, `toFloat(Money)`,
  `toFixed(Money)`, `toByte(Money)`.

Plus: the builtins are registered, overload-resolved, and return-type-inferred; the
`math::` surface gains its deliberate Money overloads (`abs`/`min`/`max`/`clamp`/
`floor`/`ceil`/`round`/`rand`, §4.7) while `pow`/`sqrt`/transcendentals keep rejecting; every
spec topic, man page, acceptance program, and `func_*` test for Money exists and passes.

### Non-goals (explicit constraints)

- No `math::` transcendental overloads for Money, no `Money2/3/4` vector variants
  (unlike Fixed) — Money is a plain financial scalar.
- No implicit conversion anywhere — every crossing is an explicit `to*` call (the whole
  point). No change to existing conversion behavior for other types; all current
  goldens byte-identical.

## 2. Current State

Conversion codegen: `toInt(Fixed)` (`builder_conversions.rs:49,61-83`), `toFloat(Fixed)`
(`:474-482`), `toFixed` (`lower_to_fixed`, `:510-558`, with `emit_integer_to_fixed_value`
`:715-736` and `emit_float_bits_to_fixed_value` `:738-810`), `toByte` (`:17`). `toString`
per-type dispatch: `builder_strings.rs:728-735` (Fixed→`emit_fixed_to_string_value`,
`:1165-1392`). Builtin names: `general.rs:6-9` (`TO_STRING/TO_INT/TO_FLOAT/TO_FIXED`,
`toByte` in `mod.rs`); own-raw-lowering conversion classification `builtins/mod.rs:167,177,248`.
Return-type inference for `toFixed`: `data_objects.rs:1032`, `builder_value_semantics.rs:628`,
`validate.rs:1309`, `builder_values.rs:716-717,783-786`. `load_numeric_as_double` gained
a Money arm in plan-29-E. Man templates in `.ai/`; driver `scripts/update_man.sh`.
Acceptance program: `tests/acceptance/src/*.mfb` (consolidated suite).

## 3. Design Overview

The base-10 formatter/parsers are *simpler* than their Q32.32 Fixed counterparts (the
Money fraction is just the decimal digits of `raw % 100000`). Each conversion mirrors an
existing Fixed/Integer conversion with the scale changed from `2^32`/`>>32` to
`100000`/`/100000`. Then the registration/inference plumbing, then docs and the test
sweep. Risk is low and mechanical; the value is completeness (the runtime-completion
gate requires executed proofs and full overload tests).

## 4. Detailed Design

### 4.1 `toString(Money [, precision])` — `builder_strings.rs`
`"Money" => emit_money_to_string_value` arm (`:728-735`). `intpart = raw / 100000`
(signed), `frac = |raw| % 100000` (5 digits, zero-padded); render `precision` fractional
digits (**default 2** — decided; consistent with `toString(Fixed/Float)`), padding with
`0` when `precision > 5`; when `precision < 5` round the truncated fraction
**half-away-from-zero** — a *fixed* presentation rule, **independent of the global
rounding mode** (decided; see Open Decisions). `toString` is thus a **pure function of
`(raw, precision)`**: the same `Money` always prints the same string regardless of any
`money::setRounding` call. It does **not** call `emit_apply_rounding`. Handle sign and
the `intpart == 0, raw < 0` case (`-0.00001`). Simpler than `emit_fixed_to_string_value`
— decimal digit extraction, no long division.

Rationale: the global mode governs how *arithmetic settles cents*; `toString` precision
is *presentation* — a different concern. Decoupling (a) keeps `toString` pure, so logs,
test goldens, and reasoning don't depend on far-away mutable state (a determinism smell
otherwise — a golden would depend on `setRounding` ordering), and (b) enables the real
workflow "accumulate with Banker's, present invoices half-up." The exact stored value is
always available via `toString(m, 5)` for callers who want to round it deliberately under
the mode.

### 4.2 Into Money — `lower_to_money` (`builder_conversions.rs`)
- `toMoney(Integer)`: `raw = value * 100000`, overflow → `ErrOverflow`.
- `toMoney(Byte)`: `raw = value * 100000` (always in range).
- `toMoney(Float)`: finite-check (NaN/Inf → `ErrInvalidFormat`), `value * 100000.0`
  rounded via the current mode (Float twin of `emit_apply_rounding`), range →
  `ErrOverflow` (mirror `emit_float_bits_to_fixed_value`).
- `toMoney(Fixed)`: `fixed_raw * 100000 / 2^32` (128-bit) finished through
  `emit_apply_rounding`, range → `ErrOverflow`.
- `toMoney(String)`: runtime decimal parse to raw — **mirror whatever mechanism
  `toFixed(String)` uses** (decided: inline scan vs. `_mfb_rt_*` helper, same choice as
  Fixed), same digit scan as `money_raw_from_decimal`; malformed → `ErrInvalidFormat`,
  out-of-range → `ErrOverflow`.

### 4.3 Out of Money — `builder_conversions.rs`
- `toInt(Money)`: `raw / 100000` truncated toward zero (mirror `emit_fixed_to_int_value`,
  `/100000` instead of `>>32`). Always fits Integer.
- `toFloat(Money)`: `raw as f64 / 100000.0` (via `load_numeric_as_double`).
- `toFixed(Money)`: `raw * 2^32 / 100000` (128-bit) finished through `emit_apply_rounding`,
  range → `ErrOverflow` (Fixed's integer part is 32-bit, so a large Money overflows Fixed).
- `toByte(Money)`: `intpart = raw / 100000`; if `0 <= intpart <= 255` → Byte, else
  `ErrOverflow` (mirror the `toByte` range check).

### 4.4 Registration & inference
- `builtins/general.rs`: `TO_MONEY = "toMoney"`; `builtins/mod.rs` classify `toMoney` as
  an own-raw-lowering conversion (beside `toFixed`, `:167,177,248`).
- Return-type inference: Money arms so `toMoney(...)` types as `Money` at
  `data_objects.rs:1032`, `builder_value_semantics.rs:628`, `validate.rs:1309`,
  `builder_values.rs:716-717,783-786`; the `Money`→Integer/Float/Fixed/Byte/String
  result types for the out-conversions follow the existing `to*` inference (they return
  the named target type regardless of source).
- `resolver` `BUILTIN_TYPES` and the syntaxcheck mapping arms are already added in
  plan-29-A; confirm every conversion overload type-checks and fill any gap.

### 4.5 Documentation
- `src/docs/spec/language/04_types.md`: §4.1 add the `Money` primitive row +
  description (decimal, 5 places, exact, range, the **dimensional algebra** — same-unit
  add, scalar scaling, `M/M` ratio, and what is a compile error: `M*M`, `M ± scalar`,
  `scalar/M`, `M MOD scalar`, `M^n`, `M`-vs-scalar comparison; overflow/divide-by-zero;
  `Money * Float` inexactness caveat); §4.10 default `0.00000`; §4.11 comparable +
  orderable **against Money only**; §4.12 `TYPE_MONEY_LITERAL_*` range check; update the
  operand-promotion table/prose to add the Money rows and note the dimensional
  restrictions. Three framing points to state explicitly in the §4.1 prose:
  - **The dimension is money-vs-dimensionless, not currency safety** — Money is a
    *unitless* currency amount; nothing stops a program adding a USD amount to a EUR
    amount. Tracking units is the program's job.
  - **`DIV` is the explicit Float escape**: `M DIV k` and `M DIV M` return `Float` like
    every other numeric pair (`DIV` is fractional division language-wide); writing
    `DIV` against Money is a deliberate dimension exit, parallel to `toFloat`.
  - **Division drift & the allocation idiom**: `M / k` rounds to the 5th decimal, so the
    shares of a split need not re-sum to the total — the drift is at most a few
    `0.00001` units (never whole cents at the language level, but visible once shares
    are themselves settled to 2 places). Document the standard idiom
    (`last = total - share * (n - 1)`) and cross-link `money::round` if adopted.
- `src/docs/spec/memory/…`: `scalar-storage` Money 8-byte row; collections Money element.
- `src/docs/spec/package/…`: `TYPE_MONEY` wire id in the type-id table.
- `src/docs/spec/language/…type-inference`: decimal literal → Money coercion + `m`/`M`
  suffix.
- Man pages (`scripts/update_man.sh`, `.ai/` templates): new `general/toMoney`; update
  `general/toString`, `general/toInt`, `general/toFloat`, `general/toFixed`,
  `general/toByte`, the `general` overview, and the `types` page to list Money and the
  `money::Rounding` enum. (The `money::` package overview + `setRounding`/`getRounding`
  man pages land in plan-29-D; ensure the `types`/package index cross-links them.)

### 4.6 Acceptance & function tests
- `tests/acceptance/src/*.mfb`: a Money section (bind, dimensional arithmetic, the full
  conversion matrix, print) exercising Money end-to-end; regenerate goldens.
- `tests/func_general_toMoney_{valid,invalid}/**` — every overload (String, Integer,
  Float, Fixed, Byte) + malformed/out-of-range failures.
- Money overload cases added to `tests/func_general_{toString,toInt,toFloat,toFixed,toByte}_valid/**`
  (+ `_invalid` where a range failure exists, e.g. `toByte(Money)` / `toFixed(Money)`).
- `tests/rt-behavior/**`: the executed arithmetic proofs from plan-29-C/E/F, now
  printing via `toString`; plus a `toString(money, 2)` case whose displayed string is
  **identical** under `Commercial` and `Banker` (proves presentation rounding is
  decoupled from the mode). A companion case shows an *arithmetic* op (e.g. `M / k`)
  whose *result* differs by mode while its `toString` rendering rule stays fixed.

### 4.7 `math::` builtin surface for Money (`src/builtins/math.rs`)

`math.rs` gates every numeric overload through a **local** `is_numeric` whitelist
(`math.rs:238`, `Integer|Float|Fixed`), per-function acceptance arms in `resolve_call`
(`:143-183`), and the `expected_arguments` strings (`:186-199`) — so nothing
auto-enabled when plan-29-A widened `numeric::is_numeric_type`. Every Money overload
below is a deliberate addition; everything not listed keeps rejecting Money with the
existing expected-arguments message.

**Accepted — dimensionally valid, result stays in (or deliberately exits) the dimension:**

- `abs(Money) → Money` — integer abs on the raw; raw `INT64_MIN` → `ErrOverflow`
  (the same most-negative check as unary negate, plan-29-C §4.3).
- `min(Money, Money) → Money`, `max(Money, Money) → Money` — signed raw
  compare+select; the `all_same_numeric` arm (`:144`) accepts once `Money` joins the
  local whitelist.
- `clamp(Money, Money, Money) → Money` — the `all_same_numeric` clamp arm (`:171`).
- `rand(Money, Money) → Money` — **approved (2026-07-11)**: a uniform draw between two
  amounts is itself an amount (test data, simulations). New `resolve_call` arm beside
  the `two_integers` RAND arm (`:180`); draw uniformly over the raw i64 range
  `[min_raw, max_raw]` using the existing per-arena PCG64 state, mirroring
  `rand(Integer, Integer)` semantics including its min>max error behavior; update
  `expected_arguments` for RAND.
- `floor(Money) → Integer`, `ceil(Money) → Integer`, `round(Money) → Integer` —
  **consistent with the existing scalar overloads** (`floor/ceil/round(Float|Fixed) →
  Integer`, `:172`): the result is the dimensionless count of whole currency units, a
  deliberate dimension exit exactly like `DIV`; re-enter with `toMoney(...)`.
  `round(Money)` uses the same **fixed half-away-from-zero** rule as `round(Float)`,
  *not* the global mode — an explicit call is presentation-like, not arithmetic
  settling (mode-aware settling is `money::round`, plan-29-D Open Decisions).
  Kernels are integer-domain: quotient/remainder of raw by `100000` with the
  floor / ceil / half-away adjustment.

**Rejected — keep rejecting (no whitelist entry / no arm):**

- `pow` — must match the operator algebra: `M ^ n` is a compile error (plan-29-A §1).
- `sqrt`, `exp`, `log`, `log10`, `sin`/`cos`/`tan`/`asin`/`acos`/`atan`/`atan2` —
  transcendentals of a dimensioned quantity are meaningless; analytics go through
  `toFloat(m)`. (`sqrt` reject — DECIDED, see Open Decisions.)
- List/SIMD overloads (`abs(List OF Money)`, …) — omitted with the other vector
  non-goals (no `Money2/3/4`, §Non-goals).

Update `expected_arguments` (`abs` → `"Integer | Float | Fixed | Money"`, etc.) and the
`math` man pages for the accepted set; add `_invalid` fixtures proving `pow`/`sqrt`
still reject Money.

## Layout / ABI Impact

Doc-only additions here (the `scalar-storage` and `package` rows describe what
plan-29-B/C already implemented). No new layout.

## Phases

### Phase 1 — conversion & toString codegen
Money converts to/from every type and prints.

- [ ] `emit_money_to_string_value` + dispatch (`builder_strings.rs`).
- [ ] `lower_to_money` (String/Integer/Float/Fixed/Byte) + `toInt`/`toFloat`/`toFixed`/
      `toByte` of Money (`builder_conversions.rs`).
- [ ] Register `toMoney`; Money return-type inference arms; confirm all overloads
      type-check.
- [ ] `math::` Money surface (§4.7): local whitelist + `resolve_call` arms +
      `expected_arguments`; `abs`/`min`/`max`/`clamp` (→ Money),
      `floor`/`ceil`/`round` (→ Integer, fixed half-away), and
      `rand(Money, Money)` (→ Money, per-arena PCG64) kernels;
      `pow`/`sqrt`/transcendentals still reject.

Acceptance: an executed program round-trips `Money → each type → Money` and prints
correctly on both backends — `toString(1.25m)`→`"1.25"`, `toString(1.25m,5)`→`"1.25000"`,
`toMoney("3.50")`, `toMoney(4)`, `toMoney(1.5)`, `toMoney(aFixed)`, `toMoney(aByte)`,
`toInt(2.99m)`→`2`, `toFloat(1.25m)`→`1.25`, `toFixed(1.25m)`, `toByte(2.00m)`→`2`;
`toMoney("x")`→`ErrInvalidFormat`, out-of-range→`ErrOverflow`, `toByte(300.00m)`→`ErrOverflow`.
Commit: —

### Phase 2 — docs, man, acceptance, function tests
Money is fully specified and tested.

- [ ] Spec updates (§4.1/4.10/4.11/4.12 in `04_types.md` incl. the dimensional algebra,
      `scalar-storage`, `package` type-id, `type-inference`).
- [ ] Man pages (toMoney + updated toString/toInt/toFloat/toFixed/toByte + general +
      types; the accepted `math::` pages — abs/min/max/clamp/floor/ceil/round) via
      `scripts/update_man.sh`.
- [ ] `tests/func_general_toMoney_{valid,invalid}/**` + Money cases in the five out-
      conversion suites; `tests/func_math_{abs,min,max,clamp,floor,ceil,round,rand}_valid/**`
      Money cases + `_invalid` fixtures proving `pow`/`sqrt` reject Money.
- [ ] Acceptance Money section + regenerated goldens.

Acceptance: `mfb spec language types` shows the Money rows + dimensional algebra;
`mfb man general toMoney` renders; all `func_*` Money tests pass; `scripts/test-accept.sh`
passes with the Money acceptance golden; a full build + test run is green.
Commit: —

## Validation Plan

- Function tests: `tests/func_general_toMoney_{valid,invalid}/**` (5 overloads) + Money
  cases in `toString`/`toInt`/`toFloat`/`toFixed`/`toByte`.
- Runtime proof: executed program covering every Money operation and every conversion
  direction on aarch64 + x86-64.
- Doc sync: `04_types.md`, `scalar-storage`, `package`, `type-inference`; man pages.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Default `toString(Money)` precision — DECIDED: `2`** (currency display, consistent
  with `toString(Fixed/Float)`), `precision`/`decimals` up to 5 exact. (§4.1)
- **`toString` presentation rounding — DECIDED: fixed half-away-from-zero, independent of
  the global rounding mode** (`toString` stays a pure function of `(raw, precision)`; the
  mode governs *arithmetic* only). Enables "compute Banker's, display half-up" and keeps
  goldens deterministic. `toString(m, 5)` exposes the exact value for deliberate
  mode-driven rounding. (§4.1)
- **`M / M` result type — DECIDED: `Float`** (dimensionless ratio; document the
  non-terminating rationale in §4.1 prose — see plan-29-A Open Decisions). (§4.5 docs)
- **`toMoney(String)` runtime path — DECIDED: mirror whatever `toFixed(String)` uses**
  (inline scan vs. `_mfb_rt_*` helper), for consistency. (§4.2)
- **`math::sqrt(Money)` — DECIDED (2026-07-11): REJECT.** If `sqrt(4.00m)` returned
  `2.00m`, the algebra would assert `2.00m * 2.00m = 4.00m` — but `M * M` is a compile
  error precisely because money² is not Money; accepting `sqrt` would un-do the
  lattice's own claim. Std-dev-style analytics go through `toFloat` (variance needs
  `M * M` anyway). `pow` likewise stays rejected, matching the `M ^ n` compile error.
  (§4.7)
- **`math::rand(Money, Money) → Money` — DECIDED (2026-07-11): ACCEPT** (dimensionally
  coherent; mirrors `rand(Integer, Integer)` over the raws). (§4.7)

## Summary

The full, explicit conversion matrix (into Money from every type, out of Money to every
type) plus the documentation and test completeness the repo standard requires. Every
conversion mirrors an existing Fixed/Integer one with the scale changed to base-10; the
formatter is simpler than Q32.32. This sub-plan makes plan-29 a shippable, specified,
tested `Money` type; all seven `plan-29-*` docs are removed in the final commit.
