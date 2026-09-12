### 1. Does not state that the input colour is unchanged
UNIT:      man-page:color/toHex  
CLAIM:     "`toHex` returns the six-digit form — a leading `#` and two lowercase hex digits each for red, green and blue."  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/color/func_to_hex.rs:BODY` reads `base.red`, `base.green`, and `base.blue` only. The probe at `/tmp/plan-125-scratch/A-iter2/man-page-color-toHex/src/main.mfb` printed `#ff0000`, then `#ff000080` for the same `rgba(255, 0, 0, 128)` value, confirming `toHex` drops alpha only from its returned text and does not change the input colour.  
SUGGESTED: `toHex returns the six-digit form — a leading # and two lowercase hex digits each for red, green, and blue. It does not change base.`