### 1. Parameter range is undocumented
UNIT:      man-page:encoding/sleb128Encode
CLAIM:     "The integer to encode."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_sleb128_encode.rs:__encoding_sleb128Encode` accepts every `Integer` without validation. Probe output included `-9223372036854775808: 128 128 128 128 128 128 128 128 128 127 => -9223372036854775808`; zero and negative values also succeeded.
SUGGESTED: The signed `Integer` to encode. Zero and negative values are valid; every `Integer` value is encoded.

### 2. Algorithm walkthrough is implementation-facing
UNIT:      man-page:encoding/sleb128Encode
CLAIM:     "Unlike unsigned LEB128, encoding continues by arithmetic (sign-extending) shift rather than logical shift: after each group value is shifted right by seven bits with the sign preserved."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/encoding/func_sleb128_encode.rs:__encoding_sleb128Encode` performs this exact `bits::sra(remaining, 7)` implementation step. It does not change how a caller supplies `value` or uses the returned list; it teaches the function’s internal algorithm.
SUGGESTED: Signed LEB128 preserves the sign when encoding negative values.

### 3. “Sign byte” misstates the unsigned distinction
UNIT:      man-page:encoding/sleb128Encode
CLAIM:     "Both non-negative and negative values are accepted; use encoding::uleb128Encode when the value is known to be non-negative and the sign byte is unwanted."
VERDICT:   misleading
EVIDENCE:  The encoder has no separate sign byte: `src/codegen/builtins/encoding/func_sleb128_encode.rs:__encoding_sleb128Encode` stores a sign bit in the final 7-bit group. Probe output shows `64: 192 0 => 64`; signed encoding needs two bytes there, while the distinction is signed interpretation/sign preservation, not removal of a dedicated byte.
SUGGESTED: Both non-negative and negative values are accepted; use `encoding::uleb128Encode` for a known non-negative value, which can use fewer bytes.