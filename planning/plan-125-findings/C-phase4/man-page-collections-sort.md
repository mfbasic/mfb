### 1. String ordering is underspecified
UNIT:      man-page:collections/sort
CLAIM:     "For `String` it is the ordering the `<` operator defines on strings"
VERDICT:   incomplete
EVIDENCE:  `func_sort.rs:lower_collection_sort_call` and `CodeBuilder::emit_index_string_less_branch` compare strings as unsigned UTF-8 bytes, with a proper prefix first. `probe-order` sorted `["z", "á", "a", "aa"]` and printed `a`, `z`, `á`; thus `z` precedes `á`, which is not locale collation.
SUGGESTED: For `String`, ordering is unsigned UTF-8 byte order: it is neither locale-aware nor case-insensitive.

### 2. Merge-pass mechanics leak implementation detail
UNIT:      man-page:collections/sort
CLAIM:     "Runs of width 1 are merged into runs of width 2, then 4, and so on, until a single run covers the list."
VERDICT:   out-of-scope
EVIDENCE:  This describes the current implementation in `src/codegen/builtins/collections/func_sort.rs:BODY` and its native fast path, not behavior a developer needs to write correct MFBASIC. The developer-visible guarantees are ascending order, stability, and complexity.
SUGGESTED: Sorting uses O(n log n) comparisons and is stable.

### 3. Tie-handling implementation is overexplained
UNIT:      man-page:collections/sort
CLAIM:     "The merge is stable: when a left-run element and a right-run element compare equal, the left-run element is emitted first, because the right-run element is taken only when it is strictly less than the left-run element."
VERDICT:   out-of-scope
EVIDENCE:  The left/right-run decision is an implementation mechanism in `func_sort.rs:BODY`; only the resulting stability guarantee matters to a caller.
SUGGESTED: Sorting is stable: elements that compare equal retain their original relative order.

### 4. Generic-instantiation explanation is compiler-facing
UNIT:      man-page:collections/sort
CLAIM:     "The constraint is enforced once the element type is known, when the generic body has been given a concrete element type."
VERDICT:   out-of-scope
EVIDENCE:  This requires a compiler mental model of generic-body specialization. The observed behavior is confirmed by `probe-invalid`, which fails with `TYPE_BINARY_OPERATOR_MISMATCH` for `List OF Boolean`; `func_sort.rs:BODY` performs `<` on elements.
SUGGESTED: The element type must support the `<` operator.

### 5. Diagnostic location exposes the implementation
UNIT:      man-page:collections/sort
CLAIM:     "Sorting a list whose element type the `<` operator does not accept — a `Boolean`, a record, a nested `List`, or a `Map` — is a compile-time `TYPE_BINARY_OPERATOR_MISMATCH` error reported against the comparison inside the merge, not a runtime failure."
VERDICT:   out-of-scope
EVIDENCE:  `probe-invalid` fails at compile time with `TYPE_BINARY_OPERATOR_MISMATCH`, but its reported location is generated sort-body detail (`main.mfb:43`), not a useful source-level contract for the caller.
SUGGESTED: A list whose element type does not support `<`, such as `Boolean`, a record, a nested `List`, or a `Map`, does not compile.