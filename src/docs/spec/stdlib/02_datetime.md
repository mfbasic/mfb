# Date and Time Model

The `datetime::` package is a built-in source package: all calendar math,
formatting, and parsing are written in MFBASIC as internal `__datetime_*`
functions, and only the OS clock and the local-zone table are platform state.
The Rust seam (`src/builtins/datetime.rs`) owns registration, syntaxcheck
metadata, and the rewrite from each public `datetime::` call onto its internal
implementation; the `.mfb` source owns the algorithm. This topic specifies the
**model** — the record shapes, the civil-calendar math, the clock/zone seam, and
the parse/format grammar. The per-function API surface is owned by
`./mfb man datetime`.

The package is injected only when a program `IMPORT datetime`; otherwise its AST
is not added to the project. [[src/builtins/datetime.rs:uses_package]]

## Value model

Every public type is a flat, copyable record or enum — no handles, no hidden
state. [[src/builtins/datetime_package.mfb:Instant]]

| Type | Fields | Meaning |
| --- | --- | --- |
| `Instant` | `seconds: Integer`, `nanos: Integer` | A point on the wall clock, seconds since the Unix epoch plus a sub-second remainder. |
| `Duration` | `seconds: Integer`, `nanos: Integer` | A signed elapsed span. |
| `Date` | `year`, `month`, `day` | A proleptic-Gregorian civil date. |
| `Time` | `hour`, `minute`, `second`, `nanos` | A wall-clock time of day. |
| `Zone` | `offsetSeconds: Integer`, `kind: Integer`, `label: String` | A time zone: fixed offset, UTC, or the host local zone. |
| `DateTime` | `date: Date`, `time: Time`, `zone: Zone`, `offset: Integer` | A civil date-time already projected into a zone, with the resolved offset cached in `offset`. |

Three enums name the discrete domains: `ZoneKind` (`Utc=0`, `FixedOffset=1`,
`Local=2`), `Weekday` (`Monday`..`Sunday`), and `Month` (`January`..`December`).
The `Zone.kind` field stores the `ZoneKind` ordinal as a raw `Integer`.
[[src/builtins/datetime_package.mfb:ZoneKind]]

### Canonical form and normalization

An `Instant`/`Duration` is **canonical** when `nanos` is in
`[0, 1_000_000_000)`. Every constructor and every arithmetic result is funnelled
through `__datetime_normInstant` / `__datetime_normDuration`, which carry a raw
`(seconds, nanos)` pair into canonical form. Because `/` truncates toward zero
and `MOD` takes the sign of the dividend, a negative `nanos` borrows one second:

```
q = nanos / 1_000_000_000          ' truncating
r = nanos MOD 1_000_000_000        ' sign of dividend
IF r < 0 THEN r = r + 1e9 : q = q - 1
RETURN [seconds + q, r]
```

The component builders compose larger fields into a second count before
normalizing — `instant`/`duration` accept 1..5 trailing `Integer` arguments
(`seconds`; `seconds,nanos`; `mins,seconds,nanos`; `hours,...`; `days,...`),
each multiplying by `60 / 3600 / 86400` as appropriate. Arithmetic
(`add`, `subtract`, `between`, `plus`, `minus`, `negate`) adds or subtracts the
raw field pairs and re-normalizes; comparison (`compare`, `isBefore`, `isAfter`,
`equals`) orders on `seconds` then `nanos`. [[src/builtins/datetime_package.mfb:__datetime_normInstant]]

`fromMillis` / `toMillis` / `toNanos` convert against the epoch with the same
borrow logic; `toMillis`/`toNanos` use checked `Integer` arithmetic, so a value
outside the `Integer` range surfaces `ErrOverflow` (`77050010`).
[[src/builtins/datetime_package.mfb:__datetime_toMillis]]

## Monotonic vs wall clock

The model keeps the two clock kinds in distinct types so they cannot be mixed:

* **Wall clock** — `now` returns an `Instant` (epoch-relative). It can jump
  backward or forward when the host clock is adjusted (NTP, manual set, DST is
  *not* a wall-clock jump — that is a zone-offset change). Use it for timestamps
  and calendar work.
