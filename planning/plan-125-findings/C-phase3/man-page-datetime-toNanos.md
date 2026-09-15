### 1. Does not state behavior for directly constructed, non-normalized inputs
UNIT:      man-page:datetime/toNanos  
CLAIM:     "Because a normalized datetime::Instant already holds its nanos field at full nanosecond resolution (0..999999999), the conversion is exact and discards nothing — no truncation or rounding occurs in either direction."  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/datetime/func_to_nanos.rs:BODY` performs only `at.seconds * 1000000000 + at.nanos`; it does not normalize `at`. The scratch probe compiled and ran with the specified binary:

```mfb
io::print("raw-large-nanos=" & toString(datetime::toNanos(datetime::Instant[0, 1000000000])))
io::print("raw-negative-nanos=" & toString(datetime::toNanos(datetime::Instant[0, -1])))
```

It printed:

```text
raw-large-nanos=1000000000
raw-negative-nanos=-1
```

A caller can therefore supply publicly constructible `Instant` records whose `nanos` field is outside the stated normalized range; `toNanos` uses those fields as-is.  
SUGGESTED:  "For an `Instant` made by `datetime::instant`, `nanos` is normalized to `0..999999999`, so this conversion preserves every nanosecond. `toNanos` uses the `seconds` and `nanos` fields as stored; it does not normalize a directly constructed `Instant`."