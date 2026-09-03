# WHATWG legacy single-byte index tables

The 27 distinct index files behind `encoding::Codepage` / `encoding::codepageDecode`
/ `encoding::codepageEncode` (plan-123).

- Source: <https://encoding.spec.whatwg.org/index-\<label\>.txt>, enumerated from the
  "Legacy single-byte encodings" group of
  <https://encoding.spec.whatwg.org/encodings.json>.
- Retrieved: 2026-09-02. Each file carries the upstream `# Identifier:` hash and
  `# Date:` line in its own header (upstream date 2024-09-18), so a re-fetch is
  diffable against what is committed here.

## Why they are vendored

The tables are the data `codepageDecode` decodes with. Committing them makes the
build network-free and makes the generated table literals auditable by `diff`
against upstream rather than by eye.

## Contents

The standard lists **28** single-byte labels over **27** index files: `ISO-8859-8-I`
has no index of its own (`index-iso-8859-8-i.txt` is HTTP 404) and shares
`ISO-8859-8`'s table, which is the spec's own position — the two differ only in bidi
display direction, not in the byte↔code-point mapping.

Each file maps a pointer `0..=127` to the code point for byte `128 + pointer`.
Nineteen files are complete (128 mappings); eight leave some bytes undefined:

| Index | Mappings |
|---|---|
| `index-iso-8859-3.txt` | 121 |
| `index-iso-8859-6.txt` | 83 |
| `index-iso-8859-7.txt` | 125 |
| `index-iso-8859-8.txt` | 92 |
| `index-windows-874.txt` | 120 |
| `index-windows-1253.txt` | 125 |
| `index-windows-1255.txt` | 118 |
| `index-windows-1257.txt` | 126 |

Total mappings across all 27 files: **3,342**
(`python3 scripts/audit-codepage-index.py`).

## Regenerating the compiler tables

`scripts/gen-codepage-tables.py` reads these files and writes
`src/codegen/builtins/encoding/helper_codepage_table.rs`. Its output is committed;
re-running it must reproduce that file byte-for-byte
(`codepage_tables_are_regenerable` in `src/codegen/builtins/encoding/mod.rs` pins
this).

## Updating

Re-fetch with `python3 scripts/fetch-codepage-index.py`, `git diff` these files to
review what upstream changed, then re-run the generator. Note that the enum's
variant order is a compatibility surface: variants may be **appended**, never
reordered or removed.
