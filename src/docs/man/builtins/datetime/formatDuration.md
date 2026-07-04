# formatDuration

Render a `Duration` as a human-readable `[-][Nd ]HH:MM:SS.mmm` span.

## Synopsis

```
datetime::formatDuration(d AS Duration) AS String
```

## Package

datetime

## Imports

```
IMPORT datetime
```

`datetime` is a built-in package, so no manifest dependency is required.
[[src/builtins/datetime.rs:augmented_project]]

## Description

`datetime::formatDuration` renders the signed span `d` as a fixed-shape string of
the form `[-][Nd ]HH:MM:SS.mmm`. The hour, minute, and second fields are always
two digits and the millisecond field always three; the day field and its trailing
space appear only when the span is at least one whole day. A span of one day, two
hours, three minutes, four-and-a-half seconds renders as `1d 02:03:04.500`, while
ninety seconds renders as `00:01:30.000` and a zero span as `00:00:00.000`.
[[src/builtins/datetime_package.mfb:__datetime_formatDuration]]

The span is reduced to whole milliseconds before formatting: the value used is
`d.seconds * 1000 + d.nanos / 1000000`, so any sub-millisecond remainder in the
`nanos` field is truncated and does not appear in the output. A negative span is
rendered as its absolute magnitude prefixed with a single leading minus sign; the
hour, minute, second, and millisecond fields are taken from the absolute value and
never carry their own sign. The day count is the full number of whole days and is
not wrapped, so a multi-day span shows a multi-digit day field; the hour field is
the remaining whole hours modulo 24, the minute field the remaining minutes modulo
60, and the second field the remaining seconds modulo 60.
[[src/builtins/datetime_package.mfb:__datetime_formatDuration]]

`datetime::formatDuration` is pure: the same `Duration` always yields the same
string, and it has no side effects. Because the reduction to milliseconds is
ordinary signed `Integer` arithmetic, a span whose second count is large enough
that multiplying by 1000 (or negating the reduced total) leaves the signed
`Integer` range traps rather than formatting.
[[src/builtins/datetime_package.mfb:__datetime_formatDuration]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `d` | `Duration` | The signed span to render. Its `seconds` field supplies the whole-second magnitude and its `nanos` field the sub-second remainder, of which only whole milliseconds are kept. May be negative, in which case the output is prefixed with a minus sign. [[src/builtins/datetime.rs:FORMAT_DURATION]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The span formatted as `[-][Nd ]HH:MM:SS.mmm`. The day component and its trailing space are present only when the span is at least one whole day; the hour, minute, second, and millisecond fields are always present and zero-padded to two, two, two, and three digits respectively. A zero span returns `00:00:00.000`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | Reducing the span to whole milliseconds — multiplying the `seconds` field by 1000 and adding the millisecond remainder, or negating the reduced total of a negative span — produces a value outside the signed `Integer` range. [[src/builtins/datetime_package.mfb:__datetime_formatDuration]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Render a sub-day span:

```
IMPORT datetime

LET d AS Duration = datetime::duration(90)
PRINT datetime::formatDuration(d)        ' 00:01:30.000
```

Render a span that includes whole days:

```
IMPORT datetime

LET d AS Duration = datetime::duration(1, 2, 3, 4, 500_000_000)
PRINT datetime::formatDuration(d)        ' 1d 02:03:04.500
```

A negative span is prefixed with a minus sign:

```
IMPORT datetime

LET d AS Duration = datetime::duration(-30)
PRINT datetime::formatDuration(d)        ' -00:00:30.000
```

## See also

- `mfb man datetime duration`
- `mfb man datetime between`
- `mfb man datetime format`
- `mfb man datetime negate`
