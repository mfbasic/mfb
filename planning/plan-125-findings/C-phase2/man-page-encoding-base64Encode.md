### 1. Input mutation is unspecified
UNIT:      man-page:encoding/base64Encode
CLAIM:     “The bytes to encode.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_base64_encode.rs:__encoding_base64Encode` delegates to `__encoding_baseEncode`; `src/codegen/builtins/encoding/helper_base_encode.rs:__encoding_baseEncode` only iterates `FOR EACH b IN data` and never changes `data`. Probe output for `[102, 111]` was `Zm8=`, then `102` and `111` when reread after the call.
SUGGESTED: The bytes to encode. The list may be empty, and this call does not mutate it.