# now

The current wall-clock instant on the UTC timeline.

## Synopsis

```
datetime::now() AS Instant
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

`datetime::now` reads the host's real-time clock and returns the `Instant` it
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
in `0 .. 999_999_999`. [[src/builtins/datetime_package.mfb:__datetime_now]]

`now` is bounded by its underlying intrinsic, which reports nanoseconds since
the epoch and is valid through roughly the year 2262. This is a limit on `now`,
not on `Instant`, whose `seconds` field spans the full `Integer` range.

`now` is one of the few `datetime` functions that is **not pure**: two calls may
return different instants, and a program's output depends on the host clock. For
reproducible logic, capture a single instant and derive everything else from it.
`now` takes no arguments, reads host clock state only, and has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `datetime::now` takes no arguments. [[src/builtins/datetime.rs:arity]] |

## Return value

| Type | Description |
| --- | --- |
| `Instant` | The current instant on the UTC timeline. The `seconds` field holds whole seconds since `1970-01-01T00:00:00Z` and the `nanos` field holds the sub-second remainder in `0 .. 999_999_999`. Two calls may return different instants depending on the host clock. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Capture the current instant:

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
```

## See also

- `mfb man datetime monotonic`
- `mfb man datetime toLocal`
- `mfb man datetime toUtc`
- `mfb man datetime inZone`
