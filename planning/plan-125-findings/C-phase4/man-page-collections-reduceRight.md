### 1. Callback failure stops traversal
UNIT:      man-page:collections/reduceRight
CLAIM:     "`f` is called exactly once per item, so an empty `value` calls `f` not at all and returns `initial` unchanged."
VERDICT:   wrong
EVIDENCE:  Probe `/tmp/plan-125-scratch/C-phase4/man-page-collections-reduceRight/src/main.mfb`, built and run with the specified release binary, called a reducer that prints then `FAIL`s: `callback=3`, `caught=stop`, `failure-result=-1`. Items `2` and `1` were not visited. `src/codegen/builtins/collections/gen_memory.rs:CodeBuilder::lower_collection_reduce_impl` exits immediately through `emit_callback_failure_exit` when the callback fails.
SUGGESTED: When `f` succeeds for every item, it is called once per item. An empty `value` calls `f` not at all and returns `initial` unchanged. If `f` fails, traversal stops and its error propagates.