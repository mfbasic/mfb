### 1. Element parameter invents a predicate form
UNIT:      man-page:collections/find
CLAIM:     "The element to look for, when using the value form rather than a predicate."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/collections/func_find.rs:register` exposes only element and contiguous-sublist overloads; the rendered overload table likewise has no predicate form.
SUGGESTED: "The element to look for. Compared with `=`."

### 2. Sublist parameter is documented as an element
UNIT:      man-page:collections/find
CLAIM:     "The element to look for, when using the value form rather than a predicate. Compared with `=`."
VERDICT:   wrong
EVIDENCE:  `src/codegen/collection/search/builder_search.rs:CodeBuilder::lower_list_find_sublist` searches a contiguous sequence when `item` is `List OF T`; the published second example compiled and ran, printing `1` for `[20, 30]`.
SUGGESTED: "The contiguous sublist to look for. Its elements are compared in order."

### 3. `start` parameter omits its valid range and boundary behavior
UNIT:      man-page:collections/find
CLAIM:     "Where to begin searching, as a zero-based index. Omit it to search from the beginning."
VERDICT:   incomplete
EVIDENCE:  Probe compiled and ran with the required binary: `find([1,2,3], [], 3)` printed `3`; element searches at `3`, `-1`, and `4` recovered `77050004`, `77050001`, and `77050001`. `lower_list_find_item` and `lower_list_find_sublist` accept `0 <= start <= len(value)`.
SUGGESTED: "Where to begin searching, as a zero-based index. Omit it for `0`. It must be from `0` through `len(value)`, inclusive; negative values and values greater than `len(value)` raise `ErrIndexOutOfRange`."

### 4. `contains` is not an alternative for a sublist search
UNIT:      man-page:collections/find
CLAIM:     "It never returns a sentinel such as -1; a search that may legitimately come up empty needs a TRAP, or collections::contains if only the yes/no answer is wanted."
VERDICT:   misleading
EVIDENCE:  A probe calling `collections::contains([1, 2, 3], [2, 3])` failed to compile with `TYPE_CALL_ARGUMENT_MISMATCH`: expected `(List OF T, T)`, not a list needle. The preceding paragraph applies to both element and sublist searches.
SUGGESTED: "It never returns a sentinel such as `-1`; a search that may legitimately come up empty needs a `TRAP`. For an element search, use `collections::contains` when only the yes/no answer is wanted."

### 5. Closing and construction details are not developer-facing behavior
UNIT:      man-page:collections/find
CLAIM:     "value is neither modified nor closed, and no new collection is built."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_find.rs:register` declares `value` as `List OF T` and the return as `Integer`; lists are not closable resources, and whether a collection is built is not observable through this API.
SUGGESTED: "`find` does not modify `value`."