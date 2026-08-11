//! Built-in `datetime::` package seam (plan-01-datetime.md).
//!
//! Mirrors `json`/`regex`: the portable calendar math, formatting, and parsing
//! live in `datetime_package.mfb` as internal `__datetime_*` functions; this
//! module owns registration, syntaxcheck metadata, and the mapping from a public
//! `datetime::` call onto its internal implementation. The only platform state
//! is reached through three intrinsics (`nowNanos`, `monotonicNanos`,
//! `localOffset`) that lower to libc runtime helpers (§8.2).

use std::borrow::Cow;

use crate::codegen::registry::{
    BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinSource, BuiltinType,
    DefaultResolver, DefaultValue, InjectionRule, Parameter, ParameterType, ReturnType, TypeKind,
};

// Public, documented surface. Each maps to an internal `__datetime_<name>`
// implementation in the `.mfb` (see `implementation_name`), except the three
// OS-seam intrinsics, which stay as runtime-helper calls.
const NOW: &str = "datetime.now";
const MONOTONIC: &str = "datetime.monotonic";
const INSTANT: &str = "datetime.instant";
const DATE: &str = "datetime.date";
const TIME: &str = "datetime.time";
const DURATION: &str = "datetime.duration";
const UTC: &str = "datetime.utc";
const LOCAL: &str = "datetime.local";
const FIXED_OFFSET: &str = "datetime.fixedOffset";
const OFFSET_AT: &str = "datetime.offsetAt";
const IN_ZONE: &str = "datetime.inZone";
const TO_UTC: &str = "datetime.toUtc";
const TO_LOCAL: &str = "datetime.toLocal";
const RESOLVE: &str = "datetime.resolve";
const CIVIL: &str = "datetime.civil";
const WITH_ZONE: &str = "datetime.withZone";
const ADD: &str = "datetime.add";
const SUBTRACT: &str = "datetime.subtract";
const BETWEEN: &str = "datetime.between";
const ADD_DAYS: &str = "datetime.addDays";
const ADD_MONTHS: &str = "datetime.addMonths";
const COMPARE: &str = "datetime.compare";
const IS_BEFORE: &str = "datetime.isBefore";
const IS_AFTER: &str = "datetime.isAfter";
const EQUALS: &str = "datetime.equals";
const NEGATE: &str = "datetime.negate";
const PLUS: &str = "datetime.plus";
const MINUS: &str = "datetime.minus";
const WEEKDAY: &str = "datetime.weekday";
const DAY_OF_YEAR: &str = "datetime.dayOfYear";
const IS_LEAP_YEAR: &str = "datetime.isLeapYear";
const DAYS_IN_MONTH: &str = "datetime.daysInMonth";
const START_OF_DAY: &str = "datetime.startOfDay";
const TO_MILLIS: &str = "datetime.toMillis";
const TO_NANOS: &str = "datetime.toNanos";
const FROM_MILLIS: &str = "datetime.fromMillis";
const FORMAT: &str = "datetime.format";
const PARSE: &str = "datetime.parse";
const TO_ISO: &str = "datetime.toIso";
const PARSE_ISO: &str = "datetime.parseIso";
const FORMAT_DURATION: &str = "datetime.formatDuration";

// OS-seam intrinsics (§8.2). Not documented; callable but only return raw
// integers. They lower to runtime helpers (`_mfb_rt_datetime_*`), so they are
// deliberately excluded from `implementation_name`.
const NOW_NANOS: &str = "datetime.nowNanos";
const MONOTONIC_NANOS: &str = "datetime.monotonicNanos";
const LOCAL_OFFSET: &str = "datetime.localOffset";

// plan-72-H: `DATETIME` is the descriptor authority. Every function's return is
// fixed, so `call_return_type_name`/`arity` derive from the descriptor.
// `instant`/`duration` (5 overloads) and `fixedOffset` (2) carry per-overload
// parameter tables (`call_param_name_overloads`, bug-349); `time`/`parse` have
// optional trailing parameters (`time`'s drive the `default_argument_padding`).
// Argument VALIDATION (`resolve_call`) and arity-keyed `implementation_name`
// (`__datetime_instant{argc}`) are argument-dependent → `DatetimeResolver`. The
// 9 builtin types are enums/records with no descriptor-modelled fields.
const fn req(name: &'static str, ty: &'static str) -> Parameter {
    Parameter::required(name, ty)
}
// Optional trailing parameter that is default-PADDED (`time`'s `second`/`nanos`
// → `0`): drives `default_argument_padding`.
const fn opt(name: &'static str, ty: &'static str, default: &'static str) -> Parameter {
    Parameter {
        name,
        aliases: &[],
        ty: ParameterType::Named(ty),
        default: DefaultValue::Fill {
            type_name: ty,
            expr: default,
        },
    }
}
// Optional trailing parameter that widens arity but is NOT padded (`parse`'s
// trailing `zone` selects `__datetime_parse{argc}` by count instead).
const fn optn(name: &'static str, ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases: &[],
        ty: ParameterType::Named(ty),
        default: DefaultValue::Optional,
    }
}
const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}
// Every datetime member is argument-dependent (arity/type resolved by
// `DatetimeResolver`), so each is declared with the registry-wide
// `BuiltinFunction::custom`. `df` keeps the compact `(name, slug, overloads)`
// call shape used across the table; docs default empty until Phase 5.
const fn df(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction::custom(name, slug, "", "", &[], overloads)
}

const I: &str = "Integer";
// The `instant`/`duration` arity overloads drop components off the front.
const DT_COMPONENTS: &[BuiltinOverload] = &[
    ov(&[req("seconds", I)], "Duration"),
    ov(&[req("seconds", I), req("nanos", I)], "Duration"),
    ov(
        &[req("mins", I), req("seconds", I), req("nanos", I)],
        "Duration",
    ),
    ov(
        &[
            req("hours", I),
            req("mins", I),
            req("seconds", I),
            req("nanos", I),
        ],
        "Duration",
    ),
    ov(
        &[
            req("days", I),
            req("hours", I),
            req("mins", I),
            req("seconds", I),
            req("nanos", I),
        ],
        "Duration",
    ),
];
const INSTANT_OVERLOADS: &[BuiltinOverload] = &[
    ov(&[req("seconds", I)], "Instant"),
    ov(&[req("seconds", I), req("nanos", I)], "Instant"),
    ov(
        &[req("mins", I), req("seconds", I), req("nanos", I)],
        "Instant",
    ),
    ov(
        &[
            req("hours", I),
            req("mins", I),
            req("seconds", I),
            req("nanos", I),
        ],
        "Instant",
    ),
    ov(
        &[
            req("days", I),
            req("hours", I),
            req("mins", I),
            req("seconds", I),
            req("nanos", I),
        ],
        "Instant",
    ),
];

// --- authored docs migrated from src/docs/man/builtins/datetime/*.md
// (intro/description/examples; citations stripped). Metadata only.
const INTRO_LOCAL_OFFSET: &str =
    r#"The host's local UTC offset in seconds at a given epoch second."#;
const DESC_LOCAL_OFFSET: &str = r#"`datetime::localOffset` returns the signed offset from UTC, in seconds, that the
host's configured local time zone applies at the absolute instant named by
`epochSeconds` — whole seconds since `1970-01-01T00:00:00Z` on the UTC timeline
(the Unix epoch, without leap seconds). A positive result places local civil
time ahead of UTC (east of the prime meridian); a negative result places it
behind UTC (west); zero means local time coincides with UTC at that instant.


This is the OS seam through which the rest of the package learns the host's
wall-clock rules. The call lowers to a libc runtime helper that hands
`epochSeconds` to `localtime_r` and reports the resolved `tm_gmtoff` for that
moment, so the result is DST-correct: it returns the standard-time offset for
instants outside daylight saving and the shifted offset for instants within it.
Two calls with epoch seconds on opposite sides of a daylight-saving transition
can therefore return different values. The offset reflects whatever zone the host
is configured to use (for example via the `TZ` environment variable or the
system zone setting), so the same program can produce different results on
different hosts.

Only the seconds value matters; there is no sub-second component. `localOffset`
is the low-level intrinsic that backs `datetime::offsetAt` for local zones and
`datetime::toLocal`; most code should prefer those higher-level functions, which
operate on `Instant` and `Zone` values rather than a raw epoch-seconds `Integer`.

`localOffset` is **not pure**: it reads the host's time-zone configuration, so
its result depends on host state. It has no side effects and reads no other
state."#;
const EX_LOCAL_OFFSET: &str = r#"The host's local offset for the current instant:

```
IMPORT datetime

SUB main()
  LET nowSeconds AS Integer = datetime::toMillis(datetime::now()) / 1000
  LET off AS Integer = datetime::localOffset(nowSeconds)
END SUB
```

Read the local offset at a fixed point on the timeline (the Unix epoch):

```
IMPORT datetime

SUB main()
  LET off AS Integer = datetime::localOffset(0)
END SUB
```"#;
const INTRO_MONOTONIC_NANOS: &str =
    r#"The raw monotonic-clock reading as a whole nanosecond count."#;
const DESC_MONOTONIC_NANOS: &str = r#"`datetime::monotonicNanos` reads the host's monotonic clock and returns the
elapsed time, in whole nanoseconds, from an arbitrary fixed origin chosen by the
operating system. It is the low-level OS-seam intrinsic that backs
`datetime::monotonic`: where `monotonic` packages the reading into a `Duration`,
`monotonicNanos` returns the same value as a single raw `Integer` count of
nanoseconds.

The clock never moves backward: a later call always returns a value that is
greater than or equal to an earlier one. The reading is unrelated to wall-clock
time, carries no calendar meaning, and is not comparable across processes or
across reboots, so the absolute value of a single reading is meaningless. The
only intended use is to measure elapsed time: take two readings and subtract the
earlier from the later, yielding an elapsed interval in nanoseconds.

Because the clock is immune to wall-clock adjustments (NTP steps, manual clock
changes, daylight saving), the difference between two readings is a reliable
interval where a difference of `datetime::nowNanos` readings would not be. Use
the wall-clock readings, not the monotonic ones, whenever you need an actual
point in time.

Internally the call lowers to a libc runtime helper that reads a single
nanoseconds-since-origin value from the OS (`clock_gettime(CLOCK_MONOTONIC)` on
the supported platforms). Prefer `datetime::monotonic` in ordinary code; reach
for `monotonicNanos` only when you want the bare integer count without
constructing a `Duration`.

`monotonicNanos` is **not pure**: two calls may return different values, and the
values depend on host clock state. It takes no arguments, reads clock state only,
and has no side effects. The reading always succeeds — the intrinsic returns an
`Integer` in the result register with the OK tag set and never raises an error."#;
const EX_MONOTONIC_NANOS: &str = r#"Measure the elapsed time around a block of work in nanoseconds:

```
IMPORT datetime

SUB main()
  LET t0 AS Integer = datetime::monotonicNanos()
  ' ... work ...
  LET elapsedNanos AS Integer = datetime::monotonicNanos() - t0
END SUB
```

Convert the measured interval to whole milliseconds:

```
IMPORT datetime

SUB main()
  LET t0 AS Integer = datetime::monotonicNanos()
  ' ... work ...
  LET elapsedMs AS Integer = (datetime::monotonicNanos() - t0) / 1000000
END SUB
```"#;
const INTRO_NOW_NANOS: &str =
    r#"The current wall-clock reading as nanoseconds since the Unix epoch."#;
const DESC_NOW_NANOS: &str = r#"`datetime::nowNanos` is the low-level OS-seam intrinsic behind `datetime::now`.
It reads the host's real-time clock (`clock_gettime(CLOCK_REALTIME)` on the
supported platforms) and returns a single `Integer` giving nanoseconds elapsed
since `1970-01-01T00:00:00Z` on the UTC timeline (the Unix epoch, without leap
seconds). The reading is formed as `tv_sec * 1_000_000_000 + tv_nsec` from the
libc `timespec`, folding whole seconds and the sub-second remainder into one
count rather than the `seconds`/`nanos` pair an `Instant` carries.


Most programs should call `datetime::now`, which splits this same reading into a
structured `Instant` whose `seconds` and `nanos` fields can be projected through
a zone with `datetime::toUtc`, `datetime::toLocal`, or `datetime::inZone`. Reach
for `nowNanos` directly only when a raw integer count of nanoseconds is what is
wanted — to stamp a log line, derive a millisecond count, or difference two
readings without building `Instant` values.

`nowNanos` reports nanoseconds since the epoch and is bounded by the range of an
`Integer`: a 64-bit signed nanosecond count overflows in the year 2262. This is
a limit on the intrinsic, not on the `Instant` type, whose `seconds` field spans
the full `Integer` range. On any correctly configured host the reading is
non-negative.

`nowNanos` is **not pure**: two calls may return different values, and a
program's output depends on the host clock. For reproducible logic, capture one
reading and derive everything else from it. It takes no arguments, reads host
clock state only, and has no side effects."#;
const EX_NOW_NANOS: &str = r#"Read the current time as a raw nanosecond count:

```
IMPORT datetime

SUB main()
  LET ns AS Integer = datetime::nowNanos()
END SUB
```

Derive a millisecond timestamp from the nanosecond reading:

```
IMPORT datetime

SUB main()
  LET ns AS Integer = datetime::nowNanos()
  LET ms AS Integer = ns / 1000000
END SUB
```"#;
const INTRO_FORMAT_DURATION: &str =
    r#"Render a `Duration` as a human-readable `[-][Nd ]HH:MM:SS.mmm` span."#;
const DESC_FORMAT_DURATION: &str = r#"`datetime::formatDuration` renders the signed span `d` as a fixed-shape string of
the form `[-][Nd ]HH:MM:SS.mmm`. The hour, minute, and second fields are always
two digits and the millisecond field always three; the day field and its trailing
space appear only when the span is at least one whole day. A span of one day, two
hours, three minutes, four-and-a-half seconds renders as `1d 02:03:04.500`, while
ninety seconds renders as `00:01:30.000` and a zero span as `00:00:00.000`.


The span is reduced to whole milliseconds before formatting: the value used is
`d.seconds * 1000 + d.nanos / 1000000`, so any sub-millisecond remainder in the
`nanos` field is truncated and does not appear in the output. A negative span is
rendered as its absolute magnitude prefixed with a single leading minus sign; the
hour, minute, second, and millisecond fields are taken from the absolute value and
never carry their own sign. The day count is the full number of whole days and is
not wrapped, so a multi-day span shows a multi-digit day field; the hour field is
the remaining whole hours modulo 24, the minute field the remaining minutes modulo
60, and the second field the remaining seconds modulo 60.


`datetime::formatDuration` is pure: the same `Duration` always yields the same
string, and it has no side effects. Because the reduction to milliseconds is
ordinary signed `Integer` arithmetic, a span whose second count is large enough
that multiplying by 1000 (or negating the reduced total) leaves the signed
`Integer` range traps rather than formatting."#;
const EX_FORMAT_DURATION: &str = r#"Render a sub-day span:

```
IMPORT datetime
IMPORT io

SUB main()
  LET d AS Duration = datetime::duration(90)
  io::print(datetime::formatDuration(d))        ' 00:01:30.000
END SUB
```

Render a span that includes whole days:

```
IMPORT datetime
IMPORT io

SUB main()
  LET d AS Duration = datetime::duration(1, 2, 3, 4, 500_000_000)
  io::print(datetime::formatDuration(d))        ' 1d 02:03:04.500
END SUB
```

A negative span is prefixed with a minus sign:

```
IMPORT datetime
IMPORT io

SUB main()
  LET d AS Duration = datetime::duration(-30)
  io::print(datetime::formatDuration(d))        ' -00:00:30.000
END SUB
```"#;
const INTRO_PARSE_ISO: &str = r#"Parse an RFC 3339 / ISO 8601 timestamp into a `DateTime`."#;
const DESC_PARSE_ISO: &str = r#"`datetime::parseIso` reads an RFC 3339 (ISO 8601 profile) timestamp from `value`
and returns the `DateTime` it names. It is the convenience inverse of
`datetime::toIso`, and a fixed-shape alternative to `datetime::parse`: rather than
taking a pattern, it expects the canonical RFC 3339 layout

```
yyyy-MM-dd<sep>HH:mm:ss[.fraction]<offset>
```

parsing `value` left to right. The components are:

- `yyyy-MM-dd` — four-digit year, two-digit month, two-digit day, each introduced
  by its literal `-` separator
- `<sep>` — the date/time separator: `T`, `t`, or a single space
- `HH:mm:ss` — two-digit hour, minute, and second on a 24-hour clock, separated by
  literal `:` characters
- `.fraction` — optional fractional second: a `.` followed by decimal digits. The
  first nine digits are scaled to nanoseconds (so `.25` becomes `250000000` ns);
  any digits beyond the ninth are consumed but ignored
- `<offset>` — required UTC offset: `Z` or `z` for UTC, otherwise a signed
  `+/-HH:MM` or `+/-HHMM` (the colon between offset hours and minutes is optional)

The numeric readers are greedy up to their stated width but also accept fewer
digits, so a field may be written with or without leading padding as long as the
surrounding separators are present. The offset is mandatory; unlike
`datetime::parse` there is no zone argument and no defaulting to UTC, because a
conforming RFC 3339 timestamp always carries its own offset. The parsed offset is
applied directly, making the result a fixed-offset moment.

Like `datetime::parse`, `parseIso` does not range-check the decoded calendar
fields: an out-of-range component such as month 13 or day 40 is carried into the
resulting `DateTime` rather than rejected. The one validated numeric range is the
offset, whose magnitude must be under 24 hours. `parseIso` is pure: it reads no
host state and has no side effects."#;
const EX_PARSE_ISO: &str = r#"Parse a UTC timestamp:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::parseIso("1969-07-20T20:17:00Z")
END SUB
```

Parse a fractional second with a positive offset:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::parseIso("2026-06-25T14:30:00.250+05:30")
END SUB
```

A space may stand in for the `T` separator:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::parseIso("2026-06-26 09:30:00-08:00")
END SUB
```

Text that is missing its required offset is not valid RFC 3339 and raises
`ErrInvalidFormat`:

```
IMPORT datetime

SUB main()
  LET bad AS DateTime = datetime::parseIso("2026-06-26T09:30:00")
END SUB
```"#;
const INTRO_TO_ISO: &str = r#"Render a `DateTime` as an RFC 3339 / ISO 8601 timestamp."#;
const DESC_TO_ISO: &str = r#"`datetime::toIso` renders `dt` as an RFC 3339 (ISO 8601 profile) timestamp with
fixed millisecond precision and an explicit UTC offset. The result is a freshly
built `String` of the shape `yyyy-MM-ddTHH:mm:ss.fffZ`, for example
`2026-06-25T14:30:00.000+05:30`, where the literal `T` separates the date from
the time and the trailing field is the offset carried by `dt`: the single letter
`Z` when the offset is zero, otherwise a signed `+HH:MM` or `-HH:MM`. The
fractional-second field is always three digits (milliseconds), zero-padded, even
when `dt` has no sub-second value.

`toIso` is the convenience form of `datetime::format` invoked with the fixed
pattern `yyyy-MM-dd'T'HH:mm:ss.fffZ`. It reads only the date fields, time
fields, and resolved offset of `dt`; it does not consult `dt`'s zone name, apply
any zone conversion, or shift the moment. The `nanos` of `dt` are truncated to
milliseconds for the `fff` field. `dt` is read only and is not modified. The
output is round-trippable: `datetime::parseIso` parses a string produced by
`toIso` back into an equivalent `DateTime`.

Because the pattern is fixed and always valid, `toIso` emits a result for every
`DateTime` and is pure: it reads no host state and has no side effects."#;
const EX_TO_ISO: &str = r#"Render the current instant in UTC, yielding a `...Z` suffix:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET text AS String = datetime::toIso(dt)
END SUB
```

