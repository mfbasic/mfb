### 1. Odd-length input error is omitted
UNIT:      man-page:encoding/hexDecode
CLAIM:     "The input length must be even, because each byte needs a pair of digits."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_hex_decode.rs:BODY` raises `error(77050003, "odd-length hex")` when the UTF-8 byte length is odd; the scratch probe `encoding::hexDecode("0")` printed `Error: 7-705-0003` and `odd-length hex`. The rendered page has no Errors table because `RegistryFunction.implementations[0].errors` is empty.
SUGGESTED: Raises `ErrInvalidFormat` when `text` has an odd number of hexadecimal digits.

### 2. Invalid-digit error is omitted
UNIT:      man-page:encoding/hexDecode
CLAIM:     "Any other character is rejected."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_hex_decode.rs:BODY` calls `__encoding_hexValue` for each input byte and raises `error(77050003, "invalid hex digit")` if either value is negative; the scratch probe `encoding::hexDecode("0g")` printed `Error: 7-705-0003` and `invalid hex digit`. `src/docs/spec/stdlib/08_encoding.md` identifies `77050003` as `ErrInvalidFormat`.
SUGGESTED: Raises `ErrInvalidFormat` when `text` contains a character outside `0`–`9`, `a`–`f`, and `A`–`F`.