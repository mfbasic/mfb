### 1. Compiler-model callback explanation
UNIT:      man-page:collections/partition  
CLAIM:     “Note that a lambda passed here may not capture an outer `MUT` binding; the callback position proven non-escaping is `collections::forEach`, not `partition`.”  
VERDICT:   out-of-scope  
EVIDENCE:  `src/codegen/builtins/mod.rs:is_nonescaping_callback_arg` defines the compiler-only “non-escaping” classification and names `forEach` as its sole member. The restriction is developer-visible, but the proof/classification rationale requires a compiler mental model.  
SUGGESTED: “A lambda passed to `partition` cannot capture an outer `MUT` binding.”

### 2. Internal memory/error-classification rationale
UNIT:      man-page:collections/partition  
CLAIM:     “Doing so needs memory, but running out of memory is not a trappable domain error, and the `append` it uses is classified infallible for exactly that reason.”  
VERDICT:   out-of-scope  
EVIDENCE:  `src/codegen/builtins/collections/func_partition.rs:BODY` shows the implementation’s `collections::append` calls; whether that operation is internally classified infallible is not a developer-facing contract. Scratch probe `/tmp/plan-125-scratch/C-phase4/man-page-collections-partition/probe` built and ran with the specified binary, printing `4 2 2 3 5 -1 0 two`; it provides no user-visible error from successful partitioning.  
SUGGESTED: “`partition` does not mutate `value`; it builds two new lists.”