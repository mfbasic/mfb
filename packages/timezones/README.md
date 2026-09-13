# timezones

Named IANA time zones, such as `America/New_York`, `Europe/London` and
`Australia/Lord_Howe`, for MFBASIC. It answers what offset and abbreviation a zone
applies at an instant, converts a wall-clock reading in a zone into a
`datetime::DateTime`, and writes and reads RFC 9557 zoned timestamps. Zone names match
ignoring case.

## Where the rules come from

The rules are **IANA tzdb 2026d**. The release tarballs are committed unmodified under
`third_party/tzdb/2026d/`, with their checksums and signature verification recorded in
`third_party/tzdb/README.md`.

`tools/tzdb/gen_timezones_data.py` compiles them with the release's own `zic` and writes
`src/data.mfb`. `scripts/check-generated.sh` regenerates that file in CI and fails on
any difference, so the committed table is always exactly what the vendored release
produces. Moving to a new release is a documented procedure: see
[`tools/tzdb/README.md`](../../tools/tzdb/README.md#updating-to-a-new-release).

## What it never does

It never reads the host's zone database: not `/usr/share/zoneinfo`, not `TZ`, not
`localtime`, and not the Windows time-zone API. It has no fallback to any of them. A
program gets the same answer on every host and every target, whatever that machine's
zone database says.

For the host's own idea of local time, use `datetime::local()`.