Render a fixed-offset moment, yielding a signed offset suffix:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(5, 30)
  LET dt AS DateTime = datetime::parse("2026-06-25 14:30:00", "yyyy-MM-dd HH:mm:ss", z)
  LET text AS String = datetime::toIso(dt)
END SUB
```

Round-trip a timestamp through `toIso` and `parseIso`:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET back AS DateTime = datetime::parseIso(datetime::toIso(dt))
END SUB
```"#;
const INTRO_PARSE: &str = r#"Parse text into a `DateTime` using the format pattern mini-language."#;
const DESC_PARSE: &str = r#"`datetime::parse` reads `value` against `pattern` and returns the `DateTime` it
describes. `pattern` uses the same token mini-language as `datetime::format`, and
`parse` is the approximate inverse of `format`: it walks `pattern` and `value`
together from left to right, consuming characters of `value` as each `pattern`
position is matched. A token (a run of one or more of the same formatting letter)
consumes and decodes the corresponding field from `value`; any other `pattern`
character is a literal that must appear verbatim at the current position in
`value`. Single quotes escape literal text exactly as in `datetime::format` (`'T'`
matches a literal `T`, `''` matches a single apostrophe).

Fields not named by any token take defaults: year `1970`, month `1`, day `1`, and
the time `00:00:00.000000000`. The recognized tokens are:

- `yyyy` / `yy` — year; `yyyy` reads up to 4 digits, `yy` reads 2 digits and adds
  2000 (so `26` becomes `2026`)
- `M` / `MM` — month number, 1-2 digits
- `MMM` / `MMMM` — month name, short or full, case-insensitive (English)
- `d` / `dd` — day of month, 1-2 digits
- `H` / `HH` — hour on a 24-hour clock, 1-2 digits
- `h` / `hh` — hour on a 12-hour clock, 1-2 digits (combine with `a`)
- `m` / `mm` — minute, 1-2 digits
- `s` / `ss` — second, 1-2 digits
- `fff`..`fffffffff` — fractional second; reads run-length digits and scales them
  to nanoseconds (`fff` = milliseconds, `fffffffff` = nanoseconds)
- `a` — AM/PM marker, case-insensitive
- `EEE` / `EEEE` — weekday name; the letters are consumed but not validated
- `Z` / `ZZ` / `ZZZ` — offset: the letter `Z` (or `z`) for UTC, else `+/-HH:MM` or
  `+/-HHMM` (the colon between offset hours and minutes is optional)

Numeric tokens are greedy up to their stated width but accept fewer digits, so the
minimal forms (`M`, `d`, `H`, `h`, `m`, `s`) read one or two digits and the padded
forms accept the same. Name tokens (month names, AM/PM) are matched without regard
to case. The weekday token only skips over the run of letters in `value`; it does
not check that the named weekday agrees with the parsed date.

`parse` does not range-check the decoded calendar fields the way `datetime::date`
and `datetime::time` do: an out-of-range component in `value` (for example month
13) is carried into the resulting `DateTime` rather than rejected. The one
validated numeric range is the offset token, whose magnitude must be under 24
hours.

An offset token sets the `DateTime`'s offset directly and makes the result a
fixed-offset moment, overriding `zone`. When `pattern` contains no offset token,
the `zone` argument supplies the offset: the two-argument overload defaults it to
`datetime::utc()`, and the three-argument overload resolves `value`'s civil fields
against the given `zone`. `parse` is pure: it reads no host state and has no side
effects."#;
const EX_PARSE: &str = r#"Parse a date and time, interpreted as UTC:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::parse("2026-06-26 09:30:00", "yyyy-MM-dd HH:mm:ss")
END SUB
```

Parse civil fields against an explicit zone:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(-5, 0)
  LET dt AS DateTime = datetime::parse("2026-06-26 09:30", "yyyy-MM-dd HH:mm", z)
END SUB
```

An offset token in the value overrides the zone argument:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::parse("2026-06-26T09:30:00+05:30", "yyyy-MM-dd'T'HH:mm:ssZZ")
END SUB
```

Text that does not match the pattern raises `ErrInvalidFormat`:

```
IMPORT datetime

SUB main()
  LET bad AS DateTime = datetime::parse("not-a-date", "yyyy-MM-dd")
END SUB
```"#;
const INTRO_FORMAT: &str = r#"Render a `DateTime` as text with the pattern mini-language."#;
const DESC_FORMAT: &str = r#"`datetime::format` renders the fields of `dt` as text by walking `pattern` from
left to right and emitting, for each position, either a literal character or the
value selected by a formatting token. The result is a freshly built `String`;
`dt` is read only and is not modified. An empty pattern yields the empty string.


A token is a run of one or more of the same ASCII letter (`A`–`Z` or `a`–`z`);
the run length selects the width or style of the field. Any character that is
not an ASCII letter is copied to the output verbatim, so separators such as
spaces, dashes, colons, and slashes appear literally. A run of a letter that is
not one of the recognized tokens below is an error, not literal text: to emit a
letter literally, wrap it in single quotes (`'T'` produces a literal `T`); to
emit a literal apostrophe, write two single quotes (`''`).


The recognized tokens are:

- `yy` — last two digits of the year, zero-padded; any other run of `y`
  zero-pads the full year to the run length (`yyyy` pads to at least 4 digits)
- `M` / `MM` — month number, minimal (1-12) / 2-digit
- `MMM` — month name, short (English); any run of 4 or more `M` gives the full name
- `d` — day of month, minimal; any run of 2 or more `d` gives the 2-digit form
- `H` — hour on a 24-hour clock (0-23), minimal; 2 or more `H` gives 2-digit
- `h` — hour on a 12-hour clock (1-12), minimal; 2 or more `h` gives 2-digit
- `m` — minute, minimal; 2 or more `m` gives 2-digit
- `s` — second, minimal; 2 or more `s` gives 2-digit
- `f` .. `fffffffff` — fractional second, fixed to the run length (`fff` = ms,
  `ffffff` = us, `fffffffff` = ns)
- `a` — AM/PM marker (`AM` before noon, `PM` at or after noon)
- `E` .. `EEE` — weekday name, short (English); any run of 4 or more `E` gives
  the full name
- `Z` — offset: the letter `Z` when the offset is zero, else `+/-HH:MM`
- `ZZ` — offset, always `+/-HH:MM` (`Z` is never substituted)
- `ZZZ` and longer — offset, `+/-HHMM` with no colon

The fractional-second token renders the `nanos` of `dt.time` as 9 digits and
keeps the leading run-length digits, so `fff` yields milliseconds, `ffffff`
microseconds, and `fffffffff` nanoseconds. Month, weekday, and AM/PM names are
English. The offset tokens read `dt.offset`, the resolved UTC offset carried by
`dt`.

Inside single quotes every character, including formatting letters, is copied
literally until the closing quote; an opening quote with no matching close runs
to the end of `pattern`. `datetime::format` is pure: it reads no host state and
has no side effects."#;
const EX_FORMAT: &str = r#"Render a `DateTime` with a full date, time, and offset:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET text AS String = datetime::format(dt, "EEEE yyyy-MM-dd HH:mm:ss Z")
END SUB
```

Use single quotes to include literal letters in the output:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET text AS String = datetime::format(dt, "yyyy-MM-dd'T'HH:mm:ss")
END SUB
```"#;
const INTRO_FROM_MILLIS: &str = r#"Build the `Instant` at a given epoch-millisecond count."#;
const DESC_FROM_MILLIS: &str = r#"`datetime::fromMillis` builds an `Instant` on the UTC timeline (Unix epoch,
leap-second-free) from a single count of whole milliseconds measured from
`1970-01-01T00:00:00Z`. A `millis` of `0` yields the epoch itself, positive
values select instants after the epoch, and negative values select instants
before it.

The count is split into a whole-second `seconds` field and a sub-second `nanos`
field by *floor* division, so the `nanos` remainder is always non-negative. The
implementation first computes the toward-zero quotient `millis / 1000` and
remainder `millis MOD 1000`; when that remainder is negative it adds `1000` to
the remainder and subtracts `1` from the quotient, borrowing one second. The
`seconds` field is therefore the mathematical floor of `millis / 1000` and the
`nanos` field is the borrowed, non-negative millisecond remainder scaled to
nanoseconds (`remainder * 1000000`), always in `0..999000000`. A `millis` of
`-1` produces `seconds` `-1` and `nanos` `999000000`, the instant one
millisecond before the epoch. Because the input carries only millisecond
resolution, the `nanos` field is always a whole number of milliseconds — its
microsecond and nanosecond digits are zero.


The arithmetic cannot overflow: dividing by `1000` only reduces the magnitude of
the `seconds` field, and the scaled remainder never exceeds `999000000`, so the
result is always representable. `datetime::fromMillis` is pure: it reads no host
state and the same `millis` always yields the same `Instant`.

`datetime::fromMillis` is the inverse of `datetime::toMillis` to
whole-millisecond precision. Because the input has no sub-millisecond component,
round-tripping an arbitrary `Instant` through `datetime::toMillis` and back loses
its microsecond and nanosecond digits; for full nanosecond precision use
`datetime::toNanos` together with `datetime::instant`."#;
const EX_FROM_MILLIS: &str = r#"Build an `Instant` from an epoch-millisecond timestamp:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::fromMillis(1_700_000_000_000)
END SUB
```

Select the instant one millisecond before the epoch:

```
IMPORT datetime

SUB main()
  LET before AS Instant = datetime::fromMillis(-1)
END SUB
```

Round-trip an instant through its millisecond count:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::now()
  LET ms AS Integer = datetime::toMillis(at)
  LET back AS Instant = datetime::fromMillis(ms)
END SUB
```"#;
const INTRO_TO_NANOS: &str =
    r#"Return the whole nanoseconds between the Unix epoch and an `Instant`."#;
const DESC_TO_NANOS: &str = r#"`datetime::toNanos` collapses the absolute point `at` into a single `Integer`
count of whole nanoseconds measured from the Unix epoch
(`1970-01-01T00:00:00Z`). Instants before the epoch yield negative counts, the
epoch itself yields `0`, and instants after the epoch yield positive counts.


The result is computed as `at.seconds * 1000000000 + at.nanos`: the
seconds-since-epoch field is scaled to nanoseconds and the sub-second `nanos`
field is added in directly. Because a normalized `Instant` already holds its
`nanos` field at full nanosecond resolution (`0..999999999`), the conversion is
exact and discards nothing — no truncation or rounding occurs in either
direction.

The arithmetic is checked. For an instant near the extreme edge of the timeline
either the `at.seconds * 1000000000` scaling or the trailing addition of
`at.nanos` can exceed the signed `Integer` range, in which case the function
raises `ErrOverflow` rather than wrapping. The range of
representable instants is therefore narrower than for `datetime::toMillis`, since
each second consumes a billion units rather than a thousand.
`datetime::toNanos` is pure: it reads no host state and depends only on `at`.


Unlike `datetime::toMillis`, `datetime::toNanos` preserves the full sub-second
precision of `at`; use it when nanosecond fidelity matters."#;
const EX_TO_NANOS: &str = r#"Epoch nanoseconds of the current instant:

```
IMPORT datetime

SUB main()
  LET ns AS Integer = datetime::toNanos(datetime::now())
END SUB
```

Compare two instants at nanosecond resolution:

```
IMPORT datetime

SUB main()
  LET a AS Integer = datetime::toNanos(datetime::now())
  LET b AS Integer = datetime::toNanos(datetime::now())
  LET elapsed AS Integer = b - a
END SUB
```"#;
const INTRO_TO_MILLIS: &str =
    r#"Return the whole milliseconds between the Unix epoch and an `Instant`."#;
const DESC_TO_MILLIS: &str = r#"`datetime::toMillis` collapses the absolute point `at` into a single `Integer`
count of whole milliseconds measured from the Unix epoch
(`1970-01-01T00:00:00Z`). Instants before the epoch yield negative counts, the
epoch itself yields `0`, and instants after the epoch yield positive counts.


The result is computed as `at.seconds * 1000 + at.nanos / 1000000`: the
seconds-since-epoch field is scaled to milliseconds and the sub-second `nanos`
field contributes its whole-millisecond part. The `nanos` division truncates,
discarding any sub-millisecond remainder (the microsecond and nanosecond
digits). Because a normalized `Instant` always holds a non-negative `nanos`
field in the range `0..999999999`, this truncation drops the fractional
millisecond rather than rounding it, in either direction.

The arithmetic is checked. For an instant near the extreme edge of the timeline
either the `at.seconds * 1000` scaling or the following addition can exceed the
signed `Integer` range, in which case the function raises `ErrOverflow` rather
than wrapping. `datetime::toMillis` is pure: it reads no host state and depends
only on `at`.

`datetime::toMillis` is the inverse of `datetime::fromMillis` to
whole-millisecond precision; sub-millisecond `nanos` in `at` are not recoverable
from the result. For full nanosecond precision use `datetime::toNanos`."#;
const EX_TO_MILLIS: &str = r#"Epoch milliseconds of the current instant:

```
IMPORT datetime

SUB main()
  LET ms AS Integer = datetime::toMillis(datetime::now())
END SUB
```

Round-trip an instant through its millisecond count:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::now()
  LET ms AS Integer = datetime::toMillis(at)
  LET back AS Instant = datetime::fromMillis(ms)
END SUB
```"#;
const INTRO_START_OF_DAY: &str = r#"Return the civil `DateTime` naming midnight at the start of a `DateTime`'s day, in its own zone."#;
const DESC_START_OF_DAY: &str = r#"`datetime::startOfDay` returns the `DateTime` naming `00:00:00` (midnight) at the
beginning of `dt`'s civil day, in `dt`'s own zone. It keeps `dt`'s calendar date
(year, month, day) and zone, replaces the wall-clock time with a `Time` of
`00:00:00` and zero nanoseconds, and re-resolves the moment through that zone.


The result is produced exactly as `datetime::civil(dt.date, Time[0, 0, 0, 0],
dt.zone)`: local midnight is interpreted in `dt`'s zone, the applicable UTC offset
is resolved for that moment, and the canonical `DateTime` naming the resulting
`Instant` is returned. Because the offset is re-resolved rather than copied from
`dt`, the result is daylight-saving correct: for the host's local zone the offset
reflects whatever DST rule applies at midnight on that date, which may differ from
the offset that applied at `dt`'s original time of day.


The day boundary is civil midnight in `dt`'s zone, not UTC midnight, so the
underlying `Instant` generally differs from `dt`'s `Instant` truncated to whole
days. Any sub-second nanoseconds carried by `dt` are dropped: the start of the day
has zero nanos. Like `datetime::civil`, the result round-trips through
`datetime::resolve` and `datetime::inZone`.

`datetime::startOfDay` is pure when `dt`'s zone is a fixed-offset zone
(`datetime::utc`, `datetime::fixedOffset`). When `dt`'s zone is the host's local
zone (`datetime::local`), the offset is resolved from the platform's zone table,
so the same `dt` can yield a different absolute instant on a host configured for a
different zone or DST rule."#;
const EX_START_OF_DAY: &str = r#"Truncate a `DateTime` to the start of its civil day:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toLocal(datetime::now())
  LET midnight AS DateTime = datetime::startOfDay(dt)
END SUB
```

Start of day in a fixed UTC zone:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::utc())
  LET midnight AS DateTime = datetime::startOfDay(dt)
END SUB
```"#;
const INTRO_DAYS_IN_MONTH: &str = r#"The number of days in a calendar month."#;
const DESC_DAYS_IN_MONTH: &str = r#"`datetime::daysInMonth` returns the number of days in the given `month` of the
given `year` under the proleptic-Gregorian calendar. The result is `31` for
January, March, May, July, August, October, and December; `30` for April, June,
September, and November; and `28` or `29` for February depending on whether
`year` is a leap year.

February's length is decided by applying the leap-year rule to `year`: a leap
February has `29` days, otherwise it has `28`. The leap rule is purely
arithmetic on the year number (divisible by `4`, except century years that are
not divisible by `400`), so it extends indefinitely into the past and future and
treats zero and negative year numbers by the same divisibility test.


Only February consults `year`; for every other month the result depends solely
on `month`, and `year` is ignored. The `month` argument is not range-checked:
any value that is not `2`, `4`, `6`, `9`, or `11` yields `31`, so out-of-range
month numbers do not raise an error but return `31` by falling through to the
default case.

The function reads no time zone, `Instant`, or current clock value and has no
side effects."#;
const EX_DAYS_IN_MONTH: &str = r#"Length of common and leap-year February:

```
IMPORT datetime
IMPORT io

SUB main()
  io::print(toString(datetime::daysInMonth(2023, 2)))   ' 28
  io::print(toString(datetime::daysInMonth(2024, 2)))   ' 29 (leap year)
END SUB
```

Lengths of the other months ignore the year:

```
IMPORT datetime
IMPORT io

SUB main()
  io::print(toString(datetime::daysInMonth(2026, 1)))   ' 31
  io::print(toString(datetime::daysInMonth(2026, 4)))   ' 30
END SUB
```

Clamp a day-of-month to the end of its month:

```
IMPORT datetime
IMPORT io

SUB main()
  LET year AS Integer = 2024
  LET month AS Integer = 2
  MUT day AS Integer = 31
  LET last AS Integer = datetime::daysInMonth(year, month)
  IF day > last THEN
    day = last
  END IF
  io::print(toString(day))   ' 29
END SUB
```"#;
const INTRO_IS_LEAP_YEAR: &str = r#"Whether a proleptic-Gregorian calendar year is a leap year."#;
const DESC_IS_LEAP_YEAR: &str = r#"`datetime::isLeapYear` applies the proleptic-Gregorian leap rule to `year` and
reports whether that year has 366 days. A year is a leap year when it is
divisible by 4, except for century years (those divisible by 100), which are
leap years only when they are also divisible by 400. So `2000` and `2024` are
leap years, while `1900` and `2023` are not.


The rule is purely arithmetic on the year number: no time zone, `Instant`, or
current clock value is consulted. The proleptic Gregorian calendar extends the
same rule indefinitely into the past and future, so years before the calendar's
historical adoption and negative (BCE-style) year numbers are evaluated by the
identical divisibility test on `4`, `100`, and `400`. The function reads no host
state and has no side effects."#;
const EX_IS_LEAP_YEAR: &str = r#"Test individual years:

```
IMPORT datetime
IMPORT io

SUB main()
  io::print(toString(datetime::isLeapYear(2000)))   ' True  (divisible by 400)
  io::print(toString(datetime::isLeapYear(1900)))   ' False (century, not /400)
  io::print(toString(datetime::isLeapYear(2024)))   ' True  (divisible by 4)
  io::print(toString(datetime::isLeapYear(2023)))   ' False
END SUB
```

Pick February's length from the leap result:

