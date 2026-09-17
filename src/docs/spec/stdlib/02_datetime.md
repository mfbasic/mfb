# Date and Time Model

The `datetime::` package is a built-in source package: all calendar math,
formatting, and parsing are written in MFBASIC as internal `__datetime_*`
functions, and only the OS clock and the local-zone table are platform state.
A compiler seam owns registration, the checkers' metadata, and the rewrite from
each public `datetime::` call onto its internal implementation; the MFBASIC
source owns the algorithm. [[src/codegen/builtins/datetime/mod.rs:register]] [[src/codegen/builtins/datetime/func_add_days.rs:register]] This topic specifies the
**model** — the record shapes, the civil-calendar math, the clock/zone seam, and
the parse/format grammar. The per-function API surface is owned by
`./mfb man datetime`.

The package is injected only when a program `IMPORT datetime`; otherwise its AST
is not added to the project. [[src/codegen/builtins/datetime/mod.rs:uses_package]]

## Value model

Every public type is a flat, copyable record or enum — no handles, no hidden
state. [[src/codegen/builtins/datetime/mod.rs:Instant]]

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
[[src/codegen/builtins/datetime/mod.rs:register]]

### Canonical form and normalization

An `Instant`/`Duration` is **canonical** when `nanos` is in
`[0, 1_000_000_000)`. Every constructor that can receive a non-canonical
`nanos` (the two-argument and component forms) and every arithmetic result is
funnelled through `__datetime_normInstant` / `__datetime_normDuration`, which
carry a raw `(seconds, nanos)` pair into canonical form; the seconds-only
constructors build the trivially canonical `[seconds, 0]` directly. Because `/` truncates toward zero
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
`equals`) orders on `seconds` then `nanos`. [[src/codegen/builtins/datetime/helper_norm_instant.rs:__datetime_normInstant]]
The builders with more than one argument and every arithmetic member use checked
`Integer` arithmetic, so a sum or product outside the `Integer` range raises
`ErrOverflow` (`77050010`). The one-argument `instant`/`duration` store the value
unchanged and never raise; comparison never raises.
[[src/codegen/builtins/datetime/func_add.rs:__datetime_add]]

`fromMillis` converts against the epoch with the same borrow logic. It only
divides, so it never raises. `toMillis`/`toNanos` are a straight multiply-add on
the already-canonical pair, so a value outside the `Integer` range raises
`ErrOverflow` (`77050010`).
[[src/codegen/builtins/datetime/func_from_millis.rs:__datetime_fromMillis]]
[[src/codegen/builtins/datetime/func_to_millis.rs:__datetime_toMillis]]

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

Both intrinsics return non-negative nanoseconds on any sane host, so the
truncating-divide split into `(seconds, nanos)` still passes through the
normalizers but their borrow branch is a no-op for non-negative input. [[src/codegen/builtins/datetime/func_monotonic.rs:__datetime_monotonic]]

## Portable civil-calendar math

All date math is platform-independent and runs in MFBASIC. The epoch-day
conversions use Howard Hinnant's branch-free civil ↔ days algorithm, valid
across the full `Integer` range; the explicit era adjustments keep every divisor
operand non-negative so truncating division equals flooring.
[[src/codegen/builtins/datetime/helper_days_from_civil.rs:__datetime_daysFromCivil]]

* `daysFromCivil(y, m, d)` → days since `1970-01-01` (the `719468` constant
  shifts from the `0000-03-01` internal era origin to the Unix epoch).
* `civilFromDays(z)` → `Date`, the inverse.

The proleptic Gregorian calendar is used for *all* years, including before its
historical adoption; there is no year 0 discontinuity special-casing beyond the
algorithm's own era arithmetic.

**Leap year:** divisible by 4, except centuries, except multiples of 400.
[[src/codegen/builtins/datetime/func_is_leap_year.rs:__datetime_isLeapYear]]

```
isLeapYear(y) = (y MOD 4 = 0 AND y MOD 100 <> 0) OR y MOD 400 = 0
```

