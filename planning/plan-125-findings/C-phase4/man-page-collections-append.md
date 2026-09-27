### 1. Nested-list parameter claim is false
UNIT:      man-page:collections/append
CLAIM:     "The element to add at the end. Appending a list appends every one of its elements, not the list itself."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/collections/func_append.rs:register` applies this text to overload 1 (`item AS T`). Scratch probe `nested/src/main.mfb` compiled and printed `outer=2` and `second=2` for `collections::append([[1, 2]], [3, 4])`: the list `[3, 4]` was appended as one `List OF Integer` element.
SUGGESTED: "The single element to add at the end. If the element type is itself a list, that list is added as one element."

### 2. `append` is not infallible
UNIT:      man-page:collections/append
CLAIM:     "`append` is **infallible**: nothing it does raises a trappable error."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/collections/gen_mutate.rs:lower_collection_end_insert` calls `lower_list_insert_collection`; `src/codegen/collection/list/list_mutate.rs:lower_list_insert_collection` calls `emit_arena_alloc_call()` and raises `ErrOutOfMemory` on allocation failure and size overflow.
SUGGESTED: "`append` may raise `ErrOutOfMemory` if creating the result cannot complete." Add the corresponding Errors-table entry and remove the claim that an inline `TRAP` handler can never run.

### 3. Overload-resolution implementation detail
UNIT:      man-page:collections/append
CLAIM:     "The compiler picks the overload from the static type of that argument: an argument whose type is exactly the element type appends one element, and an argument whose type is exactly the same list type concatenates."
VERDICT:   out-of-scope
EVIDENCE:  `.ai/man-content.md` §3 bans compiler vocabulary and compiler mechanics on `mfb man` pages. The behavior is represented by the two public signatures in `src/codegen/builtins/collections/func_append.rs:register`; the mechanism used to select them is not terminal-user documentation.
SUGGESTED: "Pass an element to add one element, or pass another `List OF T` to add that list's elements."

### 4. Reassignment guidance omits the required `MUT` binding
UNIT:      man-page:collections/append
CLAIM:     "Assigning straight back to the same local variable — `list = collections::append(list, x)` — is the cheap shape: it grows the list instead of copying it, so appending in a loop costs about the same per element however long the list gets."
VERDICT:   misleading
EVIDENCE:  Scratch probe `nonmut/src/main.mfb` compiled with the supplied release binary fails at that assignment with `TYPE_ASSIGN_REQUIRES_MUT: assignment target must be mutable`. The optimization also explicitly requires a uniquely owned `MUT` local in `src/codegen/collection/assign/builder_inplace_assign.rs:try_inplace_append_assign`.
SUGGESTED: "To retain the result while building a list in a loop, assign it back to a `MUT` binding: `list = collections::append(list, x)`."