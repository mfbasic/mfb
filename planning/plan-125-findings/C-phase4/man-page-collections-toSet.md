### 1. Compiler-internal error classification
UNIT:      man-page:collections/toSet  
CLAIM:     “While building the result it needs memory, but running out of memory is not a trappable domain error, and the `add` it is built on is classified infallible for exactly that reason.”  
VERDICT:   out-of-scope  
EVIDENCE:  `src/codegen/builtins/collections/func_to_set.rs:DESC` supplies this prose; `__collections_toSet` calls `collections::add`, whose lowering is `collections::lower_add` in `src/codegen/builtins/collections/func_add.rs`. “classified infallible” explains registry/compiler categorization rather than behavior a developer needs to write a program.  
SUGGESTED: `toSet` raises no user-trappable errors. Its argument must be a \`List OF T\` whose element type is comparable.`