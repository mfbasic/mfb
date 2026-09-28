# bug-762: a Float operation over a call the pre-pass cannot type fails to build ("has no data object")

Last updated: 2026-09-28
Effort: small
Severity: HIGH
Class: Build failure (valid program rejected at every -O level)

Status: FIXED — the pre-pass types a call as the lowering does; 8 RED
fixtures green.
Regression Tests: `tests/rt-behavior/arithmetic/bug762-*` (8 fixtures, listed below)

**The single correct behavior:** a module whose lowering emits a Float finiteness
check (`ErrFloatOverflow`/`ErrFloatNaN`/…) or a Float-into-exact `ErrInvalidFormat`
check registers that message's string data object, whatever the shape of the
operands — so every valid program builds.

## Reproduction

```
IMPORT io
FUNC mk(times AS List OF Integer) AS Float
  RETURN toFloat(len(times)) / 2.0
END FUNC
FUNC main() AS Nothing
  io::print(toString(mk([5, 7]), toByte(6)))
END FUNC
```

`mfb build .` (any `-O`) → `error: native code string literal 'Floating-point
arithmetic overflowed to infinity.' has no data object while lowering return`.
Binding `len(times)` to a `LET` first builds. Expected: prints `1.000000`.

The failure needs the offending operation to be the module's ONLY Float operation of
its kind — the message set is registered module-wide, so any plainly typed one
(`-1.0`, `a * b` over Float locals) masks it. Sweep at main (`45f567835`, debug
build), each shape alone in a module:

| Shape | Message | Before | After |
| --- | --- | --- | --- |
| `toFloat(len(xs)) / 2.0` | FloatOverflow | build error | `1.000000` |
| `-toFloat(len(s))` | FloatOverflow | build error | `-3.00` |
| `toFloat(len(os::args())) * 2.0` | FloatOverflow | build error | `0.00` |
| `math::abs(a) * a` (also `max`/`min`/`clamp`/`floor`) | FloatOverflow | build error | `2.25` |
| `f(2) * a`, `f` a non-inlined user `FUNC … AS Float` | FloatOverflow | build error | `4.50` |
| `g(a) * a`, `g AS FUNC(Float) AS Float` parameter | FloatOverflow | build error | `2.25` |
| `q * math::abs(x)`, `q AS Fixed` | InvalidFormat | build error | `3.75` |
| `m * collections::sum(xs)`, `m AS Money` | InvalidFormat | build error | `8.75` |

## Root cause

`string_symbols` (`src/codegen/memory/data/data_objects.rs`) registers the four Float
error messages only if `module_may_emit_float_numeric_error`
(`src/codegen/engine/analysis/module_analysis.rs`) finds a Float arithmetic node,
typing operands with `static_nir_value_type`
(`src/codegen/engine/types/type_utils.rs`). The lowering itself types with the
builder's own oracle and emits the check regardless. `static_nir_value_type`'s call
arm under-typed:

1. It resolved arg-typed returns only for `builtin_package_name(target) ∈
   {general, collections, strings}`. The `general` arm was dead: `general` members are
   bare names (`len`), `Registry::resolve_func` splits on `.`, so the owner lookup
   never answers `general`. `len(xs)` typed `None`, and the `?` over argument types
   then sank `toFloat(...)` (whose nominal return IS `Float`) and the whole binary.
   Introduced by plan-72-BB (`73acc0b61`), which replaced a direct
   `general::resolve_call` with that gate.
2. Every other package got only the static nominal return, which an overloaded member
   (`math::abs`) lacks.
3. `RuntimeCall` (`os::args()`), user function calls, and calls through a
   `FUNC`-typed local returned `None`.

The `ErrInvalidFormat` gate (`binary_may_consume_float_into_exact`) typed operands
only with `static_type_name_with_types`, whose hand table has no overloaded builtin.

## Fix

- `static_nir_value_type` call arm (`Call`/`CallResult`/`RuntimeCall`): a `FUNC`-typed
  local answers its return; otherwise every builtin resolves through
  `resolve_call_return_type_typed`, then the static nominal (also when an argument is
  untyped), then the module function's declared return (`FieldTypes::function_returns`).
  `module_may_emit_float_numeric_error` now builds its `FieldTypes` with
  `with_function_returns`. `static_nir_value_type`'s only consumers are these two
  pre-pass predicates, so widening it only adds registrations.
- `binary_may_consume_float_into_exact` falls back to `static_nir_value_type` when the
  hand table has no answer. The table itself is not widened (its other consumers
  keep their answers — `static_type_name_for_fold_with_types` docs).

## Non-goals

- No over-approximation (registering the Float messages for every module): that
  would shift every golden's data section.
- `static_type_name_with_types` / `CodeBuilder::static_type_name` are not changed
  (`.ai/compiler.md`: the three walks are deliberately distinct).

## Blast-radius audit

- The other message gates in `string_symbols` key on call names
  (`module_uses_any_call`), not on operand typing — unaffected.
- `value_may_emit_float_arithmetic_error`'s builtin-result arm already consults
  `static_nir_value_type(value)`; it now types `RuntimeCall`s too — only adds.
- Remaining `None` arms (`UnionWrap`, `Closure`) are never Float-valued.

## Verification

- RED: with `type_utils.rs`/`module_analysis.rs` restored to main (`45f567835`) and
  rebuilt, all 8 fixtures fail with "… has no data object while lowering …";
  with the fix they build and print the values in the table above. The repro
  fixture (`bug762-float-arith-on-general-call-rt`) builds and runs identically at
  `-O0`..`-O3`.
- `cargo test --no-fail-fast`: 4523 unit + every integration target pass; two
  timing-ratio tests (`rt_compress_bounds::decode_time_is_linear_in_output_size`,
  `rt_list_set_widening_linear`) failed once at load average ~40 and passed on
  three consecutive reruns.
- `scripts/artifact-gate.sh <release mfb> all`: 2198 goldens, 0 diffs.
- `scripts/test-accept.sh`: 1579 tests; 3 run-output mismatches under load
  (`csv-behavior`, `bug159_listdir_notdir_error`, `file-buffered-drain-integrity-rt`,
  fs "path does not exist" — they share temp paths with parallel runs) pass on a
  filtered rerun.
