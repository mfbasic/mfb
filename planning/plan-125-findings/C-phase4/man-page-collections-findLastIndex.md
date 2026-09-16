### 1. Scan bound is called the wrong thing
UNIT:      man-page:collections/findLastIndex  
CLAIM:     "Index of the last element at or before an end position that satisfies a predicate"  
VERDICT:   misleading  
EVIDENCE:  `src/codegen/builtins/collections/func_find_last_index.rs:BODY` accepts `start`, validates it with `start < 0 OR start >= len(value)`, then scans downward. The rendered parameter table likewise defines `start` as the backward scan origin; no `end` position exists.  
SUGGESTED: "Index of the last element at or before a start position that satisfies a predicate"

### 2. Compiler-proof explanation leaks into the developer page
UNIT:      man-page:collections/findLastIndex  
CLAIM:     "Note that a lambda passed here may not capture an outer `MUT` binding; the callback position proven non-escaping is `collections::forEach`, not `findLastIndex`."  
VERDICT:   out-of-scope  
EVIDENCE:  The first clause is confirmed by compiling `/tmp/plan-125-scratch/C-phase4/man-page-collections-findLastIndex/src/main.mfb` with `mfb build`: `LAMBDA(n AS Integer) -> n > threshold`, where `threshold` is `MUT`, fails with `TYPE_LAMBDA_CAPTURE_UNSUPPORTED: Lambda captures mutable local 'threshold'; mutable captures are invalid.` The “proven non-escaping callback position” rationale is compiler terminology prohibited by `.ai/man-content.md` §3.  
SUGGESTED: "A lambda passed here cannot use a mutable binding from the surrounding function."