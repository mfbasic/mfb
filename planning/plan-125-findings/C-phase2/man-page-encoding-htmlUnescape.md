### 1. Surrogate numeric references are not accepted
UNIT:      man-page:encoding/htmlUnescape
CLAIM:     “Any code point in the range 0–1114111 (0x10FFFF) is accepted, including surrogate values, which are not screened out.”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/encoding/helper_from_codepoint.rs:__encoding_fromCodepoint` constructs surrogate UTF-8 bytes, then `toString(List OF Byte)` validates them. Probe `io::print(encoding::htmlUnescape("&#55296;"))` compiled with the specified release binary and printed `Error: 7-702-0004` / `Text encoding or decoding failed.`
SUGGESTED: Numeric references from 0 through 1114111 are accepted except surrogate values (55296–57343), which raise `ErrEncoding`.

### 2. Raisable errors are omitted from the Errors table
UNIT:      man-page:encoding/htmlUnescape
CLAIM:     “The function is not total: it fails on a reference that has no ; terminator, on a numeric reference whose digits are empty or non-numeric, on an unknown entity name, and on a numeric reference whose value exceeds 1114111.”
VERDICT:   incomplete
EVIDENCE:  The rendered page has no Errors table because `src/codegen/builtins/encoding/func_html_unescape.rs:register` declares `errors: vec![]`. The body raises `77050003` for malformed, empty, nonnumeric, unknown, and out-of-range entities; probe `encoding::htmlUnescape("&#;")` printed `Error: 7-705-0003` / `unknown entity`. The surrogate probe also raises `ErrEncoding` (`7-702-0004`).
SUGGESTED: Add derived Errors rows for `ErrInvalidFormat` (missing terminator, invalid/empty numeric digits, unknown name, or value above 1114111) and `ErrEncoding` (a numeric surrogate reference), and state those conditions in the description.