# timezones

Named IANA time zones, such as `America/New_York`, `Europe/London` and
`Australia/Lord_Howe`, for MFBASIC. It answers what offset and abbreviation a zone
applies at an instant, converts a wall-clock reading in a zone into a
`datetime::DateTime`, and writes and reads RFC 9557 zoned timestamps. Zone names match
ignoring case.

## API

| Member | What it answers | Raises |
|---|---|---|
| `timezones::offsetAt(name, at) AS Integer` | The UTC offset, in seconds east, that the zone applies at an instant | `77050004` unknown zone |
| `timezones::toZone(name, at) AS datetime::Zone` | That offset as a fixed-offset `datetime::Zone`, labelled with the tzdb abbreviation (`EDT`, `LMT`, `-03`) | `77050004` |
| `timezones::civil(date, time, name) AS datetime::DateTime` | A wall-clock reading in the zone; a skipped reading moves forward and a repeated one takes the earlier instant | `77050002` bad calendar field, `77050004` |
| `timezones::civil(date, time, name, disambiguation) AS datetime::DateTime` | The same, with the choice made explicit: `Disambiguation.COMPATIBLE`, `EARLIER`, `LATER` or `REJECT` | `77050002` as above, and a skipped or repeated reading under `REJECT`; `77050004` |
| `timezones::toIso(dt, name)` / `toIso(dt, digits, name) AS String` | RFC 9557 text, `2026-07-15T09:00:00.000-04:00[America/New_York]` | `77050002` offset not the zone's or bad `digits`, `77050004` |
| `timezones::parseIso(text) AS timezones::ZonedDateTime` | The instant and zone an RFC 9557 string names | `77050003` malformed or inconsistent, `77050004` |
| `timezones::ZonedDateTime` | A record: `dateTime AS datetime::DateTime`, `name AS String` (tzdb spelling) | — |
| `timezones::Disambiguation` | An enum: `COMPATIBLE`, `EARLIER`, `LATER`, `REJECT` (see below) | — |
| `timezones::tzdbVersion() AS String` | The tzdb release every answer comes from, such as `2026e` | — |
| `timezones::zoneNames() AS List OF String` | Every accepted zone name, in tzdb spelling and sorted; links are listed beside their targets | — |
| `timezones::ERR_UNKNOWN_ZONE` | `77050004`, `errorCode::ErrNotFound` | — |

Zone names match ignoring case and are returned in tzdb spelling. A link such as
`US/Eastern` answers exactly as its target, and keeps its own name.

Every failure is one of the registry codes in `mfb spec diagnostics error-codes`:
`77050002` is `errorCode::ErrInvalidArgument`, `77050003` is
`errorCode::ErrInvalidFormat`, and `77050004` is `errorCode::ErrNotFound`. The package
defines no codes of its own.

## `timezones` or `datetime::local()`?

- **Use `timezones`** when the zone is data: a user's chosen zone, a meeting's zone, a
  timestamp that must mean the same thing on every machine. The rules are the
  committed tzdb release, so every host and target gets the same answer.
- **Use `datetime::local()`** when the question is what the machine running the program
  considers local time. It reads the host's configured zone, which is exactly right
  for "show this in the user's own clock" and exactly wrong for anything stored or
  shared.

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

FUNC main() AS Nothing
  LET winter AS datetime::Instant = datetime::instant(1768485600, 0)   ' 2026-01-15T14:00:00Z
  LET summer AS datetime::Instant = datetime::instant(1784120400, 0)   ' 2026-07-15T13:00:00Z
  io::print(toString(timezones::offsetAt("America/New_York", winter)))   ' -18000
  LET zone AS datetime::Zone = timezones::toZone("America/New_York", summer)
  io::print(zone.label & " " & toString(zone.offsetSeconds))            ' EDT -14400
  io::print(datetime::toIso(datetime::inZone(summer, zone)))            ' 2026-07-15T09:00:00.000-04:00
END FUNC
```

**A zone from `toZone` is a snapshot for that instant.** It is a fixed-offset
`datetime::Zone`, so applying it to an instant on the other side of a daylight saving
change still uses the old offset. Call `toZone` again for each instant you convert.

An unknown name raises `errorCode::ErrNotFound` (`77050004`, exported as
`timezones::ERR_UNKNOWN_ZONE`). Its message names the zone and the tzdb release.

## A clock reading in a zone

```
timezones::civil(date AS datetime::Date, time AS datetime::Time, name AS String) AS datetime::DateTime
timezones::civil(date AS datetime::Date, time AS datetime::Time, name AS String, disambiguation AS timezones::Disambiguation) AS datetime::DateTime
```

`civil` turns what a wall clock in a zone reads into a zoned `datetime::DateTime`:

```
IMPORT timezones
IMPORT datetime
IMPORT io

