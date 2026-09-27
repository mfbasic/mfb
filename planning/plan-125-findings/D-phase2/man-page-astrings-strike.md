### 1. Range semantics omitted
UNIT:      man-page:astrings/strike  
CLAIM:     “Pass it to astrings::addAttribute to mark a scalar range struck through.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/astrings/func_add_attribute.rs:DESC` defines `[start, endIndex]` as inclusive; `gen_astrings.rs:lower_scalar_len` counts Unicode scalars. Probe compiling and running against the specified release binary printed `kind=Strike` and `~~a中~~b` for spans `[1,1]` and `[0,0]`, confirming scalar-based coverage.  
SUGGESTED: Pass it to `astrings::addAttribute` to mark an inclusive range of Unicode scalar indices struck through.