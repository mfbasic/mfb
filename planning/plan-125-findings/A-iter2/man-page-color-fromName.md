### 1. Empty-name behavior is omitted from the parameter contract
UNIT:      man-page:color/fromName  
CLAIM:     "The CSS colour name. Case-insensitive; surrounding whitespace is ignored."  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/color/func_from_name.rs:BODY` trims `name`, then raises `ErrNotFound` when the resulting key is absent. Probe `lookup("")` and `lookup("   ")` each printed `err=77050004`.  
SUGGESTED: The CSS colour name. Matching is case-insensitive and trims surrounding whitespace; an empty or whitespace-only name raises ErrNotFound.