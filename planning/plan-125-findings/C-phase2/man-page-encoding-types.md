### 1. ISO-8859-8 variants do not differ in this binary

UNIT:      man-page:encoding/types  
CLAIM:     “ISO-8859-8-I -- Hebrew, logical order. The same byte mapping as `Iso8859_8`; the two differ only in display direction.”  
VERDICT:   misleading  
EVIDENCE:  `helper_codepage_table.rs:VARIANTS` assigns both variants the same `iso-8859-8` table; `codepageDecode` and `codepageEncode` only use that table. The compiled probe printed `TRUE` when comparing both decodings. No binary operation changes display direction.  
SUGGESTED: “ISO-8859-8-I -- Hebrew, logical-order label. It uses the same byte mapping as `Iso8859_8`, so encoding and decoding produce the same results.”

### 2. Undefined-byte entries omit their observable failure

UNIT:      man-page:encoding/types  
CLAIM:     “Leaves 7 high bytes undefined.”  
VERDICT:   incomplete  
EVIDENCE:  `func_codepage_decode.rs:__encoding_codepageDecode` raises `ErrInvalidFormat` for an unmapped table entry. The compiled probe decoded ISO-8859-3 byte `165`, one of its holes, and printed `ErrInvalidFormat=77050003`. The same omission recurs for the other entries that state a defined/undefined high-byte count.  
SUGGESTED: “Leaves seven of bytes 128–255 undefined; decoding one raises `ErrInvalidFormat`.”