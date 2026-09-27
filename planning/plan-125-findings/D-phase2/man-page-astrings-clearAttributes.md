### 1. Missing Errors table
UNIT:      man-page:astrings/clearAttributes
CLAIM:     “An invalid range raises.”
VERDICT:   incomplete
EVIDENCE:  Rendered with `mfb man astrings clearAttributes`: it contains no Errors table. `src/codegen/builtins/astrings/func_clear_attributes.rs:register` declares `errors: vec![]` for both overloads, while `__astrings_validateRange` raises `77050002` (“invalid attribute range”) and `77050001` (“attribute range out of bounds”). Scratch probes printed those two errors respectively for `clearAttributes("x", -1, 0)` and `clearAttributes("", 0, 0)`.
SUGGESTED: Add derived Errors rows: “ErrInvalidArgument — `start` is negative or greater than `endIndex`.” and “ErrIndexOutOfRange — either inclusive bound lies outside the visible text.”