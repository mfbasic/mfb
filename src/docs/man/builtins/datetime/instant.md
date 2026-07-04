# instant

Build an `Instant` from seconds, nanoseconds, or larger time components.

## Synopsis

```
datetime::instant(seconds AS Integer) AS Instant
datetime::instant(seconds AS Integer, nanos AS Integer) AS Instant
datetime::instant(mins AS Integer, seconds AS Integer, nanos AS Integer) AS Instant
datetime::instant(hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Instant
datetime::instant(days AS Integer, hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Instant
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

`datetime::instant` builds an `Instant` on the UTC timeline (the Unix epoch,
without leap seconds) at a given offset after `1970-01-01T00:00:00Z`. The result
carries whole seconds since the epoch in its `seconds` field and a sub-second
remainder in its `nanos` field, normalized into the range `0 .. 999_999_999`.

`instant` is overloaded by argument count, with five disjoint forms selected by
the number of `Integer` arguments (one through five).
[[src/builtins/datetime.rs:resolve_call]] The one- and two-argument forms take
whole seconds and, optionally, a nanosecond adjustment. The three-, four-, and
five-argument forms are component builders that fold larger units down into a
single second count: the three-argument form computes `mins*60 + seconds`, the
four-argument form adds `hours*3600`, and the five-argument form adds
`days*86400`, in every case adding the trailing `nanos` last.
[[src/builtins/datetime_package.mfb:__datetime_instant5]]

Whichever form is used (except the one-argument form), the supplied seconds and
nanos are normalized: any whole seconds embedded in `nanos` are carried into the
`seconds` field, and a negative `nanos` value borrows a second so the stored
`nanos` always lands in `0 .. 999_999_999`.
[[src/builtins/datetime_package.mfb:__datetime_normInstant]] Every numeric
argument may be negative, which selects an instant before the epoch. The
one-argument form performs no normalization because its `nanos` is fixed at zero.
[[src/builtins/datetime_package.mfb:__datetime_instant1]]

`instant` is overloaded, so every parameter of the form you call must be supplied
explicitly; the component forms carry no defaults.
[[src/builtins/datetime.rs:default_argument_padding]] The folding and
normalization are ordinary signed `Integer` arithmetic, so a sufficiently large
day, hour, minute, or second magnitude can overflow the `Integer` range and trap.
To shift an existing `Instant` by a span rather than build one from scratch, use
`datetime::add` or `datetime::subtract` with a `Duration`. `instant` is pure: the
same arguments always yield the same `Instant`, and it has no side effects.

## Overloads

**`datetime::instant(seconds AS Integer) AS Instant`**

The `Instant` exactly `seconds` after the epoch, with a zero nanosecond field. No
normalization is performed. [[src/builtins/datetime_package.mfb:__datetime_instant1]]

**`datetime::instant(seconds AS Integer, nanos AS Integer) AS Instant`**

The `Instant` at `seconds` plus `nanos` nanoseconds, normalized so the stored
`nanos` lands in `0 .. 999_999_999` and any whole seconds carry into `seconds`.

**`datetime::instant(mins AS Integer, seconds AS Integer, nanos AS Integer) AS Instant`**

The `Instant` at `mins*60 + seconds` whole seconds, plus `nanos`, normalized.

**`datetime::instant(hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Instant`**

The `Instant` at `hours*3600 + mins*60 + seconds` whole seconds, plus `nanos`,
normalized.

**`datetime::instant(days AS Integer, hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Instant`**

The `Instant` at `days*86400 + hours*3600 + mins*60 + seconds` whole seconds, plus
`nanos`, normalized.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `days` | `Integer` | Whole days contributing `days*86400` seconds. Present only in the five-argument form. May be negative. |
| `hours` | `Integer` | Whole hours contributing `hours*3600` seconds. Present in the four- and five-argument forms. May be negative. |
| `mins` | `Integer` | Whole minutes contributing `mins*60` seconds. Present in the three-, four-, and five-argument forms. May be negative. |
| `seconds` | `Integer` | Whole seconds. In the one- and two-argument forms this is the complete second count since the epoch; in the component forms it is the seconds contribution added to the folded larger units. May be negative. |
| `nanos` | `Integer` | A nanosecond adjustment added to the second count. Need not be in `0 .. 999_999_999`: any whole seconds it contains are carried into the `seconds` field and a negative value borrows a second during normalization. Absent only from the one-argument form, where it is fixed at zero. |

## Return value

| Type | Description |
| --- | --- |
| `Instant` | The `Instant` at the requested offset from the Unix epoch. The `seconds` field holds the normalized whole-second count (which may be negative for instants before the epoch) and the `nanos` field holds the sub-second remainder in `0 .. 999_999_999`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | Folding the components into a second count, or carrying the normalized nanoseconds into the `seconds` field, produces a value outside the signed `Integer` range. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Build an `Instant` from a whole-second epoch offset:

```
IMPORT datetime

LET t AS Instant = datetime::instant(1_700_000_000)
```

Build an `Instant` with a sub-second adjustment that normalizes into the `seconds`
field:

```
IMPORT datetime

LET t AS Instant = datetime::instant(10, 1_500_000_000)
```

Build an `Instant` from day, hour, minute, second, and nanosecond components:

```
IMPORT datetime

LET t AS Instant = datetime::instant(1, 2, 3, 4, 0)
```

## See also

- `mfb man datetime duration`
- `mfb man datetime add`
- `mfb man datetime subtract`
- `mfb man datetime fromMillis`
