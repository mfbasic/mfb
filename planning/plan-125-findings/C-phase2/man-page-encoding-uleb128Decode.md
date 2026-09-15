### 1. Non-negative result guarantee is false
UNIT:      man-page:encoding/uleb128Decode
CLAIM:     "`data` carries only magnitude, so the result is always non-negative — use `encoding::sleb128Decode` for signed values."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/encoding/func_uleb128_decode.rs:BODY` permits a terminating tenth group at shift 63. Probe input `[0x80 × 9, 0x01]` compiled and ran with the specified binary, printing `-9223372036854775808`.
SUGGESTED: Do not promise a non-negative result until the decoder rejects values outside the signed `Integer` range. After that fix: "The decoded value is non-negative; use `encoding::sleb128Decode` for signed values."

### 2. Overflow claim is false at HEAD
UNIT:      man-page:encoding/uleb128Decode
CLAIM:     "The accumulated shift may not exceed 63 bits; a sequence encoding more than 64 significant bits overflows."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/encoding/func_uleb128_decode.rs:BODY` checks `shift > 63` before reading a group, so it accepts the tenth group at shift 63. Probe input `[0x80 × 9, 0x02]`, which encodes 2^64, compiled and ran with the specified binary and printed `0`, rather than raising an overflow error.
SUGGESTED: Remove this guarantee until the decoder rejects a tenth group whose payload exceeds `1`; then say: "Values above `9223372036854775807` raise `ErrInvalidFormat`."

### 3. Malformed-input error is omitted from the derived Errors table and parameter contract
UNIT:      man-page:encoding/uleb128Decode
CLAIM:     "`data` must contain at least one byte, and the sequence must be terminated within it: if the bytes run out before a byte with a clear high bit is seen, the input is treated as truncated."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_uleb128_decode.rs:BODY` raises `error(77050003, "truncated leb128")` for empty input and an unterminated sequence, but its `RegistryFunction.implementations[0].errors` is `vec![]`, so the rendered page has no Errors table. An empty-list probe compiled and ran with the specified binary and printed `Error: 7-705-0003` followed by `truncated leb128`.
SUGGESTED: Add `ErrInvalidFormat` (`77050003`) to the function’s errors metadata, and describe `data` as: "A non-empty ULEB128 byte sequence. The first byte with a clear high bit terminates it; later bytes are ignored. An empty or unterminated sequence raises `ErrInvalidFormat`."