```
IMPORT datetime
IMPORT io

SUB main()
  LET year AS Integer = 2024
  MUT days AS Integer = 28
  IF datetime::isLeapYear(year) THEN
    days = 29
  END IF
  io::print(toString(days))   ' 29
END SUB
```"#;
const INTRO_DAY_OF_YEAR: &str = r#"The ordinal day within the year of a `DateTime`'s civil date."#;
const DESC_DAY_OF_YEAR: &str = r#"`datetime::dayOfYear` returns the ordinal position of `dt`'s civil date within
its calendar year: `1` for January 1, `2` for January 2, and so on through `365`
in a common year or `366` in a leap year (the value reached on December 31).


The result is derived solely from the calendar date fields carried by `dt` — its
year, month, and day as stored in `dt`'s own zone. The day-of-year is computed on
the proleptic-Gregorian calendar by taking the days-from-civil count of `dt`'s
date, subtracting the days-from-civil count of January 1 of the same year, and
adding one (`here - start + 1`), so leap years correctly extend the count past
February. The time-of-day fields, the sub-second nanoseconds, and the zone's UTC
offset do not affect the result; no `Instant` is resolved and no zone table is
consulted.

Because the computation reads only `dt`'s stored civil date, the same instant
projected into two different zones can report two different day-of-year values
whenever the zones place that instant on opposite sides of midnight, and across
the December 31 / January 1 boundary the two zones can even fall in different
years.

`datetime::dayOfYear` is pure: it reads no host state and has no side effects."#;
const EX_DAY_OF_YEAR: &str = r#"Find the day-of-year of a civil date in the local zone:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::local())
  LET n AS Integer = datetime::dayOfYear(dt)
END SUB
```

Compute how many days remain in the year:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::civil(datetime::date(2026, 6, 26), datetime::time(9, 30), datetime::local())
  MUT total AS Integer = 365
  IF datetime::isLeapYear(dt.date.year) THEN
    total = 366
  END IF
  LET remaining AS Integer = total - datetime::dayOfYear(dt)
END SUB
```"#;
const INTRO_WEEKDAY: &str = r#"The day of the week of a `DateTime`'s civil date."#;
const DESC_WEEKDAY: &str = r#"`datetime::weekday` returns the day of the week on which `dt`'s civil date falls,
as a value of the `Weekday` enum (`Monday`, `Tuesday`, `Wednesday`, `Thursday`,
`Friday`, `Saturday`, `Sunday`).

The result is derived solely from the calendar date fields carried by `dt` — its
year, month, and day as stored in `dt`'s own zone. The day count for that civil
date is computed on the proleptic-Gregorian calendar and reduced modulo seven
against a fixed reference (`floorMod(days + 3, 7)`), so the answer is the
wall-clock weekday a person reading `dt`'s date in its zone would name. The
time-of-day fields, the sub-second nanoseconds, and the zone's UTC offset do not
affect the result; no `Instant` is resolved and no zone table is consulted.


Because the computation reads only `dt`'s stored civil date, the same instant
projected into two different zones can report two different weekdays whenever the
zones place that instant on opposite sides of midnight. The week is treated as
starting on Monday, matching the ordering of the `Weekday` enum, so
`Weekday.Monday` is the first day and `Weekday.Sunday` is the last.


`datetime::weekday` is pure: it reads no host state and has no side effects."#;
const EX_WEEKDAY: &str = r#"Name the weekday of a civil date in the local zone:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::local())
  LET w AS Weekday = datetime::weekday(dt)
END SUB
```

Branch on whether a `DateTime` falls on the weekend:

```
IMPORT datetime
IMPORT io

SUB main()
  LET dt AS DateTime = datetime::civil(datetime::date(2026, 6, 26), datetime::time(9, 30), datetime::local())
  LET w AS Weekday = datetime::weekday(dt)
  IF w = Weekday.Saturday OR w = Weekday.Sunday THEN
    io::print("weekend")
  END IF
END SUB
```"#;
const INTRO_MINUS: &str =
    r#"Subtract one `Duration` span from another and return the resulting `Duration`."#;
const DESC_MINUS: &str = r#"`datetime::minus` returns the `Duration` `a - b`, the signed span left after
removing one span of elapsed physical time from another. It subtracts the
`seconds` field of `b` from the `seconds` field of `a` and the `nanos` field of
`b` from the `nanos` field of `a`, independently, then normalizes the result so
the stored `nanos` lands in the range `0 .. 999_999_999`, borrowing a whole
second from the `seconds` field when the nanosecond difference is negative.


Because both operands are signed `Duration`s, `minus` handles spans of either
direction: subtracting a negative `Duration` lengthens the total, and
subtracting a larger span from a smaller one yields a negative `Duration`.
`minus` pairs with `datetime::plus` and `datetime::negate`, since
`datetime::minus(a, b)` equals `datetime::plus(a, datetime::negate(b))`. A
common use is measuring elapsed time between two `datetime::monotonic` readings.

Normalization floor-divides the nanosecond difference into a whole-second borrow
and a non-negative remainder, then folds the borrow back into the `seconds`
field, so a `nanos` difference that goes negative still yields a `nanos` in
`0 .. 999_999_999`.
The arithmetic is uniform second-and-nanosecond subtraction with no awareness of
calendars, time zones, or daylight-saving transitions; it simply differences
elapsed physical time. To shift a point on the timeline rather than combine two
spans, use `datetime::subtract` on an `Instant`. The subtraction is ordinary
signed `Integer` arithmetic, so a difference whose second count falls outside the
`Integer` range overflows and traps. `minus` is pure: the same two `Duration`s
always yield the same `Duration`, and it has no side effects."#;
const EX_MINUS: &str = r#"Subtract a 500-millisecond span from a 90-second span:

```
IMPORT datetime

SUB main()
  LET a AS Duration = datetime::duration(90)
  LET b AS Duration = datetime::duration(0, 500_000_000)
  LET rest AS Duration = datetime::minus(a, b)
END SUB
```

Measure the elapsed time between two monotonic readings:

```
IMPORT datetime

SUB main()
  LET start AS Duration = datetime::monotonic()
  LET finish AS Duration = datetime::monotonic()
  LET elapsed AS Duration = datetime::minus(finish, start)
END SUB
```"#;
const INTRO_PLUS: &str = r#"Add two `Duration` spans into their combined `Duration`."#;
const DESC_PLUS: &str = r#"`datetime::plus` returns the `Duration` `a + b`, the signed span that results
from combining two spans of elapsed physical time. It adds the two `seconds`
fields and the two `nanos` fields independently, then normalizes the sum so the
stored `nanos` lands in the range `0 .. 999_999_999`, carrying any whole seconds
embedded in the nanosecond sum into the `seconds` field.


Because both operands are signed `Duration`s, `plus` handles spans of either
direction: adding a negative `Duration` shortens the total, and adding two
`Duration`s of opposite sign moves toward zero. The operation is commutative —
`datetime::plus(a, b)` and `datetime::plus(b, a)` yield the same `Duration` — and
pairs with `datetime::minus` and `datetime::negate`, since
`datetime::plus(a, datetime::negate(b))` equals `datetime::minus(a, b)`.

Normalization floor-divides the nanosecond sum into a whole-second carry and a
non-negative remainder, then folds the carry back into the `seconds` field, so a
combined `nanos` that overflows or goes negative still yields a `nanos` in
`0 .. 999_999_999`.
The arithmetic is uniform second-and-nanosecond addition with no awareness of
calendars, time zones, or daylight-saving transitions; it simply totals elapsed
physical time. To shift a point on the timeline rather than combine two spans,
use `datetime::add` on an `Instant`. The addition is ordinary signed `Integer`
arithmetic, so a combined second count that exceeds the `Integer` range
overflows and traps. `plus` is pure: the same two `Duration`s always yield the
same `Duration`, and it has no side effects."#;
const EX_PLUS: &str = r#"Combine a 90-second span with a 500-millisecond span:

```
IMPORT datetime

SUB main()
  LET a AS Duration = datetime::duration(90)
  LET b AS Duration = datetime::duration(0, 500_000_000)
  LET total AS Duration = datetime::plus(a, b)
END SUB
```

Adding a negative `Duration` shortens the total:

```
IMPORT datetime

SUB main()
  LET a AS Duration = datetime::duration(3600)
  LET total AS Duration = datetime::plus(a, datetime::duration(-600))
END SUB
```"#;
const INTRO_NEGATE: &str =
    r#"Return a `Duration` with the opposite sign — the additive inverse of a span."#;
const DESC_NEGATE: &str = r#"`datetime::negate` returns the additive inverse of `d`: the span of equal
magnitude that points the opposite way along a timeline. A forward span of `+90s`
becomes a backward span of `-90s`, a backward span becomes forward, and the zero
`Duration` negates to itself. Adding `d` to `datetime::negate(d)` yields a zero
span.

Negation acts on the whole span, not on each field independently. It negates both
the `seconds` and the `nanos` field, then re-normalizes so the stored `nanos`
always lands in the range `0 .. 999_999_999`, carrying any borrow into the
`seconds` field. So a `Duration` whose `seconds` is `0` and whose `nanos` is
`250_000_000` (a quarter second forward) negates to a `Duration` whose `seconds`
is `-1` and whose `nanos` is `750_000_000` — the same magnitude pointing
backward.

Negation is the same operation as `datetime::minus(zero, d)`. The arithmetic is
ordinary signed `Integer` arithmetic, so negating the most negative representable
`seconds` count has no positive counterpart in the `Integer` range and traps.
`negate` is pure: the same `Duration` always negates to the same result, and it
has no side effects."#;
const EX_NEGATE: &str = r#"Negate a forward span to get the matching backward span:

```
IMPORT datetime

SUB main()
  LET forward AS Duration = datetime::duration(90)
  LET backward AS Duration = datetime::negate(forward)
END SUB
```

Negation re-normalizes a sub-second span:

```
IMPORT datetime

SUB main()
  LET quarter AS Duration = datetime::duration(0, 250_000_000)
  LET back AS Duration = datetime::negate(quarter)
END SUB
```"#;
const INTRO_EQUALS: &str = r#"Test whether two instants name the same point on the UTC timeline."#;
const DESC_EQUALS: &str = r#"`datetime::equals` is a convenience predicate over instants that returns `TRUE`
when `a` and `b` name the same point on the UTC timeline and `FALSE` otherwise.
It is defined directly in terms of `datetime::compare`: the result is exactly
`datetime::compare(a, b) = 0`, so it is `TRUE` only when `compare` reports `0`
and `FALSE` when `compare` reports `-1` or `1`.


The comparison is performed field by field, matching `datetime::compare`. The
`seconds` fields are compared first; only when they are equal are the `nanos`
fields used as a tiebreaker. Two instants are equal only when both their
`seconds` and their `nanos` fields are equal, so equality is exact to the
nanosecond and there is no tolerance window. Because both arguments are points
on the same Unix-epoch, leap-second-free UTC timeline, the test is absolute and
independent of any time zone; resolve a `DateTime` to an `Instant` with
`datetime::resolve` before comparing.

`equals` is pure: the same two instants always yield the same `Boolean`, it has
no side effects, and it performs only signed comparisons (no arithmetic), so it
cannot overflow or trap. For the strict ordering tests use `datetime::isBefore`
and `datetime::isAfter`, and for a three-way sign rather than a `Boolean` use
`datetime::compare`. To measure the size of the gap between two instants rather
than just whether they coincide, use `datetime::between`."#;
const EX_EQUALS: &str = r#"Equal instants compare as equal:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(1_000)
  io::print(toString(datetime::equals(a, b)))
END SUB
```

Different instants are not equal:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(2_000)
  io::print(toString(datetime::equals(a, b)))
END SUB
```

Branch on whether two instants coincide:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::now()
  LET b AS Instant = datetime::instant(0)
  IF datetime::equals(a, b) THEN io::print("same instant")
END SUB
```"#;
const INTRO_IS_AFTER: &str =
    r#"Test whether one instant strictly follows another on the UTC timeline."#;
const DESC_IS_AFTER: &str = r#"`datetime::isAfter` is a convenience predicate over instants that returns
`TRUE` when `a` strictly follows `b` on the UTC timeline and `FALSE` otherwise.
It is defined directly in terms of `datetime::compare`: the result is exactly
`datetime::compare(a, b) > 0`, so it is `TRUE` only when `compare` reports `1`
and `FALSE` when `compare` reports `0` or `-1`.


The comparison is performed field by field, matching `datetime::compare`. The
`seconds` fields are compared first; only when they are equal are the `nanos`
fields used as a tiebreaker. As a consequence, two instants that name the same
point (equal `seconds` and equal `nanos`) are not "after" each other, so
`isAfter` returns `FALSE` for equal instants — the relation is strict, not
"after or equal". Because both arguments are points on the same Unix-epoch,
leap-second-free UTC timeline, the ordering is absolute and independent of any
time zone; resolve a `DateTime` to an `Instant` with `datetime::resolve` before
comparing.

`isAfter` is pure: the same two instants always yield the same `Boolean`, it
has no side effects, and it performs only signed comparisons (no arithmetic), so
it cannot overflow or trap. For the symmetric test use `datetime::isBefore`, for
an equality test use `datetime::equals`, and for a three-way sign rather than a
`Boolean` use `datetime::compare`. To measure the size of the gap between two
instants rather than just their order, use `datetime::between`."#;
const EX_IS_AFTER: &str = r#"A later instant is after an earlier one:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(2_000)
  LET b AS Instant = datetime::instant(1_000)
  io::print(toString(datetime::isAfter(a, b)))
END SUB
```

Equal instants are not after each other:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(1_000)
  io::print(toString(datetime::isAfter(a, b)))
END SUB
```

Branch on chronological order:

```
IMPORT datetime
IMPORT io

SUB main()
  LET past AS Instant = datetime::instant(0)
  LET nowInstant AS Instant = datetime::now()
  IF datetime::isAfter(nowInstant, past) THEN io::print("now is later")
END SUB
```"#;
const INTRO_IS_BEFORE: &str =
    r#"Test whether one instant strictly precedes another on the UTC timeline."#;
const DESC_IS_BEFORE: &str = r#"`datetime::isBefore` is a convenience predicate over instants that returns
`TRUE` when `a` strictly precedes `b` on the UTC timeline and `FALSE` otherwise.
It is defined directly in terms of `datetime::compare`: the result is exactly
`datetime::compare(a, b) < 0`, so it is `TRUE` only when `compare` reports `-1`
and `FALSE` when `compare` reports `0` or `1`.


The comparison is performed field by field, matching `datetime::compare`. The
`seconds` fields are compared first; only when they are equal are the `nanos`
fields used as a tiebreaker. As a consequence, two instants that name the same
point (equal `seconds` and equal `nanos`) are not "before" each other, so
`isBefore` returns `FALSE` for equal instants — the relation is strict, not
"before or equal". Because both arguments are points on the same Unix-epoch,
leap-second-free UTC timeline, the ordering is absolute and independent of any
time zone; resolve a `DateTime` to an `Instant` with `datetime::resolve` before
comparing.

`isBefore` is pure: the same two instants always yield the same `Boolean`, it
has no side effects, and it performs only signed comparisons (no arithmetic), so
it cannot overflow or trap. For the symmetric test use `datetime::isAfter`, for
an equality test use `datetime::equals`, and for a three-way sign rather than a
`Boolean` use `datetime::compare`. To measure the size of the gap between two
instants rather than just their order, use `datetime::between`."#;
const EX_IS_BEFORE: &str = r#"An earlier instant is before a later one:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(2_000)
  io::print(toString(datetime::isBefore(a, b)))
END SUB
```

Equal instants are not before each other:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(1_000)
  io::print(toString(datetime::isBefore(a, b)))
END SUB
```

Branch on chronological order:

```
IMPORT datetime
IMPORT io

SUB main()
  LET past AS Instant = datetime::instant(0)
  LET nowInstant AS Instant = datetime::now()
  IF datetime::isBefore(past, nowInstant) THEN io::print("past is earlier")
END SUB
```"#;
const INTRO_COMPARE: &str = r#"Order two instants on the UTC timeline as a three-way sign."#;
const DESC_COMPARE: &str = r#"`datetime::compare` returns the sign of `a - b` as a three-way ordering: `-1`
when `a` is before `b`, `0` when the two instants name the same point, and `1`
when `a` is after `b`. The result is the standard comparator value suitable for
driving a sort or a branch on ordering, and it never returns any value other
than `-1`, `0`, or `1`.

The comparison is performed field by field. The `seconds` fields are compared
first: if `a.seconds` is less than `b.seconds` the result is `-1`, and if it is
greater the result is `1`. Only when the `seconds` fields are equal are the
`nanos` fields compared the same way, so the sub-second component acts as a
tiebreaker. When both `seconds` and `nanos` are equal the instants are
identical and the result is `0`. Because both arguments are points on the same
Unix-epoch, leap-second-free UTC timeline, the ordering is absolute and
independent of any time zone; resolve a `DateTime` to an `Instant` with
`datetime::resolve` before comparing.

`compare` is pure: the same two instants always yield the same `Integer`, it
has no side effects, and it performs only signed comparisons (no arithmetic),
so it cannot overflow or trap. For a `Boolean` test rather than a three-way
sign, use `datetime::isBefore`, `datetime::isAfter`, or `datetime::equals`, each
of which is defined in terms of `compare`. To measure the size of the gap
between two instants rather than just their order, use `datetime::between`."#;
const EX_COMPARE: &str = r#"Order two instants:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(2_000)
  io::print(toString(datetime::compare(a, b)))
END SUB
```

Equal instants compare as zero:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(1_000)
  io::print(toString(datetime::compare(a, b)))
END SUB
```

Branch on the three-way ordering:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::now()
  LET b AS Instant = datetime::instant(0)
  LET order AS Integer = datetime::compare(a, b)
  IF order < 0 THEN io::print("a is earlier")
END SUB
```"#;
const INTRO_ADD_MONTHS: &str = r#"Shift a civil `DateTime` by a whole number of calendar months, clamping the day-of-month to the target month's length."#;
const DESC_ADD_MONTHS: &str = r#"`datetime::addMonths` advances `dt` by a whole number of calendar months and
returns the resulting `DateTime`. It collapses `dt`'s year and month into a
single month index (`year * 12 + month - 1`), adds `months`, and splits the sum
back into a target year and month with a flooring divide so that crossing year
boundaries in either direction is handled correctly.
 The wall-clock time of day
and the zone are taken unchanged from `dt`, and the result is re-resolved through
`dt`'s zone so the UTC offset is recomputed for the new date.


Because months vary in length, the day of month is clamped to the number of days
in the target month. If `dt`'s day-of-month exceeds the target month's length the
result lands on the last day of that month, so January 31 plus one month is
February 28 (or February 29 in a leap year), and any earlier day is preserved
exactly. The day is never carried over into the following month.


`months` is a signed count: a positive value moves `dt` later in the calendar and
a negative value moves it earlier; adding zero months returns a `DateTime` with
the same date as `dt`. The operation works purely in whole months and never
alters the hour, minute, second, or nanosecond fields; the sub-second nanosecond
component is carried through unchanged. Because the result is re-resolved through
`dt`'s zone, `addMonths` is daylight-saving aware: the wall-clock time is
preserved while the underlying instant absorbs any offset change for the new
date. For whole-day shifts use `datetime::addDays`, and for uniform physical-time
arithmetic on an `Instant` use `datetime::add`. `addMonths` is pure: the same
`DateTime` and month count always yield the same result, and it has no side
effects."#;
const EX_ADD_MONTHS: &str = r#"Advance a `DateTime` by one month:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET nextMonth AS DateTime = datetime::addMonths(dt, 1)
END SUB
```

