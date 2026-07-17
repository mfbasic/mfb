# bug-265: `ir::verify` literal-range check omits `Scalar` → out-of-range/surrogate Scalar const in a crafted `.mfp` escapes verification

Last updated: 2026-07-17
Effort: small (<1h)
Severity: LOW
Class: Security (robustness / verifier gap)

Status: Open
Regression Test: (none yet)

`verify_semantics` range-checks numeric const literals in a merged (possibly
untrusted) package, but its `numeric` predicate omits `Scalar`. A source-level
Scalar (backtick) literal is range-checked at parse time, but a hand-crafted
`.mfp` can carry `IrValue::Const { type_: "Scalar", value: <arbitrary decimal> }`
that `check_literal_range` never inspects — an out-of-range codepoint
(`> 0x10FFFF`), a UTF-16 surrogate (`0xD800..=0xDFFF`), or a value up to
`u64::MAX` passes `verify_semantics` unflagged. It is memory-safe (the immediate
encoder parses `u64` fallibly, and `emit_scalar_to_string_value` bounds its
writes), so the ceiling is a `String` holding malformed UTF-8 / a wrong
codepoint — a correctness/robustness gap, not a codegen-to-unsafe path. The single
correct behavior a fix produces: `verify_semantics` rejects an out-of-range
Scalar literal exactly as it rejects an out-of-range `Byte`/`Money`.

References:

- `planning/audit-2-package-decode.md` (PKG-08).
- `src/ir/verify/mod.rs:1678` — `let numeric = |t: &str| matches!(t, "Integer" |
  "Byte" | "Float" | "Fixed" | "Money")` — **`Scalar` absent** (verified current).
- `src/ir/verify/mod.rs:1695` `check_const_literal` (negated twin `:1782`).
- Consumers: `src/target/shared/code/builder_conversions.rs:686`
  (`emit_scalar_to_string_value`, 4-byte-bounded), `type_utils.rs:349`
  (`native_immediate_value`, fallible `u64` parse).
- Related-but-distinct: bug-190 added `Scalar` to `PRIMITIVE_TYPES` /
  `provably_data_type` (member-access confusion) — a *different* check; the
  literal-range `numeric` predicate was not updated.

## Failing Reproduction

Encode a package function returning `toString(<Scalar const = 0x110000>)` with the
const value string `"1114112"` (or a surrogate `"55296"`). Observed: builds clean,
emits a `String` with malformed UTF-8 / an out-of-range codepoint. Expected:
`verify_semantics` rejects the out-of-range Scalar literal with a
`TYPE_*_LITERAL_OVERFLOW`-class diagnostic, as it does for `Byte`/`Money`.

Contrast: an out-of-range `Byte`/`Money` const in the same position is already
rejected — only `Scalar` slips through.

## Root Cause

`check_literal_range`'s `numeric` predicate (`ir/verify/mod.rs:1678`) gates which
const types get range-checked, and `Scalar` is not in the set, so a Scalar const
never reaches `check_const_literal`. The parse-time Scalar range check does not
run on package-decoded IR, and `verify_semantics` is the sole rejecter on that
path.

## Goal

- `verify_semantics` rejects a `Scalar` const whose value fails to parse, exceeds
  `0x10FFFF`, or is a surrogate (`0xD800..=0xDFFF`), mirroring the existing
  `TYPE_*_LITERAL_OVERFLOW` diagnostics. Verifier-only.

### Non-goals (must NOT change)

- The Scalar wire id (`TYPE_SCALAR = 10`), `FIRST_TABLE_TYPE_ID = 20`, the
  const-pool encoding.
- Adding runtime UTF-8 re-validation of every String.

## Fix Design

Add `"Scalar"` to the `numeric` predicate at `verify/mod.rs:1678` and add a
`"Scalar"` arm to `check_const_literal` / `check_negated_const_literal` that
parses the decimal and rejects parse-failure, `> 0x10FFFF`, and surrogates. Add a
`tests/` (or `ir/verify/tests.rs`) case asserting the diagnostic on a
crafted-const path, matching the `merge_packages` entry point.