* **Monotonic clock** — `monotonic` returns a `Duration` measured from an
  arbitrary, unspecified origin. It never goes backward and is immune to clock
  adjustments, but its zero point is meaningless across processes. Use
  *differences* of two `monotonic` readings to measure elapsed time.

Both intrinsics return non-negative nanoseconds on any sane host, so the split
into `(seconds, nanos)` is a plain truncating divide rather than the full
borrowing normalization. [[src/builtins/datetime_package.mfb:__datetime_monotonic]]

## Portable civil-calendar math

All date math is platform-independent and runs in MFBASIC. The epoch-day
conversions use Howard Hinnant's branch-free civil ↔ days algorithm, valid
across the full `Integer` range; the explicit era adjustments keep every divisor
operand non-negative so truncating division equals flooring.
[[src/builtins/datetime_package.mfb:__datetime_daysFromCivil]]

* `daysFromCivil(y, m, d)` → days since `1970-01-01` (the `719468` constant
  shifts from the `0000-03-01` internal era origin to the Unix epoch).
* `civilFromDays(z)` → `Date`, the inverse.

The proleptic Gregorian calendar is used for *all* years, including before its
historical adoption; there is no year 0 discontinuity special-casing beyond the
algorithm's own era arithmetic.

**Leap year:** divisible by 4, except centuries, except multiples of 400.
[[src/builtins/datetime_package.mfb:__datetime_isLeapYear]]

```
isLeapYear(y) = (y MOD 4 = 0 AND y MOD 100 <> 0) OR y MOD 400 = 0
```

**Days in month:** February is 29 in leap years else 28; April, June,
September, November are 30; all others 31. [[src/builtins/datetime_package.mfb:__datetime_daysInMonth]]

**Day of week:** computed directly from the epoch-day number, not from a
table. The epoch day `1970-01-01` is a Thursday; the package re-bases it to a
Monday-origin index with `floorMod(days + 3, 7)`, mapping `0 → Monday` …
`6 → Sunday`. The same index drives the `E` format token (ISO weekday).
[[src/builtins/datetime_package.mfb:__datetime_weekday]]

`dayOfYear` is `daysFromCivil(date) - daysFromCivil(year,1,1) + 1`.

### Floor division for calendar use

The language `/` and `MOD` truncate toward zero, but projecting a possibly
negative epoch-second into a day index and a second-of-day requires *flooring*.
The package defines `floorDiv` / `floorMod` (adjust the truncated quotient down
when the remainder is negative) and uses them whenever a value can be negative —
day-of-epoch splitting, weekday index, and `addMonths` month rollover.
[[src/builtins/datetime_package.mfb:__datetime_floorDiv]]

## Zones, projection, and the OS clock/zone seam

Only two things require the host: the current time and the local zone's offset.
They are reached through three intrinsics, the **OS seam**. Everything else is
portable.

| Intrinsic | Signature | Lowering |
| --- | --- | --- |
| `datetime::nowNanos()` | `() → Integer` | `clock_gettime(CLOCK_REALTIME)` → `sec*1e9 + nsec` |
| `datetime::monotonicNanos()` | `() → Integer` | `clock_gettime(CLOCK_MONOTONIC)` → nanoseconds |
| `datetime::localOffset(epochSeconds)` | `(Integer) → Integer` | `localtime_r(&t, &tm)` → `tm.tm_gmtoff` |

These three are excluded from the public-call rewrite (`implementation_name`
returns `None`); they lower to runtime helpers
(`_mfb_rt_datetime_datetime_*`) rather than to `__datetime_*` MFBASIC code. They
take no failure path — each returns an `Integer` with the OK tag set.
[[src/builtins/datetime.rs:NOW_NANOS]]

