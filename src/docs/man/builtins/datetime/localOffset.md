# localOffset

The host's local UTC offset in seconds at a given epoch second.

## Synopsis

```
datetime::localOffset(epochSeconds AS Integer) AS Integer
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

`datetime::localOffset` returns the signed offset from UTC, in seconds, that the
host's configured local time zone applies at the absolute instant named by
`epochSeconds` — whole seconds since `1970-01-01T00:00:00Z` on the UTC timeline
(the Unix epoch, without leap seconds). A positive result places local civil
time ahead of UTC (east of the prime meridian); a negative result places it
behind UTC (west); zero means local time coincides with UTC at that instant.
[[src/builtins/datetime.rs:call_return_type_name]]

This is the OS seam through which the rest of the package learns the host's
wall-clock rules. The call lowers to a libc runtime helper that hands
`epochSeconds` to `localtime_r` and reports the resolved `tm_gmtoff` for that
moment, so the result is DST-correct: it returns the standard-time offset for
instants outside daylight saving and the shifted offset for instants within it.
Two calls with epoch seconds on opposite sides of a daylight-saving transition
can therefore return different values. The offset reflects whatever zone the host
is configured to use (for example via the `TZ` environment variable or the
system zone setting), so the same program can produce different results on
different hosts. [[src/target/shared/code/datetime.rs:lower_datetime_helper]]

Only the seconds value matters; there is no sub-second component. `localOffset`
is the low-level intrinsic that backs `datetime::offsetAt` for local zones and
`datetime::toLocal`; most code should prefer those higher-level functions, which
operate on `Instant` and `Zone` values rather than a raw epoch-seconds `Integer`.

`localOffset` is **not pure**: it reads the host's time-zone configuration, so
its result depends on host state. It has no side effects and reads no other
state.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `epochSeconds` | `Integer` | The absolute instant at which to evaluate the local offset, expressed as whole seconds since the Unix epoch (`1970-01-01T00:00:00Z`). The value selects the point on the timeline for which the host time zone is consulted; for a zone observing daylight saving it determines whether the standard or the daylight-saving offset is in force. [[src/builtins/datetime.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The signed offset from UTC, in seconds, that the host's local time zone applies at `epochSeconds`. Positive east of UTC, negative west, zero when the local zone coincides with UTC at that instant. The value is the same adjustment that converts an `Instant`'s seconds-since-epoch into local civil fields. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `epochSeconds` names an instant the host's C library cannot break down into calendar fields — its year overflows the platform `struct tm`'s `int` year (roughly `abs(epochSeconds)` beyond `6.7e16` seconds). `localtime_r` returns no result for such an instant, so no offset is defined. [[src/target/shared/code/datetime.rs:lower_datetime_helper]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Examples

The host's local offset for the current instant:

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
```

## See also

- `mfb man datetime offsetAt`
- `mfb man datetime toLocal`
- `mfb man datetime local`
- `mfb man datetime now`