A negative count moves the date earlier, and an overlong day clamps to the end of
the shorter month:

```
IMPORT datetime

SUB main()
  LET jan31 AS DateTime = datetime::civil(datetime::date(2025, 1, 31), datetime::time(9, 0, 0), datetime::utc())
  LET feb28 AS DateTime = datetime::addMonths(jan31, 1)
  LET lastYear AS DateTime = datetime::addMonths(jan31, -12)
END SUB
```"#;
const INTRO_ADD_DAYS: &str = r#"Shift a civil `DateTime` by a whole number of calendar days, preserving its wall-clock time and zone."#;
const DESC_ADD_DAYS: &str = r#"`datetime::addDays` advances `dt` by a whole number of calendar days and returns
the resulting `DateTime`. It converts `dt`'s calendar date to a serial day count,
adds `days`, converts that count back to a year-month-day date, and rebuilds the
`DateTime` from the new date, `dt`'s original wall-clock time, and `dt`'s original
zone.

Because the result is re-resolved through `dt`'s zone, `addDays` is
daylight-saving aware: the wall-clock time of day is preserved and the UTC offset
is recomputed for the new date, so crossing a DST transition shifts the
underlying instant by the appropriate 23-, 24-, or 25-hour day rather than a
fixed `86_400` seconds. The sub-second nanosecond component of the time is carried
through unchanged.

`days` is a signed count: a positive value moves `dt` later in the calendar and a
negative value moves it earlier. Adding zero days returns a `DateTime` equal to
`dt`. The operation works purely in whole days and never alters the hour, minute,
second, or nanosecond fields; for month-length-aware shifts use
`datetime::addMonths`, and for uniform physical-time arithmetic on an `Instant`
use `datetime::add`. `addDays` is pure: the same `DateTime` and day count always
yield the same result, and it has no side effects."#;
const EX_ADD_DAYS: &str = r#"Advance a `DateTime` by one week:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET nextWeek AS DateTime = datetime::addDays(dt, 7)
END SUB
```

A negative count moves the date earlier:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET yesterday AS DateTime = datetime::addDays(dt, -1)
END SUB
```"#;
const INTRO_BETWEEN: &str = r#"The signed `Duration` span between two instants."#;
const DESC_BETWEEN: &str = r#"`datetime::between` returns the signed `Duration` `finish - start`: the length of
elapsed time you would add to `start` to reach `finish`. The span is positive when
`finish` is later than `start`, negative when `finish` is earlier, and zero when
the two instants are equal. Because the result is a `Duration` it carries no anchor
on the timeline — it names a length, not a point.

The span is computed by subtracting the two `Instant`s field by field
(`finish.seconds - start.seconds` and `finish.nanos - start.nanos`) and then
normalizing the pair so the stored `nanos` lands in `0 .. 999_999_999` and any
borrow is carried into the `seconds` field. A negative nanosecond difference
borrows a whole second during normalization, so the `seconds` field of the result
is the floored whole-second component of the true difference and the `nanos` field
is the non-negative sub-second remainder.



Both instants are points on the same Unix-epoch, leap-second-free UTC timeline, so
the span is independent of any time zone; resolve a `DateTime` to an `Instant` with
`datetime::resolve` before measuring. `between` is pure: the same two instants
always yield the same `Duration`, and it has no side effects. The subtraction and
the normalizing carry are ordinary signed `Integer` arithmetic, so two instants far
enough apart that their second difference falls outside the signed `Integer` range
overflow and trap. Render the result with `datetime::formatDuration`, and combine or
apply spans with `datetime::plus`, `datetime::minus`, `datetime::negate`,
`datetime::add`, and `datetime::subtract`."#;
const EX_BETWEEN: &str = r#"Measure the span between two instants and render it:

```
IMPORT datetime
IMPORT io

SUB main()
  LET start AS Instant = datetime::instant(1_000)
  LET finish AS Instant = datetime::instant(1_090)
  LET span AS Duration = datetime::between(start, finish)
  io::print(datetime::formatDuration(span))
END SUB
```

A `finish` earlier than `start` yields a negative span:

```
IMPORT datetime

SUB main()
  LET start AS Instant = datetime::instant(1_090)
  LET finish AS Instant = datetime::instant(1_000)
  LET span AS Duration = datetime::between(start, finish)
END SUB
```

Re-apply the measured span to recover `finish` from `start`:

```
IMPORT datetime

SUB main()
  LET start AS Instant = datetime::instant(1_000)
  LET finish AS Instant = datetime::instant(1_090)
  LET span AS Duration = datetime::between(start, finish)
  LET again AS Instant = datetime::add(start, span)
END SUB
```"#;
const INTRO_SUBTRACT: &str =
    r#"Shift an `Instant` backward along the UTC timeline by a `Duration`."#;
const DESC_SUBTRACT: &str = r#"`datetime::subtract` returns the `Instant` reached by moving `at` backward along
the UTC timeline by the span `by`. It subtracts the `seconds` field of `by` from
the `seconds` field of `at` and the `nanos` field of `by` from the `nanos` field
of `at`, independently, then normalizes the difference so the stored `nanos`
lands in the range `0 .. 999_999_999`, borrowing a whole second from the
`seconds` field when the nanosecond difference is negative. The result is a point
on the same Unix-epoch, leap-second-free timeline as `at`.


Because `by` is a signed `Duration`, `subtract` covers both directions on the
timeline: a positive span moves the `Instant` earlier and a negative span moves
it later, so `datetime::add(at, by)` and `datetime::subtract(at, by)` name
opposite shifts. The arithmetic is uniform second-and-nanosecond subtraction with
no awareness of calendars, time zones, or daylight-saving transitions; it simply
counts elapsed physical time. For civil, zone-aware day and month arithmetic that
honors DST and varying month lengths, use `datetime::addDays` and
`datetime::addMonths` on a `DateTime` instead.

Normalization floor-divides the nanosecond difference into a whole-second borrow
and a non-negative remainder, then folds the borrow back into the `seconds`
field, so a subtraction that borrows across the second boundary still yields a
`nanos` in `0 .. 999_999_999`.
The subtraction is ordinary signed `Integer` arithmetic, so a span large enough
to push the combined second count past the `Integer` range overflows and traps.
`subtract` is pure: the same `Instant` and `Duration` always yield the same
`Instant`, and it has no side effects."#;
const EX_SUBTRACT: &str = r#"Move an `Instant` back by a 90-second span:

```
IMPORT datetime

SUB main()
  LET base AS Instant = datetime::instant(1_700_000_000)
  LET earlier AS Instant = datetime::subtract(base, datetime::duration(90))
END SUB
```

A negative `Duration` shifts the `Instant` forward:

```
IMPORT datetime

SUB main()
  LET base AS Instant = datetime::instant(1_700_000_000)
  LET later AS Instant = datetime::subtract(base, datetime::duration(-3600))
END SUB
```"#;
const INTRO_ADD: &str = r#"Shift an `Instant` forward along the UTC timeline by a `Duration`."#;
const DESC_ADD: &str = r#"`datetime::add` returns the `Instant` reached by advancing `at` forward along
the UTC timeline by the span `by`. It adds the two `seconds` fields and the two
`nanos` fields independently, then normalizes the sum so the stored `nanos`
lands in the range `0 .. 999_999_999`, carrying any whole seconds embedded in
the nanosecond sum into the `seconds` field. The result is a point on the same
Unix-epoch, leap-second-free timeline as `at`.


Because `by` is a signed `Duration`, `add` covers both directions on the
timeline: a positive span moves the `Instant` later and a negative span moves it
earlier, so `datetime::add(at, by)` and `datetime::subtract(at, by)` name
opposite shifts. The arithmetic is uniform second-and-nanosecond addition with
no awareness of calendars, time zones, or daylight-saving transitions; it simply
counts elapsed physical time. For civil, zone-aware day and month arithmetic
that honors DST and varying month lengths, use `datetime::addDays` and
`datetime::addMonths` on a `DateTime` instead.

Normalization floor-divides the nanosecond sum into a whole-second carry and a
non-negative remainder, then folds the carry back into the `seconds` field, so
a negative `Duration` that borrows across the second boundary still yields a
`nanos` in `0 .. 999_999_999`.
The addition is ordinary signed `Integer` arithmetic, so a span large enough to
push the combined second count past the `Integer` range overflows and traps.
`add` is pure: the same `Instant` and `Duration` always yield the same
`Instant`, and it has no side effects."#;
const EX_ADD: &str = r#"Advance an `Instant` by a 90-second span:

```
IMPORT datetime

SUB main()
  LET base AS Instant = datetime::instant(1_700_000_000)
  LET later AS Instant = datetime::add(base, datetime::duration(90))
END SUB
```

A negative `Duration` shifts the `Instant` backward:

```
IMPORT datetime

SUB main()
  LET base AS Instant = datetime::instant(1_700_000_000)
  LET earlier AS Instant = datetime::add(base, datetime::duration(-3600))
END SUB
```"#;
const INTRO_WITH_ZONE: &str =
    r#"Re-project a `DateTime` into a different `Zone`, preserving the absolute instant."#;
const DESC_WITH_ZONE: &str = r#"`datetime::withZone` returns the civil `DateTime` that an observer in `zone`
reads at the very same absolute moment named by `dt`. The underlying point on the
UTC timeline is unchanged; only the wall-clock fields, the carried `zone`, and the
resolved UTC offset are re-derived for the new zone.

The function is exactly the composition of `datetime::resolve` and
`datetime::inZone`: it collapses `dt` back to an `Instant` with `datetime::resolve`
and then projects that `Instant` into `zone` with `datetime::inZone`.


The `resolve` step reads the offset already pinned on `dt` to reach the UTC
timeline without any zone lookup (`daysFromCivil(...) * 86400 + hour * 3600 +
minute * 60 + second - dt.offset`). The `inZone` step then resolves the effective
offset for `zone` at that instant — zero for a UTC zone (`ZoneKind::Utc`), the
stored constant for a fixed-offset zone (`ZoneKind::FixedOffset`, built with
`datetime::fixedOffset`), and the DST-correct host offset for a local zone
(`ZoneKind::Local`, built with `datetime::local`) — adds it to the instant's
seconds, floor-divides into whole days and second-of-day, and splits the result
into civil year/month/day and hour/minute/second with the proleptic Gregorian
calendar.



The returned `DateTime` carries the new civil date and time, `zone` itself, and
the offset resolved for `zone`. The sub-second `nanos` field is carried through
both steps verbatim, so it equals `dt.time.nanos`. Because the instant is
preserved, `datetime::resolve` on the result returns the same `Instant` as
`datetime::resolve` on `dt`: `withZone` is an identity on the absolute moment and
changes only its civil presentation. It is pure for UTC and fixed-offset zones;
for a local zone it reads the host's time-zone configuration through the
`datetime::localOffset` OS intrinsic to resolve the offset."#;
const EX_WITH_ZONE: &str = r#"Re-project a UTC `DateTime` into a fixed +05:30 zone:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::utc())
  LET z AS Zone = datetime::fixedOffset(5, 30)
  LET shifted AS DateTime = datetime::withZone(dt, z)
END SUB
```

Convert a `DateTime` to the host's local zone without changing the instant:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::utc())
  LET local AS DateTime = datetime::withZone(dt, datetime::local())
END SUB
```"#;
const INTRO_CIVIL: &str = r#"Build a zoned `DateTime` from a civil `Date`, `Time`, and `Zone`."#;
const DESC_CIVIL: &str = r#"`datetime::civil` builds a `DateTime` by reading a calendar `date` and a
wall-clock `time` as a local time in `zone`, resolving the UTC offset that
applies to that local moment, and returning the canonical projection of the
resulting `Instant` back through `zone`. Because the result is the projection of
a concrete `Instant`, it round-trips: `datetime::resolve` on the returned
`DateTime` recovers the same `Instant`, and that `Instant` projected through
`zone` with `datetime::inZone` reproduces the same `DateTime` fields.


The `year`, `month`, and `day` of `date` and the `hour`, `minute`, and `second`
of `time` are combined into a single second count (`daysFromCivil * 86400 +
hour * 3600 + minute * 60 + second`) that names the wall-clock moment, treated
as a civil (zone-local) time. The offset for that moment is then resolved from
`zone`. For a zone with a fixed offset (built by `datetime::utc` or
`datetime::fixedOffset`) the offset is constant; for the host's local zone
(`datetime::local`) it is resolved from the platform's zone table at that
instant, so the result is daylight-saving correct.


When the named local time does not exist or is not unique because of a
daylight-saving transition, `civil` resolves it deterministically. It probes the
zone's offset one day before and one day after the named local time to bracket
any single nearby transition. If both probes agree, that offset is used
directly. If they differ, a spring-forward gap (the named local time is skipped)
shifts forward onto the post-transition offset, and a fall-back overlap (the
named local time occurs twice) takes the earlier, pre-transition offset.


The sub-second `nanos` of `time` are carried through unchanged into the
resulting `Instant` and `DateTime`; only the whole-second civil fields
participate in offset resolution. `civil` is pure: beyond what `zone` itself
resolves it reads no host state and has no side effects."#;
const EX_CIVIL: &str = r#"Combine a date and time into a `DateTime` in the local zone:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::local())
END SUB
```

Build a `DateTime` in UTC and recover its `Instant`:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 1, 1)
  LET tm AS Time = datetime::time(0, 0)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::utc())
  LET at AS Instant = datetime::resolve(dt)
END SUB
```"#;
const INTRO_RESOLVE: &str =
    r#"Collapse a civil `DateTime` back to the absolute `Instant` it names."#;
const DESC_RESOLVE: &str = r#"`datetime::resolve` is the inverse of `datetime::inZone`: where `inZone` projects
an absolute instant onto the wall-clock fields an observer in a zone reads,
`resolve` collapses those wall-clock fields — together with the UTC offset already
pinned on `dt` — back onto the single point on the UTC timeline they denote.

The computation is total and needs no zone lookup. `resolve` first converts the
civil date (`dt.date.year`, `dt.date.month`, `dt.date.day`) to a day count with
the proleptic Gregorian calendar, multiplies by `86400` to get seconds, and adds
the time-of-day contribution (`dt.time.hour * 3600 + dt.time.minute * 60 +
dt.time.second`). That sum is the local second count: the seconds-since-epoch the
wall-clock fields would name if they were UTC. It then subtracts `dt.offset` — the
resolved UTC offset in seconds carried on the `DateTime` — to shift the local
count back onto the UTC timeline, and pairs the result with `dt.time.nanos`.


Because the offset is read directly from `dt` rather than re-derived from the
zone, `resolve` is unambiguous even across daylight-saving transitions: it
reproduces exactly the instant a `DateTime` was built from. For any instant `at`
and zone `z`, `datetime::resolve(datetime::inZone(at, z))` returns `at` unchanged.
The `seconds` field participates in the date/time arithmetic; the `nanos` field is
copied through verbatim. `resolve` is pure and reads no host state."#;
const EX_RESOLVE: &str = r#"Round-trip an instant through a civil `DateTime` and back:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::now()
  LET dt AS DateTime = datetime::inZone(at, datetime::utc())
  LET back AS Instant = datetime::resolve(dt)
END SUB
```

Resolve a civil `DateTime` built in a fixed +05:30 zone:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(5, 30)
  LET dt AS DateTime = datetime::inZone(datetime::now(), z)
  LET at AS Instant = datetime::resolve(dt)
END SUB
```"#;
const INTRO_TO_LOCAL: &str =
    r#"Project an absolute `Instant` into the host's local zone to produce a civil `DateTime`."#;
const DESC_TO_LOCAL: &str = r#"`datetime::toLocal` projects the absolute instant `at` into the host's local
time zone, yielding the calendar date and wall-clock time that an observer
reading the local clock sees at that moment. It is exactly shorthand for
`datetime::inZone(at, datetime::local())`: it resolves the host's effective UTC
offset for the instant `at` (see `datetime::offsetAt`), with daylight-saving
time applied as it stood at that instant, adds that offset in seconds to the
instant's seconds-since-epoch to obtain a local second count, floor-divides that
into whole days and the second-of-day, converts the day count to a civil
year/month/day with the proleptic Gregorian calendar, and decomposes the
second-of-day into hour, minute, and second.



The returned `DateTime` carries four things: the civil date, the civil time, the
local zone, and the resolved offset. Because the resolved offset is pinned onto
the result, the `DateTime` round-trips back to the original instant via
`datetime::resolve` with no further zone lookup. The instant's sub-second
`nanos` field is preserved verbatim into the time's `nanos` field; only the
`seconds` field participates in the offset and date/time computation, so an
instant before the Unix epoch (negative `seconds`) projects correctly.


Unlike `datetime::toUtc`, `datetime::toLocal` is not pure: it reads the host's
time-zone configuration to resolve the offset, so the same instant can produce a
different civil `DateTime` on a host configured for a different zone or under a
different DST rule."#;
const EX_TO_LOCAL: &str = r#"Project the current instant into the host's local zone:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toLocal(datetime::now())
END SUB
```

Round-trip an instant through the local zone and back:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::now()
  LET dt AS DateTime = datetime::toLocal(at)
  LET back AS Instant = datetime::resolve(dt)
END SUB
```"#;
const INTRO_TO_UTC: &str =
    r#"Project an absolute `Instant` into UTC to produce a civil `DateTime`."#;
const DESC_TO_UTC: &str = r#"`datetime::toUtc` projects the absolute instant `at` into the UTC zone, yielding
the calendar date and wall-clock time that an observer reading UTC sees at that
moment. It is exactly shorthand for `datetime::inZone(at, datetime::utc())`: the
UTC zone contributes a zero offset, so the instant's seconds-since-epoch are
split directly — floor-divided into whole days and the second-of-day — into a
civil year/month/day (proleptic Gregorian calendar) and an
hour/minute/second-of-day, with no offset adjustment.


The returned `DateTime` carries four things: the civil date, the civil time, the
UTC zone, and a resolved offset of zero. Because the zero offset is pinned onto
the result, the `DateTime` round-trips back to the original instant via
`datetime::resolve` with no further zone lookup. The instant's sub-second
`nanos` field is preserved verbatim into the time's `nanos` field; only the
`seconds` field participates in the date and time computation, so an instant
before the Unix epoch (negative `seconds`) projects correctly.


Unlike `datetime::toLocal`, `datetime::toUtc` is pure: it reads no host
time-zone configuration and produces the same result on every platform. Because
the resolved offset is always zero, adding it to the instant's seconds cannot
overflow the `Integer` range, so this call raises no error of its own."#;
const EX_TO_UTC: &str = r#"Project the current instant into UTC:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
END SUB
```

Round-trip an instant through UTC and back:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::now()
  LET dt AS DateTime = datetime::toUtc(at)
  LET back AS Instant = datetime::resolve(dt)
END SUB
```"#;
const INTRO_IN_ZONE: &str =
    r#"Project an absolute `Instant` into a `Zone` to produce a civil `DateTime`."#;
const DESC_IN_ZONE: &str = r#"`datetime::inZone` is the primary "to civil time" call: it projects the absolute
instant `at` through `zone`, yielding the calendar date and wall-clock time that
an observer in that zone reads at that moment.

It first resolves the effective UTC offset for `zone` at the instant `at` — the
same quantity `datetime::offsetAt` returns: zero for a UTC zone
(`ZoneKind::Utc`), the stored constant for a fixed-offset zone (`ZoneKind::FixedOffset`,
kind `1`, built with `datetime::fixedOffset`), and the DST-correct host offset
for a local zone (`ZoneKind::Local`, kind `2`, built with `datetime::local`).
 It then adds