Platform notes from the native lowering: `CLOCK_REALTIME` is `0` on both Linux
and macOS; `CLOCK_MONOTONIC` is `1` on Linux but `6` on Darwin. `localOffset`
stashes its `epochSeconds` argument as a `time_t`, calls `localtime_r`, and
reads the `tm_gmtoff` field (offset `40` in `struct tm` on both glibc and Darwin
BSD libc). The host's TZ database / `TZ` environment variable therefore governs
local-zone results — DST transitions and historical offsets are whatever libc
reports for that instant. [[src/target/shared/code/datetime.rs:lower_datetime_helper]]

### Zone constructors

`utc` is `Zone[0, Utc, "UTC"]`; `local` is `Zone[0, Local, "Local"]` (its
`offsetSeconds` is a placeholder — the real offset is queried per-instant).
`fixedOffset` takes either total seconds or `(hours, minutes)`; the magnitude
must be under 24h (`|offset| < 86400`) and minutes `0..59`, else
`ErrInvalidArgument` (`77050002`). The label is rendered `±HH:MM`.
[[src/builtins/datetime_package.mfb:__datetime_fixedOffset1]]

`offsetAt(zone, at)` returns `localOffset(at.seconds)` for a `Local` zone and
the stored `offsetSeconds` otherwise — so a `Local` zone's effective offset is
resolved against the specific instant (DST-correct). [[src/builtins/datetime_package.mfb:__datetime_offsetAt]]

### Projection: instant ↔ civil

`inZone(at, zone)` projects an `Instant` into a zone: it adds the zone offset to
the epoch seconds, `floorDiv`/`floorMod` by `86400` to split day vs
second-of-day, runs `civilFromDays`, and packs the `DateTime` with the resolved
offset cached. `toUtc` / `toLocal` are `inZone` against the standard zones.
[[src/builtins/datetime_package.mfb:__datetime_inZone]]

`resolve(dt)` is the inverse for a `DateTime` whose offset is already known:
`epochSeconds = daysFromCivil*86400 + h*3600 + m*60 + s - dt.offset`.

`civil(date, time, zone)` constructs a `DateTime` from wall-clock fields. The
hard case is a `Local` zone where the offset depends on the very instant being
constructed. `resolveLocal` handles a single DST transition near the local time:
it probes the offset one day on each side to bracket the transition, then
applies the §"DST policy" below. `withZone(dt, z)` re-projects through
`resolve` then `inZone`. [[src/builtins/datetime_package.mfb:__datetime_resolveLocal]]

**DST policy** (`resolveLocal`): with no transition in the bracket, use the
common offset. Across a transition: an unambiguous time uses the bracketing
offset; a **fall-back overlap** (the wall time occurs twice) takes the *earlier*
offset; a **spring-forward gap** (the wall time never occurs) shifts forward
onto the post-transition offset.

### Calendar arithmetic stays DST-aware

`addDays` and `addMonths` operate on the civil wall-clock fields, then
re-resolve the offset through the value's own zone via `civil`, so they remain
DST-correct (adding a day across a transition keeps the same wall time, not the
same elapsed duration). `addMonths` clamps an overflowing day to the target
month's length (e.g. Jan 31 + 1 month → Feb 28/29). `startOfDay` is `civil` at
`00:00:00.0` in the value's zone. [[src/builtins/datetime_package.mfb:__datetime_addMonths]]

## Format grammar

`format(dt, pattern)` walks the pattern, emitting literal characters
unchanged, copying single-quoted runs verbatim (`''` is a literal quote), and
expanding **runs** of a recognized letter (the run length selects width/style).
An unrecognized letter run fails `ErrInvalidFormat` (`77050003`).
[[src/builtins/datetime_package.mfb:__datetime_formatToken]]

| Token | Meaning | Run-length behavior |
| --- | --- | --- |
| `y` | year | `yy` = last 2 digits; otherwise zero-pad to run length |
| `M` | month | `M`=numeric, `MM`=2-digit, `MMM`=short name, `MMMM`=full name |
| `d` | day | `d`=numeric, `dd`=2-digit |
| `H` | hour 0–23 | `H`=numeric, `HH`=2-digit |
| `h` | hour 1–12 | `h`=numeric, `hh`=2-digit |
| `m` | minute | `m`=numeric, `mm`=2-digit |
| `s` | second | `s`=numeric, `ss`=2-digit |
| `f` | fractional second | first *run-length* digits of the 9-digit nanos |
| `a` | AM/PM | from hour < 12 |
| `E` | weekday name | `EEEE`+ = full, shorter = abbreviated |
| `Z` | zone offset | `Z` = `Z` if offset 0 else `±HH:MM`; `ZZ` = always `±HH:MM`; `ZZZ`+ = `±HHMM` (compact) |

