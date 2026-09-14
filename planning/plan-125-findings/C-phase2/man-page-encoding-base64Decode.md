### 1. RFC 4648 conformance claim is false
UNIT:      man-page:encoding/base64Decode
CLAIM:     "`encoding::base64Decode` parses `text` as standard Base64 (RFC 4648 §4) and returns the bytes it encodes."
VERDICT:   wrong
EVIDENCE:  `/tmp/plan-125-scratch/C-phase2/man-page-encoding-base64Decode` printed `all-padding len=0` for `"===="` and `noncanonical len=1` / `noncanonical first=0` for `"AB=="`. `src/codegen/builtins/encoding/helper_base64_symbols.rs:__encoding_base64Symbols` ignores every trailing `=`, and `src/codegen/builtins/encoding/helper_base_decode_bits.rs:__encoding_baseDecodeBits` discards unused final bits. `src/codegen/builtins/encoding/mod.rs:MODULE_DESC` explicitly confirms `"AB=="` returns byte `0` rather than raising.
SUGGESTED: Decode text using the standard Base64 alphabet. This decoder accepts trailing padding without checking its count and ignores unused bits in the final group; do not use a decode round-trip to check canonical Base64.

### 2. “Each character” incorrectly includes padding and rejected characters
UNIT:      man-page:encoding/base64Decode
CLAIM:     "Each character selects a 6-bit value from the alphabet `ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/`; the values are concatenated most-significant bit first into a continuous bit stream and emitted eight bits at a time, so leftover bits that do not fill a final byte are discarded."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/encoding/helper_base64_symbols.rs:__encoding_base64Symbols` treats `=` as padding rather than a 6-bit symbol, and raises `77050003` for characters absent from the alphabet. The probe printed `bad-character raised 77050003` and `all-padding len=0`.
SUGGESTED: Each non-padding character in valid input selects a 6-bit value from the alphabet `ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/`.

### 3. Raisable invalid-format error is omitted from the rendered page
UNIT:      man-page:encoding/base64Decode
CLAIM:     "The Base64 text to decode."
VERDICT:   incomplete
EVIDENCE:  The rendered page has no Errors section because `src/codegen/builtins/encoding/func_base64_decode.rs:register` declares `errors: vec![]`. Its lowering raises `77050003` for invalid total and symbol lengths, while `src/codegen/builtins/encoding/helper_base64_symbols.rs:__encoding_base64Symbols` raises the same code for invalid characters and padding. The probe printed `bad-length raised 77050003`, `bad-character raised 77050003`, and `misplaced-pad raised 77050003`.
SUGGESTED: The standard-alphabet Base64 text to decode. Invalid characters, a non-trailing `=`, or a total length not divisible by four raise `ErrInvalidFormat`.