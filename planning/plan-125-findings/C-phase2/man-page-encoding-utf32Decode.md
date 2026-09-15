### 1. Parameter omits required validity contract
UNIT:      man-page:encoding/utf32Decode
CLAIM:     “The Unicode scalar values to decode.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_utf32_decode.rs:register` supplies this description, while `BODY` rejects `cp < 0`, `cp > 1114111`, and inclusive `55296..57343`. The `/tmp/plan-125-scratch/C-phase2/man-page-encoding-utf32Decode/probes` probe with `[-1]` printed `Error: 7-705-0003` / `invalid code point`.
SUGGESTED: `Unicode scalar values in list order. Each must be 0..1114111 inclusive and not 55296..57343; invalid elements raise ErrInvalidFormat.`

### 2. Raisable error is absent from the rendered Errors table
UNIT:      man-page:encoding/utf32Decode
CLAIM:     “Any such element fails rather than producing replacement text.”
VERDICT:   incomplete
EVIDENCE:  Rendered `mfb man encoding utf32Decode` has no Errors section. `src/codegen/builtins/encoding/func_utf32_decode.rs:BODY` executes `FAIL error(77050003, ...)`, but `register` declares `errors: vec![]`. The surrogate probe `[55296]` printed `Error: 7-705-0003` / `surrogate code point`.
SUGGESTED: `Add ErrInvalidFormat (77050003) to the descriptor’s Errors table: raised when an element is negative, greater than 1114111, or in 55296..57343 inclusive.`

### 3. Ordering and non-mutation are left implicit
UNIT:      man-page:encoding/utf32Decode
CLAIM:     “encoding::utf32Decode interprets value as a sequence of UTF-32 code points and returns the corresponding text.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_utf32_decode.rs:BODY` iterates `FOR EACH cp IN value` and only appends to local `out`; it neither reorders nor mutates `value`. The page does not state either developer-visible sharp edge.
SUGGESTED: `It decodes elements in list order and returns the resulting text without mutating value.`