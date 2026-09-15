### 1. Padding is not a 6-bit alphabet character
UNIT:      man-page:encoding/base64UrlDecode
CLAIM:     “Each character selects a 6-bit value from the alphabet ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/encoding/helper_base64_symbols.rs:__encoding_base64Symbols` treats `=` specially and appends no value. Probe output: `padded=66` for `Zg==`.
SUGGESTED: Each non-padding character selects a 6-bit value from the alphabet `ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_`.

### 2. “Inverse” overstates the accepted-input contract
UNIT:      man-page:encoding/base64UrlDecode
CLAIM:     “This is the inverse of encoding::base64UrlEncode.”
VERDICT:   misleading
EVIDENCE:  Probe output: `noncanonical=00` for `AB==`; encoding that decoded byte produces `AA`, not `AB==`. The decoder accepts padding and nonzero discarded trailing bits, while `base64UrlEncode` emits unpadded canonical text.
SUGGESTED: This decodes text produced by `encoding::base64UrlEncode`; encoding the result produces the unpadded canonical spelling.

### 3. Invalid-input error is omitted
UNIT:      man-page:encoding/base64UrlDecode
CLAIM:     “The Base64url text to decode.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/helper_base64_symbols.rs:__encoding_base64Symbols` raises `77050003` for invalid characters and non-trailing padding; `func_base64_url_decode.rs:BODY` raises it when the non-padding symbol count is `1 mod 4`. Probe output: `badlength=raised=77050003`, `badchar=raised=77050003`, `padmiddle=raised=77050003`. The rendered page has no Errors table.
SUGGESTED: Malformed Base64url text raises `ErrInvalidFormat` (`77050003`): an invalid character, a non-padding character after `=`, or a non-padding symbol count whose remainder modulo four is `1`.