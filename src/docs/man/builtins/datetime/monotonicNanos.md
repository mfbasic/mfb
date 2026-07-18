# monotonicNanos

The raw monotonic-clock reading as a whole nanosecond count.

## Synopsis

```
datetime::monotonicNanos() AS Integer
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

`datetime::monotonicNanos` reads the host's monotonic clock and returns the
elapsed time, in whole nanoseconds, from an arbitrary fixed origin chosen by the
operating system. It is the low-level OS-seam intrinsic that backs
`datetime::monotonic`: where `monotonic` packages the reading into a `Duration`,
`monotonicNanos` returns the same value as a single raw `Integer` count of
nanoseconds. [[src/builtins/datetime.rs:call_return_type_name]]

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
constructing a `Duration`. [[src/target/shared/code/datetime.rs:lower_datetime_helper]]

`monotonicNanos` is **not pure**: two calls may return different values, and the
values depend on host clock state. It takes no arguments, reads clock state only,
and has no side effects. The reading always succeeds — the intrinsic returns an
`Integer` in the result register with the OK tag set and never raises an error.
[[src/target/shared/code/datetime.rs:lower_datetime_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `datetime::monotonicNanos` takes no arguments. [[src/builtins/datetime.rs:arity]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | Whole nanoseconds elapsed from the OS-chosen monotonic origin to the moment of the call. The value is non-decreasing across calls within a single process run. Only differences between two readings are meaningful; a single reading has no fixed reference point. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Measure the elapsed time around a block of work in nanoseconds:

```
IMPORT datetime

LET t0 AS Integer = datetime::monotonicNanos()
' ... work ...
LET elapsedNanos AS Integer = datetime::monotonicNanos() - t0
```

Convert the measured interval to whole milliseconds:

```
IMPORT datetime

LET t0 AS Integer = datetime::monotonicNanos()
' ... work ...
LET elapsedMs AS Integer = (datetime::monotonicNanos() - t0) / 1000000
```

## See also

- `mfb man datetime monotonic`
- `mfb man datetime nowNanos`
- `mfb man datetime now`
- `mfb man datetime minus`
