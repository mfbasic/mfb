### 1. Round-trip claim fails at an accepted extreme
UNIT:      man-page:datetime/toUtc
CLAIM:     “Because the zero offset is pinned onto the result, the `datetime::DateTime` round-trips back to the original instant via `datetime::resolve` with no further zone lookup.”
VERDICT:   wrong
EVIDENCE:  `func_to_utc.rs:BODY` accepts `Instant[-9223372036854775808, 0]`; the probe first printed `-292277022657` and `0` for `toUtc(lo)`, then `resolve(toUtc(lo))` raised `7-705-0010 Arithmetic overflow`. The ordinary probe round-tripped epoch and `-1` seconds successfully.
SUGGESTED: The result records the UTC zone and an offset of zero.

### 2. Parameter omits required component semantics
UNIT:      man-page:datetime/toUtc
CLAIM:     “The instant to read in UTC.”
VERDICT:   incomplete
EVIDENCE:  The descriptor parameter is `at AS Instant` in `src/codegen/builtins/datetime/func_to_utc.rs:register`. The probe showed `Instant[0, 123456789]` becomes `1970-01-01 00:00:00.123456789`, while `Instant[-1, 987654321]` becomes `1969-12-31 23:59:59.987654321`; `nanos` is retained and negative seconds are accepted.
SUGGESTED: The instant to express in UTC. `seconds` is a signed count from the Unix epoch, so `0` is the epoch and negative values are before it; its `nanos` component is retained in the returned time.