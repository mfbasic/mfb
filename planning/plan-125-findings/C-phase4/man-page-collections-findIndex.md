### 1. Compiler-model explanation leaks into the page

UNIT:      man-page:collections/findIndex  
CLAIM:     “Note that a lambda passed here may not capture an outer `MUT` binding; the callback position proven non-escaping is `collections::forEach`, not `findIndex`.”  
VERDICT:   out-of-scope  
EVIDENCE:  `src/codegen/builtins/mod.rs:is_nonescaping_callback_arg` names only `collections::forEach` argument 1 as non-escaping. The scratch probe with a `MUT threshold` captured by a `findIndex` lambda fails to compile with `TYPE_LAMBDA_CAPTURE_UNSUPPORTED: Lambda captures mutable local 'threshold'; mutable captures are invalid.` This is true behavior, but “proven non-escaping callback position” explains compiler reasoning rather than what a terminal user needs.  
SUGGESTED: “A predicate lambda cannot use an outer `MUT` binding. See `mfb man lambda` for lambda rules. `findIndex` does not mutate `value`.”