that offset, in seconds, to the instant's seconds-since-epoch to obtain a local
second count, floor-divides that into whole days and the second-of-day, converts
the day count to a civil year/month/day with the proleptic Gregorian calendar,
and decomposes the second-of-day into hour, minute, and second.


The returned `DateTime` carries four things: the civil date, the civil time,
`zone` itself, and the resolved offset. Because the offset is pinned onto the
result, the `DateTime` round-trips back to the original instant via
`datetime::resolve` with no further zone lookup. The instant's sub-second `nanos`
field is preserved verbatim into the time's `nanos` field; only the `seconds`
field participates in the offset and date/time computation, so an instant before
the Unix epoch (negative `seconds`) projects correctly.


`datetime::toUtc` and `datetime::toLocal` are shorthands for calling `inZone`
with the UTC zone and the host local zone, respectively. `inZone` is pure for UTC
and fixed-offset zones; for a local zone it reads the host's time-zone
configuration through the `datetime::localOffset` OS intrinsic to resolve the
offset."#;
const EX_IN_ZONE: &str = r#"Project the current instant into UTC:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::utc())
END SUB
```

Project an instant into a fixed +05:30 zone:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(5, 30)
  LET dt AS DateTime = datetime::inZone(datetime::now(), z)
END SUB
```

Project into the host's local zone, with DST applied for that instant:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::local())
END SUB
```"#;
const INTRO_OFFSET_AT: &str = r#"A `Zone`'s signed UTC offset in seconds at a given `Instant`."#;
const DESC_OFFSET_AT: &str = r#"`datetime::offsetAt` returns the signed offset from UTC, in seconds, that
`zone` applies to the absolute instant `at`. A positive result places the
zone's civil fields ahead of UTC (east of the prime meridian), a negative
result places them behind UTC (west), and zero means the zone coincides with
UTC at that instant. This is the exact quantity `datetime::inZone` adds to an
`Instant`'s seconds-since-epoch to produce the civil fields of a `DateTime`, so
`offsetAt` exposes that adjustment on its own.


How the offset is determined depends on the zone's kind. For a UTC zone
(`ZoneKind::Utc`) and a fixed-offset zone (`ZoneKind::FixedOffset`, built with
`datetime::fixedOffset`) the function returns the zone's stored constant offset
directly and does not consult `at` — the UTC zone stores zero, and a fixed zone
stores its single configured offset. For a local zone (`ZoneKind::Local`, built
with `datetime::local`, internally zone kind `2`) the offset is resolved
against the host's configured time zone for the specific instant `at`: it reads
the host zone table and is therefore DST-correct, returning the standard-time
offset for instants outside daylight saving and the shifted offset for instants
within it. Two calls with the same local zone but instants on opposite sides of
a DST transition can therefore return different values.


Only the `seconds` field of `at` participates; the sub-second `nanos` field is
ignored. The function reads no host state for UTC and fixed zones (those are
pure); for a local zone it reads the host's time-zone configuration through the
`datetime::localOffset` OS intrinsic."#;
const EX_OFFSET_AT: &str = r#"A UTC zone always reports a zero offset:

```
IMPORT datetime

SUB main()
  LET off AS Integer = datetime::offsetAt(datetime::utc(), datetime::now())
END SUB
```

A fixed zone reports its constant offset regardless of the instant:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(5, 30)
  LET off AS Integer = datetime::offsetAt(z, datetime::now())
END SUB
```

A local zone's offset is resolved DST-correctly for the given instant:

```
IMPORT datetime

SUB main()
  LET nowOff AS Integer = datetime::offsetAt(datetime::local(), datetime::now())
END SUB
```"#;
const INTRO_FIXED_OFFSET: &str = r#"Build a `Zone` with a constant UTC offset."#;
const DESC_FIXED_OFFSET: &str = r#"`datetime::fixedOffset` builds a `Zone` whose offset from UTC is a constant
value that does not vary with the instant being projected. Unlike
`datetime::local`, whose offset is resolved against the host's configured time
zone, and unlike `datetime::utc`, the canonical zero-offset zone, a
fixed-offset `Zone` carries a single signed offset that applies to every
`Instant` projected through it. The returned `Zone` has a zone kind of
`ZoneKind::FixedOffset` and a label rendered in the form `+HH:MM` or `-HH:MM`.


A `Zone` is the bridge between the absolute UTC timeline (an `Instant`) and the
human-readable civil fields of a `DateTime`. Projecting an `Instant` through a
fixed-offset zone with `datetime::inZone` produces a `DateTime` whose year,
month, day, and time fields are shifted from UTC by exactly the offset this
function encodes: a positive offset places the civil fields ahead of UTC (east
of the prime meridian), a negative offset places them behind UTC (west).

The one-argument form takes the offset directly as a raw signed second count.
The two-argument form takes whole `hours` and a `mins` magnitude in the range
`0 .. 59`; `mins` contributes its magnitude only and inherits the sign of
`hours`. Thus `datetime::fixedOffset(-5, 30)` is `-05:30` (five hours and
thirty minutes behind UTC), and `datetime::fixedOffset(5, 30)` is `+05:30`. The
two-argument form is implemented in terms of the one-argument form by combining
the hours and minutes into a total second count of
`sign(hours) * (abs(hours) * 3600 + mins * 60)`.


In both forms the offset magnitude must be strictly under 24 hours (86400
seconds); an offset of exactly `+/-24h` or more is rejected. The function is
pure: it reads no host state and has no side effects."#;
const EX_FIXED_OFFSET: &str = r#"Build a zone five and a half hours behind UTC:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(-5, 30)
END SUB
```

Build the same zone from a raw second count:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(-19800)
END SUB
```

Project the current instant into a fixed `+09:00` zone:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::now()
  LET local AS DateTime = datetime::inZone(t, datetime::fixedOffset(9, 0))
END SUB
```"#;
const INTRO_LOCAL: &str = r#"The `Zone` representing the host's local time."#;
const DESC_LOCAL: &str = r#"`datetime::local` returns the `Zone` that represents the host's local time. The
returned `Zone` carries a zone kind of `ZoneKind::Local` (the third `ZoneKind`
variant, tag `2`), marking it as the platform-resolved local zone rather than the
canonical UTC zone built by `datetime::utc` (kind `ZoneKind::Utc`, tag `0`) or an
arbitrary fixed offset built by `datetime::fixedOffset` (kind
`ZoneKind::FixedOffset`, tag `1`).


Unlike `datetime::utc` and `datetime::fixedOffset`, whose offsets are baked into
the `Zone` at construction, the local zone holds no fixed offset of its own. The
`Zone` returned here stores a placeholder offset of zero seconds and the label
`"Local"`; the true offset is resolved per-instant from the platform's zone
table when the zone is applied to a particular moment. Projecting an `Instant`
through this zone with `datetime::inZone` consults that table for the instant
being projected, so the result is DST-correct: the same local zone yields one
offset for a summer instant and another for a winter instant when the host
observes daylight saving time. `datetime::toLocal` is the dedicated shorthand
for projecting an `Instant` through this zone.

Because the offset is resolved from host configuration, the civil fields a given
`Instant` projects to depend on the machine: two hosts in different configured
time zones project the same `Instant` to different `DateTime` fields.

`datetime::local` takes no arguments. The call itself is pure and constant: it
always returns the same placeholder `Zone`, reads no host state, and has no side
effects. The dependence on the host's configured zone enters only later, when
the zone is resolved against an instant during projection."#;
const EX_LOCAL: &str = r#"Obtain the local zone:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::local()
END SUB
```

Project the current instant into the local zone to read its civil fields:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::now()
  LET here AS DateTime = datetime::inZone(t, datetime::local())
END SUB
```

Combine a date and time into a `DateTime` in the local zone:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::local())
END SUB
```"#;
const INTRO_UTC: &str = r#"The `Zone` representing Coordinated Universal Time."#;
const DESC_UTC: &str = r#"`datetime::utc` returns the `Zone` that represents Coordinated Universal Time: a
fixed zone whose offset from UTC is a constant zero seconds and whose label is
the literal string `"UTC"`. The returned `Zone` carries a zone kind of
`ZoneKind::Utc` (the first `ZoneKind` variant, tag `0`), marking it as the
canonical UTC zone rather than an arbitrary fixed offset built with
`datetime::fixedOffset` (kind `ZoneKind::FixedOffset`).


A `Zone` is the bridge between the absolute UTC timeline (an `Instant`) and the
human-readable civil fields of a `DateTime`. Project an `Instant` through this
zone with `datetime::inZone` to obtain a `DateTime` whose year, month, day, and
time fields are expressed in UTC; `datetime::toUtc` is the dedicated shorthand
for exactly that projection. Because the offset is always zero, the civil fields
of a `DateTime` in this zone match the seconds-since-epoch of the originating
`Instant` directly, with no offset adjustment.

`datetime::utc` takes no arguments and always returns the same constant `Zone`.
It is pure: every call yields an identical UTC zone, it reads no host state, and
it has no side effects. Unlike `datetime::local`, whose offset depends on the
host's configured time zone, `datetime::utc` is wholly independent of the
environment."#;
const EX_UTC: &str = r#"Obtain the UTC zone:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::utc()
END SUB
```

Project the current instant into UTC to read its civil fields:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::now()
  LET inUtc AS DateTime = datetime::inZone(t, datetime::utc())
END SUB
```

Combine a date and time into a UTC-zoned `DateTime`:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::utc())
END SUB
```"#;
const INTRO_DURATION: &str =
    r#"Build a `Duration` span from seconds, nanoseconds, or larger time components."#;
const DESC_DURATION: &str = r#"`datetime::duration` builds a signed `Duration`, a span of elapsed time with no
anchor on any timeline. The result carries a whole-second count in its `seconds`
field and a sub-second remainder in its `nanos` field, normalized into the range
`0 .. 999_999_999`. A `Duration` measures a length of time rather than a point in
time; to name a point on the UTC timeline use `datetime::instant` instead.

`duration` is overloaded by argument count, with five disjoint forms selected by
the number of `Integer` arguments (one through five).
 The one- and two-argument forms take
whole seconds and, optionally, a nanosecond adjustment. The three-, four-, and
five-argument forms are component builders that fold larger units down into a
single second count: the three-argument form computes `mins*60 + seconds`, the
four-argument form adds `hours*3600`, and the five-argument form adds
`days*86400`, in every case adding the trailing `nanos` last.


Whichever form is used (except the one-argument form), the supplied seconds and
nanos are normalized: any whole seconds embedded in `nanos` are carried into the
`seconds` field, and a negative `nanos` value borrows a second so the stored
`nanos` always lands in `0 .. 999_999_999`.
 Every numeric
argument may be negative, which yields a negative span pointing backward in time.
The one-argument form performs no normalization because its `nanos` is fixed at
zero.

`duration` is overloaded, so every parameter of the form you call must be supplied
explicitly; the component forms carry no defaults.
 The folding and
normalization are ordinary signed `Integer` arithmetic, so a sufficiently large
day, hour, minute, or second magnitude can overflow the `Integer` range and trap.
Combine durations with `datetime::plus`, `datetime::minus`, and `datetime::negate`;
apply one to an `Instant` with `datetime::add` or `datetime::subtract`. `duration`
is pure: the same arguments always yield the same `Duration`, and it has no side
effects."#;
const EX_DURATION: &str = r#"Build a `Duration` from a whole-second span:

```
IMPORT datetime

SUB main()
  LET d AS Duration = datetime::duration(90)
END SUB
```

Build a `Duration` with a sub-second adjustment that normalizes into the `seconds`
field:

```
IMPORT datetime

SUB main()
  LET d AS Duration = datetime::duration(10, 1_500_000_000)
END SUB
```

Build a `Duration` from day, hour, minute, second, and nanosecond components:

```
IMPORT datetime

SUB main()
  LET d AS Duration = datetime::duration(1, 2, 3, 4, 0)
END SUB
```

A negative argument yields a backward span:

```
IMPORT datetime

SUB main()
  LET d AS Duration = datetime::duration(-30)
END SUB
```"#;
const INTRO_TIME: &str = r#"Validate and build a time-of-day `Time` from hour, minute, second, and sub-second components."#;
const DESC_TIME: &str = r#"`datetime::time` builds a `Time` of day from its `hour`, `minute`, `second`, and
sub-second (`nanos`) components. A `Time` names a position within a single
24-hour day and carries no calendar date and no zone; pair it with a `Date`
through `datetime::civil` to build a zoned `DateTime`.

The constructor validates each component against its civil range before
returning, and there is no normalization or wrap-around: an out-of-range
component is an error, not silently carried into the next unit. `hour` must be
in `0 .. 23`, where `0` is midnight and `23` is the final hour of the day.
`minute` and `second` must each be in `0 .. 59`; the model has no leap seconds,
so `60` is never a valid second. `nanos` is the sub-second remainder and must be
in `0 .. 999_999_999`.

`second` and `nanos` default to `0`, so a two-argument call names the top of a
minute and a three-argument call names the top of a second. Unlike
`datetime::instant` and `datetime::duration`, `time` is not overloaded but a
single signature with trailing defaults, so the defaults apply and you may omit
`second`, or both `second` and `nanos`.

`time` is pure: the same arguments always yield the same `Time`, and it has no
side effects."#;
const EX_TIME: &str = r#"Construct a time at the top of a minute (`second` and `nanos` default to `0`):

```
IMPORT datetime

SUB main()
  LET t AS Time = datetime::time(9, 30)
END SUB
```

Construct a time with whole seconds:

```
IMPORT datetime

SUB main()
  LET t AS Time = datetime::time(23, 59, 59)
END SUB
```

Combine a date and time into a zoned `DateTime`:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET t AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, t, datetime::utc())
END SUB
```

An out-of-range field raises `ErrInvalidArgument`:

```
IMPORT datetime

SUB main()
  LET bad AS Time = datetime::time(24, 0)
END SUB
```"#;
const INTRO_DATE: &str =
    r#"Validate and build a calendar `Date` from year, month, and day components."#;
const DESC_DATE: &str = r#"`datetime::date` builds a calendar `Date` on the proleptic-Gregorian calendar
from its `year`, `month`, and `day` components. The calendar is *proleptic*: the
Gregorian rules are extended uniformly to every year, including those before the
calendar's historical adoption. `year` is an unrestricted `Integer` and may be
zero or negative.

The constructor validates the date before returning it. `month` must name a real
month in `1 .. 12`, and `day` must be in range for that month and year. The upper
bound on `day` is the actual length of the given month, computed the same way as
`datetime::daysInMonth`, so it depends on both `month` and `year`: April allows
`1 .. 30`, and February allows `1 .. 29` only in a leap year and `1 .. 28`
otherwise. February 29 is therefore accepted in leap years such as 2024 and
rejected in common years such as 2026. There is no normalization or wrap-around:
an out-of-range component is an error, not silently carried into the next unit.


`date` is pure: the same arguments always yield the same `Date`, and it has no
side effects. A `Date` carries only calendar fields and no zone or time-of-day;
pair it with `datetime::time` and `datetime::civil` to build a zoned `DateTime`."#;
const EX_DATE: &str = r#"Construct a valid date:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
END SUB
```

Combine a date and time into a zoned `DateTime`:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET t AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, t, datetime::utc())
END SUB
```

An impossible calendar date raises `ErrInvalidArgument`:

```
IMPORT datetime

SUB main()
  LET bad AS Date = datetime::date(2026, 2, 29)
END SUB
```"#;
const INTRO_INSTANT: &str =
    r#"Build an `Instant` from seconds, nanoseconds, or larger time components."#;
const DESC_INSTANT: &str = r#"`datetime::instant` builds an `Instant` on the UTC timeline (the Unix epoch,
without leap seconds) at a given offset after `1970-01-01T00:00:00Z`. The result
carries whole seconds since the epoch in its `seconds` field and a sub-second
remainder in its `nanos` field, normalized into the range `0 .. 999_999_999`.

`instant` is overloaded by argument count, with five disjoint forms selected by
the number of `Integer` arguments (one through five).
 The one- and two-argument forms take
whole seconds and, optionally, a nanosecond adjustment. The three-, four-, and
five-argument forms are component builders that fold larger units down into a
single second count: the three-argument form computes `mins*60 + seconds`, the
four-argument form adds `hours*3600`, and the five-argument form adds
`days*86400`, in every case adding the trailing `nanos` last.


Whichever form is used (except the one-argument form), the supplied seconds and
nanos are normalized: any whole seconds embedded in `nanos` are carried into the
`seconds` field, and a negative `nanos` value borrows a second so the stored
`nanos` always lands in `0 .. 999_999_999`.
 Every numeric
argument may be negative, which selects an instant before the epoch. The
one-argument form performs no normalization because its `nanos` is fixed at zero.


`instant` is overloaded, so every parameter of the form you call must be supplied
explicitly; the component forms carry no defaults.
 The folding and
normalization are ordinary signed `Integer` arithmetic, so a sufficiently large
day, hour, minute, or second magnitude can overflow the `Integer` range and trap.
To shift an existing `Instant` by a span rather than build one from scratch, use
`datetime::add` or `datetime::subtract` with a `Duration`. `instant` is pure: the
same arguments always yield the same `Instant`, and it has no side effects."#;
const EX_INSTANT: &str = r#"Build an `Instant` from a whole-second epoch offset:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::instant(1_700_000_000)
END SUB
```

Build an `Instant` with a sub-second adjustment that normalizes into the `seconds`
field:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::instant(10, 1_500_000_000)
END SUB
```

Build an `Instant` from day, hour, minute, second, and nanosecond components:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::instant(1, 2, 3, 4, 0)
END SUB
```"#;
const INTRO_MONOTONIC: &str =
    r#"A monotonically non-decreasing clock reading for measuring elapsed time."#;
const DESC_MONOTONIC: &str = r#"`datetime::monotonic` reads the host's monotonic clock and returns the elapsed
span, as a `Duration`, from an arbitrary fixed origin chosen by the operating
system. The clock never moves backward: a later call always returns a span that
is greater than or equal to an earlier one. It is unrelated to wall-clock time,
carries no calendar meaning, and is not comparable across processes or across
reboots, so the absolute value of a single reading is meaningless.

The only intended use is to measure elapsed time: take two readings and subtract
the earlier from the later with `datetime::minus`. Because the clock is immune to
wall-clock adjustments (NTP steps, manual clock changes, daylight saving), the
difference is a reliable interval where `datetime::now` would not be. Use
`datetime::now`, not `monotonic`, whenever you need an actual point in time.

Internally `monotonic` reads a single nanoseconds-since-origin value from the OS
intrinsic (`datetime::monotonicNanos`, `clock_gettime(CLOCK_MONOTONIC)` on the
supported platforms), then splits it into the `seconds` and `nanos` fields of a
`Duration` by a truncating divide and remainder against `1_000_000_000`. The
divisor is a non-zero constant, so the split cannot trap, and the nanosecond
remainder already falls in `0 .. 999_999_999`.


`monotonic` is **not pure**: two calls may return different spans, and the values
depend on host clock state. It takes no arguments, reads clock state only, and
has no side effects."#;
const EX_MONOTONIC: &str = r#"Measure the elapsed time around a block of work:

```
IMPORT datetime