`toIso(dt)` is `format(dt, "yyyy-MM-dd'T'HH:mm:ss.fffZ")`. `formatDuration(d)`
renders a signed span as `[Nd ]HH:MM:SS.mmm` (millisecond resolution, leading
day part only when non-zero). [[src/builtins/datetime_package.mfb:__datetime_toIso]]

## Parse grammar

Parsing is pattern-driven: a scanner walks `pattern` and `value` in lockstep,
filling field accumulators in a `__datetime_Fields` record. Absent fields keep
epoch/zero defaults (`year=1970, month=1, day=1`, all time fields `0`). A
structural mismatch — wrong literal, missing digits, bad AM/PM, bad month name,
bad offset — fails `ErrInvalidFormat` (`77050003`). The pattern letters mirror
`format`. [[src/builtins/datetime_package.mfb:__datetime_parseFields]]

Field-read rules:

* Numeric tokens read up to a token-specific digit cap (`y` up to its run
  length; `M`/`d`/`H`/`h`/`m`/`s` up to 2; `f` up to its run length). At least
  one digit is required.
* `yy` is interpreted as `2000 + value`.
* `M` with run length ≥ 3 reads a month **name** (case-insensitive, full or
  3-letter abbreviation) via `monthFromName`; otherwise a 1–2 digit number.
* `f` reads its run-length digits then *scales up* to 9-digit nanoseconds.
* `h` sets a 12-hour flag; `a` records AM/PM. `buildFromFields` then folds the
  12-hour clock: PM + hour < 12 adds 12; AM + hour 12 becomes 0.
* `E` skips a weekday name (consumed but not validated against the date).
* `Z` reads an offset via `readOffset`: `Z`/`z` → 0, else `±HH[:]MM`. When an
  offset is present the result is a fixed-offset `DateTime`; when absent, the
  fields are resolved through the supplied `zone` (default UTC) via `civil`.

`parseIso(value)` is a dedicated, hand-rolled scanner for
`YYYY-MM-DD(T| )HH:MM:SS[.frac][offset]`: a `.`-fractional part of any length is
read then scaled (extra digits beyond 9 are skipped), and a trailing offset
(`Z`/`±HH:MM`/`±HHMM`) is required. It always yields a fixed-offset `DateTime`.
[[src/builtins/datetime_package.mfb:__datetime_parseIso]]

## Validation

`date(y, m, d)` rejects `month` outside `1..12` and `day` outside
`1..daysInMonth`; `time(h, mi, s, ns)` rejects `hour` > 23, `minute`/`second`
> 59, `nanos` > 999_999_999. All raise `ErrInvalidArgument` (`77050002`). Note
that the bare `Instant`/`Time`/`Date` *record literals* used internally by the
projection helpers do **not** re-validate — validation lives in the named
constructors. [[src/builtins/datetime_package.mfb:__datetime_date]]

## See Also

* ./mfb man datetime — the per-function API: signatures, overloads, and examples
* ./mfb spec stdlib math-rng — the other OS-seam stdlib (per-arena PRNG, entropy seam)
* ./mfb spec unicode strings-model — grapheme indexing behind `strings::mid`, used by the parse/format scanners
* ./mfb spec language types — `Integer` checked arithmetic and `ErrOverflow`, the record/enum value model
* ./mfb spec language error-model — `FAIL error(code, msg)`, `ErrInvalidArgument` / `ErrInvalidFormat`, and auto-propagation
* ./mfb spec architecture frontend — how a built-in source package is injected, monomorphized, and the public-call seam is rewritten
