# Audit 2 — Surface 1: Untrusted `.mfp` package decode + signature / IR verification

Last updated: 2026-07-14
Untrusted party: author of a `.mfp` artifact dropped on the dependency/cache path.
Must not: cause the compiler to trust unsigned/tampered bytes, inject
type-confused / linearity-violating IR that codegens to unsafe native code, or
crash/DoS the build via unbounded recursion, OOM, or size-arithmetic overflow.

Scope read: `src/binary_repr/{reader,sections,util,builder,mod}.rs`,
`src/target/package_mfp/mod.rs`, `src/manifest/{entry,package,mod}.rs`,
`src/target/shared/validate.rs`, `src/cli/build.rs`, `src/cli/resolve.rs`, plus
the reachable decode/merge/verify path (`src/ir/binary.rs`, `src/ir/verify/mod.rs`,
`src/target/shared/nir/lower.rs`) and immediate encoders.

## Verdict on prior audit-1 findings (re-verified against current code)

| ID | Prior sev | Verdict | Evidence |
|----|-----------|---------|----------|
| PKG-01 | CRITICAL | **FIXED** | `build_project` calls `verify_and_report_packages` before any decode/merge/lower (`src/cli/build.rs:223`); `classify_installed_package` (`build.rs:1139`) walks the full plan-23 §3.5 chain against the project-pinned `identKey` (never the file-embedded key, `build.rs:1162-1192`). Any broken link → `Tampered` fatal; unsigned remote requires `--unsigned`. |
| PKG-02 | CRITICAL | **FIXED** | `merge_packages` runs `crate::ir::verify_package` on each decoded package (`nir/lower.rs:87`) and `crate::ir::verify_semantics` on the merged IR before native lowering (`lower.rs:105`) — the same semantic verifier used on source-lowered IR. |
| PKG-03 | HIGH | **FIXED** | `MAX_DECODE_DEPTH = 256` with `enter()/leave()` in `decode_op` (`ir/binary.rs:808`), `decode_value` (`:1155`), `decode_link_expr` (`:428`). |
| PKG-04 | HIGH | **FIXED** | `decode_type_name` uses an `in_progress` HashSet (`reader.rs:675`) + `MAX_TYPE_GRAPH_DEPTH = 256` (`reader.rs:682`); writer mirror at `:1370`. |
| PKG-05 | MEDIUM | **FIXED** | `bounded_capacity(count, remaining, min_elem)` (`util.rs:81`) caps every count-driven allocation; `decode_vec` caps at `ir/binary.rs:471`. The one bare `Vec::with_capacity` (`reader.rs:605`) is pre-bounded by the `entries_end = 4 + count*20 ≤ len` check (`reader.rs:594-603`). |
| PKG-06 | MEDIUM | **FIXED** | duplicate section id → `Err("duplicate MFPC section id")` (`reader.rs:342`). |
| PKG-07 | LOW | **FIXED** | `checked_add` in `checked_u16_at`/`_u32_at`/`_u64_at` (`util.rs:196-228`), `cursor_*` helpers, `IrReader::need` (`binary.rs:124-141`); `checked_usize` narrows u64→usize (`util.rs:237`). |

All seven prior findings remain closed; no regression observed. The two
structural theses from audit-1 (no signature boundary; IR never re-verified) are
both closed: signature gate precedes every decode+lower, and even the permitted
"unsigned local" case still runs `verify_semantics` on the merged IR.

## New finding

### PKG-08 — LOW — Scalar constant values escape the merged-IR literal-range verifier

- Location: `src/ir/verify/mod.rs:1626` (`numeric` predicate) and `:1643`
  `check_const_literal` (negated twin `:1701`); consumed at
  `src/target/shared/code/builder_conversions.rs:686`
  (`emit_scalar_to_string_value`) and `type_utils.rs:349` (`native_immediate_value`).
- Threat/impact: author of a hand-crafted `.mfp` (bytes the front end never
  produces). A source-level Scalar (backtick) literal is range-checked at parse
  time, but a crafted package IR can carry `IrValue::Const { type_: "Scalar",
  value: <arbitrary decimal> }`. `check_literal_range`'s `numeric` set is
  `Integer | Byte | Float | Fixed | Money` — **`Scalar` is absent** — so an
  out-of-range codepoint (`> 0x10FFFF`), a surrogate (`0xD800..0xDFFF`), or a
  value up to `u64::MAX` passes `verify_semantics` unflagged.
- Mechanism / why bounded (memory-safe): `native_immediate_value` returns the
  value verbatim (`type_utils.rs:367`) and the final `immediate()` encoder parses
  `u64` **fallibly** (`src/arch/aarch64/encode/operand.rs:89-92` and x86/riscv
  twins), so a non-numeric or `>u64` value yields a clean compile error, not a
  panic — no DoS. `emit_scalar_to_string_value` branches `<0x80/<0x800/<0x10000/else`
  and writes at most 4 bytes into an 8-byte `scalar_utf8_buf` stack slot
  (`builder_conversions.rs:700`); an over-range codepoint yields malformed UTF-8
  in a String but never an OOB write, and a `>u32` value truncates on the 4-byte
  Scalar store. So the ceiling is a `String` with invalid UTF-8 / a wrong
  codepoint — a correctness/robustness gap, not a codegen-to-unsafe-native path.
- Reproduction (sketch): encode a package function returning
  `toString(<Scalar const = 0x110000>)` with the const value string `"1114112"`.
  Observed: builds clean, emits a String with malformed UTF-8. Expected:
  `verify_semantics` rejects the out-of-range Scalar literal like it rejects an
  out-of-range `Byte`/`Money`.
- Best fix (no wire/ABI change): add `"Scalar"` to `check_const_literal` /
  `check_negated_const_literal` and to the `numeric` predicate (`verify/mod.rs:1626`);
  reject parse failures, values `> 0x10FFFF`, and surrogates `0xD800..=0xDFFF`,
  mirroring the existing `TYPE_*_LITERAL_OVERFLOW` diagnostics. Verifier-only.
- Non-goals: do not change the Scalar wire id (`TYPE_SCALAR = 10`),
  `FIRST_TABLE_TYPE_ID = 20`, the const-pool encoding, or add runtime UTF-8
  re-validation of every String.

## plan-41 renumber decode paths — audited, no finding

The `TYPE_SCALAR = 10` / base-types→20 / reserved band 11–19 renumber is
consistent across decoders: `primitive_type_name` maps `TYPE_SCALAR`
(`reader.rs:810`) and `TypeTable::type_id` interns it (`sections.rs:85`) —
round-trips; reserved ids 11–19 are a hard "unknown type id" error in every
decoder (`reader.rs:703/1295`, and `AbiSerializer`'s `checked_sub(FIRST_TABLE_TYPE_ID)`
underflow→`None` at `:1396`); the composite-constructor `kind` space (`kind==10`
= ThreadWorker in `decode_type_name_body`) is a *separate namespace* from the
primitive type-id space, so no collision with `TYPE_SCALAR`. The const-pool does
not length-validate a Scalar(4)/Money(8) payload, but CONST_POOL feeds only
byte-verbatim ABI sig-hashing / default-const hashing cross-checked against
`ABI_INDEX` — a wrong length yields a hash mismatch → rejection, never a decoded
value. Not a vulnerability.

## Verdict

Surface 1 is **hardened**. PKG-01..07 all closed; one new LOW (PKG-08) —
robustness only, memory-safe. No CRITICAL/HIGH. No bug doc (LOW; verifier-only fix).
