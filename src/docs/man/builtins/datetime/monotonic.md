# monotonic

A monotonically non-decreasing clock reading for measuring elapsed time.

## Synopsis

```
datetime::monotonic() AS Duration
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

`datetime::monotonic` reads the host's monotonic clock and returns the elapsed
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
[[src/builtins/datetime_package.mfb:__datetime_monotonic]]

`monotonic` is **not pure**: two calls may return different spans, and the values
depend on host clock state. It takes no arguments, reads clock state only, and
has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `datetime::monotonic` takes no arguments. [[src/builtins/datetime.rs:arity]] |

## Return value

| Type | Description |
| --- | --- |
| `Duration` | The elapsed span from the OS-chosen monotonic origin to the moment of the call. The `seconds` field holds whole elapsed seconds and the `nanos` field holds the sub-second remainder in `0 .. 999_999_999`. Only differences between two readings are meaningful; a single reading has no fixed reference point. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Measure the elapsed time around a block of work:

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
```

## See also

- `mfb man datetime now`
- `mfb man datetime minus`
- `mfb man datetime formatDuration`
