# plan-01: `datetime::` built-in package

Status: **design**. Target: a built-in `datetime::` package covering instants,
civil dates and times, durations, time zones (UTC, fixed-offset, and the host's
local zone), and string formatting / parsing. Built like `regex::` and `json::`:
an MFBASIC **source package** (`src/builtins/datetime_package.mfb`) carrying all
of the calendar arithmetic and text handling, sitting on a **tiny Rust intrinsic
seam** (`src/builtins/datetime.rs`) that is the only part allowed to touch the OS
clock and the local-zone table.

It complements:

- `specifications/mfbasic.md`
- `specifications/standard_package.md`
- `specifications/package_format.md`
- `specifications/error_codes.md`

## 1. Goals & non-goals

Goals:

- **A single source of truth for "an instant"** — `datetime::Instant`, an absolute
  point on the UTC timeline (Unix epoch, leap-second-free). Every other operation
  is defined as a total/partial function over instants and zones.
- **Civil types** — `Date`, `Time`, `DateTime` (a date + time + resolved UTC
  offset) for human-facing calendar work.
- **Durations & arithmetic** — `Duration` (signed span), `now`/`monotonic`,
  add/subtract, difference between instants, comparison.
- **Zones, pragmatically** — `UTC`, arbitrary **fixed offsets**, and the **host's
  local zone** (DST-correct via the platform's `localtime` table). Convert an
  instant to/from civil time in any of these.
- **Formatting & parsing** — a small, explicit pattern mini-language (`yyyy-MM-dd`
  …) plus first-class **ISO 8601 / RFC 3339** helpers.
- **Plain value semantics** — every public type is a **copyable record or enum**.
  No `RES` resources, no escaping `MUT`-capturing closures, no hidden global state.
  Two programs given the same inputs produce identical results on every target.

Non-goals (v1):

- **Named IANA zones** (`"America/New_York"`). Embedding the tz database is a large,
  separately-versioned data dependency. v1 supports UTC, fixed offsets, and *local*
  only. §8 keeps the seam so a future `datetime::zone("…")` can slot in.
- **Leap seconds.** We use the POSIX convention (every day is 86 400 seconds). UTC
  instants are leap-second-free; this is intentional and matches `clock_gettime`.
- **Calendars other than the proleptic Gregorian.**
- **Locale-aware formatting** (translated month/day names, locale digit shaping).
  Names are English; numeric formats are fixed. A future locale seam is out of scope.
- **Sub-nanosecond precision**, and any monotonic-clock persistence across processes.

## 2. Core model (read this first)

Three ideas keep the package sound and total where it can be:

1. **`Instant` is the truth; everything civil is a projection.** An `Instant` is
   `seconds` (signed, Unix epoch UTC) + `nanos` (`0 .. 999_999_999`). It carries no
   zone. You only get a `DateTime` by **projecting** an instant **through a zone**;
   you only get back an instant by **resolving** a `DateTime` (which already pins its
   offset). This removes the classic "naive vs aware" ambiguity: civil values that
   came from a projection always know their offset.

2. **Calendar math is pure integer arithmetic, done in MFBASIC.** Civil ⇄ epoch uses
   Howard Hinnant's `days_from_civil` / `civil_from_days` (branch-free, valid across
   the full `Integer` range, no tables). It lives in `datetime_package.mfb`, so it is
   monomorphized and optimized like user code and needs no Rust support.

