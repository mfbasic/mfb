# timezones

Named IANA time zones, such as `America/New_York`, `Europe/London` and
`Australia/Lord_Howe`, for MFBASIC. It answers what offset and abbreviation a zone
applies at an instant, converts a wall-clock reading in a zone into a
`datetime::DateTime`, and writes and reads RFC 9557 zoned timestamps. Zone names match
ignoring case.

## Offsets at an instant

```
timezones::offsetAt(name AS String, at AS datetime::Instant) AS Integer
timezones::toZone(name AS String, at AS datetime::Instant) AS datetime::Zone
```

`offsetAt` returns the offset from UTC, in seconds east, that a zone applies at an
instant. `toZone` returns the same offset as a `datetime::Zone`, labelled with the tzdb
abbreviation in force.

```
IMPORT timezones
IMPORT datetime
IMPORT io

SUB main()
  LET winter AS datetime::Instant = datetime::instant(1768485600, 0)   ' 2026-01-15T14:00:00Z
  LET summer AS datetime::Instant = datetime::instant(1784120400, 0)   ' 2026-07-15T13:00:00Z
  io::print(toString(timezones::offsetAt("America/New_York", winter)))   ' -18000
  LET zone AS datetime::Zone = timezones::toZone("America/New_York", summer)
  io::print(zone.label & " " & toString(zone.offsetSeconds))            ' EDT -14400
  io::print(datetime::toIso(datetime::inZone(summer, zone)))            ' 2026-07-15T09:00:00.000-04:00
END SUB
```

**A zone from `toZone` is a snapshot for that instant.** It is a fixed-offset
`datetime::Zone`, so applying it to an instant on the other side of a daylight saving
change still uses the old offset. Call `toZone` again for each instant you convert.

An unknown name raises `errorCode::ErrNotFound` (`77050004`, exported as
`timezones::ERR_UNKNOWN_ZONE`). Its message names the zone and the tzdb release.

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
