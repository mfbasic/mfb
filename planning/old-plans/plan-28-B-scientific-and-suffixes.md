# plan-28-B: Scientific notation & type suffixes

Last updated: 2026-07-06
Effort: medium (1h–2h)

This sub-plan completes `plan-28` (rich number literals) by adding the two
*type-carrying* literal forms — scientific notation and the `f`/`F` type suffixes:

| Form            | Example  | Type   | Status |
|-----------------|----------|--------|--------|
| Exponent        | `1e3`    | Float  | **new** |
| Signed exponent | `1e-3`   | Float  | **new** |
| Mantissa+exp    | `2.5e2`  | Float  | **new** |
| Exponent        | `1e3f`    | Float  | **new** |
| Signed exponent | `1e-3f`   | Float  | **new** |
| Mantissa+exp    | `2.5e2f`  | Float  | **new** |
| Exponent        | `1e3F`    | Fixed  | **new** |
| Signed exponent | `1e-3F`   | Fixed  | **new** |
| Mantissa+exp    | `2.5e2F`  | Fixed  | **new** |
| Float suffix    | `2f` / `1.5f` | Float | **new** |
| Fixed suffix    | `2F` / `1.5F` | Fixed | **new** |

The single behavioral outcome: `1e3` and `2.5e2` and `1.5f` each type as a
**Float** literal (value `1000` / `250` / `1.5`), `2F` types as a **Fixed**
literal (value `2`) *without needing an expected-type context*, and each lowers
and range-checks under its resolved type. `1e400` reports
`TYPE_FLOAT_LITERAL_OVERFLOW`; a Fixed suffix out of range reports
`TYPE_FIXED_LITERAL_OVERFLOW`.

This sub-plan depends on **plan-28-A**: it reuses A's lexer number-scanner and
digit-separator handling and extends A's `numeric::classify_literal` helper (which
A landed with the Integer/Float arms and a single-source-of-truth invariant).

It complements:

- `mfb spec language lexical-structure` (§2.1 — A rewrote it for radix/separators;
  B adds the exponent and suffix rows).
- `mfb spec language type-inference` (§ "Literal Coercion" — where an *untyped*
  numeric literal coerces to Fixed at a typed slot. A `F`-suffixed literal is
  instead *intrinsically* Fixed, which is new; §4.3 below reconciles the two).
- `mfb spec language types` (Fixed/Float per-type ranges and the
  `TYPE_*_LITERAL_OVERFLOW/UNDERFLOW` checks in `src/ir/verify/mod.rs:1564,1607`).
- `mfb spec diagnostics 01_rule-codes` (reuses A's `MFB_LEX_MALFORMED_NUMBER`;
  see Open Decisions on a conflict code).

## 1. Goal

- The lexer recognizes a decimal exponent — `e`/`E`, an optional `+`/`-`, then one
  or more digits (with separators) — after an integer or fractional mantissa, and
  a single trailing type-suffix letter `f` (Float) or `F` (Fixed). Both extend the
  `Number` token; the value string stays canonical text.
- `classify_literal` (from A) becomes suffix- and exponent-aware and is the
  **only** decider of a numeric literal's type: a trailing `f`/`F` forces
  Float/Fixed; otherwise a `.` or an `e`/`E` exponent → Float; otherwise Integer.
  It returns a **suffix-free** value string so every `parse::<i64>()`/`parse::<f64>()`
  consumer keeps working.
- A `2F` literal types as Fixed with **no** expected-type context — the first way
  to write an intrinsically-Fixed literal in the language (today Fixed is only
  reachable by coercion at a typed slot).
- Float/Fixed overflow of exponent and suffixed literals reports the existing
  `TYPE_FLOAT_LITERAL_OVERFLOW` / `TYPE_FIXED_LITERAL_OVERFLOW` (+ `_UNDERFLOW`
  for negated forms), never a panic or silent `inf`.

### Non-goals (explicit constraints)

- **No new numeric *value* representation.** The value string stays a Rust
  `String`; the suffix letter is *consumed by `classify_literal`* and never
  reaches `parse`/codegen. Exponent text (`1e3`) is kept in the string and typed
  Float — `f64::from_str` accepts it directly, so `IrValue::Const.value` and the
  float codegen path are unchanged. This preserves the ~50-consumer contract A
  established.