SUB main()
  LET t0 AS Duration = datetime::monotonic()
  ' ... work ...
  LET elapsed AS Duration = datetime::minus(datetime::monotonic(), t0)
END SUB
```

Render the measured interval as text:

```
IMPORT datetime

SUB main()
  LET t0 AS Duration = datetime::monotonic()
  ' ... work ...
  LET span AS Duration = datetime::minus(datetime::monotonic(), t0)
  LET text AS String = datetime::formatDuration(span)
END SUB
```"#;
const INTRO_NOW: &str = r#"The current wall-clock instant on the UTC timeline."#;
const DESC_NOW: &str = r#"`datetime::now` reads the host's real-time clock and returns the `Instant` it
names on the UTC timeline (the Unix epoch, without leap seconds). The result
carries whole seconds since `1970-01-01T00:00:00Z` in its `seconds` field and a
sub-second `nanos` field in the range `0 .. 999_999_999`. `now` is the only
wall-clock entry point in the package; project the result through a zone with
`datetime::toUtc`, `datetime::toLocal`, or `datetime::inZone` to obtain civil
fields (year, month, day, and so on).

Internally `now` takes a single nanoseconds-since-epoch reading from the OS
intrinsic (`datetime::nowNanos`), then splits it into the `seconds` and `nanos`
fields of an `Instant` by a truncating divide and remainder against
`1_000_000_000`. The reading is non-negative and the divisor is a non-zero
constant, so the split cannot trap, and the nanosecond remainder already falls
in `0 .. 999_999_999`.

`now` is bounded by its underlying intrinsic, which reports nanoseconds since
the epoch and is valid through roughly the year 2262. This is a limit on `now`,
not on `Instant`, whose `seconds` field spans the full `Integer` range.

`now` is one of the few `datetime` functions that is **not pure**: two calls may
return different instants, and a program's output depends on the host clock. For
reproducible logic, capture a single instant and derive everything else from it.
`now` takes no arguments, reads host clock state only, and has no side effects."#;
const EX_NOW: &str = r#"Capture the current instant:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::now()
END SUB
```

Project the current instant into the local zone to read civil fields:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::now()
  LET here AS DateTime = datetime::toLocal(t)
END SUB
```"#;

const DATETIME_FUNCTIONS: &[BuiltinFunction] = &[
    df(NOW, "now", &[ov(&[], "Instant")])
        .with_intro(INTRO_NOW)
        .with_desc(DESC_NOW)
        .with_example(EX_NOW),
    df(MONOTONIC, "monotonic", &[ov(&[], "Duration")])
        .with_intro(INTRO_MONOTONIC)
        .with_desc(DESC_MONOTONIC)
        .with_example(EX_MONOTONIC),
    df(INSTANT, "instant", INSTANT_OVERLOADS)
        .with_intro(INTRO_INSTANT)
        .with_desc(DESC_INSTANT)
        .with_example(EX_INSTANT),
    df(
        DATE,
        "date",
        &[ov(
            &[req("year", I), req("month", I), req("day", I)],
            "Date",
        )],
    )
    .with_intro(INTRO_DATE)
    .with_desc(DESC_DATE)
    .with_example(EX_DATE),
    df(
        TIME,
        "time",
        &[ov(
            &[
                req("hour", I),
                req("minute", I),
                opt("second", I, "0"),
                opt("nanos", I, "0"),
            ],
            "Time",
        )],
    )
    .with_intro(INTRO_TIME)
    .with_desc(DESC_TIME)
    .with_example(EX_TIME),
    df(DURATION, "duration", DT_COMPONENTS)
        .with_intro(INTRO_DURATION)
        .with_desc(DESC_DURATION)
        .with_example(EX_DURATION),
    df(UTC, "utc", &[ov(&[], "Zone")])
        .with_intro(INTRO_UTC)
        .with_desc(DESC_UTC)
        .with_example(EX_UTC),
    df(LOCAL, "local", &[ov(&[], "Zone")])
        .with_intro(INTRO_LOCAL)
        .with_desc(DESC_LOCAL)
        .with_example(EX_LOCAL),
    df(
        FIXED_OFFSET,
        "fixedOffset",
        &[
            ov(&[req("offsetSeconds", I)], "Zone"),
            ov(&[req("hours", I), req("mins", I)], "Zone"),
        ],
    )
    .with_intro(INTRO_FIXED_OFFSET)
    .with_desc(DESC_FIXED_OFFSET)
    .with_example(EX_FIXED_OFFSET),
    df(
        OFFSET_AT,
        "offsetAt",
        &[ov(&[req("zone", "Zone"), req("at", "Instant")], I)],
    )
    .with_intro(INTRO_OFFSET_AT)
    .with_desc(DESC_OFFSET_AT)
    .with_example(EX_OFFSET_AT),
    df(
        IN_ZONE,
        "inZone",
        &[ov(&[req("at", "Instant"), req("zone", "Zone")], "DateTime")],
    )
    .with_intro(INTRO_IN_ZONE)
    .with_desc(DESC_IN_ZONE)
    .with_example(EX_IN_ZONE),
    df(TO_UTC, "toUtc", &[ov(&[req("at", "Instant")], "DateTime")])
        .with_intro(INTRO_TO_UTC)
        .with_desc(DESC_TO_UTC)
        .with_example(EX_TO_UTC),
    df(
        TO_LOCAL,
        "toLocal",
        &[ov(&[req("at", "Instant")], "DateTime")],
    )
    .with_intro(INTRO_TO_LOCAL)
    .with_desc(DESC_TO_LOCAL)
    .with_example(EX_TO_LOCAL),
    df(
        RESOLVE,
        "resolve",
        &[ov(&[req("dt", "DateTime")], "Instant")],
    )
    .with_intro(INTRO_RESOLVE)
    .with_desc(DESC_RESOLVE)
    .with_example(EX_RESOLVE),
    df(
        CIVIL,
        "civil",
        &[ov(
            &[
                req("date", "Date"),
                req("time", "Time"),
                req("zone", "Zone"),
            ],
            "DateTime",
        )],
    )
    .with_intro(INTRO_CIVIL)
    .with_desc(DESC_CIVIL)
    .with_example(EX_CIVIL),
    df(
        WITH_ZONE,
        "withZone",
        &[ov(
            &[req("dt", "DateTime"), req("zone", "Zone")],
            "DateTime",
        )],
    )
    .with_intro(INTRO_WITH_ZONE)
    .with_desc(DESC_WITH_ZONE)
    .with_example(EX_WITH_ZONE),
    df(
        ADD,
        "add",
        &[ov(
            &[req("at", "Instant"), req("by", "Duration")],
            "Instant",
        )],
    )
    .with_intro(INTRO_ADD)
    .with_desc(DESC_ADD)
    .with_example(EX_ADD),
    df(
        SUBTRACT,
        "subtract",
        &[ov(
            &[req("at", "Instant"), req("by", "Duration")],
            "Instant",
        )],
    )
    .with_intro(INTRO_SUBTRACT)
    .with_desc(DESC_SUBTRACT)
    .with_example(EX_SUBTRACT),
    df(
        BETWEEN,
        "between",
        &[ov(
            &[req("start", "Instant"), req("finish", "Instant")],
            "Duration",
        )],
    )
    .with_intro(INTRO_BETWEEN)
    .with_desc(DESC_BETWEEN)
    .with_example(EX_BETWEEN),
    df(
        ADD_DAYS,
        "addDays",
        &[ov(&[req("dt", "DateTime"), req("days", I)], "DateTime")],
    )
    .with_intro(INTRO_ADD_DAYS)
    .with_desc(DESC_ADD_DAYS)
    .with_example(EX_ADD_DAYS),
    df(
        ADD_MONTHS,
        "addMonths",
        &[ov(&[req("dt", "DateTime"), req("months", I)], "DateTime")],
    )
    .with_intro(INTRO_ADD_MONTHS)
    .with_desc(DESC_ADD_MONTHS)
    .with_example(EX_ADD_MONTHS),
    df(
        COMPARE,
        "compare",
        &[ov(&[req("a", "Instant"), req("b", "Instant")], I)],
    )
    .with_intro(INTRO_COMPARE)
    .with_desc(DESC_COMPARE)
    .with_example(EX_COMPARE),
    df(
        IS_BEFORE,
        "isBefore",
        &[ov(&[req("a", "Instant"), req("b", "Instant")], "Boolean")],
    )
    .with_intro(INTRO_IS_BEFORE)
    .with_desc(DESC_IS_BEFORE)
    .with_example(EX_IS_BEFORE),
    df(
        IS_AFTER,
        "isAfter",
        &[ov(&[req("a", "Instant"), req("b", "Instant")], "Boolean")],
    )
    .with_intro(INTRO_IS_AFTER)
    .with_desc(DESC_IS_AFTER)
    .with_example(EX_IS_AFTER),
    df(
        EQUALS,
        "equals",
        &[ov(&[req("a", "Instant"), req("b", "Instant")], "Boolean")],
    )
    .with_intro(INTRO_EQUALS)
    .with_desc(DESC_EQUALS)
    .with_example(EX_EQUALS),
    df(NEGATE, "negate", &[ov(&[req("d", "Duration")], "Duration")])
        .with_intro(INTRO_NEGATE)
        .with_desc(DESC_NEGATE)
        .with_example(EX_NEGATE),
    df(
        PLUS,
        "plus",
        &[ov(
            &[req("a", "Duration"), req("b", "Duration")],
            "Duration",
        )],
    )
    .with_intro(INTRO_PLUS)
    .with_desc(DESC_PLUS)
    .with_example(EX_PLUS),
    df(
        MINUS,
        "minus",
        &[ov(
            &[req("a", "Duration"), req("b", "Duration")],
            "Duration",
        )],
    )
    .with_intro(INTRO_MINUS)
    .with_desc(DESC_MINUS)
    .with_example(EX_MINUS),
    df(
        WEEKDAY,
        "weekday",
        &[ov(&[req("dt", "DateTime")], "Weekday")],
    )
    .with_intro(INTRO_WEEKDAY)
    .with_desc(DESC_WEEKDAY)
    .with_example(EX_WEEKDAY),
    df(DAY_OF_YEAR, "dayOfYear", &[ov(&[req("dt", "DateTime")], I)])
        .with_intro(INTRO_DAY_OF_YEAR)
        .with_desc(DESC_DAY_OF_YEAR)
        .with_example(EX_DAY_OF_YEAR),
    df(
        IS_LEAP_YEAR,
        "isLeapYear",
        &[ov(&[req("year", I)], "Boolean")],
    )
    .with_intro(INTRO_IS_LEAP_YEAR)
    .with_desc(DESC_IS_LEAP_YEAR)
    .with_example(EX_IS_LEAP_YEAR),
    df(
        DAYS_IN_MONTH,
        "daysInMonth",
        &[ov(&[req("year", I), req("month", I)], I)],
    )
    .with_intro(INTRO_DAYS_IN_MONTH)
    .with_desc(DESC_DAYS_IN_MONTH)
    .with_example(EX_DAYS_IN_MONTH),
    df(
        START_OF_DAY,
        "startOfDay",
        &[ov(&[req("dt", "DateTime")], "DateTime")],
    )
    .with_intro(INTRO_START_OF_DAY)
    .with_desc(DESC_START_OF_DAY)
    .with_example(EX_START_OF_DAY),
    df(TO_MILLIS, "toMillis", &[ov(&[req("at", "Instant")], I)])
        .with_intro(INTRO_TO_MILLIS)
        .with_desc(DESC_TO_MILLIS)
        .with_example(EX_TO_MILLIS),
    df(TO_NANOS, "toNanos", &[ov(&[req("at", "Instant")], I)])
        .with_intro(INTRO_TO_NANOS)
        .with_desc(DESC_TO_NANOS)
        .with_example(EX_TO_NANOS),
    df(
        FROM_MILLIS,
        "fromMillis",
        &[ov(&[req("millis", I)], "Instant")],
    )
    .with_intro(INTRO_FROM_MILLIS)
    .with_desc(DESC_FROM_MILLIS)
    .with_example(EX_FROM_MILLIS),
    df(
        FORMAT,
        "format",
        &[ov(
            &[req("dt", "DateTime"), req("pattern", "String")],
            "String",
        )],
    )
    .with_intro(INTRO_FORMAT)
    .with_desc(DESC_FORMAT)
    .with_example(EX_FORMAT),
    df(
        PARSE,
        "parse",
        &[ov(
            &[
                req("value", "String"),
                req("pattern", "String"),
                optn("zone", "Zone"),
            ],
            "DateTime",
        )],
    )
    .with_intro(INTRO_PARSE)
    .with_desc(DESC_PARSE)
    .with_example(EX_PARSE),
    df(TO_ISO, "toIso", &[ov(&[req("dt", "DateTime")], "String")])
        .with_intro(INTRO_TO_ISO)
        .with_desc(DESC_TO_ISO)
        .with_example(EX_TO_ISO),
    df(
        PARSE_ISO,
        "parseIso",
        &[ov(&[req("value", "String")], "DateTime")],
    )
    .with_intro(INTRO_PARSE_ISO)
    .with_desc(DESC_PARSE_ISO)
    .with_example(EX_PARSE_ISO),
    df(
        FORMAT_DURATION,
        "formatDuration",
        &[ov(&[req("d", "Duration")], "String")],
    )
    .with_intro(INTRO_FORMAT_DURATION)
    .with_desc(DESC_FORMAT_DURATION)
    .with_example(EX_FORMAT_DURATION),
    df(NOW_NANOS, "nowNanos", &[ov(&[], I)])
        .with_intro(INTRO_NOW_NANOS)
        .with_desc(DESC_NOW_NANOS)
        .with_example(EX_NOW_NANOS),
    df(MONOTONIC_NANOS, "monotonicNanos", &[ov(&[], I)])
        .with_intro(INTRO_MONOTONIC_NANOS)
        .with_desc(DESC_MONOTONIC_NANOS)
        .with_example(EX_MONOTONIC_NANOS),
    df(
        LOCAL_OFFSET,
        "localOffset",
        &[ov(&[req("epochSeconds", I)], I)],
    )
    .with_intro(INTRO_LOCAL_OFFSET)
    .with_desc(DESC_LOCAL_OFFSET)
    .with_example(EX_LOCAL_OFFSET),
];

const DATETIME_TYPES: &[BuiltinType] = &[
    BuiltinType {
        name: "Instant",
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: "Duration",
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: "Date",
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: "Time",
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: "Zone",
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: "DateTime",
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: "ZoneKind",
        kind: TypeKind::Enum,
        fields: &[],
    },
    BuiltinType {
        name: "Weekday",
        kind: TypeKind::Enum,
        fields: &[],
    },
    BuiltinType {
        name: "Month",
        kind: TypeKind::Enum,
        fields: &[],
    },
];

/// Argument-dependent resolution for datetime: `resolve_call` argument validation
/// and the arity-keyed `__datetime_*{argc}` implementation selection. Both
/// delegate to the retained `dispatch_*` helpers (`implementation_name` reads only
/// the argument COUNT, so the resolver forwards `arg_types.len()`).
struct DatetimeResolver;
impl BuiltinResolver for DatetimeResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        dispatch_resolve(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }

    fn implementation_name(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        dispatch_implementation_name(name, arg_types.len())
    }
}
static DATETIME_RESOLVER: DatetimeResolver = DatetimeResolver;

const MODULE_INTRO: &str =
    r#"Instants, civil dates and times, durations, zones, formatting, and parsing"#;
const MODULE_DESC: &str = r#"The `datetime` package models time around a single source of truth: an `Instant`,
an absolute point on the UTC timeline (Unix epoch, leap-second-free) carrying
whole seconds and a nanosecond field in the range `0 .. 999_999_999`. Everything
civil — `Date`, `Time`, and `DateTime` — is a projection of an instant through a
`Zone`, and every projection records the resolved UTC offset, so a `DateTime`
always knows its offset and round-trips back to its `Instant` without
re-consulting the zone. `datetime` is a built-in package: `IMPORT datetime` needs
no manifest dependency.

All public types are flat, copyable value records and enums — `Instant`,
`Duration`, `Date`, `Time`, `Zone`, `DateTime`, and the enums `ZoneKind`,
`Weekday`, and `Month`. There are no resources and no hidden global state, and the
types are referenced bare (`Instant`, `Date`, …), not package-qualified. Calendar
arithmetic is pure integer math (Howard Hinnant's civil ↔ epoch-day conversions)
and produces identical results on every target. Only three operations touch the
host: the wall clock (`now`), a monotonic counter (`monotonic`), and the local
zone's DST-correct offset (`local`).

Zones come in three kinds. `datetime::utc()` is fixed at offset 0;
`datetime::fixedOffset(...)` builds a constant offset rendered as `+HH:MM`; and
`datetime::local()` resolves the host's zone per-instant, so it is DST-correct at
the moment it projects. Named IANA zones are not supported in this version.
`Instant.seconds` spans the full 64-bit `Integer`, so civil dates reach far beyond
any practical need; `datetime::now()` is additionally bounded by its intrinsic
(nanoseconds since the epoch), valid through year 2262. There are no leap seconds:
every day is 86400 seconds, the POSIX convention.

Projection is the primary "to civil" operation: `inZone` maps an instant into a
zone, `toUtc` and `toLocal` are shorthands, and `resolve` maps a civil `DateTime`
back to its `Instant`. Arithmetic operates on instants and durations (`add`,
`subtract`, `between`, `plus`, `minus`, `negate`), on calendar days (`addDays`,
DST-aware) and months (`addMonths`, clamping day-of-month). Formatting and parsing
share a pattern mini-language: a pattern is literal text with token runs, where a
run of the same letter is one token whose length selects width or style, and
literal letters are wrapped in single quotes. `format` renders a `DateTime`,
`parse` reads one back, and `toIso`/`parseIso` handle RFC 3339 / ISO 8601 with a
required offset."#;

pub(crate) static DATETIME: BuiltinModule = BuiltinModule {
    name: "datetime",
    doc_intro: MODULE_INTRO,
    doc_desc: MODULE_DESC,
    functions: DATETIME_FUNCTIONS,
    types: DATETIME_TYPES,
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: Some(&DATETIME_RESOLVER),
};

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

/// The public copyable record/enum types defined in `datetime_package.mfb`.
/// Referenced bare (`Instant`, `DateTime`, …) like every other builtin type.
pub(crate) fn is_builtin_type(name: &str) -> bool {
    DATETIME.types.iter().any(|ty| ty.name == name)
}

pub(crate) fn is_datetime_call(name: &str) -> bool {
    DefaultResolver::contains(&DATETIME, name)
}

/// The expected-argument phrasing for a `datetime::` argument-mismatch diagnostic.
/// Kept hand-authored (plan-72-BB): the optional-argument `[...]` brackets
/// (`time`'s `"Integer, Integer[, Integer[, Integer]]"`, `parse`'s
/// `"String, String[, Zone]"`) and the range prose (`"1 to 5 Integer"`) are shapes
/// the descriptor's per-position type join cannot reproduce, so `builtins::expected_arguments`
/// reads this before falling back to `DefaultResolver`.
pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    let text = match name {
        NOW | MONOTONIC | UTC | LOCAL | NOW_NANOS | MONOTONIC_NANOS => "()",
        INSTANT | DURATION => "1 to 5 Integer",
        DATE => "Integer, Integer, Integer",
        TIME => "Integer, Integer[, Integer[, Integer]]",
        FIXED_OFFSET => "Integer[, Integer]",
        OFFSET_AT => "Zone, Instant",
        IN_ZONE => "Instant, Zone",
        TO_UTC | TO_LOCAL => "Instant",
        RESOLVE | WEEKDAY | DAY_OF_YEAR | START_OF_DAY | TO_ISO => "DateTime",
        CIVIL => "Date, Time, Zone",
        WITH_ZONE => "DateTime, Zone",
        ADD | SUBTRACT => "Instant, Duration",
        BETWEEN | COMPARE | IS_BEFORE | IS_AFTER | EQUALS => "Instant, Instant",
        ADD_DAYS | ADD_MONTHS => "DateTime, Integer",
        NEGATE => "Duration",
        PLUS | MINUS => "Duration, Duration",
        IS_LEAP_YEAR | FROM_MILLIS | LOCAL_OFFSET => "Integer",
        DAYS_IN_MONTH => "Integer, Integer",
        TO_MILLIS | TO_NANOS => "Instant",
        FORMAT => "DateTime, String",
        PARSE => "String, String[, Zone]",
        PARSE_ISO => "String",
        FORMAT_DURATION => "Duration",
        _ => return None,
    };
    Some(text)
}

// `call_param_names`/`call_param_name_overloads` return `&'static` borrowed
// shapes the owned `DefaultResolver` cannot produce; they stay static, PINNED
// equal to `DATETIME` by the parity test (`DefaultResolver::param_names`/
// `param_name_overloads` derive them — None for multi-overload / single-overload
// respectively, exactly the bug-349 split). `expected_arguments`/`argument_types`
// use bespoke phrasing and stay static. BB removes them.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    let params: &'static [&'static [&'static str]] = match name {
        NOW | MONOTONIC | UTC | LOCAL => &[],
        // INSTANT/DURATION drop components off the FRONT as arity falls
        // (`__datetime_instant1(seconds)` … `__datetime_instant5(days, hours,
        // mins, seconds, nanos)`, `datetime_package.mfb:113-129, 137-153`), so
        // position 0 is `seconds` at arity 1 and `days` only at arity 5. A
        // merged per-position table is positionally false at arities 1-4 — it
        // bound `instant(days := 5)` to the 1-arg `seconds` slot, i.e. 5
        // seconds rather than 5 days (bug-349, same class as bug-94). They use
        // a per-overload table instead; see `call_param_name_overloads`.
        INSTANT | DURATION => return None,
        DATE => &[&["year"], &["month"], &["day"]],
        TIME => &[&["hour"], &["minute"], &["second"], &["nanos"]],
        // FIXED_OFFSET's two overloads disagree on position 0 (`offsetSeconds`
        // in the 1-arg form vs `hours` in the 2-arg form), so it cannot use a
        // merged per-position table — a merged alias would bind `hours := N`
        // to the 1-arg `offsetSeconds` slot (bug-94). It uses a per-overload
        // table instead; see `call_param_name_overloads`.
        FIXED_OFFSET => return None,
        OFFSET_AT => &[&["zone"], &["at"]],
        IN_ZONE => &[&["at"], &["zone"]],
        TO_UTC | TO_LOCAL => &[&["at"]],
        RESOLVE => &[&["dt"]],
        CIVIL => &[&["date"], &["time"], &["zone"]],
        WITH_ZONE => &[&["dt"], &["zone"]],
        ADD | SUBTRACT => &[&["at"], &["by"]],
        BETWEEN => &[&["start"], &["finish"]],
        ADD_DAYS => &[&["dt"], &["days"]],
        ADD_MONTHS => &[&["dt"], &["months"]],
        COMPARE | IS_BEFORE | IS_AFTER | EQUALS => &[&["a"], &["b"]],
        NEGATE => &[&["d"]],
        PLUS | MINUS => &[&["a"], &["b"]],
        WEEKDAY | DAY_OF_YEAR | START_OF_DAY => &[&["dt"]],
        IS_LEAP_YEAR => &[&["year"]],
        DAYS_IN_MONTH => &[&["year"], &["month"]],
        TO_MILLIS | TO_NANOS => &[&["at"]],
        FROM_MILLIS => &[&["millis"]],
        FORMAT => &[&["dt"], &["pattern"]],
        PARSE => &[&["value"], &["pattern"], &["zone"]],
        TO_ISO => &[&["dt"]],
        PARSE_ISO => &[&["value"]],
        FORMAT_DURATION => &[&["d"]],
        LOCAL_OFFSET => &[&["epochSeconds"]],
        NOW_NANOS | MONOTONIC_NANOS => &[],
        _ => return None,
    };
    Some(params)
}

