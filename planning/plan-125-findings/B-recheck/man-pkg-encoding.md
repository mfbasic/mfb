### 1. Codepage.Utf8 is missing from codepageDecode's intro but present in codepageEncode's
UNIT:      man-pkg:encoding
PAGE:      codepageDecode (and the overview function table)
CATEGORY:  consistency
CLAIM:     "Decode a List OF Byte as text in a legacy single-byte codepage." versus the sibling's "Encode a String as bytes in a legacy single-byte codepage or as UTF-8."
VERDICT:   inconsistent
EVIDENCE:  `mfb man encoding --all` function table rows for codepageDecode and codepageEncode; `src/codegen/builtins/encoding/func_codepage_decode.rs:INTRO` vs `func_codepage_encode.rs:INTRO`. Probe `codepageDecode(encoding::Codepage.Utf8, [toByte(255)])` raised 77050003 "invalid utf-8", so the decoder does take Codepage.Utf8 exactly as the encoder does; only the intro hides it. A developer scanning the table for "decode UTF-8 or a codepage by label" finds the encoder and not the decoder.
SUGGESTED: codepageDecode intro: "Decode a List OF Byte as text in a legacy single-byte codepage or as UTF-8."

### 2. The overview says decoders raise ErrInvalidFormat, but two decoders raise ErrEncoding for the same condition the Unicode decoders report as ErrInvalidFormat
UNIT:      man-pkg:encoding
PAGE:      package-wide (overview Description; htmlUnescape; punycodeDecode)
CATEGORY:  overview-mismatch
CLAIM:     "Decoders reject malformed input with ErrInvalidFormat (77050003): an invalid character, a bad length, or a misplaced pad."
VERDICT:   misleading
EVIDENCE:  Probe (built with mfb build, scratch project under /tmp/plan-125-scratch/B-recheck/man-pkg-encoding/p1): `htmlUnescape("&#55296;")` → 77020004; `punycodeDecode("xn--ib9b")` → 77020004; `utf32Decode([55296])` → 77050003 "surrogate code point"; `utf16Decode([55296])` → 77050003 "unpaired surrogate"; `utf8Decode([237,160,128])` → 77050003 "invalid utf-8". The overview's Errors table lists ErrEncoding (77020004) but the overview prose never says when the package raises it, and a developer trapping "decode failure" on 77050003 package-wide misses the surrogate case in two functions. The individual pages are accurate; the package-level statement is not. (The underlying split is an implementation inconsistency: a decoded surrogate is ErrInvalidFormat in three decoders and ErrEncoding in two. A documentation pass cannot change that; the main thread may want to file it.)
SUGGESTED: Add to the overview after the ErrInvalidFormat sentence: "Two decoders also raise ErrEncoding (77020004): encoding::htmlUnescape and encoding::punycodeDecode, when a reference or payload resolves to a surrogate code point (U+D800–U+DFFF). The UTF-8/16/32 decoders report the same condition as ErrInvalidFormat."

### 3. No page tells a developer holding an ISO-8859-1 / latin1 label which Codepage to use
UNIT:      man-pkg:encoding
PAGE:      types (Codepage enum) and codepageDecode
CATEGORY:  coverage
CLAIM:     The Codepage enum lists 28 members; none is ISO-8859-1, and no page mentions ISO-8859-1, latin1, or US-ASCII.
VERDICT:   missing
EVIDENCE:  `grep -rn -i -E '8859-1\b|latin-?1|us-ascii' src/codegen/builtins/encoding/` → only an unrelated internal comment in helper_byte_char.rs; `helper_codepage_table.rs:VARIANTS` has no 8859-1 entry. The overview says the tables are "the legacy single-byte codepages of the WHATWG Encoding Standard"; in that standard the labels iso-8859-1, latin1, us-ascii and ascii all resolve to windows-1252, which is why no separate member exists. A developer arriving with the single most common legacy charset label has to know WHATWG's label table to pick Windows1252, and nothing on the page says so.
SUGGESTED: On the Windows1252 variant description: "windows-1252 -- Western European. The most common legacy web encoding; also what the labels ISO-8859-1, latin1 and US-ASCII mean in practice (the WHATWG Encoding Standard maps them here), so use this member for those labels too." Optionally one sentence in codepageDecode's Description.