FUNC main() AS Nothing
  LET meeting AS datetime::DateTime = timezones::civil(datetime::date(2026, 1, 15), datetime::time(9, 0, 0, 0), "America/New_York")
  io::print(datetime::toIso(meeting))   ' 2026-01-15T09:00:00.000-05:00
  LET summer AS datetime::DateTime = timezones::civil(datetime::date(2026, 7, 15), datetime::time(9, 0, 0, 0), "America/New_York")
  io::print(datetime::toIso(summer))    ' 2026-07-15T09:00:00.000-04:00
END FUNC
```

Twice a year a reading is skipped or happens twice. The three-argument `civil` resolves
both cases with the RFC 9557 **compatible** rule, which Python's `zoneinfo` also uses
with `fold=0`:

- **A skipped reading moves forward by the length of the gap.** New York skips 02:00 to
  03:00 on 2026-03-08, so `02:30` becomes `2026-03-08T03:30:00.000-04:00`.
- **A repeated reading resolves to the earlier instant.** New York passes through 01:00
  to 02:00 twice on 2026-11-01, so `01:30` becomes `2026-11-01T01:30:00.000-04:00`,
  still daylight time.

The four-argument form makes that choice explicit. `timezones::Disambiguation` has the
same four choices as Temporal's `disambiguation` option:

| Choice | A skipped reading | A repeated reading |
|---|---|---|
| `COMPATIBLE` (the default) | moves forward by the gap, using the offset before the change | the earlier instant |
| `EARLIER` | the same as `COMPATIBLE` | the same as `COMPATIBLE` |
| `LATER` | moves back by the gap, using the offset after the change | the later instant |
| `REJECT` | raises `77050002` | raises `77050002` |

```
IMPORT timezones
IMPORT datetime
IMPORT io

FUNC main() AS Nothing
  LET skipped AS datetime::Date = datetime::date(2026, 3, 8)
  LET at AS datetime::Time = datetime::time(2, 30, 0, 0)
  io::print(datetime::toIso(timezones::civil(skipped, at, "America/New_York")))                                 ' 2026-03-08T03:30:00.000-04:00
  io::print(datetime::toIso(timezones::civil(skipped, at, "America/New_York", timezones::Disambiguation.LATER))) ' 2026-03-08T01:30:00.000-05:00
END FUNC
```

Pick `REJECT` when a skipped or repeated reading is a mistake the caller must see, such
as a booking or a scheduled job. It is the only choice that tells the caller the reading
was ambiguous:

```
IMPORT timezones
IMPORT datetime
IMPORT io

FUNC main() AS Integer
  LET day AS datetime::Date = datetime::date(2026, 11, 1)
  LET repeated AS datetime::Time = datetime::time(1, 30, 0, 0)
  io::print(datetime::toIso(timezones::civil(day, repeated, "America/New_York", timezones::Disambiguation.REJECT)))
  RETURN 0

  TRAP(problem)
    io::print("refused: " & toString(problem.code))   ' refused: 77050002
    RETURN 0
  END TRAP
END FUNC
```

01:30 occurs twice on 2026-11-01, so `REJECT` refuses it. A reading that occurs once is
accepted under every choice.

**The result carries a fixed-offset snapshot.** Its `zone` is `timezones::toZone` at that
instant. `datetime::addDays` on it shifts the date, keeps the time, and keeps that
offset, even across a daylight saving change. To move by calendar days *in the zone*,
read the clock again:

```
LET tomorrow AS datetime::DateTime = timezones::civil(datetime::addDays(dt, 1).date, dt.time, "America/New_York")
```

A calendar field out of range raises `errorCode::ErrInvalidArgument` (`77050002`). An
unknown zone raises `errorCode::ErrNotFound` (`77050004`).

## Writing and reading a zoned time

```
EXPORT TYPE ZonedDateTime        ' dateTime AS datetime::DateTime, name AS String
timezones::toIso(dt AS datetime::DateTime, name AS String) AS String
timezones::toIso(dt AS datetime::DateTime, digits AS Integer, name AS String) AS String
timezones::parseIso(text AS String) AS timezones::ZonedDateTime
```

An RFC 3339 timestamp such as `2026-07-15T09:00:00.000-04:00` records an instant and an
offset, but not the zone. RFC 9557 appends the zone in brackets, and these two members
write and read that form:

```
IMPORT timezones
IMPORT datetime
IMPORT io