**Days in month:** February is 29 in leap years else 28; April, June,
September, November are 30; all others 31. [[src/codegen/builtins/datetime/func_days_in_month.rs:__datetime_daysInMonth]]

**Day of week:** computed directly from the epoch-day number, not from a
table. The epoch day `1970-01-01` is a Thursday; the package re-bases it to a
Monday-origin index with `floorMod(days + 3, 7)`, mapping `0 → Monday` …
`6 → Sunday`. The same index drives the `E` format token (ISO weekday).
[[src/codegen/builtins/datetime/func_weekday.rs:__datetime_weekday]]

`dayOfYear` is `daysFromCivil(date) - daysFromCivil(year,1,1) + 1`.

### Floor division for calendar use

The language `/` and `MOD` truncate toward zero, but projecting a possibly
negative epoch-second into a day index and a second-of-day requires *flooring*.
The package defines `floorDiv` / `floorMod` (adjust the truncated quotient down
when the remainder is negative) and uses them whenever a value can be negative —
day-of-epoch splitting, weekday index, and `addMonths` month rollover.
[[src/codegen/builtins/datetime/helper_floor_div.rs:__datetime_floorDiv]]

## Zones, projection, and the OS clock/zone seam

Only two things require the host: the current time and the local zone's offset.
They are reached through three intrinsics, the **OS seam**. Everything else is
portable.

| Intrinsic | Signature | Lowering |
| --- | --- | --- |
| `datetime::nowNanos()` | `() → Integer` | `clock_gettime(CLOCK_REALTIME)` → checked `sec*1e9 + nsec` |
| `datetime::monotonicNanos()` | `() → Integer` | `clock_gettime(CLOCK_MONOTONIC)` → checked `sec*1e9 + nsec` |
| `datetime::localOffset(epochSeconds)` | `(Integer) → Integer` | `localtime_r(&t, &tm)` → `tm.tm_gmtoff` |

These three are excluded from the public-call rewrite; they lower to runtime
helpers
(`_mfb_rt_datetime_datetime_*`) rather than to `__datetime_*` MFBASIC code.
All three use the fallible result form. `nowNanos` and `monotonicNanos` return
an `Integer` with the OK tag set, or raise `ErrOverflow` (`77050010`) when the
reading's nanosecond count does not fit an `Integer` — for the wall clock, a
reading before `1677-09-21T00:12:43.145224192Z` or after
`2262-04-11T23:47:16.854775807Z`. `localOffset` can fail with
`ErrInvalidArgument` (see below). The error propagates through the MFBASIC
wrappers `now` and `monotonic`, which call these helpers without a `TRAP`, so all
four members declare `ErrOverflow`. [[src/codegen/builtins/datetime/func_now_nanos.rs:lower_now_nanos]] [[src/codegen/builtins/datetime/func_monotonic_nanos.rs:lower_monotonic_nanos]] [[src/codegen/builtins/datetime/func_local_offset.rs:lower_local_offset]] [[src/codegen/builtins/datetime/func_now.rs:__datetime_now]] [[src/codegen/builtins/datetime/func_monotonic.rs:__datetime_monotonic]]

The overflow check is exact, not a bound on the multiply alone. The libc fold
computes `tv_sec*1e9 + tv_nsec` as a 128-bit value — the signed-high multiply
gives the product's high word, and `tv_nsec` is added sign-extended through two
explicit-carry limbs — and the result fits exactly when the high word equals the
sign extension of the low word. A separate multiply check would wrongly reject a
reading such as `(-9223372037, 145224192)`, whose product leaves the range but
whose sum is exactly `Integer` min. Every failure branches to one fail tail
placed after the OK return. [[src/codegen/builtins/datetime/gen_shared.rs:emit_libc_clock_nanos]] [[src/codegen/builtins/datetime/gen_shared.rs:emit_clock_overflow_tail]]

