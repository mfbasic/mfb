# duration

Build a `Duration` span from seconds, nanoseconds, or larger time components.

## Synopsis

```
datetime::duration(seconds AS Integer) AS Duration
datetime::duration(seconds AS Integer, nanos AS Integer) AS Duration
datetime::duration(mins AS Integer, seconds AS Integer, nanos AS Integer) AS Duration
datetime::duration(hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Duration
datetime::duration(days AS Integer, hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Duration
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

`datetime::duration` builds a signed `Duration`, a span of elapsed time with no
anchor on any timeline. The result carries a whole-second count in its `seconds`
field and a sub-second remainder in its `nanos` field, normalized into the range
`0 .. 999_999_999`. A `Duration` measures a length of time rather than a point in
time; to name a point on the UTC timeline use `datetime::instant` instead.

`duration` is overloaded by argument count, with five disjoint forms selected by
the number of `Integer` arguments (one through five).
[[src/builtins/datetime.rs:resolve_call]] The one- and two-argument forms take
whole seconds and, optionally, a nanosecond adjustment. The three-, four-, and
five-argument forms are component builders that fold larger units down into a
single second count: the three-argument form computes `mins*60 + seconds`, the
four-argument form adds `hours*3600`, and the five-argument form adds
`days*86400`, in every case adding the trailing `nanos` last.
[[src/builtins/datetime_package.mfb:__datetime_duration5]]

Whichever form is used (except the one-argument form), the supplied seconds and
nanos are normalized: any whole seconds embedded in `nanos` are carried into the
`seconds` field, and a negative `nanos` value borrows a second so the stored
`nanos` always lands in `0 .. 999_999_999`.
[[src/builtins/datetime_package.mfb:__datetime_normDuration]] Every numeric
argument may be negative, which yields a negative span pointing backward in time.
The one-argument form performs no normalization because its `nanos` is fixed at
zero. [[src/builtins/datetime_package.mfb:__datetime_duration1]]

`duration` is overloaded, so every parameter of the form you call must be supplied
explicitly; the component forms carry no defaults.
[[src/builtins/datetime.rs:default_argument_padding]] The folding and
normalization are ordinary signed `Integer` arithmetic, so a sufficiently large
day, hour, minute, or second magnitude can overflow the `Integer` range and trap.
Combine durations with `datetime::plus`, `datetime::minus`, and `datetime::negate`;
apply one to an `Instant` with `datetime::add` or `datetime::subtract`. `duration`
is pure: the same arguments always yield the same `Duration`, and it has no side
effects.

## Overloads

**`datetime::duration(seconds AS Integer) AS Duration`**

A span of exactly `seconds` whole seconds, with a zero nanosecond field. No
normalization is performed. [[src/builtins/datetime_package.mfb:__datetime_duration1]]

**`datetime::duration(seconds AS Integer, nanos AS Integer) AS Duration`**

A span of `seconds` plus `nanos` nanoseconds, normalized so the stored `nanos`
lands in `0 .. 999_999_999` and any whole seconds carry into `seconds`.

**`datetime::duration(mins AS Integer, seconds AS Integer, nanos AS Integer) AS Duration`**

A span of `mins*60 + seconds` whole seconds, plus `nanos`, normalized.

**`datetime::duration(hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Duration`**

A span of `hours*3600 + mins*60 + seconds` whole seconds, plus `nanos`, normalized.

**`datetime::duration(days AS Integer, hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Duration`**

A span of `days*86400 + hours*3600 + mins*60 + seconds` whole seconds, plus
`nanos`, normalized.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `days` | `Integer` | Whole days contributing `days*86400` seconds. Present only in the five-argument form. May be negative. |
| `hours` | `Integer` | Whole hours contributing `hours*3600` seconds. Present in the four- and five-argument forms. May be negative. |
| `mins` | `Integer` | Whole minutes contributing `mins*60` seconds. Present in the three-, four-, and five-argument forms. May be negative. |
| `seconds` | `Integer` | Whole seconds. In the one- and two-argument forms this is the complete second count of the span; in the component forms it is the seconds contribution added to the folded larger units. May be negative. |
| `nanos` | `Integer` | A nanosecond adjustment added to the second count. Need not be in `0 .. 999_999_999`: any whole seconds it contains are carried into the `seconds` field and a negative value borrows a second during normalization. Absent only from the one-argument form, where it is fixed at zero. |

## Return value

| Type | Description |
| --- | --- |
| `Duration` | The signed span of the requested length. The `seconds` field holds the normalized whole-second count (which may be negative for a backward span) and the `nanos` field holds the sub-second remainder in `0 .. 999_999_999`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | Folding the components into a second count, or carrying the normalized nanoseconds into the `seconds` field, produces a value outside the signed `Integer` range. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Build a `Duration` from a whole-second span:

```
IMPORT datetime

LET d AS Duration = datetime::duration(90)
```

Build a `Duration` with a sub-second adjustment that normalizes into the `seconds`
field:

```
IMPORT datetime

LET d AS Duration = datetime::duration(10, 1_500_000_000)
```

Build a `Duration` from day, hour, minute, second, and nanosecond components:

```
IMPORT datetime

LET d AS Duration = datetime::duration(1, 2, 3, 4, 0)
```

A negative argument yields a backward span:

```
IMPORT datetime

LET d AS Duration = datetime::duration(-30)
```

## See also

- `mfb man datetime instant`
- `mfb man datetime plus`
- `mfb man datetime minus`
- `mfb man datetime negate`
- `mfb man datetime add`
- `mfb man datetime formatDuration`
