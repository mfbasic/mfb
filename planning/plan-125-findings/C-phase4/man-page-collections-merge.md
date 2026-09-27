### 1. Unqualified no-error claim ignores `ErrOutOfMemory`
UNIT:      man-page:collections/merge  
CLAIM:     "`merge` invokes no user callback and raises no error."  
VERDICT:   wrong  
EVIDENCE:  `src/codegen/builtins/collections/func_merge.rs:copy_map_with_capacity` calls `self.raise_error_bare("ErrOutOfMemory")` when constructing the result cannot obtain memory; the source-generic `__collections_merge` body also calls `collections::set`, which can require result growth.  
SUGGESTED: `merge invokes no user callback and raises no trappable domain error. It can still raise ErrOutOfMemory if the result cannot be created.`