# tools/iso4217

The generator for `packages/currency/src/data.mfb`, the ISO 4217 code table behind the
`currency` package. `scripts/check-generated.sh` re-runs it in CI and fails if the
committed file no longer matches. Never hand-edit the artifact: change the generator or
the vendored list, then regenerate.

    python3 tools/iso4217/gen_currency_data.py > packages/currency/src/data.mfb

## What it does

- **Input.** The unmodified ISO 4217 List One in `third_party/iso4217/<PUBLISHED>/`,
  and its `SHA256SUMS`. It needs Python 3 and nothing else.
- **Pipeline.**
  1. Check `list-one.xml` against `SHA256SUMS`.
  2. Check that the file's `Pblshd` attribute equals `PUBLISHED` in the generator.
  3. Read every `<CcyNtry>` that names a `<Ccy>`. Antarctica's entry names none and is
     skipped.
- **Fail-closed checks.** The generator exits 1 and names the code if any of these
  fails:
  - every code is three ASCII capitals;
  - every code's minor units are `0` to `5`, or `N.A.`;
  - a code used by several countries has the same minor units in every entry.

  `currency` relies on exactly these premises: its custom codes start with `_`, so they
  can never collide with an ISO code, and `money::round` takes at most five places.
- **Output.** `data.mfb` goes to stdout: `isoPublished()`, `isoCodeList()` (ascending),
  and `isoMinorUnits(code)`, which returns the decimal places, `-2` for `N.A.`, or `-1`
  for a code not in the list. One statistics line goes to stderr. For 2026-09-17 it is:

      codes 178 minor-units 0:17 2:139 3:7 4:2 N.A.:13

## Updating to a new list

1. Download `list-one.xml` from
   <https://www.six-group.com/dam/download/financial-information/data-center/iso-currrency/lists/list-one.xml>.
   Its `Pblshd` attribute is the new list's date.
2. Replace `third_party/iso4217/<old>/` with `third_party/iso4217/<date>/`, and write its
   `SHA256SUMS` with `shasum -a 256 list-one.xml`. Update the date and the verification
   record in `third_party/iso4217/README.md`.
3. Bump `PUBLISHED` in `gen_currency_data.py`.
4. Regenerate `packages/currency/src/data.mfb`. Explain the statistics line's changes in
   the commit, naming each code added or withdrawn.
5. Update the counts and dates the tests pin (`packages/currency/src/test_codes.mfb`
   checks the code count, the first and last codes, and `isoListDate`), and bump the
   package version in `packages/currency/project.json`: a withdrawn code stops being
   valid, which can break a program that uses it.
6. Run `target/release/mfb test packages/currency`.
