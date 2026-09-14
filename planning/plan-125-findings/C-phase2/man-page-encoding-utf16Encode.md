### 1. Does not state the returned list is new
UNIT:      man-page:encoding/utf16Encode
CLAIM:     "`encoding::utf16Encode` returns the UTF-16 encoding of `value` as a list of numeric code units, one element per 16-bit unit."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_utf16_encode.rs:__encoding_utf16Encode` creates `result` as `[]` and returns it; it does not alter `value`. The compiled probe `edge_cases.mfb` printed `0`, `1 65`, `2 55357 56832`, and `4 65 55357 56832 66` for empty, BMP, astral, and mixed input respectively.
SUGGESTED: "`encoding::utf16Encode` returns a new list containing the UTF-16 code units for `value`; it does not change the string."