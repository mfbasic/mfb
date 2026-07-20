# time

Validate and build a time-of-day `Time` from hour, minute, second, and sub-second components.

## Synopsis

```
datetime::time(hour AS Integer, minute AS Integer, second AS Integer = 0, nanos AS Integer = 0) AS Time
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

`datetime::time` builds a `Time` of day from its `hour`, `minute`, `second`, and
sub-second (`nanos`) components. A `Time` names a position within a single
24-hour day and carries no calendar date and no zone; pair it with a `Date`
through `datetime::civil` to build a zoned `DateTime`.

The constructor validates each component against its civil range before
returning, and there is no normalization or wrap-around: an out-of-range
component is an error, not silently carried into the next unit. `hour` must be
in `0 .. 23`, where `0` is midnight and `23` is the final hour of the day.
`minute` and `second` must each be in `0 .. 59`; the model has no leap seconds,
so `60` is never a valid second. `nanos` is the sub-second remainder and must be
in `0 .. 999_999_999`. [[src/builtins/datetime_package.mfb:__datetime_time]]

`second` and `nanos` default to `0`, so a two-argument call names the top of a
minute and a three-argument call names the top of a second. Unlike
`datetime::instant` and `datetime::duration`, `time` is not overloaded but a
single signature with trailing defaults, so the defaults apply and you may omit
`second`, or both `second` and `nanos`. [[src/builtins/datetime.rs:default_argument_padding]]

`time` is pure: the same arguments always yield the same `Time`, and it has no
side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `hour` | `Integer` | The hour of the day. Must be in `0 .. 23`, where `0` is midnight and `23` is the last hour. Any value outside this range is an error. [[src/builtins/datetime_package.mfb:__datetime_time]] |
| `minute` | `Integer` | The minute of the hour. Must be in `0 .. 59`. Any value outside this range is an error. [[src/builtins/datetime_package.mfb:__datetime_time]] |
| `second` | `Integer` | The second of the minute. Must be in `0 .. 59`; there are no leap seconds, so `60` is rejected. Defaults to `0` when omitted. [[src/builtins/datetime_package.mfb:__datetime_time]] |
| `nanos` | `Integer` | The sub-second remainder in nanoseconds. Must be in `0 .. 999_999_999`. Defaults to `0` when omitted. [[src/builtins/datetime_package.mfb:__datetime_time]] |

## Return value

| Type | Description |
| --- | --- |
| `Time` | A `Time` holding the validated `hour`, `minute`, `second`, and `nanos`. Returned only when all four components fall within their civil ranges. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `hour` is outside `0 .. 23`, `minute` or `second` is outside `0 .. 59`, or `nanos` is outside `0 .. 999_999_999` (for example `datetime::time(24, 0)`). [[src/builtins/datetime_package.mfb:__datetime_time]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Examples

Construct a time at the top of a minute (`second` and `nanos` default to `0`):

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
```

## See also

- `mfb man datetime date`
- `mfb man datetime civil`
- `mfb man datetime instant`
- `mfb man datetime duration`
