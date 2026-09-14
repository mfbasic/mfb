### 1. Malformed input failures are omitted

UNIT:      man-page:encoding/formUrlDecode  
CLAIM:     “After the whole input has been decoded, the resulting byte sequence is validated as UTF-8 and returned as a String.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/encoding/helper_percent_decode_bytes.rs:__encoding_percentDecodeBytes` raises `ErrInvalidFormat` for a truncated `%` escape, non-hex escape, and invalid decoded UTF-8; the rendered page has no Errors table or equivalent prose. Scratch probes printed: `x%4` → `Error: 7-705-0003 / truncated percent escape`; `x%G0` → `invalid percent escape`; `%FF` → `invalid utf-8`.  
SUGGESTED: “Malformed input raises `ErrInvalidFormat`: a `%` must be followed by two hexadecimal digits, and the decoded bytes must form valid UTF-8.”