# MFBASIC `errorCode` Package Implementation Plan

Last updated: 2026-06-21

This document plans the implementation of the built-in `errorCode` package
specified in `specifications/standard_package.md` §13. It exports a named
`Integer` constant for every standard runtime and toolchain error in the
canonical registry `specifications/error_codes.md`.

It complements:

- `specifications/standard_package.md` §13
- `specifications/error_codes.md` (the canonical, normative registry)
- `specifications/architecture.md`

## 1. Goal

Make this work, exactly as the spec already shows:

```basic
IMPORT errorCode

IF err.code = errorCode::ErrNotFound THEN
  io::print("missing")
END IF
```

Requirements from §13:

- `errorCode` is a built-in package: `IMPORT errorCode` resolves with no file or
  manifest entry.
- It exports one `Integer` constant per registry entry. The constant name equals
  the registry `Name` (e.g. `ErrNotFound`), and the value is the canonical code
  string with hyphens removed (e.g. `7-705-0004` → `77050004`).
- The constants are referenced as `errorCode::<Name>` and used wherever an
  `Integer` is expected (comparisons, `MATCH` guards, `TRAP` handlers).
- The package is constants only — no functions, no types, no resources.

## 2. Current State

`errorCode` is **not implemented** as a usable package:

- It is **not** a built-in import: `src/builtins/mod.rs` `is_builtin_import`
  lists only `fs | io | json | math | net | strings | thread`. `IMPORT errorCode`
  does not resolve.
- There is no `src/builtins/errorcode.rs` and no constant table for the error
  names. `grep` for `errorCode` in `src/` finds only man-page text
  (`src/man/builtins/**`) and an unrelated `entry_error_code` symbol — no
  resolver/typecheck/codegen support.
- The canonical names and values exist only as documentation rows in
  `specifications/error_codes.md` (~197 `errorCode::*` entries today).

So today, every spec example and man-page snippet that writes
`errorCode::ErrNotFound` refers to a package that cannot be imported. Programs
must hardcode raw integers (`77050004`), which §13 explicitly tells them not to.

## 3. Design: Follow the `math` Constant Precedent

The `errorCode` package is the simplest possible built-in: a flat set of
`Integer` constants with compile-time-known values. The existing `math` package
already implements exactly this shape for `math::pi` and friends, and that is the
model to copy — **not** the `json` MFBASIC-source model and **not** a runtime
helper.

How `math` constants work (the template):

- `src/builtins/math.rs` exposes `is_math_constant(name)`,
  `constant_type_name(name)`, and `constant_value(name)` (returns the literal as
  a string).
- `src/builtins/mod.rs` `is_builtin_member` ORs in `math::is_math_constant`.
- `src/ir.rs` (around the constant-folding sites) and `src/typecheck.rs` consult
  `is_math_constant`/`constant_type_name`/`constant_value` to type the reference
  and **fold it to a literal**. A `math::pi` reference lowers to a `Float`
  literal in IR; there is no runtime call and no runtime helper.

`errorCode` is even simpler than `math`: every constant is the same type
(`Integer`) and every value is an integer literal. Folding to an `Integer`
literal means:

- **No `RuntimeHelper` variant, no codegen, no `binary_repr` change.** The
  constants disappear into literals before lowering, exactly like `math::pi`.
- **No new error codes, no new types, no new resources.**

## 4. Single Source of Truth — Avoid Drift From `error_codes.md`

The one real risk is the constant table drifting from the canonical registry
(~197 entries, and growing — e.g. `ErrTlsFailed`, the native-binding codes, the
float-domain codes were all added over time). `error_codes.md` is normative; the
`errorCode` package must match it exactly. Do **not** hand-maintain a parallel
197-row `match` arm by eyeball.

Recommended (pick one; **(a)** preferred):

- **(a) Generate the table from `error_codes.md` at build time.** Add a
  `build.rs` step (or extend the existing one) that parses the "Runtime and
  Standard Package Errors" rows of `specifications/error_codes.md` and emits a
  generated Rust table (`name -> integer-literal string`) included by
  `src/builtins/errorcode.rs`. The registry's row format is regular
  (`` | `G-SSS-EEEE` | `integer` | `Name` | … `` ), so a tolerant line parser is
  enough. This makes the doc the literal source of truth and makes drift
  impossible.
- (b) Check in a generated `errorcode_constants.rs` plus a **test** that re-parses
  `error_codes.md` and asserts the table matches (names, values, and that hyphen
  removal of the canonical code equals the integer column). Lower build
  complexity, but the table is a checked-in artifact that a contributor could
  forget to regenerate; the test is the backstop.

Either way, add a test that asserts: for every registry row, hyphen-stripping the
`G-SSS-EEEE` code yields the integer column **and** the exported
`errorCode::<Name>` value. This catches both doc typos and table drift.

Scope note: §13 says `errorCode` exports a constant for *every standard runtime
and toolchain error in the canonical registry*. The registry also contains
compiler/toolchain diagnostic rule codes (e.g. `TYPE_MATCH_NOT_EXHAUSTIVE`,
the `1-1xx`/`2-2xx`/`3-3xx`/`5-5xx`/`6-6xx` ranges). Decide and document the
exported set explicitly:

