### 1. Browser-equivalence claim is false for an undefined windows-874 byte
UNIT:      man-page:encoding/codepageDecode
CLAIM:     "The tables are the WHATWG Encoding Standard's legacy single-byte indexes, so the text matches what a browser would show for the same bytes and the same label."
VERDICT:   wrong
EVIDENCE:  The MFBASIC probe `codepageDecode(Windows874, [toByte(219)])`, built and run with the specified release binary, printed `win874-hole=ERR`; `func_codepage_decode.rs:__encoding_codepageDecode` raises `ErrInvalidFormat` for its `U+FFFD` table sentinel. In contrast, `new TextDecoder("windows-874").decode(Uint8Array.of(0xdb))` printed code point `f8c1`, not an error or replacement character.
SUGGESTED: "This decoder uses bundled legacy-codepage tables. A byte left undefined by its selected table raises `ErrInvalidFormat`; browser decoding may handle such bytes differently."