/// Per-overload parameter names for datetime builtins whose overloads have
/// structurally different positional layouts (a named arg binds a different
/// index depending on which overload it selects). Each entry is one overload's
/// parameter names, in order. See `net::call_param_name_overloads` for the
/// pattern and bug-94 for the `fixedOffset` motivation.
pub(crate) fn call_param_name_overloads(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        FIXED_OFFSET => Some(&[&["offsetSeconds"], &["hours", "mins"]]),
        INSTANT | DURATION => Some(&[
            &["seconds"],
            &["seconds", "nanos"],
            &["mins", "seconds", "nanos"],
            &["hours", "mins", "seconds", "nanos"],
            &["days", "hours", "mins", "seconds", "nanos"],
        ]),
        _ => None,
    }
}

/// The argument-validating return-type resolution, invoked through the descriptor
/// resolver by `resolve_call`. The component builders accept 1..=5 (or 1..=2)
/// `Integer` args; the others require exact typed signatures.
fn dispatch_resolve<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let all_integer = |types: &[String]| types.iter().all(|t| t == "Integer");
    let return_type: &str = match name {
        NOW if arg_types.is_empty() => "Instant",
        MONOTONIC if arg_types.is_empty() => "Duration",
        UTC | LOCAL if arg_types.is_empty() => "Zone",
        NOW_NANOS | MONOTONIC_NANOS if arg_types.is_empty() => "Integer",
        // Component builders: 1..=5 / 1..=2 Integer args (§5.1.1).
        INSTANT if (1..=5).contains(&arg_types.len()) && all_integer(arg_types) => "Instant",
        DURATION if (1..=5).contains(&arg_types.len()) && all_integer(arg_types) => "Duration",
        FIXED_OFFSET if (1..=2).contains(&arg_types.len()) && all_integer(arg_types) => "Zone",
        DATE if exact(arg_types, &["Integer", "Integer", "Integer"]) => "Date",
        TIME if (2..=4).contains(&arg_types.len()) && all_integer(arg_types) => "Time",
        OFFSET_AT if exact(arg_types, &["Zone", "Instant"]) => "Integer",
        IN_ZONE if exact(arg_types, &["Instant", "Zone"]) => "DateTime",
        TO_UTC | TO_LOCAL if exact(arg_types, &["Instant"]) => "DateTime",
        RESOLVE if exact(arg_types, &["DateTime"]) => "Instant",
        CIVIL if exact(arg_types, &["Date", "Time", "Zone"]) => "DateTime",
        WITH_ZONE if exact(arg_types, &["DateTime", "Zone"]) => "DateTime",
        ADD | SUBTRACT if exact(arg_types, &["Instant", "Duration"]) => "Instant",
        BETWEEN if exact(arg_types, &["Instant", "Instant"]) => "Duration",
        ADD_DAYS if exact(arg_types, &["DateTime", "Integer"]) => "DateTime",
        ADD_MONTHS if exact(arg_types, &["DateTime", "Integer"]) => "DateTime",
        COMPARE if exact(arg_types, &["Instant", "Instant"]) => "Integer",
        IS_BEFORE | IS_AFTER | EQUALS if exact(arg_types, &["Instant", "Instant"]) => "Boolean",
        NEGATE if exact(arg_types, &["Duration"]) => "Duration",
        PLUS | MINUS if exact(arg_types, &["Duration", "Duration"]) => "Duration",
        WEEKDAY if exact(arg_types, &["DateTime"]) => "Weekday",
        DAY_OF_YEAR if exact(arg_types, &["DateTime"]) => "Integer",
        IS_LEAP_YEAR if exact(arg_types, &["Integer"]) => "Boolean",
        DAYS_IN_MONTH if exact(arg_types, &["Integer", "Integer"]) => "Integer",
        START_OF_DAY if exact(arg_types, &["DateTime"]) => "DateTime",
        TO_MILLIS | TO_NANOS if exact(arg_types, &["Instant"]) => "Integer",
        FROM_MILLIS if exact(arg_types, &["Integer"]) => "Instant",
        FORMAT if exact(arg_types, &["DateTime", "String"]) => "String",
        PARSE
            if exact(arg_types, &["String", "String"])
                || exact(arg_types, &["String", "String", "Zone"]) =>
        {
            "DateTime"
        }
        TO_ISO if exact(arg_types, &["DateTime"]) => "String",
        PARSE_ISO if exact(arg_types, &["String"]) => "DateTime",
        FORMAT_DURATION if exact(arg_types, &["Duration"]) => "String",
        LOCAL_OFFSET if exact(arg_types, &["Integer"]) => "Integer",
        _ => return None,
    };
    Some(ResolvedCall {
        return_type: Cow::Borrowed(return_type),
    })
}

/// The machine-readable positional argument-type signature (bug-340 A1): the
/// concrete per-parameter types IR lowering hands to `call_argument_expected_type`,
/// read directly instead of parsing the `expected_arguments` diagnostic string.
/// The variable-arity constructors (`instant`/`duration`), the no-argument clocks,
/// and the optional-tail members (`time`/`fixedOffset`/`parse`) have no single
/// fixed positional signature, so they return `None` (as the string parse's bail
/// conditions did before this package was wired in).
pub(crate) fn argument_types(name: &str) -> Option<&'static [&'static str]> {
    let types: &'static [&'static str] = match name {
        DATE => &["Integer", "Integer", "Integer"],
        OFFSET_AT => &["Zone", "Instant"],
        IN_ZONE => &["Instant", "Zone"],
        TO_UTC | TO_LOCAL => &["Instant"],
        RESOLVE | WEEKDAY | DAY_OF_YEAR | START_OF_DAY | TO_ISO => &["DateTime"],
        CIVIL => &["Date", "Time", "Zone"],
        WITH_ZONE => &["DateTime", "Zone"],
        ADD | SUBTRACT => &["Instant", "Duration"],
        BETWEEN | COMPARE | IS_BEFORE | IS_AFTER | EQUALS => &["Instant", "Instant"],
        ADD_DAYS | ADD_MONTHS => &["DateTime", "Integer"],
        NEGATE => &["Duration"],
        PLUS | MINUS => &["Duration", "Duration"],
        IS_LEAP_YEAR | FROM_MILLIS | LOCAL_OFFSET => &["Integer"],
        DAYS_IN_MONTH => &["Integer", "Integer"],
        TO_MILLIS | TO_NANOS => &["Instant"],
        FORMAT => &["DateTime", "String"],
        PARSE_ISO => &["String"],
        FORMAT_DURATION => &["Duration"],
        _ => return None,
    };
    Some(types)
}

/// The internal `__datetime_*` implementation for a public call, given the
/// supplied argument count. Routes through the descriptor resolver (which reads
/// only the count). Returns `None` for the OS-seam intrinsics (runtime helpers).
pub(crate) fn implementation_name(name: &str, argc: usize) -> Option<String> {
    // The resolver hook takes argument TYPES, but datetime's selection depends
    // only on the COUNT, so pass a length-`argc` placeholder.
    let arg_types = vec![String::new(); argc];
    DATETIME
        .resolver?
        .implementation_name(&DATETIME, name, &arg_types)
}

/// The arity-keyed `__datetime_*{argc}` implementation selection, invoked through
/// the descriptor resolver by `implementation_name`.
fn dispatch_implementation_name(name: &str, argc: usize) -> Option<String> {
    let internal = match name {
        NOW_NANOS | MONOTONIC_NANOS | LOCAL_OFFSET => return None,
        INSTANT => format!("__datetime_instant{argc}"),
        DURATION => format!("__datetime_duration{argc}"),
        FIXED_OFFSET => format!("__datetime_fixedOffset{argc}"),
        PARSE => format!("__datetime_parse{argc}"),
        _ => format!("__datetime_{}", name.strip_prefix("datetime.")?),
    };
    Some(internal)
}

/// Default trailing arguments injected during IR lowering. Only `time` carries
/// trailing defaults (`second`, `nanos` default to 0); the overloaded
/// constructors return EMPTY so the supplied argument count selects the right
/// `.mfb` overload (§5.1.1). Returns a `&'static` borrowed slice, so it stays
/// static — PINNED equal to `time`'s optional parameters by the parity test
/// (`DefaultResolver::default_padding` derives the same slots).
pub(crate) fn default_argument_padding(
    name: &str,
    provided: usize,
) -> &'static [(&'static str, &'static str)] {
    const TIME_DEFAULTS: &[(&str, &str)] = &[("Integer", "0"), ("Integer", "0")];
    match name {
        TIME => &TIME_DEFAULTS[(provided.saturating_sub(2)).min(TIME_DEFAULTS.len())..],
        _ => &[],
    }
}

use crate::builtins::exact;
use crate::target::shared::runtime::{RuntimeHelper, RuntimeHelperAbi, RuntimeHelperSpec};

mod native;
pub(crate) use native::lower_datetime_helper;

// `datetime::` OS-seam intrinsics (plan-01-datetime.md §8.2). `nowNanos` /
// `monotonicNanos` take no arguments; `localOffset` takes the epoch-seconds
// instant in `x0`. All return an `Integer` in the standard result-value register
// with the OK tag set. `nowNanos` / `monotonicNanos` cannot fail; `localOffset`
// raises `ErrInvalidArgument` (ERR tag) for an instant `localtime_r` cannot
// represent (bug-42). These `RuntimeHelperSpec`s register in the shared runtime
// catalog (`target/shared/runtime/catalog.rs`), which imports them from here.
pub(crate) const DATETIME_NOW_NANOS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Datetime,
    call: "datetime.nowNanos",
    abi: RuntimeHelperAbi { returns: "Integer" },
};

pub(crate) const DATETIME_MONOTONIC_NANOS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Datetime,
    call: "datetime.monotonicNanos",
    abi: RuntimeHelperAbi { returns: "Integer" },
};

pub(crate) const DATETIME_LOCAL_OFFSET_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Datetime,
    call: "datetime.localOffset",
    abi: RuntimeHelperAbi { returns: "Integer" },
};

/// Whether `name` is one of datetime's three OS-seam runtime intrinsics — lowered
/// natively via [`lower_datetime_helper`], not through the source companion. The
/// shared runtime-call recognizer (`target/shared/runtime/mod.rs`) delegates here.
pub(crate) fn is_datetime_runtime_call(name: &str) -> bool {
    matches!(
        name,
        "datetime.nowNanos" | "datetime.monotonicNanos" | "datetime.localOffset"
    )
}

/// Parses the built-in `datetime` package source. Unlike the source packages that
/// inline member bodies (`Implementation::Mfb`), datetime's members are `Custom`
/// with an arity-keyed one-to-many mapping onto `__datetime_*<n>` bodies, so the
/// bodies stay in the `package.mfb` companion (parsed verbatim). Synthetic path
/// label preserved byte-for-byte from the pre-migration `package_source_glue!`.
pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        std::path::Path::new("<builtin-datetime>"),
        "builtins/datetime.mfb",
        include_str!("package.mfb"),
    )
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "datetime")
    })
}

pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    if !uses_package(ast) {
        return Ok(ast.clone());
    }
    let mut augmented = ast.clone();
    augmented.files.push(source_file()?);
    Ok(augmented)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(src: &str) -> crate::ast::AstProject {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse source");
        crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    #[test]
    fn builtin_types() {
        for t in [
            "Instant", "Duration", "Date", "Time", "Zone", "DateTime", "ZoneKind", "Weekday",
            "Month",
        ] {
            assert!(is_builtin_type(t), "{t}");
        }
        assert!(!is_builtin_type("Nope"));
        assert!(!is_builtin_type("Integer"));
    }

    #[test]
    fn is_call_recognizes_all_and_rejects_unknown() {
        for n in [
            NOW,
            MONOTONIC,
            INSTANT,
            DATE,
            TIME,
            DURATION,
            UTC,
            LOCAL,
            FIXED_OFFSET,
            OFFSET_AT,
            IN_ZONE,
            TO_UTC,
            TO_LOCAL,
            RESOLVE,
            CIVIL,
            WITH_ZONE,
            ADD,
            SUBTRACT,
            BETWEEN,
            ADD_DAYS,
            ADD_MONTHS,
            COMPARE,
            IS_BEFORE,
            IS_AFTER,
            EQUALS,
            NEGATE,
            PLUS,
            MINUS,
            WEEKDAY,
            DAY_OF_YEAR,
            IS_LEAP_YEAR,
            DAYS_IN_MONTH,
            START_OF_DAY,
            TO_MILLIS,
            TO_NANOS,
            FROM_MILLIS,
            FORMAT,
            PARSE,
            TO_ISO,
            PARSE_ISO,
            FORMAT_DURATION,
            NOW_NANOS,
            MONOTONIC_NANOS,
            LOCAL_OFFSET,
        ] {
            assert!(is_datetime_call(n), "{n}");
        }
        assert!(!is_datetime_call("datetime.nope"));
        assert!(!is_datetime_call("other.now"));
    }

    #[test]
    fn param_names_present_and_unknown_none() {
        assert_eq!(call_param_names(NOW), Some(&[][..] as &[&[&str]]));
        // INSTANT/DURATION have no merged per-position table either: their
        // overloads drop components off the FRONT, so position 0 is `seconds`
        // at arity 1 and `days` only at arity 5. The merged table this line
        // used to assert bound `instant(days := 5)` to the 1-arg `seconds`
        // slot — 5 seconds, not 5 days (bug-349, the sibling bug-94 missed).
        assert_eq!(call_param_names(INSTANT), None);
        assert_eq!(call_param_names(DURATION), None);
        assert_eq!(
            call_param_name_overloads(INSTANT),
            Some(
                &[
                    &["seconds"][..],
                    &["seconds", "nanos"][..],
                    &["mins", "seconds", "nanos"][..],
                    &["hours", "mins", "seconds", "nanos"][..],
                    &["days", "hours", "mins", "seconds", "nanos"][..],
                ][..]
            )
        );
        assert_eq!(call_param_names(DATE).unwrap().len(), 3);
        // FIXED_OFFSET has no merged per-position table (its overloads disagree
        // on position 0); it uses a per-overload table instead (bug-94).
        assert_eq!(call_param_names(FIXED_OFFSET), None);
        assert_eq!(
            call_param_name_overloads(FIXED_OFFSET),
            Some(&[&["offsetSeconds"][..], &["hours", "mins"][..]][..])
        );
        assert_eq!(call_param_names(NOW_NANOS), Some(&[][..] as &[&[&str]]));
        assert!(call_param_names("datetime.nope").is_none());
    }

    #[test]
    fn argument_types_machine_table() {
        // bug-340 A1: the concrete positional signatures IR lowering reads. The
        // no-argument clocks, the variadic constructors (`instant`/`duration`),
        // and the optional-tail members (`time`/`fixedOffset`/`parse`) have no
        // single fixed signature -> None.
        assert_eq!(
            argument_types(DATE),
            Some(&["Integer", "Integer", "Integer"][..])
        );
        assert_eq!(argument_types(OFFSET_AT), Some(&["Zone", "Instant"][..]));
        assert_eq!(argument_types(CIVIL), Some(&["Date", "Time", "Zone"][..]));
        assert_eq!(argument_types(FORMAT), Some(&["DateTime", "String"][..]));
        assert_eq!(argument_types(PARSE_ISO), Some(&["String"][..]));
        // No fixed positional signature:
        assert_eq!(argument_types(NOW), None); // no arguments
        assert_eq!(argument_types(INSTANT), None); // 1..=5 Integer (variadic)
        assert_eq!(argument_types(TIME), None); // optional tail
        assert_eq!(argument_types(PARSE), None); // optional Zone
        assert_eq!(argument_types("datetime.nope"), None);
    }

    #[test]
    fn implementation_name_mapping() {
        assert_eq!(
            implementation_name(INSTANT, 3),
            Some("__datetime_instant3".to_string())
        );
        assert_eq!(
            implementation_name(DURATION, 2),
            Some("__datetime_duration2".to_string())
        );
        assert_eq!(
            implementation_name(FIXED_OFFSET, 1),
            Some("__datetime_fixedOffset1".to_string())
        );
        assert_eq!(
            implementation_name(PARSE, 3),
            Some("__datetime_parse3".to_string())
        );
        assert_eq!(
            implementation_name(NOW, 0),
            Some("__datetime_now".to_string())
        );
        assert_eq!(
            implementation_name(FORMAT_DURATION, 1),
            Some("__datetime_formatDuration".to_string())
        );
        // OS-seam intrinsics stay as runtime helpers -> None
        assert_eq!(implementation_name(NOW_NANOS, 0), None);
        assert_eq!(implementation_name(MONOTONIC_NANOS, 0), None);
        assert_eq!(implementation_name(LOCAL_OFFSET, 1), None);
    }

    #[test]
    fn default_padding_time_only() {
        // TIME with 2 provided -> two defaults; 3 -> one; 4 -> none.
        assert_eq!(default_argument_padding(TIME, 2).len(), 2);
        assert_eq!(default_argument_padding(TIME, 3).len(), 1);
        assert_eq!(default_argument_padding(TIME, 4).len(), 0);
        assert_eq!(default_argument_padding(TIME, 5).len(), 0);
        assert_eq!(default_argument_padding(NOW, 0), &[]);
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    /// The parameter names of `FUNC <func>(...)` as written in `package.mfb` —
    /// the ground truth a param-name table must match.
    fn mfb_param_names(func: &str) -> Vec<String> {
        let source = include_str!("package.mfb");
        let prefix = format!("FUNC {func}(");
        let rest = source
            .lines()
            .find_map(|line| line.trim_start().strip_prefix(&prefix).map(str::to_string))
            .unwrap_or_else(|| panic!("`{func}` is not declared in datetime_package.mfb"));
        let close = rest.find(')').expect("parameter list closes");
        if rest[..close].trim().is_empty() {
            return Vec::new();
        }
        crate::builtins::split_top_level_commas(&rest[..close])
            .into_iter()
            .map(|param| {
                param
                    .split_whitespace()
                    .next()
                    .expect("a parameter has a name")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn arity_dispatched_param_tables_match_the_mfb_overloads() {
        // bug-349: a param-name table must be positionally TRUE against the
        // overload the call actually selects, not merely unambiguous.
        // `no_named_argument_alias_repeats_across_positions` only rules out an
        // alias appearing twice; INSTANT/DURATION passed it while binding
        // `instant(days := 5)` to the 1-arg `seconds` slot — 5 seconds, not 5
        // days — because their overloads drop components off the FRONT.
        //
        // The rule this asserts: for every arity-dispatched family, the names
        // declared for arity N must equal `__datetime_<name>{N}`'s actual
        // parameters. A merged table is only legal when the family is
        // leading-aligned (each shorter overload is a prefix of the longer).
        for (builtin, stem, arities) in [
            (INSTANT, "__datetime_instant", 1..=5),
            (DURATION, "__datetime_duration", 1..=5),
            (FIXED_OFFSET, "__datetime_fixedOffset", 1..=2),
            (PARSE, "__datetime_parse", 2..=3),
        ] {
            for argc in arities {
                let actual = mfb_param_names(&format!("{stem}{argc}"));
                assert_eq!(
                    actual.len(),
                    argc,
                    "`{stem}{argc}` should take {argc} parameters"
                );

                // Whichever table the builtin declares, resolve the names it
                // claims sit at positions 0..argc for this arity.
                let declared: Vec<String> = if let Some(overloads) =
                    call_param_name_overloads(builtin)
                {
                    let params = overloads
                        .iter()
                        .find(|params| params.len() == argc)
                        .unwrap_or_else(|| panic!("`{builtin}` declares no arity-{argc} overload"));
                    params.iter().map(|p| (*p).to_string()).collect()
                } else {
                    let merged = call_param_names(builtin)
                        .unwrap_or_else(|| panic!("`{builtin}` declares no param names at all"));
                    // A merged table names position i once, for every arity.
                    merged
                        .iter()
                        .take(argc)
                        .map(|group| group[0].to_string())
                        .collect()
                };

                assert_eq!(
                    declared, actual,
                    "`{builtin}` at arity {argc} names its parameters {declared:?}, but \
                     `{stem}{argc}` takes {actual:?} — a named argument would bind to the \
                     wrong slot (bug-349)"
                );
            }
        }
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT datetime\nSUB main\nEND SUB\n");
        assert!(uses_package(&ast));
        let augmented = augmented_project(&ast).expect("augment");
        assert_eq!(augmented.files.len(), ast.files.len() + 1);
    }

    #[test]
    fn augmented_project_noop_without_import() {
        let ast = project("SUB main\nEND SUB\n");
        assert!(!uses_package(&ast));
        assert_eq!(
            augmented_project(&ast).expect("a").files.len(),
            ast.files.len()
        );
    }

    /// The `const fn` descriptor constructors (`req`/`opt`/`optn`/`ov`/`df`) are
    /// only invoked in `const` context by `DATETIME_FUNCTIONS`, so they carry no
    /// runtime coverage. Call each at runtime and assert the fields it builds.
    #[test]
    fn const_constructors_build_expected_fields() {
        let r = req("year", "Integer");
        assert_eq!(r.name, "year");
        assert!(r.aliases.is_empty());
        assert_eq!(r.ty, ParameterType::Named("Integer"));
        assert_eq!(r.default, DefaultValue::None);

        // `opt` is default-PADDED (`time`'s trailing `second`/`nanos` -> "0").
        let o = opt("second", "Integer", "0");
        assert_eq!(o.name, "second");
        assert!(o.aliases.is_empty());
        assert_eq!(o.ty, ParameterType::Named("Integer"));
        assert_eq!(
            o.default,
            DefaultValue::Fill {
                type_name: "Integer",
                expr: "0"
            }
        );

        // `optn` widens arity but is NOT padded (`parse`'s trailing `zone`).
        let n = optn("zone", "Zone");
        assert_eq!(n.name, "zone");
        assert_eq!(n.ty, ParameterType::Named("Zone"));
        assert_eq!(n.default, DefaultValue::Optional);

        // `ov` takes a `&'static [Parameter]`; a runtime temporary would be E0716,
        // so the parameter slice is a `const`.
        const PARAMS: &[Parameter] = &[req("year", "Integer")];
        let overload = ov(PARAMS, "Date");
        assert_eq!(overload.return_type, ReturnType::Fixed("Date"));
        assert_eq!(overload.params.len(), 1);
        assert_eq!(overload.params[0].name, "year");

        // `df` takes a `&'static [BuiltinOverload]`; the overload slice is a `const`.
        const OVS: &[BuiltinOverload] = &[ov(&[req("year", "Integer")], "Date")];
        let f = df("datetime.date", "date", OVS);
        assert_eq!(f.name, "datetime.date");
        assert_eq!(f.doc_slug, "date");
        assert_eq!(f.overloads.len(), 1);
        use crate::codegen::registry::{BuiltinFlags, Implementation, Lowering};
        assert_eq!(f.implementation, Implementation::Custom);
        assert_eq!(f.lowering, Lowering::Helper);
        assert_eq!(f.flags, BuiltinFlags::default());
    }

    #[test]
    fn expected_arguments_every_arm() {
        // Zero-argument members render as "()".
        for n in [NOW, MONOTONIC, UTC, LOCAL, NOW_NANOS, MONOTONIC_NANOS] {
            assert_eq!(expected_arguments(n), Some("()"), "{n}");
        }
        assert_eq!(expected_arguments(INSTANT), Some("1 to 5 Integer"));
        assert_eq!(expected_arguments(DURATION), Some("1 to 5 Integer"));
        assert_eq!(expected_arguments(DATE), Some("Integer, Integer, Integer"));
        assert_eq!(
            expected_arguments(TIME),
            Some("Integer, Integer[, Integer[, Integer]]")
        );
        assert_eq!(expected_arguments(FIXED_OFFSET), Some("Integer[, Integer]"));
        assert_eq!(expected_arguments(OFFSET_AT), Some("Zone, Instant"));
        assert_eq!(expected_arguments(IN_ZONE), Some("Instant, Zone"));
        assert_eq!(expected_arguments(TO_UTC), Some("Instant"));
        assert_eq!(expected_arguments(TO_LOCAL), Some("Instant"));
        for n in [RESOLVE, WEEKDAY, DAY_OF_YEAR, START_OF_DAY, TO_ISO] {
            assert_eq!(expected_arguments(n), Some("DateTime"), "{n}");
        }
        assert_eq!(expected_arguments(CIVIL), Some("Date, Time, Zone"));
        assert_eq!(expected_arguments(WITH_ZONE), Some("DateTime, Zone"));
        assert_eq!(expected_arguments(ADD), Some("Instant, Duration"));
        assert_eq!(expected_arguments(SUBTRACT), Some("Instant, Duration"));
        for n in [BETWEEN, COMPARE, IS_BEFORE, IS_AFTER, EQUALS] {
            assert_eq!(expected_arguments(n), Some("Instant, Instant"), "{n}");
        }
        assert_eq!(expected_arguments(ADD_DAYS), Some("DateTime, Integer"));
        assert_eq!(expected_arguments(ADD_MONTHS), Some("DateTime, Integer"));
        assert_eq!(expected_arguments(NEGATE), Some("Duration"));
        assert_eq!(expected_arguments(PLUS), Some("Duration, Duration"));
        assert_eq!(expected_arguments(MINUS), Some("Duration, Duration"));
        for n in [IS_LEAP_YEAR, FROM_MILLIS, LOCAL_OFFSET] {
            assert_eq!(expected_arguments(n), Some("Integer"), "{n}");
        }
        assert_eq!(expected_arguments(DAYS_IN_MONTH), Some("Integer, Integer"));
        assert_eq!(expected_arguments(TO_MILLIS), Some("Instant"));
        assert_eq!(expected_arguments(TO_NANOS), Some("Instant"));
        assert_eq!(expected_arguments(FORMAT), Some("DateTime, String"));
        assert_eq!(expected_arguments(PARSE), Some("String, String[, Zone]"));
        assert_eq!(expected_arguments(PARSE_ISO), Some("String"));
        assert_eq!(expected_arguments(FORMAT_DURATION), Some("Duration"));
        assert_eq!(expected_arguments("datetime.nope"), None);
    }

    fn resolved(name: &str, args: &[&str]) -> Option<String> {
        let types: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        dispatch_resolve(name, &types).map(|r| r.return_type.into_owned())
    }

    #[test]
    fn dispatch_resolve_every_branch() {
        assert_eq!(resolved(NOW, &[]), Some("Instant".into()));
        assert_eq!(resolved(MONOTONIC, &[]), Some("Duration".into()));
        assert_eq!(resolved(UTC, &[]), Some("Zone".into()));
        assert_eq!(resolved(LOCAL, &[]), Some("Zone".into()));
        assert_eq!(resolved(NOW_NANOS, &[]), Some("Integer".into()));
        assert_eq!(resolved(MONOTONIC_NANOS, &[]), Some("Integer".into()));
        // Component builders accept 1..=5 (fixedOffset 1..=2) Integer args.
        assert_eq!(resolved(INSTANT, &["Integer"]), Some("Instant".into()));
        assert_eq!(
            resolved(
                INSTANT,
                &["Integer", "Integer", "Integer", "Integer", "Integer"]
            ),
            Some("Instant".into())
        );
        assert_eq!(
            resolved(DURATION, &["Integer", "Integer"]),
            Some("Duration".into())
        );
        assert_eq!(resolved(FIXED_OFFSET, &["Integer"]), Some("Zone".into()));
        assert_eq!(
            resolved(FIXED_OFFSET, &["Integer", "Integer"]),
            Some("Zone".into())
        );
        assert_eq!(
            resolved(DATE, &["Integer", "Integer", "Integer"]),
            Some("Date".into())
        );
        assert_eq!(resolved(TIME, &["Integer", "Integer"]), Some("Time".into()));
        assert_eq!(
            resolved(TIME, &["Integer", "Integer", "Integer", "Integer"]),
            Some("Time".into())
        );
        assert_eq!(
            resolved(OFFSET_AT, &["Zone", "Instant"]),
            Some("Integer".into())
        );
        assert_eq!(
            resolved(IN_ZONE, &["Instant", "Zone"]),
            Some("DateTime".into())
        );
        assert_eq!(resolved(TO_UTC, &["Instant"]), Some("DateTime".into()));
        assert_eq!(resolved(TO_LOCAL, &["Instant"]), Some("DateTime".into()));
        assert_eq!(resolved(RESOLVE, &["DateTime"]), Some("Instant".into()));
        assert_eq!(
            resolved(CIVIL, &["Date", "Time", "Zone"]),
            Some("DateTime".into())
        );
        assert_eq!(
            resolved(WITH_ZONE, &["DateTime", "Zone"]),
            Some("DateTime".into())
        );
        assert_eq!(
            resolved(ADD, &["Instant", "Duration"]),
            Some("Instant".into())
        );
        assert_eq!(
            resolved(SUBTRACT, &["Instant", "Duration"]),
            Some("Instant".into())
        );
        assert_eq!(
            resolved(BETWEEN, &["Instant", "Instant"]),
            Some("Duration".into())
        );
        assert_eq!(
            resolved(ADD_DAYS, &["DateTime", "Integer"]),
            Some("DateTime".into())
        );
        assert_eq!(
            resolved(ADD_MONTHS, &["DateTime", "Integer"]),
            Some("DateTime".into())
        );
        assert_eq!(
            resolved(COMPARE, &["Instant", "Instant"]),
            Some("Integer".into())
        );
        assert_eq!(
            resolved(IS_BEFORE, &["Instant", "Instant"]),
            Some("Boolean".into())
        );
        assert_eq!(
            resolved(IS_AFTER, &["Instant", "Instant"]),
            Some("Boolean".into())
        );
        assert_eq!(
            resolved(EQUALS, &["Instant", "Instant"]),
            Some("Boolean".into())
        );
        assert_eq!(resolved(NEGATE, &["Duration"]), Some("Duration".into()));
        assert_eq!(
            resolved(PLUS, &["Duration", "Duration"]),
            Some("Duration".into())
        );
        assert_eq!(
            resolved(MINUS, &["Duration", "Duration"]),
            Some("Duration".into())
        );
        assert_eq!(resolved(WEEKDAY, &["DateTime"]), Some("Weekday".into()));
        assert_eq!(resolved(DAY_OF_YEAR, &["DateTime"]), Some("Integer".into()));
        assert_eq!(resolved(IS_LEAP_YEAR, &["Integer"]), Some("Boolean".into()));
        assert_eq!(
            resolved(DAYS_IN_MONTH, &["Integer", "Integer"]),
            Some("Integer".into())
        );
        assert_eq!(
            resolved(START_OF_DAY, &["DateTime"]),
            Some("DateTime".into())
        );
        assert_eq!(resolved(TO_MILLIS, &["Instant"]), Some("Integer".into()));
        assert_eq!(resolved(TO_NANOS, &["Instant"]), Some("Integer".into()));
        assert_eq!(resolved(FROM_MILLIS, &["Integer"]), Some("Instant".into()));
        assert_eq!(
            resolved(FORMAT, &["DateTime", "String"]),
            Some("String".into())
        );
        assert_eq!(
            resolved(PARSE, &["String", "String"]),
            Some("DateTime".into())
        );
        assert_eq!(
            resolved(PARSE, &["String", "String", "Zone"]),
            Some("DateTime".into())
        );
        assert_eq!(resolved(TO_ISO, &["DateTime"]), Some("String".into()));
        assert_eq!(resolved(PARSE_ISO, &["String"]), Some("DateTime".into()));
        assert_eq!(
            resolved(FORMAT_DURATION, &["Duration"]),
            Some("String".into())
        );
        assert_eq!(resolved(LOCAL_OFFSET, &["Integer"]), Some("Integer".into()));
        // Mismatched arity / types / unknown name -> None.
        assert_eq!(resolved(DATE, &["Integer", "Integer"]), None);
        assert_eq!(resolved(INSTANT, &["String"]), None);
        assert_eq!(resolved(OFFSET_AT, &["Instant", "Zone"]), None);
        assert_eq!(resolved("datetime.nope", &[]), None);
    }

    #[test]
    fn argument_types_remaining_arms() {
        assert_eq!(argument_types(IN_ZONE), Some(&["Instant", "Zone"][..]));
        for n in [TO_UTC, TO_LOCAL, TO_MILLIS, TO_NANOS] {
            assert_eq!(argument_types(n), Some(&["Instant"][..]), "{n}");
        }
        for n in [RESOLVE, WEEKDAY, DAY_OF_YEAR, START_OF_DAY, TO_ISO] {
            assert_eq!(argument_types(n), Some(&["DateTime"][..]), "{n}");
        }
        assert_eq!(argument_types(WITH_ZONE), Some(&["DateTime", "Zone"][..]));
        for n in [ADD, SUBTRACT] {
            assert_eq!(argument_types(n), Some(&["Instant", "Duration"][..]), "{n}");
        }
        for n in [BETWEEN, COMPARE, IS_BEFORE, IS_AFTER, EQUALS] {
            assert_eq!(argument_types(n), Some(&["Instant", "Instant"][..]), "{n}");
        }
        for n in [ADD_DAYS, ADD_MONTHS] {
            assert_eq!(argument_types(n), Some(&["DateTime", "Integer"][..]), "{n}");
        }
        assert_eq!(argument_types(NEGATE), Some(&["Duration"][..]));
        assert_eq!(argument_types(FORMAT_DURATION), Some(&["Duration"][..]));
        for n in [PLUS, MINUS] {
            assert_eq!(
                argument_types(n),
                Some(&["Duration", "Duration"][..]),
                "{n}"
            );
        }
        for n in [IS_LEAP_YEAR, FROM_MILLIS, LOCAL_OFFSET] {
            assert_eq!(argument_types(n), Some(&["Integer"][..]), "{n}");
        }
        assert_eq!(
            argument_types(DAYS_IN_MONTH),
            Some(&["Integer", "Integer"][..])
        );
    }

    #[test]
    fn implementation_name_routes_and_no_arg_func_has_no_params() {
        // Non-intrinsic call routes through the descriptor resolver.
        assert_eq!(
            implementation_name(NOW, 0),
            Some("__datetime_now".to_string())
        );
        // OS-seam intrinsic -> runtime helper, so None.
        assert_eq!(implementation_name(LOCAL_OFFSET, 1), None);
    }
}
