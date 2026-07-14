# plan-29-B: Money type — constant lowering & .mfp encoding

Last updated: 2026-07-07
Effort: small (<1h)

This sub-plan adds the exact decimal→raw-i64 conversion for `Money` literals and
wires it into IR constant lowering and the `.mfp` binary-representation format, so a
compiled `Money` constant round-trips through a package. It depends on plan-29-A
(the `Money` type identity and `LiteralType::Money`). It produces no runnable
executable on its own — native materialization of the constant is plan-29-C — but it
makes a `Money` constant survive lowering and package encode/decode, verified by
round-trip unit tests.

It complements:

- `./mfb spec package` (the `.mfp` type-id and constant-section format; canonical spec + code under `src/binary_repr/**`)

## 1. Goal

- `money_raw_from_decimal(value) -> Result<i64, String>` converts a decimal `Money`
  literal string to its scaled raw i64 (SCALE = 100000, round-half-up beyond 5
  fractional digits, exact integer arithmetic, no `f64`), the single source of truth
  shared by IR lowering, native immediate emission (plan-29-C), and `.mfp` encoding.
- A `Money` constant lowers to IR with its raw i64 value and encodes/decodes through
  `.mfp` under a new wire type id `TYPE_MONEY`.

### Non-goals (explicit constraints)

- No native codegen (the raw i64 is computed and stored/encoded, not yet emitted as a
  machine immediate — that is plan-29-C).
- No change to `fixed_raw_from_decimal` or any existing `.mfp` type id; `TYPE_MONEY`
  takes the next free id and every existing id/const payload stays byte-identical.
- No `f64` in the conversion — `Money` is exact decimal; using a float bound (as
  `Fixed` does) would defeat the purpose.

## 2. Current State

`fixed_raw_from_decimal` (`src/numeric.rs:85-143`) is the Q32.32 converter: it parses
whole/fraction, scales the fraction by `1<<32`, rounds half-up, and range-checks into
i64. It is called from IR lowering (`src/ir/lower.rs:3261-3277`, the bug-07 min-Fixed
fold), native immediate emission (`type_utils.rs:291`), and `.mfp` const encoding
(`src/binary_repr/sections.rs:373-375`, payload = `fixed_raw_from_decimal(...).to_le_bytes()`).
There is also a **duplicate** copy of the Fixed conversion inline in
`src/binary_repr/writer.rs:870-911` (used by the writer path).

Wire type ids live in `src/binary_repr/mod.rs:49-53` (`TYPE_FIXED: u32 = 5`);
encode name→id at `sections.rs:83` (`"Fixed" => TYPE_FIXED`), decode id→name at
`reader.rs:787` (`TYPE_FIXED => Some("Fixed")`). Literal-type tagging during lowering:
`ir/lower.rs:2029,3519` and `monomorph/lower.rs:1450` (`LiteralType::Fixed => "Fixed"`,
added for Money in plan-29-A Phase 3).

## 3. Design Overview

Two pieces:

1. **`money_raw_from_decimal`** — a decimal-scale sibling of `fixed_raw_from_decimal`.
   Simpler than the Fixed version (scale is base-10, so no `SCALE = 1<<32` fractional
   long-division): take the whole part × 100000, add the first 5 fractional digits
   (zero-padded), round half-up using the 6th digit, apply sign, range-check into i64.
   Reuse `expand_scientific_notation` for `e`/`E` literals.

2. **Constant plumbing** — tag the constant type `"Money"` (done in A) and route its
   raw through IR lowering + `.mfp` encode/decode, mirroring every Fixed site with a
   new `TYPE_MONEY` wire id.

## 4. Detailed Design

### 4.1 `money_raw_from_decimal` (`src/numeric.rs`)
```
const SCALE: i128 = 100_000;               // 5 decimal places
const FRAC_DIGITS: usize = 5;
// expand scientific notation, split sign / whole / fractional (as in fixed_...).
// whole_raw   = whole.parse::<i128>()? * SCALE
// frac digits: take up to FRAC_DIGITS, zero-pad on the right to exactly 5;
//   the 6th digit (if present) drives round-half-up (>=5 rounds the 5-digit value up).
// raw = whole_raw + frac_value (+1 if rounded);  carry into whole on frac==SCALE.
// apply sign; i64::try_from(raw) or Err("Money constant `{value}` is out of range").
```
Errors on malformed input / overflow with the same message style as
`fixed_raw_from_decimal`. Unit-test: `"1.25"`→`125000`, `"0"`→`0`, `"-0.00001"`→`-1`,
`"1.234565"`→`123457` (round half-up at 6th digit), `"1.234564"`→`123456`,
`"92233720368547.75807"`→`i64::MAX`, `"92233720368547.75808"`→`Err`.