- **No change to the numeric-promotion or coercion lattice.** `binary_result_type`
  (`src/numeric.rs`) and `expression_compatible`
  (`mfb spec language type-inference`) are unchanged. A `F`-suffixed literal is
  simply *already Fixed* before those rules run; an unsuffixed literal still
  coerces exactly as today.
- **No hex/oct/bin exponent or suffix.** Exponent (`e`) and the `f`/`F` suffix
  apply to **decimal** literals only — `0xFFf` is not a Float; after A's `0xFF`
  hex scan a trailing `f` is a hex *digit*, and `0xFF` + identifier otherwise.
  (Base-16 already owns `f` as a digit, so a hex Float suffix would be
  irredeemably ambiguous — excluded by construction.)
- **No `d`/`D` (double) or other suffixes.** Exactly `f` and `F`.
- **No signed mantissa.** A leading `-` is unary minus; only the *exponent* may
  carry a `+`/`-` sign, inside the literal.
- **No grammar-production change** beyond the numeric terminal.

## 2. Current State

After plan-28-A, `lex_number` (`src/lexer.rs:343`) scans decimal/radix integer
parts and a `.`-fraction, strips separators, and emits canonical text; the value
still has no exponent and no suffix. `numeric::classify_literal` exists with
Integer/Float(`contains('.')`) arms and is the single typing decider consumed by
`src/ir/lower.rs:2016,2606`, `src/monomorph/lower.rs:1428`, and
`src/syntaxcheck/helpers.rs` `numeric_literal_type`.

Fixed today is reachable **only by coercion**: `expression_compatible` widens an
*untyped numeric literal* into a Fixed slot (`mfb spec language type-inference`,
"Literal Coercion": `E=Fixed ∧ A∈{Integer,Float} ∧ expr = Number`), and
`src/ir/lower.rs:2606-2617` sets the const type to Fixed only when `expected ==
Some("Fixed")`. There is no way to write a literal whose *own* type is Fixed —
which is exactly what the `F` suffix must introduce. `src/syntaxcheck/types.rs:197`
and `numeric_literal_type` (`src/syntaxcheck/helpers.rs`) also assume a numeric
literal is Integer-or-Float.

Range checks (`src/ir/verify/mod.rs:1564` `check_const_literal`, `:1607`
`check_negated_const_literal`) already handle all three types and already treat a
non-finite `f64` parse as Float overflow (`:1584` `!f.is_finite()`) and `>= 2^31`
as Fixed overflow (`:1594`). They receive the resolved `type_` and the value
string — the value string must be **suffix-free** for their `parse::<f64>()` to
work, which `classify_literal` guarantees.

**Precedent:** the Fixed-const path at `src/ir/lower.rs:2606` (`expected ==
Some("Fixed")`) shows exactly where an intrinsic-Fixed literal must also produce a
Fixed `IrValue::Const`; B generalizes that from "expected is Fixed" to "the literal
is Fixed (by suffix) OR expected is Fixed".

## 3. Design Overview

Two pieces, lowest-risk first:

1. **Scientific notation.** A lexer extension (scan `e`/`E`, optional sign,
   digits+separators) plus one `classify_literal` arm (exponent ⇒ Float). This
   reuses the existing Float representation and codegen entirely — `1e3` is just a
   Float literal whose string happens to contain `e`. Overflow is already handled
   by the `!f.is_finite()` check. Lands alone.
2. **`f`/`F` suffixes.** The lexer consumes one trailing suffix letter;
   `classify_literal` maps `f`⇒Float, `F`⇒Fixed and strips it. The `F` path is the
   **only** genuinely-new typing: it must make an intrinsic-Fixed literal reach
   `IrValue::Const{type_:"Fixed"}`, `numeric_literal_type`, and the inference
   sites *without* an expected type. This is where the correctness risk
   concentrates (a new standalone literal type flowing through inference, lowering,
   and range-checking), so it lands last behind tests.

Both pieces stay funnelled through `classify_literal`, so the "one place decides
literal type" invariant from A holds.