The Windows lowerings are checked to the same contract. `nowNanos` rebases
`GetSystemTimePreciseAsFileTime` as `(FILETIME - 116444736000000000) * 100`: a
FILETIME at or above `2^63` overflows outright, below it the rebase cannot wrap,
and the `* 100` is checked with the same signed-high comparison.
`monotonicNanos` folds `QueryPerformanceCounter`/`QueryPerformanceFrequency` as
`(counter/freq)*1e9 + ((counter%freq)*1e9)/freq`; the whole-second product is
checked through its unsigned high word and its low word's sign, and the final add
of two non-negative values overflows exactly when the sum reads negative. The
fraction's intermediate `(counter%freq)*1e9` is unchecked `u64` arithmetic and
is exact only while `freq` is below `2^64 / 1e9` (about 18.4 GHz).
[[src/codegen/builtins/datetime/func_now_nanos.rs:lower_now_nanos]] [[src/codegen/builtins/datetime/func_monotonic_nanos.rs:lower_monotonic_nanos]]

Platform notes from the native lowering: `CLOCK_REALTIME` is `0` on both Linux
and macOS; `CLOCK_MONOTONIC` is `1` on Linux but `6` on Darwin. `localOffset`
stashes its `epochSeconds` argument as a `time_t`, calls `localtime_r`, and
reads the `tm_gmtoff` field (offset `40` in `struct tm` on both glibc and Darwin
BSD libc). The host's TZ database / `TZ` environment variable therefore governs
local-zone results — DST transitions and historical offsets are whatever libc
reports for that instant. `localtime_r` returns `NULL` when the instant's year
overflows `tm_year`'s `int` (roughly `abs(epochSeconds)` beyond `6.7e16`),
leaving `tm` unwritten; `localOffset` branches on that return and raises
`ErrInvalidArgument` (`77050002`) rather than reading the uninitialized buffer,
and the error propagates through `offsetAt`/`toLocal` for a `Local` zone.
[[src/codegen/builtins/datetime/func_local_offset.rs:lower_local_offset]]

### Zone constructors

`utc` is `Zone[0, Utc, "UTC"]`; `local` is `Zone[0, Local, "Local"]` (its
`offsetSeconds` is a placeholder — the real offset is queried per-instant).
`fixedOffset` takes either total seconds or `(hours, minutes)`; the magnitude
must be under 24h (`|offset| < 86400`) and minutes `0..59`, else
`ErrInvalidArgument` (`77050002`). The label is rendered `±HH:MM`.
[[src/codegen/builtins/datetime/mod.rs:__datetime_fixedOffset1]]

`offsetAt(zone, at)` returns `localOffset(at.seconds)` for a `Local` zone and
the stored `offsetSeconds` otherwise — so a `Local` zone's effective offset is
resolved against the specific instant (DST-correct). [[src/codegen/builtins/datetime/func_offset_at.rs:__datetime_offsetAt]]

### Projection: instant ↔ civil

`inZone(at, zone)` projects an `Instant` into a zone: it adds the zone offset to
the epoch seconds, `floorDiv`/`floorMod` by `86400` to split day vs
second-of-day, runs `civilFromDays`, and packs the `DateTime` with the resolved
offset cached. `toUtc` / `toLocal` are `inZone` against the standard zones.
[[src/codegen/builtins/datetime/func_in_zone.rs:__datetime_inZone]]

`resolve(dt)` is the inverse for a `DateTime` whose offset is already known:
`epochSeconds = daysFromCivil*86400 + h*3600 + m*60 + s - dt.offset`.

`civil(date, time, zone)` constructs a `DateTime` from wall-clock fields. The
hard case is a `Local` zone where the offset depends on the very instant being
constructed. `resolveLocal` handles a single DST transition near the local time:
it probes the offset one day on each side to bracket the transition, then
applies the §"DST policy" below. `withZone(dt, z)` re-projects through
`resolve` then `inZone`. [[src/codegen/builtins/datetime/helper_resolve_local.rs:__datetime_resolveLocal]]