### 4.2 IR constant lowering (`src/ir/lower.rs`)
Mirror the Fixed fold path (`:3261-3277`) for the `"Money"` constant type — including
the negated-literal case so the most-negative Money (`-92233720368547.75808`, which
has no positive-magnitude literal) is representable, exactly as bug-07 handles the min
Fixed. Use `money_raw_from_decimal`.

### 4.3 `.mfp` encoding
- `src/binary_repr/mod.rs:49-53`: add `pub(crate) const TYPE_MONEY: u32 = <next free id>;`
  (verify the next unused value; do not renumber existing ids).
- `src/binary_repr/sections.rs:83`: `"Money" => TYPE_MONEY` (encode).
- `src/binary_repr/sections.rs:373-375`: const payload = `money_raw_from_decimal(value)?.to_le_bytes()`.
- `src/binary_repr/reader.rs:787`: `TYPE_MONEY => Some("Money")` (decode).
- `src/binary_repr/writer.rs:870-911`: **route the writer's const path through
  `money_raw_from_decimal`** for the Money case (decided — no third inline copy);
  confirm which path the encode actually uses and cover it.

## Layout / ABI Impact

`.mfp` gains one new scalar type id `TYPE_MONEY` and a Money const payload (8-byte
little-endian raw i64). Every existing type id and const payload is unchanged, so all
current package goldens stay byte-identical. Document the new id in `mfb spec package`
(the type-id table) in plan-29-G's doc sweep.

## Phases

### Phase 1 — `money_raw_from_decimal`
The exact decimal converter, with unit tests, no callers yet.

- [ ] `src/numeric.rs`: add `money_raw_from_decimal` per §4.1 (reuse
      `expand_scientific_notation`).
- [ ] Unit tests in `src/numeric.rs` covering the cases in §4.1 (rounding, min/max,
      out-of-range).

Acceptance: unit tests pass; `cargo build` clean; no behavior change (no callers).
Commit: —

### Phase 2 — IR constant + .mfp round-trip
A `Money` constant lowers and round-trips through `.mfp`.

- [ ] `src/ir/lower.rs`: Money constant fold (incl. negated-min case) via
      `money_raw_from_decimal`.
- [ ] `src/binary_repr/mod.rs`: `TYPE_MONEY` id; `sections.rs` encode + const payload;
      `reader.rs` decode; `writer.rs` duplicate path if used.
- [ ] Tests in `src/binary_repr/tests.rs`: `type_id("Money") == TYPE_MONEY` and a
      `("Money", "1.25")` const round-trips to raw `125000` and back to name `"Money"`
      (mirror the existing `("Fixed","1.25")` test at `tests.rs:753`).

Acceptance: the binary_repr round-trip test for a Money constant passes; existing
`.mfp` golden/round-trip tests unchanged.
Commit: —

## Validation Plan

- Unit tests: `money_raw_from_decimal` (rounding + bounds), binary_repr Money
  round-trip.
- Doc sync: the `mfb spec package` type-id table gains `TYPE_MONEY` — folded into
  plan-29-G's doc sweep (note the id here so G picks it up).
- Acceptance: `scripts/test-accept.sh …` — no drift (no acceptance program uses Money
  yet).

## Open Decisions

- **`.mfp` writer converter — DECIDED: route `writer.rs` through
  `money_raw_from_decimal`** (no third inline copy). Stretch: collapse the existing
  Fixed inline duplicate too. (§4.3)
- **Excess-precision literals (>5 fractional digits) — DECIDED (2026-07-11): round
  half-away (§4.1), plus a warn-severity diagnostic
  `TYPE_MONEY_LITERAL_PRECISION`.** A literal can always carry more digits than any scale, so rounding must
  exist; rejecting outright was considered and declined. The warning fires **only when
  the digits beyond the 5th change the value** (`1.234567m` warns — stored `1.23457`;
  `1.250000m` is exact and silent), keeping the exactness story honest while the
  silence is simply writing the representable value. Implement beside the
  `TYPE_MONEY_LITERAL_*` range check (plan-29-A §4.4/§4.5), rule + spec row in the
  same commit. Runtime `toMoney(String)` (plan-29-G) rounds without warning — data
  ingestion is not a literal.

## Summary

A small, low-risk sub-plan: one exact converter plus the constant/`.mfp` plumbing,
all mirroring `Fixed` with a new wire id. No codegen, no golden drift.
