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
| A release build of the compiler exists | `test -x target/release/mfb` | MET (2026-10-20, in `.claude/worktrees/P-181` after `cargo build --release`) |
| The currency package suite passes on the unchanged tree | `target/release/mfb test packages/currency` | MET (2026-10-20: `Tests: 77  Pass: 77  Fail: 0`) |

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
- [x] In `src/codegen/builtins/money/mod.rs`, add `pkg.add_record(RegistryRecord { name: RATE_TYPE, export: true, props: [RecordProp { name: "scaled", ty: ParameterType::Integer, description: ... }] })`, following `big/mod.rs`'s `INT_TYPE` declaration, with a comment that the single field's meaning (value × 10^12) is a contract. — `target/release/mfb man money types` renders `money::Rate` with its `scaled │ Integer` row.
- [x] Add the constant `RATE_SCALE: i64 = 1_000_000_000_000` in the same module. — used by `func_scale.rs:lower_money_scale` (B3).
- [x] Unit tests (added): `money::tests::rate_is_a_builtin_record_and_to_rate_returns_it` and the companion-source asserts in `money_registered_on_the_clean_room_registry` — `cargo test --release --bin mfb -- money citations_resolve` → `84 passed; 0 failed`.

Acceptance (corrected, see Corrections 1): `target/release/mfb man money types` lists `money::Rate`; a probe project containing `IMPORT money` and `LET r AS money::Rate = money::Rate[scaled := 842000000000]` followed by `io::print(toString(r.scaled))` builds and prints `842000000000`. — MET: probe `/tmp/p181/a1` printed `842000000000` (and `TRUE` for `r = money::Rate[scaled := 842000000000]`).

Commit: d13d8bf4a

### Phase A2 — `toRate(Integer)`

Tasks:
- [x] Add `src/codegen/builtins/money/func_to_rate_integer.rs`: range-check `n`, multiply by 10^12 with the checked `Integer` `*`, return `Rate[scaled := …]`. Written as an MFBASIC source body (`__money_toRateInteger`), not a native lowering — Corrections 2.
- [x] Register it with its error declarations (`ErrInvalidArgument`, `ErrOverflow`, `ErrOutOfMemory`) — as the `Integer` overload of the one `toRate` member, registered by the added `func_to_rate.rs` (Corrections 3).

Acceptance: `tests/rt-behavior/money/money_rate_integer/` prints `1000000000000` for `toRate(1).scaled`, `64000000000000` for `toRate(64).scaled`, and traps `ErrOverflow` for `toRate(9223373)`, `ErrInvalidArgument` for `toRate(0)` and `toRate(-1)`. — MET: golden run prints `1 1000000000000`, `64 64000000000000`, `9223372 9223372000000000000`, `9223373 error 77050010`, `0 error 77050002`, `-1 error 77050002` (`scripts/test-accept.sh target/release/mfb /tmp/p181/accept 'money_*'`).

Commit: d13d8bf4a

### Phase A3 — `toRate(String)`

Tasks:
- [x] Add `src/codegen/builtins/money/func_to_rate_string.rs` (MFBASIC source body `__money_toRateString`, Corrections 2). Parse the grammar in Design: integer digits, the fraction digits (at most 12, else `ErrInvalidFormat`), the optional `-`, then the range checks in the order `ErrInvalidFormat`, `ErrInvalidArgument` (value ≤ 0), `ErrOverflow`. A `.` needs a digit on each side (`".5"`, `"1."` are `ErrInvalidFormat`) — Corrections 4.
- [x] Build the scaled value from the integer part times 10^12 plus the fraction padded to 12 digits. The whole part stops accumulating past `9223372` (so a long digit run cannot overflow the accumulator) and the final add is guarded by an explicit comparison, so overflow is `ErrOverflow` from the range rule, not from a trapped operator.
- [x] Added: `tests/syntax/money/money_rate_invalid/` rejects `toRate(Float)`, `toRate(Fixed)`, both wrong arities and a `String` `scaled` (`.ai/compiler.md` requires an invalid fixture per new function).

