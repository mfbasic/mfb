### 1. Grapheme-cluster sharp edge omitted
UNIT:      man-page:encoding/utf32Encode  
CLAIM:     "The scalars are produced by decoding the string's UTF-8 bytes in order: each 1-to-4-byte sequence contributes exactly one code point, so the returned list has one element per Unicode scalar in value (which may be fewer than its byte length)."  
VERDICT:   incomplete  
EVIDENCE:  Scratch probe `/tmp/plan-125-scratch/C-phase2/man-page-encoding-utf32Encode/src/main.mfb`, built and run with the specified release binary, printed `count=2`, `101`, `769` for `"é"`: one user-perceived grapheme made from `e` plus a combining accent produces two elements. `src/codegen/builtins/encoding/helper_codepoints.rs:__encoding_codepoints` emits one result per decoded scalar.  
SUGGESTED: `The result has one element per Unicode scalar, not per grapheme cluster; for example, an e followed by a combining accent produces two elements.`

### 2. Parameter description omits empty-input behavior
UNIT:      man-page:encoding/utf32Encode  
CLAIM:     "The string to encode."  
VERDICT:   incomplete  
EVIDENCE:  The parameter description in `src/codegen/builtins/encoding/func_utf32_encode.rs:register` gives no accepted-input boundary or empty-input result. The scratch probe above printed `count=0` for `encoding::utf32Encode("")`; `__encoding_codepoints` returns its initially empty list when `len(data)` is zero.  
SUGGESTED: `The string to encode. Any String is accepted; an empty string returns an empty list.`