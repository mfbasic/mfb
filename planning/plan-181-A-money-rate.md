# plan-181-A: money::Rate record, toRate and toString

Last updated: 2026-10-19
Overall Effort: x-large (1d–3d) — the whole plan-181 feature (estimate; sub-plans A, B, C)
Effort: large (3h–1d)
Depends on: nothing

This sub-plan adds the exact decimal rate type to the built-in `money` package: a
record `money::Rate` holding one `Integer` that is the rate scaled by 10^12, the
constructors `money::toRate(String)` and `money::toRate(Integer)`, and a
`toString(Rate)` override. It adds no arithmetic on rates. Sub-plan B adds
`money::convert` and `money::scale`; sub-plan C moves `currency` onto the type.

References:

- `planning/plan-181-B-money-convert.md`, `planning/plan-181-C-currency-rates.md` (the sub-plans that consume this one)
- `src/codegen/builtins/money/mod.rs` (the `money` package registration and `MODULE_DESC`)
- `src/codegen/builtins/big/mod.rs` (`add_record` for `big::Int`, the precedent for a built-in record type)
- `src/docs/spec/stdlib/13_money.md` (the `Money` raw-carrier and rounding rules)
- `src/docs/spec/language/04_types.md` (record visibility, constructors, comparability)
- `src/docs/spec/language/18_builtin-functions.md` (overridable `toString` and package-type overrides)
- `src/docs/spec/language/04_types.md` line 40 (`toFixed(String)`: exact text parsing, the pattern `toRate(String)` follows)
- `.ai/specifications.md` and `.ai/man-content.md` (required spec and man updates)

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| A release build of the compiler exists | `test -x target/release/mfb` | MET (checked 2026-10-19) |
| The currency package suite passes on the unchanged tree | `target/release/mfb test packages/currency` | MET (77 pass, 0 fail, 2026-10-19) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue.

## 1. Goal

- `money::Rate` is an exported record with one field, `scaled AS Integer`, holding the rate multiplied by 10^12. `money::Rate[scaled := 842000000000]` is the rate 0.842.
- `money::toRate("0.842")` returns the `Rate` whose `scaled` is 842000000000. The text is parsed exactly, with no binary step.
- `money::toRate(1)` returns the `Rate` whose `scaled` is 1000000000000.
- `toString` of a `Rate` prints its value in decimal with trailing zeros removed, keeping at least one digit before the point and none after it when the fraction is empty. `toString(toRate("0.84200"))` is `0.842`; `toString(toRate("1"))` is `1`; `toString(toRate("0.0067"))` is `0.0067`.

### Non-goals (explicit constraints)

- **`Fixed` is unchanged.** Its representation, arithmetic, literals, `toFixed(String)`, `toString(Fixed)` and the spec and man text for `Fixed` do not change.
- **No `toRate(Float)` and no `toRate(Fixed)`.** Those overloads would convert a binary value and reintroduce the error the type removes.
- **No new scalar type.** `Rate` is a record in the built-in `money` package, with no operators.
- **No change to `Money`**: its arithmetic, rounding, `money::Rounding`, `toString(Money)` and the spec sections for them stay as they are.
- **No change to the package type table.** Measured: `grep -n -i "big::Int\|Rounding" src/docs/spec/package/04_type-table.md` finds no entry for either built-in type, so built-in package types are not listed there and that file is not edited.
- **The scale is 10^12 and is not configurable.** It is a constant of the `money` package, not a parameter of any function.

## 2. Current State

