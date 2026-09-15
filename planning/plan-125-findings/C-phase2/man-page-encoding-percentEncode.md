### 1. Return/mutation behavior omitted
UNIT:      man-page:encoding/percentEncode
CLAIM:     "`encoding::percentEncode` percent-encodes text following the RFC 3986 rules for the unreserved character set."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_percent_encode.rs:BODY` builds and returns `out` without changing `text`. Scratch probe printed `a b/c` followed by `a%20b%2Fc`, confirming the input value remains unchanged.
SUGGESTED: `encoding::percentEncode` returns a new String containing the RFC 3986 percent-encoding of text; the input value is unchanged.