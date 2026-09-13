### 1. Core binding forms are undocumented
UNIT:      man-topic:link
PAGE:      package-wide
CATEGORY:  coverage
CLAIM:     The topic lists `LINK`, `RESOURCE`, `SYMBOL`, `ABI`, `CONST`, `SUCCESS_ON`, `ERROR_ON`, `RESULT`, and `FREE`, but never explains `CSTRUCT`, `BIND IN`, `BIND STATE`, or `BUFFER`.
VERDICT:   missing
EVIDENCE:  `rg -n "CSTRUCT|BIND IN|BIND STATE|BUFFER" src/ast/link_items.rs` shows `Parser::parse_cstruct`, `Parser::parse_bind_in`, `Parser::parse_bind_state`, and `Parser::parse_link_function` accepting all four forms; the rendered page only exposes them incidentally in diagnostic names.
SUGGESTED: Add a “Structured data and buffers” section with syntax and a minimal example for `CSTRUCT`, `BIND IN`, `BIND STATE`, and `BUFFER`; link it from the opening list of binding forms.

### 2. The page teaches removed `RESULT` syntax
UNIT:      man-topic:link
PAGE:      Native functions
CATEGORY:  overview-mismatch
CLAIM:     “A value-returning wrapper must expose exactly one result with `return` or a `RESULT` expression.”
VERDICT:   wrong
EVIDENCE:  `mfb build /tmp/plan-125-scratch/B-phase1/man-topic-link/result-project` rejects `RESULT value` with `MFB_PARSE_UNEXPECTED_STATEMENT` and says native functions permit `RETURN`; the equivalent `RETURN value` probe passes parsing and reaches only the expected missing-library manifest error. `Parser::parse_link_function` in `src/ast/link_items.rs` likewise states that `RETURN <expr>` replaced `RESULT`.
SUGGESTED: “A value-returning wrapper must expose exactly one result with `RETURN <expression>`. The expression may name an ABI slot or compute a result from ABI slots.”