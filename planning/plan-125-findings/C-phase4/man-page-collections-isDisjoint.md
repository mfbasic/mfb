### 1. Missing sentence boundary obscures the error/type contract
UNIT:      man-page:collections/isDisjoint
CLAIM:     "`isDisjoint` raises no user-trappable error of its own.Both arguments must be the same `Set OF T`."
VERDICT:   misleading
EVIDENCE:  Rendered with `mfb man collections isDisjoint`; `src/codegen/builtins/collections/func_is_disjoint.rs:DESC` contains the same missing space. The descriptor’s `errors: vec![]` confirms no declared user-trappable errors.
SUGGESTED: "`isDisjoint` raises no user-trappable errors. Both sets must have the same comparable element type."

### 2. Compiler-resolution terminology leaks into a developer page
UNIT:      man-page:collections/isDisjoint
CLAIM:     "A call whose arguments are not both sets of the same element type does not resolve and is rejected at compile time."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_is_disjoint.rs:register` declares both parameters as `Set OF T`; compiling `disjoint_mismatch.mfb` produced `TYPE_CALL_ARGUMENT_MISMATCH` for `Set OF String` passed after `Set OF Integer`. “Does not resolve” describes compiler processing rather than the developer-visible result.
SUGGESTED: "Both sets must have the same comparable element type; a call with different element types is rejected at compile time."