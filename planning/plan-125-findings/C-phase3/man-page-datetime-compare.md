### 1. Unnormalized `Instant` values contradict timeline-order claim
UNIT:      man-page:datetime/compare
CLAIM:     "`datetime::compare` returns the sign of `a - b` as a three-way ordering: `-1` when `a` is before `b`, `0` when the two instants name the same point, and `1` when `a` is after `b`."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/datetime/func_compare.rs:BODY` compares `seconds` then `nanos` without normalization. The probe `datetime::Instant[0, 1_000_000_000]` versus `datetime::Instant[1, 0]` printed `-1`, then `1`; `datetime::toNanos` printed `1000000000` for both values. Thus accepted values representing the same epoch-nanosecond count do not compare equal.
SUGGESTED: "`datetime::compare` compares the stored `seconds` and `nanos` fields: it returns `-1`, `0`, or `1`. Instants built with `datetime::instant` are normalized, so this gives their UTC-timeline order; if you construct an `Instant` record directly, normalize its fields first."