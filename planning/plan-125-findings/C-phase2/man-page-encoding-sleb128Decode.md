### 1. Missing error contract
UNIT:      man-page:encoding/sleb128Decode
CLAIM:     "`data` must contain at least one byte, and the sequence must be terminated within it: if the bytes run out before a byte with a clear high bit is seen, the input is treated as truncated."
VERDICT:   incomplete
EVIDENCE:  The rendered page has no Errors table, but `src/codegen/builtins/encoding/func_sleb128_decode.rs:__encoding_sleb128Decode` raises error `77050003` for both empty input and an unterminated sequence. The probes `empty/src/main.mfb` and `truncated/src/main.mfb` each printed `Error: 7-705-0003` / `truncated leb128` and exited `255`.
SUGGESTED: Add an Errors row: “ErrInvalidFormat — `data` is empty, or it ends before a byte with bit `0x80` clear.”

### 2. Parameter description omits validity requirements
UNIT:      man-page:encoding/sleb128Decode
CLAIM:     "The SLEB128 bytes to decode."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_sleb128_decode.rs:__encoding_sleb128Decode` requires a nonempty list and raises `77050003` for `[]` or an unterminated sequence. The empty-input probe printed `Error: 7-705-0003` / `truncated leb128`; the valid-example probe printed `-123456`, `-2`, and `2`.
SUGGESTED: “A nonempty `List OF Byte` containing a terminated signed LEB128 sequence. An empty or unterminated sequence raises ErrInvalidFormat; bytes after the first terminating byte are ignored.”

### 3. Claimed overflow detection is false at HEAD
UNIT:      man-page:encoding/sleb128Decode
CLAIM:     "The accumulated shift may not exceed `63` bits; a sequence encoding more than 64 significant bits overflows."
VERDICT:   wrong
EVIDENCE:  In `src/codegen/builtins/encoding/func_sleb128_decode.rs:__encoding_sleb128Decode`, overflow is checked before reading a byte (`IF shift > 63`), so the tenth byte is processed at shift `63`. Probe `overflow/src/main.mfb` decoded nine `0x80` bytes followed by `0x02`—an encoding beyond 64 significant bits—and printed `0` rather than raising.
SUGGESTED: No documentation rewrite should mask this: correct `__encoding_sleb128Decode` to reject a terminating tenth byte whose payload exceeds the valid 64-bit signed range, then retain the stated overflow contract.