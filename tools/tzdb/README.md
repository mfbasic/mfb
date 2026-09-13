# tools/tzdb

The generator for `packages/timezones/src/data.mfb`, the zone table behind the
`timezones` package (plan-135). `scripts/check-generated.sh` re-runs it in CI and fails
if the committed file no longer matches. Never hand-edit the artifact: change the
generator or the vendored release, then regenerate.

    python3 tools/tzdb/gen_timezones_data.py > packages/timezones/src/data.mfb

## What it does

- **Inputs.** The unmodified IANA release tarballs in `third_party/tzdb/<RELEASE>/`, and
  their `SHA256SUMS`. It needs Python 3.9 or newer, `cc` and `make`. Nothing is read
  from the host's zone database.
- **Pipeline.**
  1. Check both tarballs against `SHA256SUMS`.
  2. Unpack both into one temporary directory and run `make zic` there.
  3. Compile the release's default source set with `zic -b slim`. That set is
     `africa antarctica asia australasia europe northamerica southamerica etcetera
     backward factory`, so no `backzone`.
  4. Parse every compiled file as RFC 8536 TZif.
- **Grouping.** Names whose TZif bytes are identical share one zone string, so a link
  and its target always return the same data.
- **Fail-closed checks.** The generator exits 1 and names the zone if any of these
  fails:
  - every footer DST date is `Mm.w.d`;
  - a footer that names a DST designation has a `,start,end` rule;
  - designations use only `[A-Za-z0-9+-]`;
  - no footer holds `|`, `;`, `"` or `\`;
  - every `|utoff| < 172800`;
  - a zone's consecutive transitions are more than 345,600 s apart;
  - no name holds `"` or `\`;
  - no two names are equal ignoring case.

  `timezones`' footer evaluator and its `civil` window rely on exactly these premises.
  A release that breaks one fails here, not in a caller's program.
- **Output.** `data.mfb` goes to stdout. One statistics line goes to stderr. For 2026d
  it is:

      names 598 distinct 345 transitions 17018 types 1598 footers 94

  Each zone string is `types|transitions|footer` (see the file's header comment).
  `zoneData` and `canonicalName` dispatch on the lower-cased first letter and then
  `MATCH` the lower-cased name, so lookups ignore case.

## Updating to a new release

1. Download the new release's four files from
   <https://data.iana.org/time-zones/releases/>: `tzdata<R>.tar.gz`,
   `tzcode<R>.tar.gz`, and their `.asc` signatures.
2. Run `gpg --verify` on both signatures, with the key from
   `third_party/tzdb/README.md`. Record the output there.
3. Replace `third_party/tzdb/<old>/` with `third_party/tzdb/<R>/`, and write its
   `SHA256SUMS` with `shasum -a 256 tzdata<R>.tar.gz tzcode<R>.tar.gz`.
4. Bump `RELEASE` in `gen_timezones_data.py`.
5. Regenerate `packages/timezones/src/data.mfb`. Explain the statistics line's changes
   in the commit.
6. Bump `packages/timezones/oracle/requirements.txt` to the matching `tzdata` wheel.
   The wheel's minor number is the release letter's ordinal, so 2026d is `2026.4`.
7. Run `packages/timezones/oracle/run.sh` and `target/release/mfb test packages/timezones`.
8. Bump `version` in `packages/timezones/project.json`.