The two are the package's opposite zone operations and are easy to confuse
(bug-518 was `withZone`'s own parameter row asserting the wrong one).
`withZone(dt, z)` **preserves the instant** and re-derives the civil fields:
`resolve(withZone(dt, z)) = resolve(dt)` for every `z`. `civil(dt.date, dt.time,
z)` **preserves the civil fields** and therefore names a different instant. The
`withZone` identity is pinned by
`tests/rt-behavior/datetime/datetime-withzone-instant-rt`, because prose cannot
be gated but the property it describes can.

**DST policy** (`resolveLocal`): with no transition in the bracket, use the
common offset. Across a transition: an unambiguous time uses the bracketing
offset; a **fall-back overlap** (the wall time occurs twice) takes the *earlier*
offset; a **spring-forward gap** (the wall time never occurs) shifts forward
onto the post-transition offset.

### Calendar arithmetic stays DST-aware

`addDays` and `addMonths` operate on the civil wall-clock fields and keep the
value's offset when it is still valid for the new wall clock; only otherwise do
they re-resolve through the value's own zone via `civil`. So they remain
DST-correct (adding a day across a transition keeps the same wall time, not the
same elapsed duration), a zero shift returns the value unchanged, and a result
in a fall-back overlap stays on the side the original offset names when that
offset is one of the overlap's two (bug-520 S6; Java's `ZonedDateTime.plusDays`
rule). For a `Local` zone the result depends on the host's zone rules.
[[src/codegen/builtins/datetime/helper_civil_keep_offset.rs:__datetime_civilKeepOffset]] `addMonths` clamps an overflowing day to the target
month's length (e.g. Jan 31 + 1 month → Feb 28/29). `startOfDay` is `civil` at
`00:00:00.0` in the value's zone. [[src/codegen/builtins/datetime/func_add_months.rs:__datetime_addMonths]]

## Format grammar

`format(dt, pattern)` walks the pattern, emitting literal characters
unchanged, copying single-quoted runs verbatim (`''` is a literal quote), and
expanding **runs** of a recognized letter (the run length selects width/style).
An unrecognized letter run fails `ErrInvalidFormat` (`77050003`).
[[src/codegen/builtins/datetime/helper_format_token.rs:__datetime_formatToken]]

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
| `Z` | zone offset | `Z` = `Z` if offset 0 else `±HH:MM`; `ZZ` = always `±HH:MM`; `ZZZ`+ = `±HHMM` (compact). An offset that is not whole minutes appends seconds: `±HH:MM:SS` / `±HHMMSS` |

`toIso(dt)` is `format(dt, "yyyy-MM-dd'T'HH:mm:ss.fffZ")`. It is arity-split: the
two-argument `toIso(dt, digits)` selects the fractional width from `{0, 3, 6, 9}`
(`0` omits the fractional field; any other value is `ErrInvalidArgument`,
`77050002`), and the one-argument form is defined as `toIso(dt, 3)`, so the
default output is millisecond-fixed by construction. Because a `DateTime` carries
nanoseconds, **only `digits = 9` round-trips through `parseIso` losslessly**
(bug-521: the page previously promised a round trip and truncated); every
narrower width truncates toward zero. `formatDuration(d)`
renders a signed span as `[Nd ]HH:MM:SS.mmm` (millisecond resolution, leading
day part only when non-zero). [[src/codegen/builtins/datetime/func_to_iso.rs:__datetime_toIso]]

## Parse grammar

Parsing is pattern-driven: a scanner walks `pattern` and `value` in lockstep,
filling field accumulators in a `__datetime_Fields` record. Absent fields keep
epoch/zero defaults (`year=1970, month=1, day=1`, all time fields `0`). A
structural mismatch — wrong literal, missing digits, bad AM/PM, bad month name,
bad offset, or text left over after the pattern ends — fails `ErrInvalidFormat`
(`77050003`). The pattern must consume the whole value. The pattern letters
mirror `format`. [[src/codegen/builtins/datetime/helper_parse_fields.rs:__datetime_parseFields]]

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
* `Z` reads an offset via `readOffset`: `Z`/`z` → 0, else `±HH:MM[:SS]` or
  `±HHMM[SS]`, every field exactly two digits. The seconds form is what `format`
  writes for an offset that is not whole minutes, so writer and reader agree on
  every offset (bug-520 S1/S2). When an
  offset is present the result is a fixed-offset `DateTime`; when absent, the
  fields are resolved through the supplied `zone` (default UTC) via `civil`.

