### 1. Unnormalized records have no stated ordering rule
UNIT:      man-page:datetime/isAfter  
CLAIM:     "Because both arguments are points on the same Unix-epoch, leap-second-free UTC timeline, the ordering is absolute and independent of any time zone; resolve a `datetime::DateTime` to a `datetime::Instant` with `datetime::resolve` before comparing."  
VERDICT:   incomplete  
EVIDENCE:  `/tmp/plan-125-scratch/C-phase3/man-page-datetime-isAfter/probe/src/raw_record.mfb` constructs `datetime::Instant[1, 1000000000]` and `datetime::Instant[2, 0]`; the probe prints `FALSE` then `TRUE`. `src/codegen/builtins/datetime/func_is_after.rs:BODY` compares stored fields without normalization.  
SUGGESTED: `isAfter compares the stored seconds and nanos fields and does not normalize them. Build instants with datetime::instant before comparing.`

### 2. Signed-comparison implementation detail
UNIT:      man-page:datetime/isAfter  
CLAIM:     "`isAfter` is pure: the same two instants always yield the same `Boolean`, it has no side effects, and it performs only signed comparisons (no arithmetic), so it cannot overflow or trap."  
VERDICT:   out-of-scope  
EVIDENCE:  `src/codegen/builtins/datetime/func_is_after.rs:BODY` implements the predicate through a comparison; whether those are signed comparisons and contain no arithmetic is an implementation detail, not information needed to call the function.  
SUGGESTED: `isAfter does not change either instant and does not raise an error.`