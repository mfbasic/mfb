# bug-673: a builtin package's own globals initialize after the program's, so a top-level initializer that calls the package reads them as zero

Last updated: 2026-09-21
Effort: small (<1h)
Severity: HIGH
Class: Correctness

Status: Fixed
Regression Test: `tests/rt-behavior/scope/global-init-calls-builtin-tables-valid`

**STATUS: FIXED.** Found by plan-145-A Phase 2: the field-kind harness's S5 program
for `json::Json` declares `MUT gR AS Rec = Rec[a := json::parse("[1]"), …]` at
module level, and it failed at startup.

## Failing Reproduction

```basic
IMPORT io
IMPORT json

MUT g AS json::Json = json::parse("[1]")

FUNC main() AS Integer
  io::print(json::stringify(g))
  RETURN 0
END FUNC
```

- Observed (at `4e0c50a8b`): `Error: 7-705-0024 invalid JSON format: nested too
  deeply`, exit 255, before `main` runs.
- Expected: `[1]`.

The same happens to any top-level initializer that calls a builtin function whose
package declares a top-level table: `compress::crc32`/`deflate`/`inflate`
(`__COMPRESS_*`), `crypto` (`__CRYPTO_*`), `color::fromName` (`__COLOR_NAMES`),
`regex` (`__REGEX_PARSE_DEPTH_LIMIT`). Each reads its table as zero.

## Root Cause

A builtin package's source is injected as extra files appended **after** the
program's files (`Registry::augment_project`, `src/codegen/registry/mod.rs`, and
the dedicated late passes), so its top-level bindings come last in
`IrProject::bindings`. The global initializer stores bindings in vector order
(`lower_global_initializer`, `src/target/shared/nir/lower.rs`). bug-613 reordered
manifest packages before their importers (`order_bindings_dependencies_first`), but
a builtin package is not a manifest package, so its bindings stayed behind the
program's. `json::parse` compares against `__JSON_DEPTH_LIMIT` (0 at that point)
and rejects every document.

## Fix

`order_builtin_bindings_first` (`src/ir/package.rs`), called in `merge_packages`
right after bug-613's ordering, moves every builtin binding to the front, keeping
their own order. A builtin binding is recognized by its internalized name
(`__JSON_DEPTH_LIMIT` lexes to `#JSON_DEPTH_LIMIT`), which no program can spell;
the other `#` form, a user file's mangled `PRIVATE` name, is excluded
(`internal_name::is_builtin_internal`). No builtin binding reads a program's or a
manifest package's, so moving them first is sound.

The spec's initialization-order paragraph
(`src/docs/spec/language/13_modules-and-packages.md`) now says so.

## Validation

- RED: `test-accept.sh <main's release mfb> … global-init-calls-builtin-tables-valid`
  → "1 mismatch(es)"; GREEN on the fixed compiler → passed. The fixture recomputes
  each global in `main` and prints `TRUE` for each match.
- Unit: `cargo test --bin mfb builtin_bindings_initialize_first` → ok.
- `scripts/artifact-gate.sh target/release/mfb all` → "1480 tests, 1655 build(s),
  2092 golden(s) checked, 0 diff(s)": no committed fixture's initializer order moved.
- `cargo test --bin mfb spec` → 43 passed.
- The full suites run in plan-145's final gate (plan-145-I Phase 3).
