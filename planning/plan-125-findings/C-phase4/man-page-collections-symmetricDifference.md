### 1. Implementation detail in behavior description
UNIT:      man-page:collections/symmetricDifference
CLAIM:     "It is computed as a two-pass fold: it keeps each element of a that collections::contains reports as absent from b, then adds each element of b that is absent from a."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_symmetric_difference.rs:BODY` implements this loop, but its fold structure is not needed to use the function.
SUGGESTED: Remove this sentence.

### 2. Internal error-classification explanation
UNIT:      man-page:collections/symmetricDifference
CLAIM:     "Running out of memory is not a trappable domain error, and the add it is built on is classified infallible."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_symmetric_difference.rs:register` declares `errors: vec![]`; `BODY` calls `collections::add`, but its internal classification is not a developer-facing contract.
SUGGESTED: "symmetricDifference raises no user-trappable error of its own."

### 3. Compiler-resolution mental model
UNIT:      man-page:collections/symmetricDifference
CLAIM:     "T is inferred from the element type and must be comparable, which every Set OF T already requires."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_symmetric_difference.rs:register` expresses this through generic `ParameterType::var(\"T\")`; type inference is a compiler mechanism rather than information needed to call the function.
SUGGESTED: "Both arguments must be sets with the same comparable element type."

### 4. Compiler-resolution wording
UNIT:      man-page:collections/symmetricDifference
CLAIM:     "A call whose arguments are not both sets of the same element type does not resolve and is rejected at compile time."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_symmetric_difference.rs:register` requires two `Set OF T` parameters; “does not resolve” describes compiler dispatch rather than the developer-facing requirement.
SUGGESTED: "Both arguments must be sets with the same element type."