`parseIso(value)` is a dedicated, hand-rolled scanner for
`YYYY-MM-DD(T|t| )HH:MM:SS[.frac][offset]`: a `.`-fractional part of any length
is read then scaled (extra digits beyond 9 are skipped), and a trailing offset
(`Z`/`z`/`±HH:MM[:SS]`/`±HHMM[SS]`) is required and must end the value. It always yields a fixed-offset `DateTime`.
[[src/codegen/builtins/datetime/func_parse_iso.rs:__datetime_parseIso]]

### Decoded fields are range-checked

Both readers bound the decoded calendar fields before assembling them, against
exactly the ranges the `date`/`time` constructors enforce (see §"Validation").
`parse` checks in `buildFromFields`, after the 12-hour/AM-PM fold and before the
`Date`/`Time` record literals; `parseIso` checks after the offset read, at the
same point. The offset is bounded as it is read: hours `00..23`, minutes and
seconds `00..59`, each exactly two digits, so its magnitude is under 24 h and
the zone constructor's `ErrInvalidArgument` is never reached (bug-520 S4/S5).
[[src/codegen/builtins/datetime/helper_read_offset.rs:__datetime_readOffset]] A field out of range fails `ErrInvalidFormat` (`77050003`) — the
structural-mismatch code, not the constructors' `ErrInvalidArgument`, because
the argument is a well-formed `String` and it is the *text* that is malformed.
[[src/codegen/builtins/datetime/helper_check_fields.rs:__datetime_checkFields]]

This bound is what keeps the readers separate from the calendar arithmetic. The
civil-days conversion is deliberately *total* — month 13 is "twelve months plus
one", day 45 is "day 1 plus 44 days" — because `addMonths`/`addDays` need
exactly that rollover. Reached with no prior bound (bug-519), that totality
laundered invalid text into a valid-looking date: `parse("2026-13-45 25:70:99",
"yyyy-MM-dd HH:mm:ss")` returned `2027-02-15T02:11:39Z` with no error, while
`date(2026, 13, 45)` refused the identical fields. The arithmetic is unchanged;
the readers no longer reach it with input the constructors would reject.

## Validation

`date(y, m, d)` rejects `month` outside `1..12` and `day` outside
`1..daysInMonth`; `time(h, mi, s, ns)` rejects `hour` outside `0..23`,
`minute`/`second` outside `0..59`, `nanos` outside `0..999_999_999`. All raise `ErrInvalidArgument` (`77050002`). Note
that the bare `Instant`/`Time`/`Date` *record literals* used internally by the
projection helpers do **not** re-validate — validation lives at the package's
input boundary. [[src/codegen/builtins/datetime/func_date.rs:__datetime_date]]

There are exactly three such boundaries, and all three enforce the same ranges:
the two constructors above, and the parse readers (`__datetime_checkFields`,
above), which raise `ErrInvalidFormat` (`77050003`) instead because their input
is text. An out-of-range `Date`/`Time` is therefore not constructible from
outside the package, which is why `civil` trusts its arguments rather than
re-checking them.

## See Also

* ./mfb man datetime — the per-function API: signatures, overloads, and examples
* ./mfb spec stdlib math-rng — the other OS-seam stdlib (per-arena PRNG, entropy seam)
* ./mfb spec unicode strings-model — grapheme indexing behind `strings::mid`, used by the parse/format scanners
* ./mfb spec language types — `Integer` checked arithmetic and `ErrOverflow`, the record/enum value model
* ./mfb spec language error-model — `FAIL error(code, msg)`, `ErrInvalidArgument` / `ErrInvalidFormat`, and auto-propagation
* ./mfb spec architecture frontend — how a built-in source package is injected, monomorphized, and the public-call seam is rewritten
