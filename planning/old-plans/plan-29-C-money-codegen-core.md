# plan-29-C: Money type — native codegen storage, add/sub/compare

Last updated: 2026-07-07
Effort: medium (1h–2h)

This sub-plan makes `Money` a real, runnable 8-byte scalar in native codegen: storage
class, collection element tag, literal immediate materialization, unary negate, the
`+`/`-` operators, and all six comparisons. Because `Money`'s carrier is a signed i64
(exactly like `Integer`), add/subtract/negate/compare are the **integer** paths with
overflow checking — no scaling. It depends on plan-29-A (typing) and plan-29-B
(constants). After this sub-plan a program that binds, adds, subtracts, negates,
compares, and defaults `Money` values **runs on both backends** and produces the
correct raw results. (`toString`/print of a Money is plan-29-G; this sub-plan proves
behavior by comparing raw i64s via `toInt`-of-Money once G lands, or by an
intermediate integer-equality assertion in a test harness — see Acceptance.)

It complements:

- `./mfb spec memory scalar-storage` (a new 8-byte `Money` payload row)
- `./mfb spec memory collections` (Money as an 8-byte collection element)

## 1. Goal

- `Money` is an 8-byte / 8-align scalar `StorageClass`, stored and moved exactly like
  `Integer`/`Fixed` in locals, globals, records, and collections.
- A `Money` literal materializes to its raw i64 immediate (via
  `money_raw_from_decimal`).
- `Money + Money`, `Money - Money`, unary `-Money`, and `Money`↔`Money` comparisons
  (`= <> < > <= >=`) emit checked-integer code and produce correct raw results on
  aarch64 and x86-64. `MUT m AS Money` defaults to raw `0`. (The front end (plan-29-A)
  already rejects `Money ± scalar` and `Money`-vs-scalar comparisons, so codegen only
  ever sees Money-with-Money here — no operand promotion is involved.)

### Non-goals (explicit constraints)

- No `*`, `/`, `MOD` (scalar scaling / ratio / remainder) — those land in plan-29-E
  (Money↔Integer/Byte, `M / M`, `M MOD M`) and plan-29-F (Money↔Float/Fixed). No `^`
  (rejected in the front end, plan-29-A).
- No conversions or `toString` (plan-29-G).
- `Money` add/sub/negate/compare **must reuse the existing checked-Integer emitters**,
  not new kernels — the carrier is identical to `Integer`. No new overflow codes
  (`ErrOverflow` 77050010 / `ErrUnderflow` 77050011 as Integer already uses).
- No change to `Integer`/`Fixed` storage, layout, or goldens.

## 2. Current State

Storage class: `StorageClass` enum `src/target/shared/plan/mod.rs:130-135,332`; the
Fixed row `type_ == "Fixed" => (StorageClass::Fixed, 8, 8)` at
`src/target/shared/plan/lower.rs:164-165`. Collection element tag:
`COLLECTION_TYPE_FIXED = 5` at `src/target/shared/code/error_constants.rs:437`, mapped
from the type name at `type_utils.rs:70,87`; 8-byte slots in
`builder_collection_layout.rs:48,1450,1548,1691`; by-value scalar handling in
`builder_value_semantics.rs:58` and `builder_arena_transfer.rs:211`; signed-i64 lane
compare in `builder_collection_compare.rs:81,207,307,407`.

Immediate: `native_immediate_value` (`type_utils.rs:278-291`) dispatches
`"Fixed" => fixed_raw_from_decimal(value)`; materialized at `builder_values.rs:221`.

Arithmetic dispatch: `lower_arithmetic_binary` (`builder_numeric.rs:88`) computes the
result type (`numeric::binary_result_type`) and switches (`:183-227`) —
`Byte|Integer => emit_integer_binary` (`:830`), `Fixed => emit_fixed_binary` (`:915`),
`Float => emit_float_binary`. Unary negate at `:298-299` (INT64_MIN check). Comparison
promotion at `:587-616`. The integer emitter already does checked add/sub/negate and
signed compares — Money reuses it verbatim.

Numerous scalar-type `matches!` arms across `src/target/shared/**` (e.g.
`data_objects.rs:335`, and the many `"Boolean"|"Byte"|…|"Fixed"|…` enumerations the
census flagged) must gain `"Money"` to treat it as an 8-byte scalar.

## 3. Design Overview

Money is "Integer with a scale tag". For everything that does not involve the scale —
storage, movement, add, subtract, negate, compare, default — Money is byte-for-byte an
Integer. So this sub-plan is almost entirely: **add `"Money"` to the arms that
currently say `"Fixed"` or `"Integer"` for 8-byte-scalar handling, and route
Money arithmetic to `emit_integer_binary`** (not `emit_fixed_binary`, which would
apply Q32.32 scaling). The correctness risk is small but wide: missing one scalar-type
match arm makes Money mis-handled in a specific container/transfer path.

## 4. Detailed Design