- **How a built-in package is registered.** `src/codegen/builtins/money/mod.rs` registers `money` with `add_enum` for `Rounding` (the enum), one module per function (`func_round.rs`, `func_set_rounding.rs`, `func_get_rounding.rs`), and the text the man page renders (`MODULE_DESC`).
- **A built-in record type.** `src/codegen/builtins/big/mod.rs` declares `big::Int` with `pkg.add_record(RegistryRecord { name: INT_TYPE, export: true, props: [...] })`. Its comment states that field order is a contract, because lowering reads slots by position. `money::Rate` follows the same pattern with one prop.
- **Record field visibility.** `src/codegen/registry/mod.rs:RecordProp` has `name`, `ty` and `description` and no visibility. `RegistryRecord` has an `export` flag only. So a built-in record's field is readable and writable by any code that can see the record (`src/docs/spec/language/04_types.md`, the `EXPORT` rule). The consequence is handled in Design, below.
- **Exact text parsing.** `toFixed(String)` converts the text exactly with no binary64 step (`src/docs/spec/language/04_types.md:40`, helper `src/codegen/engine/convert/builder_fixed_parse.rs:emit_parse_decimal_string_to_fixed_raw`). `toMoney(String)` is in `src/codegen/builtins/general/func_to_money.rs`. The rate parser follows the `toFixed(String)` helper, with 12 places in place of 32 binary bits.
- **Overridable `toString`.** `src/docs/spec/language/18_builtin-functions.md:155` states that a built-in package's `toString` override is keyed by the type's package-qualified identity, with `toString(color::Color)` and the `vector::VecN` renderers as precedents. `toString(money::Rate)` uses the same mechanism.
- **Equality on records.** `src/docs/spec/language/04_types.md:605`: a record whose fields are all comparable is comparable. `Rate` equality therefore needs no new code.
- **Error codes.** `src/docs/man/types/numeric.md:233-235` lists `77050002` ErrInvalidArgument, `77050003` ErrInvalidFormat and `77050010` ErrOverflow. These are the registry codes used below.
- **Gates.** `scripts/artifact-gate.sh <mfb> <builtin|all>` regenerates byte-identity dumps for one built-in name or for `all`. `scripts/test-accept.sh <mfb> <outdir> [name-glob ...]` runs the acceptance fixtures, filtered by name glob.

## 3. Design

- **The record.** `money::Rate` is an exported record with the single exported prop `scaled AS Integer`. The rate is `scaled / 10^12`, so the smallest rate is `0.000000000001` (`scaled` 1) and the largest is `9223372.036854775807` (`scaled` 9223372036854775807, the largest `Integer`).
- **Invalid rates can exist.** Because `scaled` is exported (see Current State), `money::Rate[scaled := -5]` and `money::Rate[scaled := 0]` compile. This sub-plan does not prevent them. Sub-plan B checks `scaled > 0` on every use that would otherwise give a wrong result, and sub-plan C checks it in `currency::setRate`. Decided by the owner: check on use, not compiler visibility support.
- **`toRate(String)` grammar.** Optional leading `-`, then digits, then optionally `.` and digits; at least one digit is required; no leading or trailing whitespace, no `+`, no exponent, no underscores. Anything else is `ErrInvalidFormat`.
- **Places.** A written fraction with more than 12 digits is `ErrInvalidFormat`, including trailing zeros (`0.8420000000000` has 13 places and is rejected). Nothing is rounded.
- **Range.** A parsed value at or below zero is `ErrInvalidArgument`. A value above `9223372.036854775807` is `ErrOverflow`.
- **`toRate(Integer)`.** `n` at or below zero is `ErrInvalidArgument`. `n` above `9223372` is `ErrOverflow`. Otherwise `scaled` is `n × 10^12`.
- **`toString(Rate)`.** Prints a leading `-` for a negative `scaled`, then the integer part, then `.` and the fraction digits with trailing zeros removed, omitting the point when the fraction is empty. It does not validate: a directly built record prints what it holds.

### Rejected alternatives

- **`toRate(Float)` and `toRate(Fixed)`.** Rejected: both convert a binary value, so `toRate(0.842F)` stores `0.84199999995`.
- **Rounding excess places.** Rejected: a rate that silently differs from the text written is the failure this type exists to prevent.
- **Fixed-width text in `toString`.** Rejected: the owner chose trimmed output.
- **Compiler field visibility.** Rejected for this feature: it is a change to the registry and needs its own sub-plan. Check-on-use covers the consequences here.

## 4. Phases

### Phase A1 — record registration

Tasks:
- In `src/codegen/builtins/money/mod.rs`, add `pkg.add_record(RegistryRecord { name: RATE_TYPE, export: true, props: [RecordProp { name: "scaled", ty: ParameterType::Integer, description: ... }] })`, following `big/mod.rs`'s `INT_TYPE` declaration, with a comment that the single field's meaning (value × 10^12) is a contract.
- Add the constant `RATE_SCALE: i64 = 1_000_000_000_000` in the same module.

Acceptance: `target/release/mfb man money` lists `Rate`; a probe project containing `IMPORT money` and `LET r AS money::Rate = money::Rate[scaled := 842000000000]` followed by `io::print(toString(r.scaled))` builds and prints `842000000000`.

Commit: —

### Phase A2 — `toRate(Integer)`

