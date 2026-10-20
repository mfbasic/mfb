# plan-181-C: currency moves onto money::Rate

Last updated: 2026-10-19
Effort: medium (1h–2h)
Depends on: plan-181-A and plan-181-B (both must be complete)

This sub-plan changes the `currency` package's rates table from `Fixed` to
`money::Rate`, and routes its cross-currency arithmetic through `money::convert`.
The `Amount` record, the ISO list, the code rules, the error codes and every
`Amount` operation's signature are unchanged. The package is 0.1.0 and
unreleased, so the rate API breaks without a migration path.

References:

- `planning/plan-181-A-money-rate.md`, `planning/plan-181-B-money-convert.md`
- `packages/currency/src/rates.mfb` (the table and its functions)
- `packages/currency/src/math.mfb` (`inCurrency`, `add`, `subtract`, `compare`, `convert`)
- `packages/currency/src/core.mfb` (the error codes and `Amount`)
- `packages/currency/README.md` (the API table, the rates section, the thread example)
- `packages/currency/runtime-smoke.sh`, `packages/currency/check-doc-examples.sh`
- `.ai/testing-gates.md` (the golden rules: a test expectation is never changed to pass)

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-181-A complete: `money::toRate` exists | `target/release/mfb man money \| grep -c 'toRate'` → at least 2 | NOT MET (plan-181-A not yet implemented, 2026-10-19) |
| plan-181-B complete: `money::convert` exists | `target/release/mfb man money \| grep -c 'convert'` → at least 1 | NOT MET (plan-181-B not yet implemented, 2026-10-19) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue.

## 1. Goal

- `currency::setRate(code AS String, rate AS money::Rate)`, `currency::getRate(code AS String) AS money::Rate`, `currency::exportRates() AS Map OF String TO money::Rate` and `currency::importRates(table AS Map OF String TO money::Rate)` replace their `Fixed` forms.
- `currency::hasBaseCurrency` is true when some code's rate has `scaled` equal to `10^12` (the rate 1), exactly.
- `currency::inCurrency`, and so `add`, `subtract`, `compare` and `convert`, compute with `money::convert`, with the error order unchanged: a missing rate is reported before a missing base currency.
- The package's own suite, its doc examples and its runtime smoke test all pass.

### Non-goals (explicit constraints)

- **`Amount` is unchanged**: its fields, `currency::amount`, and the rule that every `Amount` operation checks its code.
- **The ISO list, the code rules and `minorUnits` are unchanged.** `data.mfb` is generated and is not edited.
- **Error codes are unchanged**: `93170001` through `93170004` keep their numbers and their meanings.
- **`Fixed` is not used by `currency` at all after this change**, so no `currency` function takes or returns `Fixed`.
- **Threading is unchanged**: the table is per thread, `exportRates` returns a copy, and `importRates` replaces the whole table atomically.
- **No expected test value changes.** The test edits in this sub-plan change rate literals and the call forms only. Each expected value is the exact decimal already written in its comment; none is re-baselined.

## 2. Current State

- **The table.** `packages/currency/src/rates.mfb`: `PRIVATE MUT rates AS Map OF String TO Fixed`, with `checkRate(code AS String, rate AS Fixed)` rejecting `rate <= 0F` (`ErrorInvalidRate`), and `setRate`, `removeRate`, `clearRates`, `hasRate`, `getRate AS Fixed`, `hasBaseCurrency` (which compares `rate = 1F`), `exportRates`, `importRates` and `rateOf AS Fixed`.
- **The conversion.** `packages/currency/src/math.mfb:inCurrency` computes `value * sourceRate / targetRate` on `Money` and `Fixed`, after `rateOf(target)` and `rateOf(source)` (in that order, so the missing-target error is reported first) and the base-currency check.
- **Uses of `Fixed`.** Measured: `cat packages/currency/src/*.mfb | grep -c "Fixed"` → 50, in `core.mfb` (1), `math.mfb` (6), `rates.mfb` (11), `test_rates.mfb` (22) and `test_math.mfb` (10). `data.mfb` (0) and `test_codes.mfb` (0) do not use it.
- **The test counts.** `grep -c TCASE` counts both the opening and the `END TCASE` line: `test_rates.mfb` 44 → 22 cases, `test_codes.mfb` 34 → 17, `test_math.mfb` 76 → 38, total 77, matching `target/release/mfb test packages/currency` → 77 pass.
- **Documentation.** `packages/currency/README.md` has the API table (`setRate` through `importRates`), the rates-table section with `setRate("JPY", 0.0067F)`-style examples, and the threading example using `importRates(saved)` with a `Map OF String TO Fixed`.
- **The gates.** `bash packages/currency/check-doc-examples.sh target/release/mfb` builds and runs the 12 `EXAMPLE` blocks and compares each `io::print` line against its trailing comment. `sh packages/currency/runtime-smoke.sh target/release/mfb` builds a consumer and checks three output lines. Run `check-doc-examples.sh` with `bash`, not `sh`: it uses process substitution.

## 3. Design

