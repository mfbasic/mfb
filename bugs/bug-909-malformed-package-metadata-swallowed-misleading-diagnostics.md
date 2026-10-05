# bug-909: a malformed `.mfp` that only fails the lossy metadata readers is reported as an unrelated type/symbol/manifest error, never as an invalid package

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Footgun

Status: Open
Regression Test: tests/cli (proposed: build against a crafted .mfp, expect IMPORT_PACKAGE_INVALID)

Several build-time readers of installed packages call
`binary_repr::read_package_info(path)` and discard the error (`let Ok(info) = ... else
{ continue }` / `.ok()?`), so a package those readers cannot decode silently contributes
nothing. When the main import path does not happen to touch the corrupt field, the user
gets a diagnostic about *their* code or *their* project.json instead of
`IMPORT_PACKAGE_INVALID`:

1. Manifest `author` stringId out of range (the main reader never resolves `author`, but
   `read_package_info` does): every function call into the package is typed `Unknown`
   (`TYPE_CALL_ARGUMENT_MISMATCH: Call to toString has argument type(s) (Unknown)`), and an
   exported global is reported as `SYMBOL_UNKNOWN_IDENTIFIER: Package gpkg does not export LIMIT`.
2. A dependency's IMPORT_TABLE `pin` byte = 2 (or any import-table corruption): the
   dependency-closure check reports `PACKAGE_DEPENDENCIES_INCONSISTENT: package geom lists
   requiredBy [top], but it is required by no declared package; run mfb pkg update to
   rewrite project.json` — telling the user to rewrite a correct project.json.

`mfb pkg info` on the same files reports the real cause (`unknown string id 9999`,
`unsupported import pin value 2`). Correct behavior: any installed package that a
build-time reader fails to decode is reported once as `IMPORT_PACKAGE_INVALID` /
`PACKAGE_INVALID` naming the decode error, and no downstream diagnostic is derived from
the missing data.

References:

- `src/docs/spec/package/12_verifier-rules.md` — container/payload rejections "are surfaced as descriptive error ... strings from the reader".
- `src/docs/spec/package/03_metadata-encoding.md` — "Readers check indexes as they resolve them".
- Code comment `src/manifest/package.rs:imported_global_defs`: "lossy ... a package whose metadata cannot be read contributes nothing".

## Failing Reproduction

`patch_mfp.py` is in `/tmp/hardening/B10/repro/` (replaces bytes in one section, re-lays
the section table, re-stamps the payload hash).

```
cd /tmp/hardening/B10/repro
mfb init-pkg gpkg
printf 'EXPORT LET LIMIT AS Integer = 7\nEXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n' > gpkg/src/lib.mfb
(cd gpkg && mfb build)
mfb init gcons && cd gcons && mfb pkg add file:///tmp/hardening/B10/repro/gpkg/gpkg.mfp
printf 'IMPORT io\nIMPORT gpkg\nFUNC main() AS Integer\n  io::print(toString(gpkg::answer()) & " " & toString(gpkg::LIMIT))\n  RETURN 0\nEND FUNC\n' > src/main.mfb
mfb build && ./build/gcons.out          # 42 7
# MANIFEST: author stringId 2 -> 9999 (offset 24)
N=000000000000000001000000020000000200000002000000
python3 ../patch_mfp.py ../gpkg/gpkg.mfp packages/gpkg.mfp 1 0000000000000000010000000200000002000000020000000200000002000000 ${N}0f27000002000000
mfb build
mfb pkg info packages/gpkg.mfp
```

- Observed (`mfb build`): `uses gpkg - [Unsigned]`, then
  `error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER] Package gpkg does not export LIMIT.` and
  `error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH] Call to toString has argument type(s) (Unknown) ...`
  (with only `gpkg::answer()` in main, just the TYPE_CALL_ARGUMENT_MISMATCH).
  `mfb pkg info`: `error: failed to read 'packages/gpkg.mfp': unknown string id 9999`.
- Expected: `error[2-201-0001 IMPORT_PACKAGE_INVALID] ... unknown string id 9999`.

Second case (measured with `/tmp/hardening/B10/m20.py`): package `top` depends on `geom`;
patching top's IMPORT_TABLE pin byte to `2` gives
`error[6-605-0013 PACKAGE_DEPENDENCIES_INCONSISTENT] ... package geom lists requiredBy [top],
but it is required by no declared package; run mfb pkg update to rewrite project.json`,
while `mfb pkg info top_pin2.mfp` says `unsupported import pin value 2`.

Contrast: corruptions the main reader sees (bad magic, string-pool UTF-8, export kind,
section table) are reported correctly as IMPORT_PACKAGE_INVALID / PACKAGE_INVALID.

## Root Cause

Error-swallowing readers over the same files:
`src/manifest/package.rs:imported_global_defs_from_files` (`let Ok(info) = binary_repr::read_package_info(package) else { continue; }`),
`src/manifest/closure.rs` (`binary_repr::read_package_info(path).ok()?` at ~319 and
`if let Ok(info) = ...` at ~601), and the type/function export readers that turn an
unreadable package into `Unknown` types. `read_package_info` resolves fields
(`src/binary_repr/builder.rs:package_info` → `string_at(author/url)`) the main path skips.

## Goal

- Any decode failure in a build-time package reader surfaces as one package diagnostic
  carrying the reader's message; downstream lossy consumers never run on that package.

### Non-goals (must NOT change)

- Correct packages; `mfb pkg info` output.

## Blast Radius

- `imported_global_defs_from_files`, `imported_type_defs_from_files` (`src/manifest/package.rs`) — fixed by this bug
- `src/manifest/closure.rs` dependency-closure reads — fixed by this bug
