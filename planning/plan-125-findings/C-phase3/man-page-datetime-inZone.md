### 1. Compiler-internal “OS intrinsic” explanation

UNIT:      man-page:datetime/inZone  
CLAIM:     "`inZone` is pure for UTC and fixed-offset zones; for a local zone it reads the host's time-zone configuration through the `datetime::localOffset` OS intrinsic to resolve the offset."  
VERDICT:   out-of-scope  
EVIDENCE:  `src/codegen/builtins/datetime/func_in_zone.rs:DESC` names `datetime::localOffset` as an “OS intrinsic”; its lowering is implementation detail in `src/codegen/builtins/datetime/func_local_offset.rs:lower_local_offset`.  
SUGGESTED: `For a local zone, the result uses the host's configured time-zone rules for that instant.`