3. **The OS is touched in exactly three places.** The Rust seam (§8) exposes only:
   the wall clock (`now`), a monotonic counter (`monotonic`), and the **local UTC
   offset for a given instant** (DST-aware, from the platform's zone table). Nothing
   else in the package is platform-dependent, so behavior is identical everywhere
   except where the host clock/zone legitimately differ.

**Range & precision.** `Instant.seconds` spans the full 64-bit `Integer`, so civil
dates range far beyond any practical need. `datetime::now()` is additionally bounded
by its intrinsic (returns nanoseconds-since-epoch, valid through **year 2262**); this
is documented at the call, not a limit on `Instant` itself.

## 3. Types & enums

All records are **copyable value types** (every field is a copyable primitive). No
field is ever a resource or a collection of resources.

```basic
TYPE Instant
    seconds AS Integer    ' whole seconds since 1970-01-01T00:00:00Z (UTC), may be negative
    nanos   AS Integer    ' 0 .. 999_999_999, always non-negative
END TYPE

TYPE Duration
    seconds AS Integer    ' signed; whole seconds
    nanos   AS Integer    ' 0 .. 999_999_999; the represented span is seconds + nanos/1e9
END TYPE

TYPE Date
    year  AS Integer      ' proleptic Gregorian, e.g. 2026; year 0 = 1 BC
    month AS Integer      ' 1 .. 12
    day   AS Integer      ' 1 .. 31 (valid for the month/year)
END TYPE

TYPE Time
    hour   AS Integer     ' 0 .. 23
    minute AS Integer     ' 0 .. 59
    second AS Integer     ' 0 .. 59 (no leap seconds)
    nanos  AS Integer     ' 0 .. 999_999_999
END TYPE

TYPE Zone
    offsetSeconds AS Integer   ' fixed UTC offset for FixedOffset/Utc; ignored for Local
    kind          AS Integer   ' ZoneKind, stored as Integer (Utc=0, FixedOffset=1, Local=2)
    label         AS String    ' display label, e.g. "UTC", "+05:30", "Local"
END TYPE

TYPE DateTime
    date   AS Date
    time   AS Time
    zone   AS Zone        ' the zone this civil value was projected through
    offset AS Integer     ' resolved UTC offset in seconds (DST already applied)
END TYPE
```

Enums:

```basic
datetime::ZoneKind   Utc, FixedOffset, Local
datetime::Weekday    Monday, Tuesday, Wednesday, Thursday, Friday, Saturday, Sunday
datetime::Month      January, February, March, April, May, June, July, August, _
                     September, October, November, December
```

Notes:

- `Zone.kind` is stored as `Integer` inside the record so `Zone` stays a flat
  copyable record; constructors (`datetime::utc()`, `datetime::fixedOffset(...)`,
  `datetime::local()`) are the supported way to build one. `ZoneKind`/`Weekday`/
  `Month` enums are returned by accessors (`datetime::weekday(...)` → `Weekday`).
- `DateTime.offset` is the *resolved* offset (DST applied at that instant), so a
  `DateTime` is always self-describing and round-trips back to its `Instant`
  without re-consulting the zone.

## 4. Error handling

`datetime::` reuses the generic package-helper range `7-705-*` (see
`specifications/error_codes.md`); no new subsystem prefix is introduced, matching
how `regex::` and `collections::` reuse it.

| Condition | Code |
|-----------|------|
| Out-of-range field passed to a constructor (e.g. `month = 13`, `day = 32`, `hour = 24`) | `errorCode::ErrInvalidArgument` (`77050002`) |
| String fails to parse for the given pattern / not valid ISO 8601 | `errorCode::ErrInvalidFormat` (`77050003`) |
| Unknown / unsupported pattern token in a format or parse string | `errorCode::ErrInvalidFormat` (`77050003`) |
| Arithmetic overflows the `Integer` range (e.g. adding a huge `Duration`) | `errorCode::ErrOverflow` (`77050010`) |
| Feature reserved for a later version (e.g. a named IANA zone) | `errorCode::ErrUnsupported` (`77050007`) |

Constructors are **validating**: `datetime::date(2026, 2, 29)` fails with
`ErrInvalidArgument` (2026 is not a leap year). Total/clamping variants are noted in
the surface (§5) where they exist.

## 5. Function surface

Signatures use MFBASIC syntax. Functions are pure unless they read the host clock or
zone (only `now`, `monotonic`, `local`, and zone-resolution through `Local` do that).

### 5.1 Clock & construction

| Function | Signature | Behavior |
|----------|-----------|----------|
| `datetime::now` | `FUNC now() AS Instant` | Current wall-clock instant (UTC). Reads the host clock; valid through year 2262 (§2). |
| `datetime::monotonic` | `FUNC monotonic() AS Duration` | A monotonically non-decreasing span from an arbitrary fixed origin, for measuring elapsed time. Not related to wall time; not comparable across processes. |
| `datetime::instant` | `FUNC instant(seconds AS Integer) AS Instant` | Builds the `Instant` at `seconds` after the Unix epoch (nanos 0). |
| `datetime::instant` | `FUNC instant(seconds AS Integer, nanos AS Integer) AS Instant` | As above; normalizes `nanos` into `[0,1e9)` carrying into `seconds`. |
| `datetime::instant` | `FUNC instant(mins AS Integer, seconds AS Integer, nanos AS Integer) AS Instant` | Component builder (§5.1.1): the instant `mins*60 + seconds` (plus `nanos`) after the epoch. |
| `datetime::instant` | `FUNC instant(hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Instant` | Component builder: `hours*3600 + mins*60 + seconds` after the epoch. |
| `datetime::instant` | `FUNC instant(days AS Integer, hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Instant` | Component builder: `days*86400 + hours*3600 + mins*60 + seconds` after the epoch. |
| `datetime::date` | `FUNC date(year AS Integer, month AS Integer, day AS Integer) AS Date` | Validating constructor. Fails `ErrInvalidArgument` on an impossible calendar date. |
| `datetime::time` | `FUNC time(hour AS Integer, minute AS Integer, second AS Integer = 0, nanos AS Integer = 0) AS Time` | Validating constructor. Not overloaded, so its trailing defaults apply. |
| `datetime::duration` | `FUNC duration(seconds AS Integer) AS Duration` | A span of `seconds` (nanos 0). |
| `datetime::duration` | `FUNC duration(seconds AS Integer, nanos AS Integer) AS Duration` | Normalizes `nanos` into `[0,1e9)`. |
| `datetime::duration` | `FUNC duration(mins AS Integer, seconds AS Integer, nanos AS Integer) AS Duration` | Component builder (§5.1.1): `mins*60 + seconds` plus `nanos`. |
| `datetime::duration` | `FUNC duration(hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Duration` | Component builder: `hours*3600 + mins*60 + seconds` plus `nanos`. |
| `datetime::duration` | `FUNC duration(days AS Integer, hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Duration` | Component builder: `days*86400 + hours*3600 + mins*60 + seconds` plus `nanos`. |

#### 5.1.1 Why the constructor overloads carry no defaults

`instant` and `duration` are **overloaded** (component builders for ergonomics:
`duration(2, 30, 0)` = 2 min 30 s). MFBASIC's overloading rules (`mfbasic.md` §6)
force two design choices here:

- **No default arguments in an overload set.** Trailing defaults are filled only for
  a name with a *single* declaration; within an overload set every parameter must be
  supplied. So `nanos` is **mandatory** on every component form, and the plain
  seconds form is split into two explicit overloads (`(seconds)` and
  `(seconds, nanos)`) instead of one `(seconds, nanos = 0)`.
- **Disjoint arities.** An overload's identity is its ordered parameter *types*. Since
  every parameter is `Integer`, `(seconds, nanos)` and a hypothetical `(mins, seconds)`
  would be the *same* signature `(Integer, Integer)` — a duplicate-symbol error.
  Making `nanos` mandatory pushes each component builder up one arity, giving the set
  the disjoint arities 1/2/3/4/5 that overload resolution needs.

The same applies to `fixedOffset` below (its two forms are arity 1 vs 2 — disjoint).
`time` and `date` are *not* overloaded, so `time` keeps its trailing defaults.

**Why not shim-padded defaults (the `http`/`tls` approach)?** As a built-in package,
`datetime`'s call sites are typechecked through the Rust shim (§8.2), and a builtin
*could* realize a trailing default via `default_argument_padding` the way
`http::read`/`tls::connect` do (`plan-03-http.md` §1). That mechanism is wrong here:
each component overload has a **different meaning per arity**, so padding
`instant(60, 30)` up to `instant(60, 30, 0, 0, 0)` would silently reinterpret it as
`days=60, hours=30`. Padding only fits *one* function with optional trailing args; it
cannot select among genuinely distinct overloads. So the component forms are real
`.mfb` overloads selected by the §6 rule (exact arity + positional types), and
default-padding is deliberately disabled for `instant`/`duration` so the supplied
argument count is preserved for that selection.

### 5.2 Zones

| Function | Signature | Behavior |
|----------|-----------|----------|
| `datetime::utc` | `FUNC utc() AS Zone` | The UTC zone (offset 0). |
| `datetime::local` | `FUNC local() AS Zone` | The host's local zone. Its offset is resolved per-instant (DST-correct) at projection time. |
| `datetime::fixedOffset` | `FUNC fixedOffset(offsetSeconds AS Integer) AS Zone` | A fixed-offset zone from a raw second count; `label` is rendered as `±HH:MM`. Fails `ErrInvalidArgument` if `\|offset\|` ≥ 24h. |
| `datetime::fixedOffset` | `FUNC fixedOffset(hours AS Integer, mins AS Integer) AS Zone` | A fixed-offset zone from `hours:mins`. `mins` is a magnitude in `0..59` and takes the **sign of `hours`** (`fixedOffset(-5, 30)` = −05:30). Fails `ErrInvalidArgument` if `mins` is out of range or the total is ≥ 24h. (Sub-hour *negative* offsets, which don't occur in practice, use the seconds form.) |
| `datetime::offsetAt` | `FUNC offsetAt(zone AS Zone, at AS Instant) AS Integer` | The zone's UTC offset (seconds) effective at instant `at`. For `Local`, consults the host zone table; for others, constant. |

### 5.3 Projection (instant ⇄ civil)

| Function | Signature | Behavior |
|----------|-----------|----------|
| `datetime::inZone` | `FUNC inZone(at AS Instant, zone AS Zone) AS DateTime` | Projects `at` into `zone`, resolving and storing the effective offset. The primary "to civil time" call. |
| `datetime::toUtc` | `FUNC toUtc(at AS Instant) AS DateTime` | Shorthand for `inZone(at, utc())`. |
| `datetime::toLocal` | `FUNC toLocal(at AS Instant) AS DateTime` | Shorthand for `inZone(at, local())`. |
| `datetime::resolve` | `FUNC resolve(dt AS DateTime) AS Instant` | Maps a civil `DateTime` back to its absolute `Instant` using `dt.offset`. Total (the offset is already pinned). |
| `datetime::civil` | `FUNC civil(date AS Date, time AS Time, zone AS Zone) AS DateTime` | Builds a `DateTime` from civil parts in `zone`, resolving the offset for that local time. See §5.7 for DST gaps/overlaps. |
| `datetime::withZone` | `FUNC withZone(dt AS DateTime, zone AS Zone) AS DateTime` | Same instant, re-projected into a different zone (the wall-clock fields change, the instant does not). |

### 5.4 Arithmetic & comparison

| Function | Signature | Behavior |
|----------|-----------|----------|
| `datetime::add` | `FUNC add(at AS Instant, by AS Duration) AS Instant` | Instant + duration. Fails `ErrOverflow` past the `Integer` range. |
| `datetime::subtract` | `FUNC subtract(at AS Instant, by AS Duration) AS Instant` | Instant − duration. |
| `datetime::between` | `FUNC between(start AS Instant, finish AS Instant) AS Duration` | Signed span `finish − start`. |
| `datetime::addDays` | `FUNC addDays(dt AS DateTime, days AS Integer) AS DateTime` | Calendar-day arithmetic on civil time (re-resolves offset; DST-aware). |
| `datetime::addMonths` | `FUNC addMonths(dt AS DateTime, months AS Integer) AS DateTime` | Month arithmetic, clamping day-of-month (Jan 31 +1mo → Feb 28/29). |
| `datetime::compare` | `FUNC compare(a AS Instant, b AS Instant) AS Integer` | `-1 / 0 / 1`. (`DateTime`s are compared by resolving to instants.) |
| `datetime::isBefore` / `isAfter` / `equals` | `FUNC isBefore(a AS Instant, b AS Instant) AS Boolean` (and friends) | Convenience predicates over instants. |
| `datetime::negate` / `plus` / `minus` | `FUNC plus(a AS Duration, b AS Duration) AS Duration` (and friends) | Duration algebra. |

### 5.5 Accessors

| Function | Signature | Behavior |
|----------|-----------|----------|
| `datetime::weekday` | `FUNC weekday(dt AS DateTime) AS Weekday` | Day of week for the civil date. |
| `datetime::dayOfYear` | `FUNC dayOfYear(dt AS DateTime) AS Integer` | 1 .. 366. |
| `datetime::isLeapYear` | `FUNC isLeapYear(year AS Integer) AS Boolean` | Proleptic Gregorian leap rule. |
| `datetime::daysInMonth` | `FUNC daysInMonth(year AS Integer, month AS Integer) AS Integer` | 28 .. 31. |
| `datetime::startOfDay` | `FUNC startOfDay(dt AS DateTime) AS DateTime` | Civil midnight (DST-aware) in the value's zone. |
| `datetime::toMillis` / `toNanos` | `FUNC toMillis(at AS Instant) AS Integer` | Epoch milliseconds / nanoseconds (fails `ErrOverflow` if outside range). |
| `datetime::fromMillis` | `FUNC fromMillis(millis AS Integer) AS Instant` | Inverse of `toMillis`. |

### 5.6 Formatting & parsing

| Function | Signature | Behavior |
|----------|-----------|----------|
| `datetime::format` | `FUNC format(dt AS DateTime, pattern AS String) AS String` | Renders `dt` using the pattern mini-language (§6). |
| `datetime::parse` | `FUNC parse(value AS String, pattern AS String, zone AS Zone = utc()) AS DateTime` | Parses `value` against `pattern`; fields absent from the pattern default (date → epoch date, time → 00:00:00). If the pattern has no offset token, `zone` supplies it. Fails `ErrInvalidFormat`. |
| `datetime::toIso` | `FUNC toIso(dt AS DateTime) AS String` | RFC 3339 / ISO 8601, e.g. `2026-06-25T14:30:00.000+05:30` (`Z` for UTC). |
| `datetime::parseIso` | `FUNC parseIso(value AS String) AS DateTime` | Parses RFC 3339 (offset required, `Z` accepted). Fails `ErrInvalidFormat`. |
| `datetime::formatDuration` | `FUNC formatDuration(d AS Duration) AS String` | Human span, e.g. `1d 02:03:04.500`. |

### 5.7 DST gaps & overlaps (`civil` resolution)

When `datetime::civil(date, time, zone)` names a local time that **does not exist**
(spring-forward gap) or is **ambiguous** (fall-back overlap), v1 resolves
deterministically:

- **Gap** → shift forward by the gap (pick the post-transition offset).
- **Overlap** → pick the **earlier** offset (pre-transition).

This is documented, total, and matches the common "lenient" convention. A future
`civil` overload could take an explicit disambiguation policy (non-goal v1).

## 6. Format / parse mini-language

A pattern is literal text with **token runs**. A run of the same letter is one token;
its length usually selects width or style. Unknown letters fail `ErrInvalidFormat`.
Wrap literal letters in single quotes (`'T'`); `''` is a literal apostrophe.

| Token | Meaning | Example |
|-------|---------|---------|
| `yyyy` / `yy` | year, 4-digit / 2-digit | `2026` / `26` |
| `M` / `MM` | month number, min / 2-digit | `6` / `06` |
| `MMM` / `MMMM` | month name, short / full (English) | `Jun` / `June` |
| `d` / `dd` | day of month | `5` / `05` |
| `H` / `HH` | hour 0–23 | `9` / `09` |
| `h` / `hh` | hour 1–12 | `9` / `09` |
| `m` / `mm` | minute | `3` / `03` |
| `s` / `ss` | second | `4` / `04` |
| `fff` … `fffffffff` | fractional second, fixed width (3/6/9 = ms/µs/ns) | `500` |
| `a` | AM/PM marker | `PM` |
| `EEE` / `EEEE` | weekday name, short / full | `Thu` / `Thursday` |
| `Z` | offset, `Z` for UTC else `±HH:MM` | `+05:30` |
| `ZZ` | offset, always `±HH:MM` (UTC → `+00:00`) | `+00:00` |
| `ZZZ` | offset, `±HHMM` | `+0530` |

On **parse**, numeric tokens accept their stated width (min-width tokens like `M`
accept 1–2 digits); name tokens are case-insensitive; `Z`/`ZZ`/`ZZZ` set the offset
(overriding the `zone` argument). Common patterns get the helpers `toIso`/`parseIso`
so callers rarely hand-write RFC 3339.

## 7. Canonical program

```basic
IMPORT datetime
IMPORT io

SUB main()
    LET start AS datetime::Instant = datetime::now()

    ' Project the same instant into two zones.
    LET here AS datetime::DateTime = datetime::toLocal(start)
    LET india AS datetime::DateTime = datetime::inZone(start, datetime::fixedOffset(5, 30))

    io::print("local: " & datetime::format(here, "EEEE yyyy-MM-dd HH:mm:ss Z"))
    io::print("india: " & datetime::toIso(india))

    ' Calendar arithmetic stays civil and DST-aware.
    LET nextWeek AS datetime::DateTime = datetime::addDays(here, 7)
    io::print("in 7 days: " & datetime::format(nextWeek, "EEE, d MMM yyyy"))

    ' Parse, then measure a span between two instants.
    LET launch AS datetime::DateTime = datetime::parseIso("1969-07-20T20:17:00Z")
    LET since AS datetime::Duration = datetime::between(datetime::resolve(launch), start)
    io::print("since Apollo 11: " & datetime::formatDuration(since))
END SUB
```

## 8. Implementation architecture

Mirror `regex::` (`src/builtins/regex.rs` + `regex_package.mfb`): the source package
carries everything portable; a thin Rust module owns registration and the OS seam.

### 8.1 MFBASIC source package — `src/builtins/datetime_package.mfb`

Holds **all** types (§3), enums, constructors, projection, arithmetic, accessors,
formatting, and parsing. Pure integer/string code:

- `days_from_civil(y, m, d)` / `civil_from_days(z)` (Hinnant) for the epoch ⇄ civil core.
- Field validation, normalization, DST gap/overlap policy (§5.7).
- The format/parse interpreter (§6) over `String`.

Embedded exactly like the existing source packages: `source_file()` →
`include_str!("datetime_package.mfb")`, `uses_package()` gate, and `augmented_project`
appends the parsed AST to `ast.files` (pattern at `src/builtins/collections.rs:230`,
`src/builtins/regex.rs:104`).

### 8.2 Rust intrinsic seam — `src/builtins/datetime.rs`

The package depends on **three** primitive intrinsics (lowercase-underscore names,
not part of the public surface; the `.mfb` wraps them):

| Intrinsic | Signature | Lowering |
|-----------|-----------|----------|
| `datetime::__nowNanos` | `() AS Integer` | `clock_gettime(CLOCK_REALTIME)` → `sec*1e9 + nsec`. |
| `datetime::__monotonicNanos` | `() AS Integer` | `clock_gettime(CLOCK_MONOTONIC)` → nanoseconds. |
| `datetime::__localOffset` | `(epochSeconds AS Integer) AS Integer` | local UTC offset (seconds) at that instant, via `localtime_r` (`tm_gmtoff`). DST-correct. |

`datetime.rs` provides the usual builtin hooks (compare `src/builtins/math.rs`):
`is_datetime_call`, `arity`, `call_param_names`, `call_return_type_name`,
`resolve_call`. For the overloaded constructors, `arity` reports the min/max span
(`instant`/`duration` → `(1, 5)`, `fixedOffset` → `(1, 2)`) and `resolve_call`
returns the result type for the matched argument shape; `default_argument_padding`
returns **empty** for `instant`/`duration`/`fixedOffset` so the supplied argument
count reaches the `.mfb` overload selection unchanged (§5.1.1). Registration: add `datetime` to the module list and to
`is_builtin_import` (`src/builtins/mod.rs:1`, `:18`), wire `is_datetime_call` into
`is_builtin_call` (`:130`), and add `check_datetime_builtin_call` in
`src/typecheck.rs` alongside `check_math_builtin_call` (`:5244`).

**Codegen.** The three intrinsics lower to libc calls. `clock_gettime` is already
imported and called on both targets — reuse the existing path
(`src/target/shared/code/mod.rs:1995`, the import sites in
`src/target/macos_aarch64/plan.rs:33` and `src/target/linux_aarch64/plan.rs:58`).
Add `localtime_r` as a new `PlatformImport` on each target (libSystem `_localtime_r`
/ libc `localtime_r`) and emit it via `platform.emit_libc_call(...)`. Stack-allocate
the `time_t` input and `struct tm` output; read `tm_gmtoff`. Honor the known ABI
gotchas in memory (arena_alloc clobbers; entry stack 16-aligned; don't hold live
values across the `bl`).

**No builtin record type IDs needed.** Because every public type is built in
MFBASIC (a normal package table type) and the intrinsics traffic only in `Integer`,
we avoid the `FIRST_TABLE_TYPE_ID` / high-reserved-range mechanism entirely
(`src/binary_repr.rs:29`). This is the main reason to push types into the `.mfb`.

### 8.3 Why this split

- Calendar math is portable and testable as ordinary MFBASIC; only the clock and the
  zone table are genuinely platform state, and those are 3 small leaves.
- Keeps the Rust surface minimal (no new type-ID plumbing, no record marshalling).
- Matches an established, working pattern (`regex`, `json`, `collections`).

## 9. Implementation phases

1. **Seam & clock.** Add `datetime.rs`, register the package, implement
   `__nowNanos` + `__monotonicNanos` (reuse `clock_gettime`). Land `datetime_package.mfb`
   with `Instant`/`Duration`, `now`, `monotonic`, `instant`, `duration`, add/subtract/
   between/compare. Test: round-trip and elapsed-time measurement.
2. **Civil core.** `Date`/`Time`/`DateTime`/`Zone`, Hinnant conversions, validating
   constructors, `utc`/`fixedOffset`, `inZone`/`toUtc`/`resolve`/`civil`, accessors
   (`weekday`, `isLeapYear`, `daysInMonth`, `dayOfYear`). Test against known dates.
3. **Local zone.** Add the `__localOffset` intrinsic (`localtime_r` import on both
   targets), `local()`, `toLocal`, `offsetAt`, `withZone`, DST gap/overlap policy
   (§5.7). Test across a DST boundary on-device (macOS + Linux aarch64).
4. **Formatting.** `format` + the mini-language renderer (§6), `toIso`,
   `formatDuration`. Golden-output tests.
5. **Parsing.** `parse` + `parseIso`, with the documented error behavior. Round-trip
   `format`→`parse` and `toIso`→`parseIso` tests, plus malformed-input failures.
6. **Calendar arithmetic & polish.** `addDays`/`addMonths`/`startOfDay`, the
   component `instant`/`duration` overloads (§5.1.1), `fromMillis`/`toMillis`,
   duration algebra. Edge cases (month clamp, leap day, negative instants). Add an
   overload-resolution test covering each constructor arity (1–5).
7. **Docs & man pages.** §10; update `specifications/standard_package.md` (new
   `datetime::` section, mirroring the regex table) and `error_codes.md` notes.

Each phase: add a `tests/datetime-*-valid/` and, where it applies, a
`tests/datetime-*-invalid/` directory (matching the repo's existing test layout,
e.g. `tests/lambda-mut-foreach-valid/`).

## 10. Man pages

Follow the current man infrastructure (`src/man/mod.rs`, `build.rs:45`):

1. `src/man/builtins/datetime/package.txt` — summary + function list.
2. `src/man/builtins/datetime/{now,inZone,format,parse,toIso,…}.txt` — one page per
   public function.
3. `build.rs`: add a `datetime_dir`, call `man_pages(&datetime_dir, "datetime")`, and
   `write_pages(..., "DATETIME_FUNCTION_PAGES", ...)`.
4. `src/man/mod.rs`: `parse_package(include_str!("builtins/datetime/package.txt"), …)`
   in `PACKAGES`, and add `"datetime" => Some(generated::DATETIME_FUNCTION_PAGES)`.

## 11. Testing

- **Pure MFBASIC unit tests** for the conversion/format/parse core (no clock
  dependence): fixed instants → expected civil fields and strings, and round-trips.
- **Known-answer vectors**: epoch 0 = `1970-01-01T00:00:00Z` (Thursday); a leap day
  (`2024-02-29`); a negative instant (pre-1970); year-2262 boundary for `now`.
- **DST**: with `TZ` set to a DST zone, assert `offsetAt`/`toLocal` flip across the
  transition and that `civil` applies the §5.7 policy. Run on-device on both targets.
- **Failure paths**: invalid constructors → `ErrInvalidArgument`; bad pattern / bad
  ISO → `ErrInvalidFormat`; overflowing arithmetic → `ErrOverflow`.

## 12. Open questions / future work

- **Named IANA zones** — the big one. Decide later between embedding a compiled tzdb
  (à la the regex Unicode table) vs. reading the host's `/usr/share/zoneinfo`. The
  `Zone.kind` enum and `offsetAt` seam already accommodate a third resolution path.
- **`civil` disambiguation policy** parameter for DST gaps/overlaps.
- **Locale-aware names / formatting**; week-of-year and ISO-week tokens.
- **Whether `now`/`monotonic` should be a single intrinsic returning a record** if a
  builtin-record-return path is later added for other packages.
