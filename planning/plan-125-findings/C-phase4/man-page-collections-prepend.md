### 1. List-typed elements are not always rejected
UNIT:      man-page:collections/prepend
CLAIM:     "A list-typed item is rejected explicitly as well."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/collections/func_prepend.rs:lower_prepend` accepts an item whose type is the element type `T`; `/tmp/plan-125-scratch/C-phase4/man-page-collections-prepend/nested_item` built and printed `1` for `collections::prepend([[2]], [1])`.
SUGGESTED: A list is valid as `item` when the list's element type is itself a list; only passing a whole `List OF T` as the item to a `List OF T` is rejected.

### 2. Optimization implementation leaks into the developer page
UNIT:      man-page:collections/prepend
CLAIM:     "When the compiler can prove the target is a uniquely same local being reassigned — the `list = collections::prepend(list, x)` shape, on a non-`by_ref` local that is not the live iterable of an enclosing `FOR EACH` — it inserts into the existing list rather than copying it instead of a full copy."
VERDICT:   out-of-scope
EVIDENCE:  This describes compiler proof conditions and lowering selection in `src/codegen/collection/assign/builder_inplace_assign.rs:try_inplace_prepend_assign`, including the `by_ref` and active-`FOR EACH` gates; those facts do not help a developer write a correct call.
SUGGESTED: Remove this sentence. Keep only the observable performance guidance: repeated prepends take O(n) per call.

### 3. Compiler data-structure terminology obscures the performance edge
UNIT:      man-page:collections/prepend
CLAIM:     "Note that prepending must shift every existing lookup entry right by one, so a repeated prepend stays O(n) per call even on the in-place path, unlike `append`."
VERDICT:   out-of-scope
EVIDENCE:  “lookup entry” and “in-place path” describe the implementation in `src/codegen/collection/list/list_mutate.rs:lower_list_prepend_in_place`; the runtime probe built from `/tmp/plan-125-scratch/C-phase4/man-page-collections-prepend/probe` printed `2 2 1 2 3 3 5`, confirming the public ordering behavior without requiring that model.
SUGGESTED: "Each prepend shifts existing elements one position later, so repeated prepends take O(n) per call; use `append` when building a list in forward order."

### 4. `value` omits its empty-list behavior in the parameter table
UNIT:      man-page:collections/prepend
CLAIM:     "The list to prepend to. Not modified — you get a new list back."
VERDICT:   incomplete
EVIDENCE:  The page later says an empty list yields a one-element result, and the probe at `/tmp/plan-125-scratch/C-phase4/man-page-collections-prepend/probe` printed original length `2`, result length `3`, and result order `1 2 3`; the parameter row itself does not state that an empty list is valid or its result.
SUGGESTED: "The list to prepend to. It may be empty; prepending to an empty list returns a one-element list. The input list is not modified — you get a new list back."