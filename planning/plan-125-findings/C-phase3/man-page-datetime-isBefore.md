### 1. Non-canonical `Instant` values are an undocumented comparison sharp edge
UNIT:      man-page:datetime/isBefore  
CLAIM:     “The comparison is performed field by field, matching `datetime::compare`.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/datetime/func_is_before.rs:BODY` delegates to `__datetime_compare`; `src/codegen/builtins/datetime/func_compare.rs:BODY` compares raw `seconds`, then raw `nanos`. Probe compiled and ran with the prescribed binary:

```mfb
LET canonical AS datetime::Instant = datetime::instant(1)
LET noncanonical AS datetime::Instant = datetime::Instant[0, 1_000_000_000]
io::print(toString(datetime::isBefore(noncanonical, canonical)))
io::print(toString(datetime::isBefore(canonical, noncanonical)))
```

Output:

```text
TRUE
FALSE
```

The two records represent the same elapsed point if the nanoseconds field is normalized, but `isBefore` orders their stored fields instead.  
SUGGESTED: `isBefore` compares the stored seconds and nanoseconds fields. Construct instants with `datetime::instant`; manually created non-canonical `Instant` records are not normalized before comparison.