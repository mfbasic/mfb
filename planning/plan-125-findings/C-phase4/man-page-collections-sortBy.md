### 1. Single-element callback claim is false
UNIT:      man-page:collections/sortBy
CLAIM:     "In that case `keyFn` is never called, because the key pass is skipped along with the sort."
VERDICT:   wrong
EVIDENCE:  `/tmp/plan-125-scratch/C-phase4/man-page-collections-sortBy/probe.mfb`, built and run with the required release binary, printed `short-calls=1` after sorting `[]` and `[9]`; `src/codegen/builtins/collections/func_sort_by.rs:lower_collection_sortby_call` builds keys before its merge loop.
SUGGESTED: "For an empty list, `keyFn` is not called. For a one-element list, `keyFn` is called once and the element remains in place."

### 2. Callback failures are undocumented
UNIT:      man-page:collections/sortBy
CLAIM:     "The key projection, applied once to each element."
VERDICT:   incomplete
EVIDENCE:  The same probe’s `boom` key function calls `FAIL error(77050002, "boom")`; sorting `[1, 2]` printed `caught=boom`, proving the callback error reaches the caller. `func_sort_by.rs:BODY` invokes `collections::transform(value, keyFn)` without handling callback errors.
SUGGESTED: "If `keyFn` raises an error for an element, `sortBy` raises that error and returns no list."

### 3. Rendered description loses a sentence boundary
UNIT:      man-page:collections/sortBy
CLAIM:     "The elements themselves are never compared; only the keys are.`keyFn` is applied to the whole list up front, in one pass, via `collections::transform`, producing a parallel list of keys."
VERDICT:   misleading
EVIDENCE:  The required render prints `only the keys are.keyFn is applied`, with no separating whitespace. The adjacent string text in `src/codegen/builtins/collections/func_sort_by.rs:DESC` is `are.\`keyFn\``.
SUGGESTED: "The elements themselves are never compared; only the keys are. `keyFn` is applied once to each element, in index order, before comparisons begin."

### 4. Sorting implementation details leak into the developer page
UNIT:      man-page:collections/sortBy
CLAIM:     "The sort is a bottom-up merge sort with O(n log n) comparisons, merging runs of width 1, then 2, then 4, and so on."
VERDICT:   out-of-scope
EVIDENCE:  The rendered page names the algorithm and its internal run widths; `.ai/man-content.md` §3 excludes lowering/codegen mechanics from this developer-facing surface. `func_sort_by.rs:BODY` confirms these are implementation details (`width`, `lo`, `mid`, merge buffers), not required call semantics.
SUGGESTED: "The sort uses O(n log n) comparisons and keeps elements with equal keys in their input order."

### 5. Internal merge-location diagnostic explanation is out of scope
UNIT:      man-page:collections/sortBy
CLAIM:     "A `U` the `<` operator does not accept is a compile-time `TYPE_BINARY_OPERATOR_MISMATCH` error, reported once the element type is known, against the key comparison inside the merge."
VERDICT:   out-of-scope
EVIDENCE:  “Against the key comparison inside the merge” requires knowledge of the implementation. `src/codegen/builtins/collections/func_sort_by.rs:BODY` contains that generated comparison; `.ai/man-content.md` §3 prohibits this compiler mental model on a man page.
SUGGESTED: "If `keyFn` returns a type that cannot be ordered with `<`, the program does not compile."