### 4. "A caller holding a charset label has one entry point" — but nothing turns a label into a Codepage
UNIT:      man-pkg:encoding
PAGE:      codepageDecode, codepageEncode, overview
CATEGORY:  coverage
CLAIM:     "Codepage.Utf8 instead decodes the whole input as UTF-8, so a caller holding a charset label has one entry point for both cases."
VERDICT:   misleading
EVIDENCE:  No function in the package or in general converts a String label to a Codepage member: `grep -rn -i -E 'enum.*(parse|fromName|byName)|parseEnum|enumFrom' src/codegen/builtins/*/mod.rs src/codegen/registry/*.rs` → nothing; `mfb man general toString` and `toInt` only go enum → String / enum → index (probe: toString(Codepage.Windows1252) = "Windows1252", toInt(...) = 21). The label-to-member step the sentence presumes is the developer's own SELECT/MATCH over strings, and the codepageDecode example titled "Pick the codepage from a charset the caller already knows" does not actually do that step either; it hard-codes the member.
SUGGESTED: Either soften to "so one call site can handle UTF-8 and the legacy codepages alike once you have mapped the charset label to a member" and show a small MATCH on a lowercased label ("utf-8" → Codepage.Utf8, "windows-1252"/"iso-8859-1" → Codepage.Windows1252, …) in the codepageDecode example; or note plainly that no label parser is provided.

### 5. utf8Decode does not cross-reference toString(List OF Byte), while utf8Encode does cross-reference strings::toBytes
UNIT:      man-pkg:encoding
PAGE:      utf8Decode
CATEGORY:  consistency
CLAIM:     utf8Encode: "The byte form is exactly strings::toBytes(value)." utf8Decode has no equivalent sentence.
VERDICT:   inconsistent
EVIDENCE:  `mfb man general toString`: "A List OF Byte is read as UTF-8 text ... Bytes that are not well-formed UTF-8 raise ErrEncoding". Probe: `toString([toByte(255)])` → 77020004; `utf8Decode([237,160,128])` → 77050003. So the language has a core, no-IMPORT UTF-8 decoder whose only difference from utf8Decode is the error kind, and the decode page never mentions it, whereas the encode page points at its core twin. A developer who finds toString(bytes) first will not learn that utf8Decode raises a different error for the same bad input.
SUGGESTED: In utf8Decode's Description: "The byte form does what toString(bytes) does, except that malformed UTF-8 raises ErrInvalidFormat here and ErrEncoding from toString; use utf8Decode when you also hold a List OF Integer or want the same error kind as the other decoders in this package."

### 6. varintDecode's parameter description drops the contract its two siblings state
UNIT:      man-pkg:encoding
PAGE:      varintDecode
CATEGORY:  consistency
CLAIM:     "The varint bytes to decode." versus uleb128Decode/sleb128Decode: "The ULEB128 bytes to decode: a non-empty, terminated sequence. Bytes after the terminating byte are ignored."
VERDICT:   inconsistent
EVIDENCE:  `func_varint_decode.rs:77 desc: "The varint bytes to decode."` vs `func_uleb128_decode.rs:74`. The varintDecode Description does state the non-empty, terminated and trailing-bytes-ignored rules, so the behaviour is the same; only the parameter table differs across three pages a developer compares side by side.
SUGGESTED: "The varint bytes to decode: a non-empty, terminated sequence. Bytes after the terminating byte are ignored."

Checked and found consistent: the overview's grouping covers all 30 functions in the table; the base32/base64 non-canonical-group claims hold (probe: `base32Decode("AB======")` → one 0x00 byte, `base32Encode(base32Decode("my======"))` → "MY======"); uleb128Encode(-1) raises 77050003 as documented; the Codepage enum's 28 members match the WHATWG single-byte index set exactly; the ten-byte limit wording is identical across the three LEB128/varint decoders; percent/formUrl pages agree on the + and %XX differences; every page's See also names its inverse, and the types page is reachable from the overview and both codepage pages.