## 4. Detailed Design

### 4.1 Scientific notation (Phase 1)

In `lex_number`, after the integer part and optional `.`-fraction, if the next
char is `e`/`E` **and** it is a well-formed exponent (optional `+`/`-` then at
least one digit): consume `e`, the optional sign, and the digits, pushing them to
`value`. The exponent-digit scan is **new code in B** and must itself allow A's
between-digits `_` separators (`1_0e1_0`, `1e1_0`) — A only wired separators into
the integer/fraction loops, so B extends the same rule to the exponent digits. If
`e`/`E` is **not** followed by a valid exponent (`1e`, `1e+`, `1eq`) the `e` is
*not* part of the number — end the literal so `1e` lexes as `1` then identifier
`e` (matches the "`.` only consumed when a digit follows" precedent in §2.1). No
panic. A type suffix (§4.2) may follow the exponent (`1e3F`, `2.5e2f`); it is
scanned after the exponent and retypes the whole literal (§4.3).

`classify_literal` arm: a value containing `e`/`E` (outside a suffix) ⇒ Float.
`f64::from_str("1e3")` works, so the value string is left as-is; the Float const
and codegen path are unchanged. Overflow: `f64::from_str("1e400")` → `inf` →
`check_const_literal` already emits `TYPE_FLOAT_LITERAL_OVERFLOW` via
`!f.is_finite()` — verify a test covers it; add handling if the parse path differs.

### 4.2 `f`/`F` suffix scanning (Phase 2, lexer)

After the full mantissa+exponent is scanned, if the next char is `f` or `F` **and**
the char after it is not an identifier-continue char (so `1foo` is `1` then `foo`,
but `1f` is a suffixed literal): consume exactly one suffix letter and push it to
`value`. The suffix letter is the *last* char of the token value and is the signal
`classify_literal` reads; it never reaches a numeric `parse`.

Guard: a suffix may not follow a radix literal (A's hex/oct/bin decode already
emitted decimal and consumed its digits; `f`/`F` after `0x…` is out of scope per
Non-goals — and for hex, `f`/`F` were consumed as digits anyway). Restrict suffix
scanning to the decimal path.

### 4.3 `classify_literal` type decision + intrinsic-Fixed wiring (Phase 2)

Final `classify_literal(text)`:

1. If `text` ends with `f` → `(text[..len-1], Float)`.
2. If `text` ends with `F` → `(text[..len-1], Fixed)`.
3. Else if `text` contains `.` or `e`/`E` → `(text, Float)`.
4. Else → `(text, Integer)`.

The returned value string is suffix-free and `parse`-ready for both `i64` and
`f64` consumers. A suffix composes with an exponent: `1e3F` → step 2 strips `F` →
`("1e3", Fixed)`, and `1e3f` → step 1 → `("1e3", Float)` — so the exponent+suffix
rows in the form table fall out of this ordering with no extra code (the suffix
check precedes the `contains('e')` check). **Decided:** exponent+suffix is allowed.

**Decided — an explicit suffix always wins over the expected type.** A suffixed
literal keeps its intrinsic type and is then checked by ordinary assignability; the
expected type never overrides the suffix. So `2F` in a `Float` slot is a *Fixed
value assigned to a Float slot* — a type error unless the coercion lattice permits
it (it does not: `compatible(Float, Fixed)` is false and no literal-coercion rule
widens toward Float), **not** a silent Float. Only an *unsuffixed* untyped-Integer
literal takes the `expected == Fixed/Byte` coercion.

Wire the **Fixed** result through the typing/lowering sites so a suffixed literal
is Fixed with no expected type:

- `src/ir/lower.rs:2606-2617` — build the `IrValue::Const` type from
  `classify_literal` first (Fixed/Float/Integer) and use it whenever the literal is
  intrinsically Fixed or Float (suffix or `.`/exponent shape); only for an
  *unsuffixed untyped-Integer* literal keep the existing `expected == Fixed/Byte`
  coercion. This encodes "suffix wins".
- `src/ir/lower.rs:2016-2021` and `src/monomorph/lower.rs:1428` — return Fixed for
  an `F`-suffixed literal (currently only Integer/Float).