Acceptance: `tests/rt-behavior/money/money_rate_string/` checks these `scaled` values: `"0.842"` → 842000000000, `"0.0067"` → 6700000000, `"1"` → 1000000000000, `"9223372.036854775807"` → 9223372036854775807, `"0.000000000001"` → 1.
`tests/rt-error/money/money_rate_string_bad/` traps with the expected code for each: `"0.0000000000001"` (13 places) → `77050003`; `"abc"` → `77050003`; `""` → `77050003`; `" 1"` → `77050003`; `"0"` → `77050002`; `"-1"` → `77050002`; `"9223372.036854775808"` → `77050010`. — MET: golden run prints each value above, and the bad fixture prints `[0.0000000000001] 77050003` … `[-1] 77050002` (plus `0.8420000000000`, `1 `, `+1`, `1e3`, `.5`, `1.`, `-`, `1.2.3` → `77050003`; `0.000`, `-99999999999999999999` → `77050002`; `9223373`, `99999999999999999999999` → `77050010`) and then traps uncaught with `Error: 7-705-0010` on `"9223372.036854775808"`, `[exit 255]`. One program can trap only once, so every case but the last is caught with `TRAP` and its code printed.

Commit: d13d8bf4a

### Phase A4 — `toString(Rate)`

Tasks:
- [x] Add `src/codegen/builtins/money/func_to_string_rate.rs` as the package `toString` override for `money::Rate` (`add_override` → helper `__money_rateToString`, the `toString(color::Color)` mechanism), using the Design rule for trimming.

Acceptance: `tests/rt-behavior/money/money_rate_tostring/` prints `0.842` for `toString(toRate("0.84200"))`, `1` for `toString(toRate("1"))`, `0.0067` for `toString(toRate("0.0067"))`, and `0.000000000001` for `toString(toRate("0.000000000001"))`. — MET: golden run prints those four, then `9223372.036854775807`, `64`, `1.5`, `0` (hand-built zero), `-0.000000000005`, `-9223372.036854775808` (the `Integer` minimum, which cannot be negated before the split), and `TRUE` for `toRate("0.50") = toRate("0.5")`.

Commit: d13d8bf4a

### Phase A5 — spec and man

Tasks:
- [x] `src/docs/spec/stdlib/13_money.md`: add a section on `money::Rate` covering the scale, the grammar, the places and range rules, and the error codes. Cross-reference from the `Money` section. — new `## money::Rate` section, a paragraph in "The exact base-10 model", a **Rates** item in "Error and overflow model", See Also rows.
- [x] `src/codegen/builtins/money/mod.rs` `MODULE_DESC`: add a paragraph on `Rate`, since the man page is registry-driven (`mfb man money`). — **Exact rates.** paragraph; `MODULE_INTRO` now "Rounding-mode control and exact rates for Money arithmetic".
- [x] Check the description strings of the new function, its two overloads' parameters and the record, following `.ai/man-content.md`: no memory vocabulary, no internal names. Bullets are one source line each (a wrapped bullet renders broken, seen on the first render).
- [x] Added: every example on the money pages builds and runs — `MFB=target/release/mfb bash scripts/man-run-examples.sh money --run` → `examples: 11   built: 11   ran: 11   not run: 0   failed: 0`, output matching each example's text.

Acceptance (corrected, Corrections 1): `target/release/mfb man money` renders the new paragraph and lists `money::toRate` (its page shows both overloads); `target/release/mfb man money types` lists `money::Rate`; `target/release/mfb spec stdlib money` shows the new section. — MET: all three rendered and read.

Commit: d13d8bf4a

### Phase A6 — gates

Tasks:
- [x] Run the scoped byte-identity gate for the `money` built-in: `scripts/artifact-gate.sh target/release/mfb money` → after the re-baseline below, `1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)`.
- [x] Run the scoped acceptance tests for the new and existing money fixtures: `scripts/test-accept.sh target/release/mfb /tmp/p181/accept 'money_*'` → `acceptance tests passed (15 test(s) ran)`; the one money importer the glob misses, `money-min-integer-divisor-rounding`, → `acceptance tests passed (1 test(s) ran)`.
- [x] Added: before re-baselining, the full `scripts/artifact-gate.sh target/release/mfb all` → `1715 tests, 1894 build(s), 2378 golden(s) checked, 13 diff(s)`, and a full `scripts/test-accept.sh target/release/mfb /tmp/p181/accept-all` → `8 mismatch(es) (1749 test(s) ran)`. Every diff is a fixture that imports `money`: 8 `.ir` (each, with `"line"` numbers normalised, differs from its golden by the same 259 added lines and no removed or changed line — the companion) and the 5 `.ncodesum` of `tests/byte-identity/money` (new companion functions, Corrections 5). No `build.log` (run output) changed. Re-baselined: the 8 `.ir` from the run's actuals, the 5 sums with `bash scripts/regen-native-goldens.sh target/release/mfb tests/byte-identity/money` → `5 golden(s) rewritten, 0 failure(s)`.
- [x] Added: the unit census `field_kind_census_covers_every_record_field_type` failed on the full `cargo test --release --bin mfb` run (`5029 passed; 1 failed`): `money.Rate (InlinedFixed) has no FIELD_KIND_TABLE row`. Classified `Arm('F')` like `color.Color` (one inline scalar); `tests/runtime/inplace_self_update/field_kinds.tsv` regenerated with its generator (`python3 …/field_kinds_gen.py target/release/mfb`, one added `money::Rate` line; `field_expect_gen.py` output unchanged). `cargo test --release --test inplace_self_update_census` → `3 passed`; `MFB_SELF_UPDATE_FILTER='money::Rate' cargo test --release --test rt_inplace_self_update -- --ignored` → `every_field_kind_meets_its_expectation ... ok`, `every_via_let_field_cell_meets_its_expectation ... ok` (the third test, collection cases only, refuses a filter matching none of its cases by design); `cargo test --release --bin mfb -- field_kind_census money citations_resolve` → `85 passed; 0 failed`.

