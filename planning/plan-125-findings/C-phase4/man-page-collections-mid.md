### 1. `value` omits empty-list behavior
UNIT:      man-page:collections/mid
CLAIM:     “The list to take a run from. Not modified — you get a new list back.”
VERDICT:   incomplete
EVIDENCE:  `lower_list_mid` in `src/codegen/collection/search/builder_search.rs` permits `start = 0, count = 0` for an empty list and rejects `count = 1`. Probe printed `empty-list-boundary=OK 0` and `empty-list-past=ERR 77050001`.
SUGGESTED: “The list to take a run from. It may be empty; then only `start := 0, count := 0` is valid. Not modified — you get a new list back.”

### 2. `start` omits its valid range and negative behavior
UNIT:      man-page:collections/mid
CLAIM:     “The zero-based index of the first element to take.”
VERDICT:   incomplete
EVIDENCE:  `CodeBuilder::lower_list_mid` rejects negative `start` and `start > len(value)` with `ErrIndexOutOfRange`. The probe printed `negative-start=ERR 77050001` and `past=ERR 77050001`.
SUGGESTED: “The zero-based index of the first element to take. It must be from `0` through the length of `value` inclusive; a negative or larger value raises `ErrIndexOutOfRange`.”

### 3. `count` omits its valid lower bound
UNIT:      man-page:collections/mid
CLAIM:     “How many elements to take. `start + count` must not exceed the length — this raises rather than clamping.”
VERDICT:   incomplete
EVIDENCE:  `CodeBuilder::lower_list_mid` checks `count >= 0` before checking the sum. The probe printed `negative-count=ERR 77050001`.
SUGGESTED: “How many elements to take. It must be `0` or greater, and `start + count` must not exceed the length; either violation raises `ErrIndexOutOfRange` rather than clamping.”

### 4. Storage-model explanation leaks into the page
UNIT:      man-page:collections/mid
CLAIM:     “The result is a new, independent list of the same type as `value`; `value` itself is neither modified nor closed, and its elements are copied into the new list rather than shared with `value`.”
VERDICT:   out-of-scope
EVIDENCE:  `.ai/man-content.md` §4 limits `alias` reasoning to `RES` handles and directs pages to state developer-observable behavior rather than explain sharing. `collections::mid` is generic (`List OF T`) in `func_mid.rs`; “closed” and “rather than shared” add an unnecessary ownership-style model for ordinary list values.
SUGGESTED: “The result is a new list of the same type as `value`; `value` is not modified.”