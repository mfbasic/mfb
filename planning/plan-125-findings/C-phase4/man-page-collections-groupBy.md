### 1. Missing out-of-memory error
UNIT:      man-page:collections/groupBy  
CLAIM:     "`groupBy` itself raises no error of its own."  
VERDICT:   wrong  
EVIDENCE:  `src/codegen/builtins/collections/func_group_by.rs:CodeBuilder::lower_collection_group_by_call` calls `raise_error_bare("ErrOutOfMemory")` for its scratch and bucket allocations; its generic `BODY` also constructs projected lists, buckets, and a map.  
SUGGESTED: `groupBy` raises no trappable domain error. Building the projected lists, buckets, and result needs memory; running out of memory cannot be caught by TRAP.`

### 2. Callback failure cannot discard a partially built map
UNIT:      man-page:collections/groupBy  
CLAIM:     "If either callback fails, its error propagates out of `groupBy` to the caller and can be caught by the caller's `TRAP` block; the partially built map is discarded."  
VERDICT:   misleading  
EVIDENCE:  `src/codegen/builtins/collections/func_group_by.rs:BODY` runs both `collections::transform` calls before it creates `result`; `CodeBuilder::lower_collection_group_by_call` likewise lowers both transforms before its element/bucket loop. The rendered third example compiled and ran from `/tmp/plan-125-scratch/C-phase4/man-page-collections-groupBy/run-example3`, printing `failed: 77050002`.  
SUGGESTED: `If either callback fails, its error propagates to the caller and can be caught by the caller's TRAP block. No result map is produced.`

### 3. Result-map ordering is omitted
UNIT:      man-page:collections/groupBy  
CLAIM:     "Because the walk proceeds in list order and each value is appended to the end of its bucket, the items inside a bucket appear in the same relative order they had in `value`."  
VERDICT:   incomplete  
EVIDENCE:  The probe at `/tmp/plan-125-scratch/C-phase4/man-page-collections-groupBy/run-probe` printed `keys=1,0` for input `[3, 2, 1, 4]`, matching first occurrence of each key, as well as stable buckets `bucket1=30,10` and `bucket0=20,40`. `src/codegen/builtins/collections/func_group_by.rs:BODY` inserts each key’s bucket on first use; `CodeBuilder::lower_collection_group_by_call` preserves first-seen keys through `keyOrder`. The page says nothing about the resulting map’s iteration order.  
SUGGESTED: `The result has one bucket per distinct key. Map iteration order is stable for an unchanged map but implementation-defined; current builds insert groups in the order each key first appears in value.`

### 4. Compiler-inference explanation is out of scope
UNIT:      man-page:collections/groupBy  
CLAIM:     "Calling it with two arguments is a compile-time error, because the compiler cannot infer the template argument `V` (it appears only in the return type)."  
VERDICT:   out-of-scope  
EVIDENCE:  `.ai/man-content.md` §3 bans compiler and template-implementation explanations from `mfb man`; the rendered page uses both to explain an ordinary call requirement.  
SUGGESTED: `Pass both a key projection and a value projection. To keep the original items, pass an identity FUNC as valFn.`

### 5. Generic-template implementation detail is out of scope
UNIT:      man-page:collections/groupBy  
CLAIM:     "`groupBy` is generic over three template parameters: `T`, the element type of `value`; `K`, the key type returned by `keyFn`; and `V`, the value type returned by `valFn`."  
VERDICT:   out-of-scope  
EVIDENCE:  `.ai/man-content.md` §3 excludes compiler/template vocabulary from developer-facing pages. The declaration already expresses the developer-visible types.  
SUGGESTED: `The return map uses keys produced by keyFn and bucket items produced by valFn.`