Tasks:
- Add `src/codegen/builtins/money/func_to_rate_integer.rs` with the lowering: range-check `n`, multiply by `RATE_SCALE` with the checked multiply the `Integer` operators use, return `Rate[scaled := …]`.
- Register it in `mod.rs` with its error declarations (`ErrInvalidArgument`, `ErrOverflow`).

Acceptance: `tests/rt-behavior/money/money_rate_integer/` prints `1000000000000` for `toRate(1).scaled`, `64000000000000` for `toRate(64).scaled`, and traps `ErrOverflow` for `toRate(9223373)`, `ErrInvalidArgument` for `toRate(0)` and `toRate(-1)`.

Commit: —

### Phase A3 — `toRate(String)`

Tasks:
- Add `src/codegen/builtins/money/func_to_rate_string.rs`. Parse the grammar in Design: integer digits, the fraction digits (at most 12, else `ErrInvalidFormat`), the optional `-`, then the range checks in the order `ErrInvalidFormat`, `ErrInvalidArgument` (value ≤ 0), `ErrOverflow`.
- Build the scaled value from the integer part times `RATE_SCALE` plus the fraction padded to 12 digits, with the checked add and multiply, so overflow is `ErrOverflow`.

Acceptance: `tests/rt-behavior/money/money_rate_string/` checks these `scaled` values: `"0.842"` → 842000000000, `"0.0067"` → 6700000000, `"1"` → 1000000000000, `"9223372.036854775807"` → 9223372036854775807, `"0.000000000001"` → 1.
`tests/rt-error/money/money_rate_string_bad/` traps with the expected code for each: `"0.0000000000001"` (13 places) → `77050003`; `"abc"` → `77050003`; `""` → `77050003`; `" 1"` → `77050003`; `"0"` → `77050002`; `"-1"` → `77050002`; `"9223372.036854775808"` → `77050010`.

Commit: —

### Phase A4 — `toString(Rate)`

Tasks:
- Add `src/codegen/builtins/money/func_to_string_rate.rs` as the package `toString` override for `money::Rate`, using the Design rule for trimming.

Acceptance: `tests/rt-behavior/money/money_rate_tostring/` prints `0.842` for `toString(toRate("0.84200"))`, `1` for `toString(toRate("1"))`, `0.0067` for `toString(toRate("0.0067"))`, and `0.000000000001` for `toString(toRate("0.000000000001"))`.

Commit: —

### Phase A5 — spec and man

Tasks:
- `src/docs/spec/stdlib/13_money.md`: add a section on `money::Rate` covering the scale, the grammar, the places and range rules, and the error codes. Cross-reference from the `Money` section.
- `src/codegen/builtins/money/mod.rs` `MODULE_DESC`: add a paragraph on `Rate`, since the man page is registry-driven (`mfb man money`).
- Check the description strings of the three new functions and the record, following `.ai/man-content.md`: no memory vocabulary, no internal names.

Acceptance: `target/release/mfb man money` renders the new paragraph and lists the three new members; `target/release/mfb spec stdlib money` shows the new section.

Commit: —

### Phase A6 — gates

Tasks:
- Run the scoped byte-identity gate for the `money` built-in: `scripts/artifact-gate.sh target/release/mfb money`.
- Run the scoped acceptance tests for the new and existing money fixtures: `scripts/test-accept.sh target/release/mfb <outdir> 'money_*'`.

Expected: the byte-identity fixtures under `tests/byte-identity/money/` may differ, because the injected money declarations gain `Rate` and the three new functions. A diff there is the plan working. The existing `money_*` acceptance fixtures pass unchanged. Any diff outside `tests/byte-identity/money/` is a bug and is root-caused with `objdump` on one fixture before any re-baseline.

Commit: —

## 5. Validation Plan

Landing tier, run once at the end of this sub-plan: A6's two commands. The full suite runs in the background under the project's policy.

## 6. Decisions (owner, recorded as fact)

- The rate is one `Integer` scaled by 10^12 in a built-in record `money::Rate`; no new scalar type.
- `Fixed` is unchanged.
- `toRate` has two overloads, `String` and `Integer`; no `Float` or `Fixed` overloads.
- `toRate(String)` rejects more than 12 decimal places with `ErrInvalidFormat`; it does not round.
- Rates must be above zero and at most `9223372.036854775807`.
- Invalid `Rate` values built directly are checked on use, not prevented by compiler visibility.
- `toString(Rate)` prints trimmed decimal text.
