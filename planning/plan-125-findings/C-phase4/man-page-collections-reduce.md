### 1. Implementation cleanup detail leaks into user documentation
UNIT:      man-page:collections/reduce
CLAIM:     "No cleanup runs on that path, since the accumulator may still be the initial value you passed in."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/gen_memory.rs:CodeBuilder::lower_collection_reduce_impl` emits `emit_callback_failure_exit(None)` on reducer failure; that implementation choice is not developer-observable. The scratch `semantics.mfb` probe printed `visited=1`, `visited=2`, `caught=boom`, and `recovered=-1`, verifying the user-visible behavior already stated immediately before it.
SUGGESTED: When the reducer fails, the fold stops at that element, later elements are not visited, and the reducer’s error reaches the caller unchanged.