### 1. `removeAt` now has an in-place assignment path
UNIT:      man-page:collections/removeAt  
CLAIM:     “Unlike `append`, `prepend`, and `set`, there is no cheap in-place shape for `removeAt`: every call copies the list.”  
VERDICT:   wrong  
EVIDENCE:  `src/codegen/collection/assign/builder_inplace_assign.rs:try_inplace_remove_at_assign` recognizes `xs = collections::removeAt(xs, i)` and calls `lower_list_remove_at_in_place`; `src/codegen/collection/list/list_mutate.rs:lower_list_remove_at_in_place` shifts elements in the existing buffer without creating a replacement list.  
SUGGESTED: Delete this implementation-detail sentence. The preceding explanation of the returned shortened list is the developer-visible contract.

### 2. Allocation-order detail is out of scope
UNIT:      man-page:collections/removeAt  
CLAIM:     “The bounds test runs before the result is built, so a rejected index builds nothing.”  
VERDICT:   out-of-scope  
EVIDENCE:  This describes lowering/allocation order rather than behavior a MFBASIC developer needs. The observable condition is already stated: an invalid index raises `ErrIndexOutOfRange`.  
SUGGESTED: “An out-of-range `index` raises `ErrIndexOutOfRange`.”