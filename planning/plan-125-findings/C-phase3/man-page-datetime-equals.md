### 1. Equality does not always mean the same timeline point
UNIT:      man-page:datetime/equals
CLAIM:     “Test whether two instants name the same point on the UTC timeline.”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/datetime/func_equals.rs:BODY` delegates to `__datetime_compare`, which compares `seconds` and `nanos` fields directly. Probe:

```mfb
LET a AS datetime::Instant = datetime::instant(1_000)
LET raw AS datetime::Instant = datetime::Instant[999, 1_000_000_000]
io::print(toString(datetime::equals(a, raw)))
```

compiled and printed `FALSE`. `src/codegen/builtins/datetime/func_instant.rs:BODY_2` normalizes `(999, 1_000_000_000)` to `(1000, 0)`, so these encode the same timeline point.
SUGGESTED: Returns `TRUE` exactly when both arguments have equal `seconds` and `nanos` fields.

### 2. An Instant has no zone
UNIT:      man-page:datetime/equals
CLAIM:     “The second instant. Compared as instants, so two values in different zones naming the same moment are equal.”
VERDICT:   misleading
EVIDENCE:  The declared parameter type is `datetime::Instant`, and `src/codegen/builtins/datetime/mod.rs:register` defines `Instant` with only `seconds` and `nanos`; zone information belongs to `datetime::DateTime`. A caller cannot pass two `Instant` values “in different zones.”
SUGGESTED: The second instant. To compare civil `datetime::DateTime` values, first resolve each with `datetime::resolve`.

