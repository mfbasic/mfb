### 1. Missing documented error

UNIT:      man-page:encoding/varintDecode  
CLAIM:     "`data` must contain at least one byte, and the sequence must be terminated within it: if the bytes run out before a byte with a clear high bit is seen, the input is treated as truncated."  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/encoding/func_uleb128_decode.rs:__encoding_uleb128Decode` fails empty or unterminated input with `error(77050003, "truncated leb128")`; `varintDecode` calls it directly. Probe `varintDecode([])` built with the required release binary printed `Error: 7-705-0003` and `truncated leb128`. The rendered page has no Errors table because `func_varint_decode.rs:register` declares `errors: vec![]`.  
SUGGESTED: `Raises ErrInvalidFormat (7-705-0003) when data is empty, ends before a terminator byte, or exceeds the supported length.`

### 2. Claimed overflow boundary does not match HEAD

UNIT:      man-page:encoding/varintDecode  
CLAIM:     "The accumulated shift may not exceed 63 bits; a sequence encoding more than 64 significant bits overflows."  
VERDICT:   wrong  
EVIDENCE:  In `src/codegen/builtins/encoding/func_uleb128_decode.rs:__encoding_uleb128Decode`, the overflow check is `shift > 63` before reading each byte. A probe decoding nine `0x80` bytes followed by `0x02`—a terminated 10-byte sequence encoding `2^64`—built and ran with the required release binary, printing `0` rather than raising an error.  
SUGGESTED: `The decoder raises ErrInvalidFormat only when it needs to read an eleventh byte; a terminated tenth byte is accepted even when its final group exceeds the 64-bit range.`