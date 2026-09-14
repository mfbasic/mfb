### 1. Raised error is absent from the page

UNIT:      man-page:encoding/base32Decode  
CLAIM:     “a symbol count whose remainder modulo eight is `1`, `3`, or `6` cannot occur in any well-formed Base32 encoding and is rejected.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/encoding/func_base32_decode.rs:BODY` raises `error(77050003, ...)` for invalid length, padding, and characters, but its descriptor declares `errors: vec![]`, so the rendered page has no Errors table. Scratch probe `encoding::base32Decode("A=======")` compiled with the specified release binary and printed:
```text
Error: 7-705-0003
invalid base32 length
```  
SUGGESTED: `Raises ErrInvalidFormat when text has an invalid character, a length that is not a multiple of eight, padding followed by a non-padding character, or an impossible non-padding-symbol count.`