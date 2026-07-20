# between

The signed `Duration` span between two instants.

## Synopsis

```
datetime::between(start AS Instant, finish AS Instant) AS Duration
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

`datetime::between` returns the signed `Duration` `finish - start`: the length of
elapsed time you would add to `start` to reach `finish`. The span is positive when
`finish` is later than `start`, negative when `finish` is earlier, and zero when
the two instants are equal. Because the result is a `Duration` it carries no anchor
on the timeline — it names a length, not a point. [[src/builtins/datetime.rs:BETWEEN]]

The span is computed by subtracting the two `Instant`s field by field
(`finish.seconds - start.seconds` and `finish.nanos - start.nanos`) and then
normalizing the pair so the stored `nanos` lands in `0 .. 999_999_999` and any
borrow is carried into the `seconds` field. A negative nanosecond difference
borrows a whole second during normalization, so the `seconds` field of the result
is the floored whole-second component of the true difference and the `nanos` field
is the non-negative sub-second remainder.
[[src/builtins/datetime_package.mfb:__datetime_between]]
[[src/builtins/datetime_package.mfb:__datetime_normDuration]]

Both instants are points on the same Unix-epoch, leap-second-free UTC timeline, so
the span is independent of any time zone; resolve a `DateTime` to an `Instant` with
`datetime::resolve` before measuring. `between` is pure: the same two instants
always yield the same `Duration`, and it has no side effects. The subtraction and
the normalizing carry are ordinary signed `Integer` arithmetic, so two instants far
enough apart that their second difference falls outside the signed `Integer` range
overflow and trap. Render the result with `datetime::formatDuration`, and combine or
apply spans with `datetime::plus`, `datetime::minus`, `datetime::negate`,
`datetime::add`, and `datetime::subtract`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `start` | `Instant` | The earlier reference point, subtracted from `finish`. A point on the UTC timeline whose `seconds` field is whole seconds since `1970-01-01T00:00:00Z` (possibly negative before the epoch) and whose `nanos` field is the sub-second remainder. [[src/builtins/datetime.rs:BETWEEN]] |
| `finish` | `Instant` | The later reference point, from which `start` is subtracted. A point on the UTC timeline. When `finish` precedes `start` the returned span is negative. |

## Return value

| Type | Description |
| --- | --- |
| `Duration` | The signed span `finish - start`. Its `seconds` field holds the normalized whole-second component (negative when `finish` precedes `start`) and its `nanos` field holds the sub-second remainder in `0 .. 999_999_999`. Equal instants yield a zero `Duration`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | Subtracting the `seconds` fields, or carrying the normalized nanoseconds into the `seconds` field, produces a value outside the signed `Integer` range. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Measure the span between two instants and render it:

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
```

## See also

- `mfb man datetime add`
- `mfb man datetime subtract`
- `mfb man datetime compare`
- `mfb man datetime duration`
- `mfb man datetime formatDuration`
