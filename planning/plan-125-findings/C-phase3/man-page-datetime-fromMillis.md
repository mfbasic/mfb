### 1. Parameter omits zero behavior
UNIT:      man-page:datetime/fromMillis  
CLAIM:     “Milliseconds since the Unix epoch. Negative values name instants before 1970.”  
VERDICT:   incomplete  
EVIDENCE:  The descriptor `src/codegen/builtins/datetime/func_from_millis.rs:register` supplies this parameter text. Probe output for `datetime::fromMillis(0)` was `0,0,0`, confirming it yields the epoch.  
SUGGESTED: “Milliseconds since the Unix epoch. Zero yields 1970-01-01T00:00:00Z; negative values name instants before 1970.”

### 2. Implementation mechanics leak into the developer page
UNIT:      man-page:datetime/fromMillis  
CLAIM:     “The implementation first computes the toward-zero quotient `millis / 1000` and remainder `millis MOD 1000`; when that remainder is negative it adds `1000` to the remainder and subtracts `1` from the quotient, borrowing one second.”  
VERDICT:   out-of-scope  
EVIDENCE:  This describes the injected builtin body `src/codegen/builtins/datetime/func_from_millis.rs:BODY`, rather than behavior a developer needs to use the function. `.ai/man-content.md` §3 explicitly excludes implementation/lowering mechanics. The probe printed `-1,999000000,-1` for `fromMillis(-1)`, which establishes the user-visible outcome without teaching the algorithm.  
SUGGESTED: “For negative millisecond counts, the returned `nanos` field remains non-negative: `fromMillis(-1)` has `seconds` `-1` and `nanos` `999000000`.”

### 3. “Arbitrary” round trip can raise instead
UNIT:      man-page:datetime/fromMillis  
CLAIM:     “Because the input has no sub-millisecond component, round-tripping an arbitrary `datetime::Instant` through `datetime::toMillis` and back loses its microsecond and nanosecond digits; for full nanosecond precision use `datetime::toNanos` together with `datetime::instant`.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/datetime/func_to_millis.rs:BODY` multiplies `at.seconds * 1000`, and its descriptor declares `ErrOverflow`. The extreme probe printed `toMillis-overflow=TRUE` for `datetime::toMillis(datetime::instant(9223372036854775807))`; no round trip occurs.  
SUGGESTED: “For an instant whose millisecond count fits an `Integer`, round-tripping through `datetime::toMillis` and back drops any sub-millisecond part. For full nanosecond precision, use `datetime::toNanos` with `datetime::instant` when the nanosecond count fits an `Integer`.”