### 1. `RES` elements contradict independence claim
UNIT:      man-page:collections/take  
CLAIM:     “The result is a new list; the elements are copied into it, so nothing you do with the result affects `value`. `value` is not modified.”  
VERDICT:   wrong  
EVIDENCE:  Probe opened an `fs::File`, put it in `List OF RES fs::File`, called `take`, closed the selected handle from the result, then read through the original handle. It printed `error=77030004` (`ErrResourceClosed`). `src/codegen/builtins/collections/func_take.rs:register` permits any `T`, including `RES` element types.  
SUGGESTED: The result is a new list and `value` is not modified. For ordinary values, changing the result does not change `value`. If the elements are `RES` handles, corresponding handles in the two lists are aliases: closing one closes the same handle in the other list.

### 2. Implementation mechanics and malformed first description sentence
UNIT:      man-page:collections/take  
CLAIM:     “`collections::take` returns a new list containing the leading `count` elements of `value`, in their original order.`take(value, count)` is defined as the half-open range `[0, count)` of `value`, The selected elements are copied as a bulk range copy, and that is what defines the boundary behavior: the range start is clamped into `[0, len]` and the range stop into `[start, len]`.”  
VERDICT:   out-of-scope  
EVIDENCE:  Rendered by `mfb man collections take`; it exposes “bulk range copy” and internal range-normalization mechanics, and renders `order.take` without a separating space. The actual behavior was verified by a probe printing `0,0,1,2,4,0` for negative, zero, ordinary, oversized, and empty-source calls; `src/codegen/builtins/collections/gen_slice.rs:lower_list_slice_range` implements the clamping.  
SUGGESTED: `collections::take` returns a new list containing up to `count` leading elements from `value`, in their original order. A count at or below zero returns an empty list; a count at or above the list length returns the whole list.