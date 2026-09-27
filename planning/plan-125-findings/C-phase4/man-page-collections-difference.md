### 1. Compiler-internals error rationale

UNIT:      man-page:collections/difference  
CLAIM:     “Running out of memory is not a trappable domain error, and the add it is built on is classified infallible.Both arguments must be the same Set OF T.”  
VERDICT:   out-of-scope  
EVIDENCE:  The rendered page contains this run-on sentence. `src/codegen/builtins/collections/func_difference.rs:BODY` implements the operation with `collections::add`, but whether that member is “classified infallible” is compiler-source detail, not useful developer-facing behavior. Probes confirmed the documented behavior: `probe.mfb` printed `d=3,4,`, preserved `a=3,1,4,2,` and `b=1,2,9,`, and printed `self=0`; the mismatched-set probe was rejected with `TYPE_CALL_ARGUMENT_MISMATCH`. Both shipped examples compiled and printed `2` and `0`.  
SUGGESTED: `difference raises no user-trappable errors. Both arguments must be sets with the same element type; otherwise the call is rejected at compile time.`