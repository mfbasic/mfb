### 1. Empty-input behavior is omitted for `a`
UNIT:      man-page:collections/intersection
CLAIM:     "The first set, walked to decide element order. Not modified. T must be a comparable type."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/collections/func_intersection.rs:__collections_intersection` initializes an empty result and only adds elements while iterating `a`; therefore an empty `a` returns an empty set.
SUGGESTED: The first set, walked to decide element order. Not modified. If it is empty, the result is empty. T must be a comparable type.

### 2. Empty-input behavior is omitted for `b`
UNIT:      man-page:collections/intersection
CLAIM:     "The second set, of the same type as a, tested for membership. Not modified."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/collections/func_intersection.rs:__collections_intersection` adds an element only when `collections::contains(b, x)` succeeds; an empty `b` therefore produces an empty result.
SUGGESTED: The second set, of the same type as a, tested for membership. Not modified. If it is empty, the result is empty.

### 3. Implementation detail, and two sentences are accidentally joined
UNIT:      man-page:collections/intersection
CLAIM:     "Running out of memory is not a trappable domain error, and the add it is built on is classified infallible.Both arguments must be the same Set OF T."
VERDICT:   out-of-scope
EVIDENCE:  Rendered with `mfb man collections intersection`; `src/codegen/builtins/collections/func_intersection.rs:DESC` contains the same missing space. The claim that the function is built on `add` and that `add` is “classified infallible” is an implementation concern, not terminal-user guidance.
SUGGESTED: `intersection` raises no user-trappable errors. Both arguments must be `Set OF T` with the same element type.

