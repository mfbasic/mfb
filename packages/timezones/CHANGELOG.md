# Changelog

All notable changes to the `timezones` package. The package follows
[Semantic Versioning](https://semver.org/): from 1.0.0 on, a breaking change to the
exported API, to an error code, or to a result for a given input requires a new major
version. A new tzdb release is a minor version, because it can change what a zone
answers for a date. A program that stores answers should record `tzdbVersion()`
beside them.

## 1.0.0

The first stable release. The package was 0.1.0 while its design was settled. The
changes below are relative to that version.

### Added

- `timezones::Disambiguation`, an enum of `COMPATIBLE`, `EARLIER`, `LATER` and
  `REJECT`, and a four-argument `timezones::civil` that takes one. The three-argument
  form is `COMPATIBLE`, as before. `LATER` takes the offset after a change, so a
  skipped reading moves back by the gap and a repeated one is the later instant.
  `REJECT` raises `77050002` for a skipped or repeated reading. The four choices match
  Temporal's `disambiguation` option, and `EARLIER` and `LATER` match Python's
  `fold=0` and `fold=1`.
- `timezones::tzdbVersion()`, the tzdb release the answers come from (`2026e`).
- `timezones::zoneNames()`, every accepted zone name in tzdb spelling and sorted.
  Links are listed beside their targets, as in the tzdb.

### Changed

- The rules are IANA tzdb **2026e** (was 2026d), signature-verified and vendored
  unmodified. Two zones change, both upstream corrections:
  - America/Winnipeg, America/Rainy_River and Canada/Central stay on permanent -05
    (`EST`) from 2026-11-01. Under 2026d they went to CST, -06, on that date.
  - Europe/Dublin's 1925 fall-back was on 09-20, not 10-04.
- `timezones::parseIso` names the missing offset when a zoned string has none before
  its `[zone]`, as in `2026-07-15T09:00:00[America/New_York]`. It was refused before,
  with a message from `datetime::parseIso` that did not say why.
- An empty annotation after the zone, as in `[America/New_York][]`, is reported as an
  empty annotation, not as a second zone.

### Build and gates

- The generator checks the spacing between footer-derived transitions with a `zic -b
  fat` build, so the `civil` window premise is checked across the stored and footer
  boundary, not only among the stored transitions.
- `packages/timezones/check-doc-examples.sh` builds and runs every `EXAMPLE` and every
  whole-program README block. `runtime-smoke.sh` checks a consumer build of the
  `.mfp`, now against 1.0.0 and the enum and release exports.
- `tests/gate/timezones_package.rs` runs the package's test suite, the smoke test and
  the example check under `cargo test`.
- The oracle compares `disambiguation` against `zoneinfo`'s folds, over every civil
  reading in the corpus.