Expected (corrected, Corrections 5): the byte-identity fixtures under `tests/byte-identity/money/` differ, because the injected money declarations gain `Rate` and its source bodies. The existing `money_*` acceptance fixtures keep their run output, and their `.ir` gains the same companion text. Any diff in a fixture that does not import `money` is a bug and is root-caused on one fixture before any re-baseline.

Commit: d13d8bf4a

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

## Corrections

1. **`mfb man money` does not list records.** A1/A5 said `mfb man money` lists `Rate`, and plan-181-B/C's prerequisites grep `mfb man money | grep -c '^  *Rate'`. Measured: after A1, `target/release/mfb man money | grep -c '^  *Rate'` → `0`; the package page lists functions only and its See also points to `mfb man money types`, which renders `money::Rate` with its field table. Acceptance corrected to `mfb man money types`; B's prerequisite rows corrected to match.
2. **`toRate` and `toString(Rate)` are MFBASIC source bodies, not native lowerings.** The plan said "lowering" for A2/A3. A built-in package can carry source bodies (`Body::mfb`; `color::fromHex` parses text this way, `toString(color::Color)` is a source override), and an `Integer`-only parse with the checked `*`/`+` is exact by construction, so the source form meets every rule in Design with no binary step. `money`'s companion therefore imports `strings` (`toBytes`, `padLeft`). The native lowering is kept where it is needed: `convert`/`scale` (plan-181-B) need a 128-bit product.
3. **One registry member per overload set.** `func_to_rate_integer.rs` and `func_to_rate_string.rs` each provide one `Implementation`; `toRate` is one `RegistryFunction` with two implementations, registered by the added `func_to_rate.rs`, which also holds the shared man text.
4. **A `.` needs a digit on each side.** The Design grammar ("digits, then optionally `.` and digits; at least one digit") was read strictly: `".5"` and `"1."` are `ErrInvalidFormat`, as is a `-` with no digits. Recorded in the spec section and covered by `money_rate_string_bad`.
5. **"The existing `money_*` acceptance fixtures pass unchanged" was a wrong prediction.** Inspected one: `diff tests/rt-error/money/money_round_bad_decimals/golden/money_round_bad_decimals.ir <actual>` — the added lines are exactly the injected companion (`"name": "money.Rate"` type, `#money_rateToString`, `#money_toRateString`, `#money_toRateInteger`) and the shifted `"line"` of `Rounding`; every program that imports `money` dumps the whole companion into its `.ir`. Their run output (`build.log`) is unchanged. The `.ncode` of `tests/byte-identity/money` also moves: `_mfb_ifn_money_5FrateToString`, `_mfb_ifn_money_5FtoRateInteger`, `_mfb_ifn_money_5FtoRateString` plus the runtime helpers they use (`_mfb_rt_int_to_string`, `_mfb_rt_string_concat`, `_mfb_ctor_Error`) are new symbols. That is the established model for source-bodied members — a one-call `color` program emits all 45 `_mfb_ifn_color_*` functions — and it costs a `money` importer 16,512 bytes of executable (`money_codegen_cover_rt.out` 66,808 → 83,320 bytes on macos-aarch64). Expected diffs, regenerated after the full gates (A6).
6. **A and B landed as one commit.** Both edit `money/mod.rs` (registration, `MODULE_DESC`, the unit tests) and share the golden re-baseline, and the gates were run once on the A+B compiler; splitting them would have meant committing goldens no binary produced. Every A and B phase's `Commit:` line names that one commit.
7. **A new built-in record needs a self-update census row.** Not in the plan: `FIELD_KIND_TABLE` (`self_update/tests/census/mod.rs`) must classify every record field kind, and `tests/runtime/inplace_self_update/field_kinds.tsv` is generated from every `mfb man <pkg> types` record. Both gained `money.Rate` (A6).