- **Recommendation:** export the full set of named codes in the registry,
  including the compiler/toolchain diagnostic names, so any code a program might
  observe (or want to compare against in tooling) is nameable. §13 already
  notes these diagnostics are not normally produced as runtime `Error` values,
  but exporting them is harmless and keeps "one name per registry entry" simple
  and uniform. The generator in §4(a) naturally produces all rows; do not filter.

## 5. Implementation Steps

### 5.1 New module `src/builtins/errorcode.rs`

Mirror the `math` constant API:

```text
fn is_errorcode_constant(name: &str) -> bool          // "errorCode.ErrNotFound" etc.
fn constant_type_name(name: &str) -> Option<&str>     // always Some("Integer")
fn constant_value(name: &str) -> Option<&str>         // "77050004" etc. (generated)
```

Use the package-qualified key convention already used by the other built-ins
(`"errorCode.<Name>"`, matching how `math.pi`/`strings.trim` are keyed
internally). Back `constant_value`/`is_errorcode_constant` with the generated
table from §4.

### 5.2 Register in `src/builtins/mod.rs`

- Add `errorcode` to the `mod` list and `is_builtin_import`:
  `"errorCode"` resolves as a built-in package. (Confirm the import name casing:
  the spec imports it as `errorCode`. `is_builtin_import` matches the source
  package name, so the arm must be `"errorCode"`.)
- Extend `is_builtin_member` to OR in `errorcode::is_errorcode_constant`
  (alongside `math::is_math_constant`).
- `errorCode` has no calls, so it does **not** appear in `is_builtin_call`,
  `call_return_type_name`, or `call_param_names`. It has no types, so it stays
  out of `is_builtin_type`.

### 5.3 Resolve / typecheck / fold

- Wherever `math::is_math_constant` is consulted in `src/typecheck.rs` (the
  package-member resolution and "is this a constant vs a call" sites near
  lines ~2802, ~2847, ~2939) and `src/ir.rs` (the constant-folding sites near
  lines ~1974, ~2451), add the parallel `errorcode` lookups so an
  `errorCode::<Name>` member reference types as `Integer` and folds to an
  integer literal.
- A reference to an unknown `errorCode::<Name>` must produce the same
  "unknown package member" diagnostic the resolver already emits for a bad
  `math::<x>` — no new diagnostic code needed.

### 5.4 No backend work

Because references fold to `Integer` literals, there is nothing to add to
`RuntimeHelper`, `src/target/shared/code/**`, the target `plan.rs` files, or
`src/binary_repr.rs`. Confirm by grepping that no `errorCode`/`errorcode`
helper symbol is ever emitted.

### 5.5 Documentation / man pages

- The man pages under `src/man/builtins/**` already reference
  `errorCode::*`; add an `errorCode` package man entry if the man system expects
  one per built-in package.
- No change to `error_codes.md` itself — it remains the source of truth and is
  now consumed by the generator.

## 6. Validation Plan

### 6.1 Consistency test (the important one)

- A Rust test that parses `specifications/error_codes.md` and asserts the
  exported table is exactly the set of registry rows, with
  `value == code.replace("-", "")` for every entry. This is what prevents drift
  and is the single most valuable test here.

### 6.2 Function/usage tests

Follow the existing `tests/func_*` convention (there is no `errorCode` call, so
these are usage tests, akin to how `math` constants would be exercised):

- `tests/func_errorcode_constant_valid/**`:
  - `IMPORT errorCode` then compare `errorCode::ErrNotFound = 77050004`,
    `errorCode::ErrInvalidArgument = 77050002`,
    `errorCode::ErrVerificationFailed = 33020001` (the three examples §13 calls
    out by value), plus a sample from each subsystem range.
  - Use a constant in a `TRAP` (the §3.2 `find`/`ErrNotFound` pattern from
    `standard_package.md`) and in a `MATCH`/`IF` guard.
- `tests/func_errorcode_constant_invalid/**`:
  - `errorCode::NotARealName` → unknown-member diagnostic.
  - Using `errorCode` without `IMPORT` → unimported-package diagnostic.
  - Attempting to *call* `errorCode::ErrNotFound()` → not-callable diagnostic.

### 6.3 Acceptance

- Run `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- Confirm the same folded values appear whether compiled directly or imported as
  an MFP package (constants fold to literals, so the two paths must agree by
  construction).

## 7. Recommended Sequence

1. Add the §4 generator (or generated table + consistency test) sourced from
   `error_codes.md`.
2. Add `src/builtins/errorcode.rs` with the `math`-style constant API over the
   generated table.
3. Register `errorCode` in `mod.rs` (`is_builtin_import`, `is_builtin_member`).
4. Wire the `errorcode` constant lookups into the `typecheck.rs` and `ir.rs`
   constant sites next to the existing `math` constant handling.
5. Add the consistency test and the valid/invalid usage tests.
6. Run acceptance.

## 8. Non-Goals

- No functions, types, or resources in the package — constants only.
- No new error codes or registry rows (the package consumes the registry; it does
  not extend it).
- No runtime helper or codegen — references fold to `Integer` literals.
- Re-keying programs off raw integers onto `errorCode::*` across the existing
  test corpus is optional cleanup, not part of landing the package.

## 9. Bottom Line

`errorCode` is the cheapest built-in to add: copy the `math` compile-time
constant mechanism, but make the constant table **generated from
`error_codes.md`** so it cannot drift from the canonical registry. No backend,
no runtime, no new codes — just resolution, folding, and a consistency test.
