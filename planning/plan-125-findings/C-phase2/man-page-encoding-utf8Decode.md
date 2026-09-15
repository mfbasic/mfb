### 1. Invalid-input failure is undocumented

UNIT:      man-page:encoding/utf8Decode  
CLAIM:     "input is validated in full before the string is produced: utf8Decode accepts only a well-formed UTF-8 sequence and rejects an invalid lead byte, a missing or stray continuation byte, a truncated multi-byte sequence, an overlong encoding, a surrogate code point (U+D800–U+DFFF), and any scalar above U+10FFFF."  
VERDICT:   incomplete  
EVIDENCE:  The rendered page has no Errors table. `src/codegen/builtins/encoding/helper_utf8_decode.rs:BODY` raises error `77050003` for malformed UTF-8 and invalid integer units. Probe output: `stray=77050003:invalid utf-8`, `truncated=77050003:invalid utf-8`, `overlong=77050003:invalid utf-8`, `surrogate=77050003:invalid utf-8`, and `tooHigh=77050003:invalid utf-8`. `mfb spec diagnostics error-codes` identifies `77050003` as `ErrInvalidFormat`.  
SUGGESTED: `Malformed UTF-8 raises ErrInvalidFormat; this includes invalid or stray bytes, truncated sequences, overlong encodings, surrogate code points, and scalars above U+10FFFF.` Add `ErrInvalidFormat` to the descriptor’s derived Errors table.

### 2. Parameter descriptions omit the Integer overload’s range and failure behavior

UNIT:      man-page:encoding/utf8Decode  
CLAIM:     "The UTF-8 byte or code-unit sequence to decode."  
VERDICT:   incomplete  
EVIDENCE:  Both rendered parameter rows use this identical description. `src/codegen/builtins/encoding/helper_utf8_decode.rs:BODY` requires every `List OF Integer` element to be in `0..255`; the probe printed `negative=77050003:invalid utf-8 code unit` and `tooLarge=77050003:invalid utf-8 code unit`. The same probe printed `emptyBytes=.` and `emptyInts=.`, confirming both empty lists decode to an empty string.  
SUGGESTED: For `List OF Integer`: `UTF-8 code units to decode. Each integer must be 0 through 255 inclusive; a negative value or a value above 255 raises ErrInvalidFormat. An empty list returns the empty string.` For `List OF Byte`, say explicitly that an empty list returns the empty string and that the sequence must be well-formed UTF-8.

### 3. Compiler-dispatch detail leaks into the developer page

UNIT:      man-page:encoding/utf8Decode  
CLAIM:     "The overload is settled once the argument type is known, so the selection is a compile-time decision, not a runtime dispatch."  
VERDICT:   out-of-scope  
EVIDENCE:  This is an implementation-selection fact, not behavior a developer needs to use `utf8Decode`; `.ai/man-content.md` §3 excludes compiler mechanics from man pages. `src/codegen/builtins/encoding/func_utf8_decode.rs:register` confirms the two public forms are selected by parameter type, but that does not make compilation timing relevant to the caller.  
SUGGESTED: `Pass a List OF Byte for UTF-8 bytes or a List OF Integer for integer code units.`