FUNC main() AS Nothing
  LET meeting AS datetime::DateTime = timezones::civil(datetime::date(2026, 7, 15), datetime::time(9, 0, 0, 0), "America/New_York")
  LET text AS String = timezones::toIso(meeting, "America/New_York")
  io::print(text)                                    ' 2026-07-15T09:00:00.000-04:00[America/New_York]
  LET back AS timezones::ZonedDateTime = timezones::parseIso(text)
  io::print(back.name & " " & back.dateTime.zone.label)   ' America/New_York EDT
END FUNC
```

- **Names ignore case.** `[america/new_york]` reads as `America/New_York`, and
  `toIso(dt, "america/new_york")` writes `[America/New_York]`. A link stays a link:
  `[us/eastern]` reads as `US/Eastern`.
- **The offset is required, and must agree with the zone, always.** RFC 3339 makes the
  offset part of the timestamp, so `2026-07-15T09:00:00[America/New_York]` is refused
  (`77050003`); write `Z` or `-04:00`. `toIso` refuses (`77050002`) a `DateTime` whose
  offset is not the zone's offset at its instant. `parseIso` refuses (`77050003`) text
  whose offset disagrees with its zone, whether or not the zone is marked critical with
  `!`. RFC 9557 would let a reader ignore an elective mismatch; this package treats it as
  corrupt data.
- **Two spellings read back exactly.** `Z[America/New_York]` names the instant and takes
  the zone's offset. A local-mean-time offset rounded to the minute
  (`1850-07-01T07:03:58-04:56[America/New_York]`) reads back to the exact instant.
  `toIso` itself writes such an offset with seconds (`-04:56:02`).
- **Suffix tags.** `[u-ca=iso8601]` is accepted and any other calendar is refused, when
  it is the first calendar tag. RFC 9557 §3.3 says that a repeated elective key is
  resolved by its first occurrence, so `[u-ca=iso8601][u-ca=gregory]` is accepted and
  `[u-ca=gregory][u-ca=iso8601]` is refused. An unknown tag such as `[foo=bar]` is
  ignored, and `[!foo=bar]` is refused. A numeric annotation like `[+05:30]` is refused;
  use `datetime::parseIso` for plain offsets.
- **Only `digits = 9` keeps every nanosecond.** The two-argument `toIso` writes
  milliseconds.

## Where the rules come from

The rules are **IANA tzdb 2026e**. The release tarballs are committed unmodified under
`third_party/tzdb/2026e/`, with their checksums and signature verification recorded in
`third_party/tzdb/README.md`. `timezones::tzdbVersion()` reports the release at run time.

`tools/tzdb/gen_timezones_data.py` compiles them with the release's own `zic` and writes
`src/data.mfb`. `scripts/check-generated.sh` regenerates that file in CI and fails on
any difference, so the committed table is always exactly what the vendored release
produces. Moving to a new release is a documented procedure: see
[`tools/tzdb/README.md`](../../tools/tzdb/README.md#updating-to-a-new-release).

Changes in 2026e that a program can observe include America/Winnipeg, America/Rainy_River
and Canada/Central staying on permanent -05 (`EST`) from 2026-11-01, and Europe/Dublin's
1925 fall-back falling on 09-20. The [CHANGELOG](CHANGELOG.md) records each release.

## Stability

This is version 1.0.0. The API above is stable: a change that removes or changes a member,
an error code, or a result for a given input is a major version. A new tzdb release is
a minor version, because it can change what a zone answers for a date, and a program that
stores answers should record `tzdbVersion()` beside them.

## What it never does

It never reads the host's zone database: not `/usr/share/zoneinfo`, not `TZ`, not
`localtime`, and not the Windows time-zone API. It has no fallback to any of them. A
program gets the same answer on every host and every target, whatever that machine's
zone database says.

For the host's own idea of local time, use `datetime::local()`.

It keeps no state. Every call decodes the zone it needs and returns, so calls from any
number of threads are independent, and there is no lock to contend on. A lookup costs
about 14 µs on the worst zone in the table (macOS arm64; see `src/zone.mfb`).

## Verification

From the repository root:

```sh
mfb build packages/timezones
mfb test packages/timezones                              # the package's own tests
packages/timezones/runtime-smoke.sh target/release/mfb   # a consumer builds and runs against the .mfp
packages/timezones/check-doc-examples.sh target/release/mfb  # every EXAMPLE and whole-program README block
sh scripts/check-generated.sh                            # the table matches the vendored release
packages/timezones/oracle/run.sh                         # zoneinfo, Temporal, and the probe agree
```

`oracle/run.sh` needs Python 3 and Node 24 or newer. It compares every offset, reading
and disambiguation choice against Python's `zoneinfo` on the same tzdb release, and
checks the RFC 9557 syntax against Temporal. The divergences it allows are declared with
reasons in `oracle/divergences.json`. See [`oracle/README.md`](oracle/README.md).