### 4.1 Storage & element tag
- `StorageClass`: add a **distinct `Money` variant** (decided — not a reuse of
  `Integer`'s class), so the immediate/const-fold path selects `money_raw_from_decimal`
  cleanly and future divergence (e.g. display) stays localized. `plan/lower.rs`:
  `type_ == "Money" => (StorageClass::Money, 8, 8)`.
- `error_constants.rs`: `COLLECTION_TYPE_MONEY = <next free tag>`; `type_utils.rs:70,87`
  map `"Money" =>` it; `builder_collection_layout.rs` 8-byte slot; compare as signed
  i64 lanes (`builder_collection_compare.rs`); by-value scalar
  (`builder_value_semantics.rs:58`, `builder_arena_transfer.rs:211`).
- Sweep every scalar-type enumeration in `src/target/shared/**` that lists
  `"Fixed"`/`"Integer"` for 8-byte-scalar treatment and add `"Money"` (census listed
  the sites; grep `"Fixed"` across `src/target/shared` and add `"Money"` beside each
  8-byte-scalar arm).

### 4.2 Immediate
- `native_immediate_value` (`type_utils.rs:291`): `"Money" => money_raw_from_decimal(value)`.
- Const-fold passthrough copies for scientific-notation literals
  (`validate.rs:552-557`, `plan/symbols.rs:658-663`,
  `builder_value_semantics.rs:571-576`): add `"Money"` where `"Fixed"` is listed so a
  Money constant folds identically.

### 4.3 Arithmetic dispatch (`builder_numeric.rs`)
- `lower_arithmetic_binary` switch (`:183-227`): route result type `"Money"` with a
  `+`/`-` operator to `emit_integer_binary` (the Money `+`/`-` result type only ever
  arises from a Money-with-Money pair, per plan-29-A). `*`/`/`/`MOD` with a Money
  operand are handled by the `emit_money_binary` dispatch introduced in plan-29-E —
  **land C, D (rounding), and E together** (decided) so `builder_numeric` never carries a
  half-built Money dispatch and the integer-arithmetic kernels have their rounding mode
  available.
- Unary negate (`:298-299`): Money uses the integer negate + INT64_MIN check (raw
  `-9223372036854775808` = `-92233720368547.75808`, the min Money, must negate-check
  like Integer).
- Comparison (`:587-616`): a Money-with-Money comparison compares the raws as signed
  i64 directly (same scale ⇒ raw order = value order); no promotion. Add `"Money"` to
  the compare-as-signed-integer arm. (A Money-vs-scalar comparison never reaches
  codegen — the front end rejected it.)

### 4.4 Default value
`MUT m AS Money` with no initializer → raw `0`. Money is defaultable (plan-29-A);
confirm the default-init codegen path treats Money as an 8-byte zero scalar (same arm
as Integer/Fixed).

## Layout / ABI Impact

`mfb spec memory scalar-storage` gains a `Money — 8 bytes` row; collections gain an
8-byte Money element. No existing scalar's size/align/tag changes; all current goldens
byte-identical. (Doc rows land in plan-29-G.)

## Phases

### Phase 1 — Storage, element tag, immediate, default
Money is an 8-byte scalar that stores, moves, defaults, and materializes its literal.

- [ ] `StorageClass::Money` + `plan/lower.rs` 8/8 row.
- [ ] `COLLECTION_TYPE_MONEY` + `type_utils.rs` mapping + collection layout/compare/
      value-semantics/arena-transfer arms.
- [ ] `native_immediate_value` Money + const-fold passthrough arms.
- [ ] Sweep `src/target/shared/**` scalar-type match arms for `"Money"`.
- [ ] Default-init path treats Money as 8-byte zero.

Acceptance: a program that binds `LET a AS Money = 1.25`, stores it in a `List OF
Money` and a record field, reads it back, and a `MUT m AS Money` default — compiles
and runs on both backends; a codegen/artifact-gate run is byte-stable across a
rebuild (determinism). Round-trip verified via a test that stores/loads and (once E
lands) prints, or via `toInt(m)` equality in the D/E harness.
Commit: —

### Phase 2 — add / sub / negate / compare
Money arithmetic and comparison run correctly.

- [ ] `builder_numeric.rs`: result-type `"Money"` → `emit_integer_binary` for `+`/`-`;
      unary negate INT64_MIN-checked; comparison as signed i64.
- [ ] Overflow: `Money + Money` past the i64 raw range fails with `ErrOverflow`
      (reuses the Integer checked-add path).
- [ ] Tests: `tests/rt-behavior/**` program computing e.g. `1.25m + 2.50m`,
      `10.00m - 3.33m`, `-x`, and every comparison, asserting results (via the plan-29-G
      `toString`/print once available; land the runtime proof in F and cross-reference,
      or use an integer-raw assertion harness here).

Acceptance: `Money ± Money`, `-Money`, and all six comparisons produce correct raw
results on aarch64 and x86-64; an overflowing add fails with `ErrOverflow`; existing
Integer/Fixed goldens unchanged. Verified by an executed program (not just golden
text) per the runtime-completion gate.
Commit: —

## Validation Plan

- Runtime proof: an executed `.mfb` program exercising bind/store/default/add/sub/
  negate/compare on both backends (the observable result via plan-29-G print; sequence
  C→D→E→F→G so the proof program grows).
- Codegen determinism: `scripts/artifact-gate.sh` (execution-free) for the codegen
  diff, then a full run for the runtime proof.
- Acceptance: `scripts/test-accept.sh …`.

## Open Decisions

- **`StorageClass::Money` — DECIDED: distinct variant** (not a reuse of `Integer`), so
  the immediate/const-fold path selects `money_raw_from_decimal` cleanly and future
  divergence stays localized. (§4.1)
- **Landing order — DECIDED: C, D (rounding), and E land together** (one commit or close
  succession) so `builder_numeric` never carries a half-built Money dispatch and the
  integer-arithmetic kernels have the rounding mode available. Split by effort for
  reviewability. (§4.3)

## Summary

Money is an 8-byte Integer-carrier for every non-scaling operation, so this sub-plan
is mostly "add `Money` to the scalar arms and route +/-/compare to the integer
emitters." The risk is breadth (a missed match arm), not depth; the artifact gate and
an executed proof catch regressions.
