### 1. Compiler-internal “OS-seam” terminology
UNIT:      man-page:datetime/monotonicNanos  
CLAIM:     “It is the low-level OS-seam intrinsic that backs `datetime::monotonic`”  
VERDICT:   out-of-scope  
EVIDENCE:  Rendered with `mfb man datetime monotonicNanos`; `src/codegen/builtins/datetime/func_monotonic_nanos.rs:DESC` contains the sentence. “OS-seam intrinsic” describes implementation structure, not something a terminal user needs to use the function.  
SUGGESTED: `datetime::monotonic` represents this kind of reading as a \`datetime::Duration\`; \`monotonicNanos\` returns it as an \`Integer\` nanosecond count.