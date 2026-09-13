# bug-611: `datetime` arithmetic members declare no errors, and which inputs raise is unmeasured

Last updated: 2026-09-13
Effort: small (<1h)
Severity: LOW
Class: Documentation (registry error declaration)

Status: Open
Regression Test: none yet — see Phase 1

Found while fixing bug-520 (S8). Every `datetime` descriptor declared `errors: vec![]`.
bug-520 probed and filled in the members that raise on bad arguments or out-of-range
host instants: `date`, `time`, `toIso(dt, digits)`, `parse`, `parseIso`, `fixedOffset`
(both), `localOffset`, `offsetAt`, `toLocal`, `inZone` and `civil`. The rest still
list nothing, and nobody has measured which of them can raise.

## Failing Reproduction

```
grep -l 'errors: vec!\[\]' src/codegen/builtins/datetime/func_*.rs
```

The members most likely to raise are the ones doing checked `Integer` arithmetic on
caller values, which fails `ErrOverflow` (`77050010`):

- `add`, `subtract`, `plus`, `minus`, `negate`, `between`
- `addDays`, `addMonths`, `startOfDay` (via `civil`: `ErrOverflow`, and for a Local
  zone `ErrInvalidArgument`)
- `toMillis`, `toNanos`, `fromMillis`, `instant`, `duration`

Each is a **guess** until probed. For example, `civil` with a UTC year of `3·10^14`
raises 77050010 (measured in bug-520), so `addDays` from such a value very likely
does too.

## Fix Design

For each member: read the body, probe the boundary inputs (Integer max/min, huge
day/month counts, a Local zone far from the epoch), and list exactly the codes seen
in its descriptor `errors`, the way bug-520's S8 table did. Descriptor `errors` feed
the `.ir` goldens; regenerate and inspect.

## Probe findings (2026-09-13)

Measured on macOS aarch64 with `target/release/mfb` at `9423d8d22`. Every call was
wrapped in `TRAP` and printed its code. It ran under the host zone and again under
`TZ=America/New_York`, and the output was identical (both exit 0). The raises are
pinned by `tests/rt-behavior/datetime/datetime-arith-errors-rt`, and the declarations by
`datetime::tests::members_declare_the_errors_they_raise`.

| Member (overload) | Raising input | Code |
| --- | --- | --- |
| `add` | `add(instant(max), duration(1))` | `ErrOverflow` |
| `subtract` | `subtract(instant(min), duration(1))` | `ErrOverflow` |
| `plus` / `minus` | `duration(max)` + 1 / `duration(min)` − 1 | `ErrOverflow` |
| `negate` | `negate(duration(min))` | `ErrOverflow` |
| `between` | `between(instant(min), instant(max))` | `ErrOverflow` |
| `instant` 2–5, `duration` 2–5 | `instant(max, 1000000000)`, `instant(max, 0, 0)`, … | `ErrOverflow` |
| `toMillis` / `toNanos` | `instant(max)` / `instant(10^10)` | `ErrOverflow` |
| `addDays` | `days = max` / Local zone, `days = 10^12` | `ErrOverflow` / `ErrInvalidArgument` |
| `addMonths` | `months = max` / Local zone, `months = 10^11` | `ErrOverflow` / `ErrInvalidArgument` |
| `startOfDay` | `toUtc(instant(max))` (the `±86400` probe in `resolveLocal`) / Local zone, year `3·10^9` | `ErrOverflow` / `ErrInvalidArgument` |
| `withZone` | `toUtc(instant(max))` into `fixedOffset(3600)` / into `local()` from `instant(10^17)` | `ErrOverflow` / `ErrInvalidArgument` |
| `format` | unknown token `"q"` / a record with offset `min` and pattern `ZZ` | `ErrInvalidFormat` / `ErrOverflow` |
| `formatDuration` | `duration(max)` (`seconds * 1000`) | `ErrOverflow` |
| `toIso` (both) | a record with offset `min` (`-s` in the offset label) | `ErrOverflow` |
| `dayOfYear`, `weekday`, `resolve` | a record with year `max` | `ErrOverflow` |

- **Only a directly built record raises in the last three rows and `format`/`toIso`.**
  `datetime::DateTime[datetime::Date[max, 1, 1], …, offset]` compiles. Values that come
  from the constructors did not raise there: `dayOfYear`, `weekday`, `resolve` and
  `toIso` of `toUtc(instant(max))` and `toUtc(instant(min))` all returned.
- **No raise at the extremes:** `fromMillis` (min, max), `toUtc` (min, max), `compare`,
  `equals`, `isBefore`, `isAfter`, `isLeapYear(min)`, `daysInMonth(min, 2)` and
  `daysInMonth(2026, 13)`, `now`, `nowNanos`, `monotonic`, `monotonicNanos`, `local`,
  `utc`. They keep `errors: vec![]`.
- **`format` was outside the doc's list.** `__datetime_formatToken` ends in
  `FAIL error(77050003, …)` for an unrecognised letter, so the page claimed no error
  for a documented failure.

## Phases

### Phase 1 — probe and declare

- [ ] Probe each member above and record its raising inputs and codes here.
- [ ] Fill in `errors` for every member that raises; leave `vec![]` only where the probe
      shows none.

Acceptance: `mfb man datetime <member>` shows an Errors table for every member that
raises, and nothing else changes.
Commit: —
