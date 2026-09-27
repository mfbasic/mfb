### 1. Run-together description sentences
UNIT:      man-page:collections/drop
CLAIM:     “collections::drop returns a new list containing everything in value except its leading count elements, in their original order.drop(value, count) is defined as the half-open range [count, len(value)) of value.”
VERDICT:   misleading
EVIDENCE:  Rendering `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man collections drop` prints `order.drop(value, count)`. `src/codegen/builtins/collections/func_drop.rs:DESC` likewise has no separating whitespace after `order.`.
SUGGESTED: `collections::drop returns a new list containing everything in value except its first count elements, in their original order.`

### 2. Implementation mechanism presented as behavior
UNIT:      man-page:collections/drop
CLAIM:     “The remaining elements are copied as a bulk range copy, and that is what defines the boundary behavior: the range start is clamped into [0, len] and the range stop into [start, len].”
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_drop.rs:BODY` delegates to `__collections_slice`; `src/codegen/builtins/collections/gen_slice.rs:CodeBuilder::lower_list_slice_range` implements the bulk-copy and internal start/stop clamping. The rendered page exposes that lowering mechanism, which a developer does not need. Probe compiled and ran with the specified release binary: negative, zero, one, length, and oversized counts printed `3,3,20,30,0,0,10`.
SUGGESTED: `A count at or below 0 returns a new list with every element; a count at or above the list length returns an empty list. Otherwise, the result contains the elements after the first count elements.`