- `src/syntaxcheck/helpers.rs` `numeric_literal_type` — return `Type::Fixed` for an
  `F`-suffixed literal; keep the leading-`-` peel (a negated `-2F` is Fixed).
- Confirm `check_const_literal`/`check_negated_const_literal`
  (`src/ir/verify/mod.rs`) receive the suffix-free value (they already handle the
  Fixed type + range); add a Fixed-suffix overflow test.
- Audit the ~50 `Expression::Number` consumers (A's list) for any that assume the
  string is a bare integer/float and would choke on a trailing `f`/`F`; all
  numeric decisions must go through `classify_literal`. `src/ast/serialize.rs:1119`
  (AST round-trip) must preserve the suffix so a serialized `2F` re-parses as Fixed.

## Layout / ABI Impact

None. Float and Fixed const encodings, string/collection layout, copy/move/transfer
rules, and goldens are unchanged: exponent literals reuse the existing Float const
path, and a Fixed-suffixed literal produces the same `IrValue::Const{type_:"Fixed",
value}` that an expected-Fixed coercion produces today. `mfb spec memory` /
`mfb spec package` need no edits; only `mfb spec language` (§2.1 + a note in
type-inference on intrinsic-Fixed literals) changes.

## Phases

### Phase 1 — Scientific notation

Delivers `1e3` / `1e-3` / `2.5e2` as Float literals. Safe alone: reuses the Float
representation; only one `classify_literal` arm and one lexer branch.

- [ ] Extend `lex_number` (`src/lexer.rs:343`) to scan a valid exponent after the
      mantissa; leave `e` unconsumed when the exponent is malformed (`1e`, `1e+`).
- [ ] Wire A's between-digits `_` separators into the new exponent-digit scan
      (`1_0e1_0`, `1e1_0`) — this is B-side code, not inherited from A.
- [ ] Add the exponent⇒Float arm to `classify_literal` (`src/numeric.rs`).
- [ ] Confirm `1e400` → `TYPE_FLOAT_LITERAL_OVERFLOW` (`!f.is_finite()`,
      `src/ir/verify/mod.rs:1584`); add the test.
- [ ] Update §2.1 in `src/docs/spec/language/02_lexical-structure.md`: add the
      exponent grammar + the "`e` only consumed when a valid exponent follows"
      rule; drop the "no exponent" clause.
- [ ] Tests: lexer unit tests (`1e3`, `1E3`, `1e+3`, `1e-3`, `2.5e2`, `1_0e1_0`);
      malformed (`1e`, `1e+`) lexes as number-then-identifier; acceptance runtime
      proof that `PRINT 2.5e2` emits `250` (Float formatting) and `1e-3` prints
      `0.001`.

Acceptance: exponent literals type Float and print the right value in unit +
runtime tests; `1e400` reports `TYPE_FLOAT_LITERAL_OVERFLOW`; malformed exponents
never panic; full acceptance green.
Commit: —

### Phase 2 — `f`/`F` suffixes (highest-risk: intrinsic-Fixed literal)

Delivers `2f` (Float) and `2F` (Fixed). Last because `F` introduces a standalone
Fixed literal type through inference, lowering, and range-checking.

- [ ] Scan one trailing `f`/`F` suffix on the decimal path in `lex_number`
      (`src/lexer.rs:343`), guarding against identifier-continue and radix forms
      (§4.2).
- [ ] Finalize `classify_literal` with the suffix arms (§4.3) returning a
      suffix-free value; extend its unit tests (`2f→(2,Float)`, `2F→(2,Fixed)`,
      `1.5F→(1.5,Fixed)`).
- [ ] Wire intrinsic-Fixed/Float through `src/ir/lower.rs:2016,2606`,
      `src/monomorph/lower.rs:1428`, and `numeric_literal_type`
      (`src/syntaxcheck/helpers.rs`); preserve the suffix through
      `src/ast/serialize.rs:1119` round-trip.
- [ ] Audit the `Expression::Number` consumers (plan-28-A §2 list) so none parses
      a suffixed string directly; route all through `classify_literal`. In
      particular the LINK-integer sites `src/ir/lower.rs:378` and `:395`
      (`text.parse::<i64>().unwrap_or(0)` / `IrLinkExpr::Int`) must **reject** a
      now-parseable-looking Float/exponent/suffixed literal (`1e3`, `2F`) rather
      than silently coercing it to `0` — decide the diagnostic and wire it.
- [ ] Add the exponent + suffix rows to §2.1 in
      `src/docs/spec/language/02_lexical-structure.md`, and a note in
      `src/docs/spec/language/type-inference` that an `F`-suffixed literal is
      *intrinsically* Fixed (vs. coerced) and how a suffix interacts with an
      expected type (Open Decisions).
- [ ] Tests: `tests/` valid+invalid coverage — a standalone `LET x = 2F` binds a
      Fixed (no annotation); `2f` binds a Float; the exponent+suffix combos from
      the form table (`1e3f`/`2.5e2f` → Float, `1e3F`/`1e-3F`/`2.5e2F` → Fixed);
      Fixed-suffix overflow (`3000000000F` → `TYPE_FIXED_LITERAL_OVERFLOW`), negated
      `-2F` underflow path; `1foo` still lexes as `1` then `foo`; the
      suffix-wins-over-expected conflict (`LET x AS Float = 2F` is a type error, not
      a silent Float). Runtime proof that `2F` and `2f` produce the correct
      Fixed/Float printed values.

Acceptance: `2F` types Fixed and `2f`/`1.5f` type Float with **no** expected-type
context (unit + runtime); Fixed/Float suffix overflow reports the right code; the
consumer audit is complete (full acceptance byte-identical for all pre-existing
literals); §2.1 shows the suffix rows.
Commit: —

## Validation Plan

- Lexer unit tests: all valid/invalid exponent and suffix forms above.
- Function/type coverage: standalone-typed `LET x = 2F` (Fixed) and `LET y = 2f`
  (Float) bindings; overflow/underflow cases for both suffixed and exponent forms.
- Runtime proof: a compiled+run program printing `2.5e2`, `1e-3`, `2F`, `2f` and
  asserting stdout (`250`, `0.001`, and the Fixed/Float renderings) — real
  behavior, not just golden text.
- Doc sync: `src/docs/spec/language/02_lexical-structure.md` (§2.1 exponent +
  suffix) and a `type-inference` note on intrinsic-Fixed literals.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Resolved Decisions

- **Suffix vs. expected-type conflict — the explicit suffix wins** (decided). A
  suffixed literal keeps its intrinsic type and is then checked by ordinary
  assignability; the expected type never overrides the suffix. `LET x AS Float =
  2F` is therefore a *type error* (Fixed value into a Float slot — the lattice does
  not widen toward Float), not a silent Float. A suffix is a real type annotation.
  Wired at `src/ir/lower.rs:2606` (§4.3): intrinsic Float/Fixed always wins; only an
  unsuffixed untyped-Integer literal takes the `expected == Fixed/Byte` coercion.
- **Fixed-suffix exponent — allowed** (decided). `1e3F` is a Fixed literal and
  `1e3f`/`2.5e2f` are Float; `classify_literal` strips the suffix before the
  exponent text is parsed, so the exponent+suffix rows in the form table fall out
  of the §4.3 ordering with no extra code. Covered by the Phase 2 test list.

## Non-Goals

- Suffixes other than `f`/`F` (no `d`/`D`, no `i`/`u` integer-width suffixes).
- Hex/oct/bin exponent or suffix (decimal only).
- Signed mantissa or any new numeric value representation.
- Changes to the numeric-promotion or literal-coercion lattice.

## Summary

The risk is not scientific notation — that is a Float literal that merely contains
`e`, fully served by the existing Float path and overflow check. The real
engineering is the `F` suffix: it introduces the first *intrinsically-Fixed*
literal, which must flow as Fixed through inference (`numeric_literal_type`),
lowering (`IrValue::Const`), and range-checking without an expected-type context —
and the `classify_literal` funnel plus the `Expression::Number` consumer audit are
what keep that single new type-signal from leaking a raw suffix into a numeric
`parse` or a golden. Everything downstream of the resolved type is unchanged.
