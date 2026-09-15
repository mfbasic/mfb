### 1. UTF-8 omitted from the summary

UNIT:      man-page:encoding/codepageEncode  
CLAIM:     “Encode a String as bytes in a legacy single-byte codepage.”  
VERDICT:   misleading  
EVIDENCE:  `src/codegen/builtins/encoding/func_codepage_encode.rs:BODY` branches `Codepage.Utf8` to `__encoding_utf8Encode`. Probe built and ran with the required release binary: `encoding::codepageEncode(Codepage.Utf8, "e\u{0301}")` printed `65cc81`.  
SUGGESTED: “Encode a String as bytes in a legacy single-byte codepage or UTF-8.”

### 2. ISO-8859-7 claim incorrectly excludes ASCII letters

UNIT:      man-page:encoding/codepageEncode  
CLAIM:     “A character the selected codepage has no byte for — 世 in windows-1252, or any letter outside the Greek block in ISO-8859-7 — is rejected with ErrInvalidFormat (77050003) rather than replaced by ? or an HTML numeric reference.”  
VERDICT:   wrong  
EVIDENCE:  `func_codepage_encode.rs:BODY` emits every scalar below 128 directly. The required release-binary probe `encoding::codepageEncode(Codepage.Iso8859_7, "A")` printed `41`; `A` is outside the Greek Unicode block.  
SUGGESTED: “A character the selected codepage has no byte for — 世 in windows-1252, for example — is rejected with ErrInvalidFormat (77050003) rather than replaced by ? or an HTML numeric reference.”

### 3. `codepage` parameter omits its UTF-8 behavior

UNIT:      man-page:encoding/codepageEncode  
CLAIM:     “The codepage to write the bytes in.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/encoding/func_codepage_encode.rs:BODY` treats `Codepage.Utf8` differently from every single-byte table. The required release-binary probe printed `65cc81` for `Codepage.Utf8` and `e\u{0301}`.  
SUGGESTED: “The codepage to write the bytes in. `Codepage.Utf8` encodes the whole text as UTF-8 instead.”