### 1. This is not an IDNA conversion or hostname validator
UNIT:      man-page:encoding/punycodeEncode  
CLAIM:     "`encoding::punycodeEncode` converts a Unicode hostname `domain` to the ASCII representation used by internationalized domain names (IDNA), applying the Punycode Bootstring algorithm of RFC 3492."  
VERDICT:   misleading  
EVIDENCE:  `src/codegen/builtins/encoding/func_punycode_encode.rs:BODY` only splits on ASCII `.` and Punycode-encodes labels containing a scalar at or above 128; it performs neither IDNA mapping nor hostname validation. The scratch probe printed `space=a b.example` for `"a b.example"` and `idna-dot=xn--foobar-rr3e` for `"foo\u{3002}bar"`—the IDNA dot-equivalent was encoded within one label rather than treated as a separator.  
SUGGESTED: `encoding::punycodeEncode` applies RFC 3492 Punycode independently to labels separated by ASCII `.`. It does not validate hostname syntax or apply IDNA mapping rules.

### 2. The scalar/grapheme sharp edge is buried in internal wording
UNIT:      man-page:encoding/punycodeEncode  
CLAIM:     "The input `String` is decoded to Unicode scalar values through the package's UTF-8 decoder before encoding."  
VERDICT:   out-of-scope  
EVIDENCE:  The sentence exposes an implementation route that is irrelevant to an MFBASIC developer; `src/codegen/builtins/encoding/func_punycode_encode.rs:BODY` calls `__encoding_codepoints`, whose implementation is `src/codegen/builtins/encoding/helper_codepoints.rs:BODY`, not the public UTF-8 decoder. The observable sharp edge is scalar-based processing: the scratch probe printed `combining=xn--e-xbb.example` for `e\u{301}.example`, rather than treating the combining sequence as one grapheme.  
SUGGESTED: Punycode processes Unicode scalar values, not grapheme clusters; combining sequences are encoded as their individual scalars.

### 3. The parameter does not state empty-input behavior
UNIT:      man-page:encoding/punycodeEncode  
CLAIM:     "The Unicode domain name to encode."  
VERDICT:   incomplete  
EVIDENCE:  The parameter description omits the required empty-input result. The scratch probe printed `empty=[]` for `encoding::punycodeEncode("")`; `func_punycode_encode.rs:BODY` preserves the empty split/rejoin result.  
SUGGESTED: The text to Punycode-encode, with labels separated by ASCII `.`. An empty string returns an empty string.