- **Table type.** `PRIVATE MUT rates AS Map OF String TO money::Rate`. Keys stay upper case, canonicalised by `checkedCode`.
- **Validation.** `setRate` and `importRates` reject a rate whose `scaled` is at or below zero with `ErrorInvalidRate`. This is the check-on-use rule from plan-181-A: a rate built as `money::Rate[scaled := 0]` is caught here, not stored.
- **Base currency.** A rate is the base when `scaled` equals `1000000000000`. `money::toRate("1")` is that value, so the README's base-currency examples use it.
- **Arithmetic.** `inCurrency(value, source, target)` returns `value` when the codes are equal, and otherwise `money::convert(value, rateOf(source), rateOf(target))`. The `rateOf` lookups and the base check keep their current order, so the existing error tests keep their expectations.
- **Test edits.** Each `Fixed` literal `1.25F` becomes `money::toRate("1.25")`. Each `setRate("USD", 1F)` becomes `setRate("USD", money::toRate("1"))`. Expectations stay as written: the comments already give exact decimal results, and every rate used is exact in both forms.
- **README and doc examples.** Each `EXAMPLE` is rewritten to the new call forms. Its trailing expected output is kept unless the example prints a rate: `toString(currency::getRate("EUR"), toByte(2))` becomes `toString(currency::getRate("EUR"))`, and it still prints `1.08`.

### Rejected alternatives

- **Keeping a `Fixed` overload of `setRate`.** Rejected: a `Fixed` rate reintroduces the binary rounding this change removes.
- **A compatibility layer for `Fixed` callers.** Rejected: the package is unreleased, and a layer would leave `Fixed` in the public API.
- **Storing `Money` rates.** Rejected: a rate below 0.00001 would not be representable.

## 4. Phases

### Phase C1 — the rates table

Tasks:
- In `packages/currency/src/rates.mfb`: change the table's value type to `money::Rate`; change `checkRate` to test `rate.scaled <= 0` and fail with `ErrorInvalidRate`; change the signatures of `setRate`, `getRate`, `exportRates`, `importRates` and `rateOf`; change `hasBaseCurrency` to compare `rate.scaled` with `1000000000000`.
- Add `IMPORT money` to the file's import list, next to `IMPORT collections`.

Acceptance: `target/release/mfb test packages/currency` builds (the test files are updated in C3).

Commit: —

### Phase C2 — cross-currency arithmetic

Tasks:
- In `packages/currency/src/math.mfb`: rewrite `inCurrency` to call `money::convert` as in Design, leaving the lookup order and the `ErrorNoBaseCurrency` check unchanged.
- Remove every `Fixed` reference from `math.mfb` (the 6 measured).

Acceptance: `grep -c "Fixed" packages/currency/src/math.mfb` → 0.

Commit: —

### Phase C3 — the package's tests

Tasks:
- `packages/currency/src/test_rates.mfb`: change each `Fixed` literal to `money::toRate("…")` and each `expectFixed(getRate(…), 1.25F)` to compare with `money::toRate("1.25")`. Keep every expected value.
- `packages/currency/src/test_math.mfb`: the same change, for the 10 measured `Fixed` references.
- Add `test_rates` cases: `getRate("EUR")` after `setRate("EUR", money::toRate("0.842"))` prints `0.842` through `toString`; `setRate` with `money::Rate[scaled := 0]` traps `ErrorInvalidRate`; `importRates` with that record traps `ErrorInvalidRate`; the USD→JPY conversion of `10000000000m` at the JPY rate `"0.0067"` now succeeds at `1492537313432.83582`.

Acceptance: `target/release/mfb test packages/currency` → all pass, with the count equal to 77 plus the cases added here. `grep -c "Fixed" packages/currency/src/*.mfb` → 0 in every file.

Commit: —

### Phase C4 — README and doc examples

Tasks:
- `packages/currency/README.md`: change the API table's rate rows to `money::Rate`; rewrite the rates section's examples with `money::toRate`; rewrite the threading example's worker signature to `Map OF String TO money::Rate`; state the 10^12 scale and the 9223372.036854775807 limit in the rates section; remove every statement that the rates are `Fixed` or binary.
- Rewrite the `EXAMPLE` blocks in `src/*.mfb` that call `setRate`, `getRate` or `importRates` to the new call forms, keeping each trailing expected output.

Acceptance: `bash packages/currency/check-doc-examples.sh target/release/mfb` prints `all 12 example(s) built and ran`.

Commit: —

### Phase C5 — runtime smoke

Tasks:
- `packages/currency/runtime-smoke.sh`: change its consumer program's `setRate("USD", 1F)` and `setRate("EUR", 1.25F)` to `money::toRate` forms and keep its three expected lines.

Acceptance: `sh packages/currency/runtime-smoke.sh target/release/mfb` prints `currency runtime smoke passed`.

Commit: —

## 5. Validation Plan

Landing tier, run once at the end of this sub-plan:
- `target/release/mfb test packages/currency`
- `bash packages/currency/check-doc-examples.sh target/release/mfb`
- `sh packages/currency/runtime-smoke.sh target/release/mfb`
- `sh scripts/check-generated.sh` (confirms `data.mfb` is untouched)

The compiler gates for the `money` built-in run in sub-plan A6 and B5. This sub-plan adds no compiler change, so the full suite runs in the background under the project's policy.

## 6. Decisions (owner, recorded as fact)

- The rate API breaks now; there is no `Fixed` overload and no compatibility layer.
- Rates are `money::Rate`, with `setRate`, `getRate`, `exportRates` and `importRates` changed to match.
- Error codes and `Amount` are unchanged.
