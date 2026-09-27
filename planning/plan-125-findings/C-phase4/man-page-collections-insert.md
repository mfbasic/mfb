### 1. False “every call copies” claim
UNIT:      man-page:collections/insert
CLAIM:     "Unlike append, prepend, and set, there is no cheap in-place shape for insert at an arbitrary index: every call copies the list."
VERDICT:   wrong
EVIDENCE:  `src/codegen/collection/assign/builder_inplace_assign.rs:2135` calls `lower_list_insert_in_place` for reassignment; `src/codegen/collection/list/list_mutate.rs:2403` implements it. `rg -n -i 'insert.?in.?place|insert_inplace' src` locates both symbols.
SUGGESTED: Omit this implementation-specific sentence. Keep the observable contract: "insert does not change value; use the returned list."

### 2. Wrong diagnostic promised for a list item
UNIT:      man-page:collections/insert
CLAIM:     "item must have exactly the element type T; passing another List OF T resolves no overload, and the call rejects a list-typed item explicitly with \"insert expects a single item, not a list\"."
VERDICT:   wrong
EVIDENCE:  Building `/tmp/plan-125-scratch/C-phase4/man-page-collections-insert/src/main.mfb` with `collections::insert([1, 2], 1, [3])` printed `TYPE_CALL_ARGUMENT_MISMATCH`: "Call to `collections.insert` has argument type(s) (List OF Integer, Integer, List OF Integer), expected List OF T, Integer, T." The quoted diagnostic exists only in `src/codegen/builtins/collections/func_insert.rs:154`, after type checking has already rejected this source.
SUGGESTED: "item must have the list's element type. Passing a List OF T as item is a type mismatch."

### 3. Errors table omits a raisable error
UNIT:      man-page:collections/insert
CLAIM:     "insert is fallible: the range check is a real trappable domain error, so an inline TRAP on an insert call compiles and catches the out-of-range failure rather than being reported as a dead handler."
VERDICT:   incomplete
EVIDENCE:  The page’s Errors table lists only ErrIndexOutOfRange. The scratch probe `collections::insert([1.0], 5, big * big) TRAP(e)`, built and run with the specified release binary, printed `77050015:Floating-point arithmetic overflowed to infinity.` `src/codegen/builtins/collections/func_insert.rs:152` invokes `observe_float_vr` for item before insertion.
SUGGESTED: "An out-of-range index raises ErrIndexOutOfRange. A non-finite Float item raises its corresponding floating-point error before insertion." Add ErrFloatOverflow to the rendered Errors table.

### 4. False failure-order claim and compiler-model leakage
UNIT:      man-page:collections/insert
CLAIM:     "The bounds test runs before any anything for the result, so a rejected index builds nothing."
VERDICT:   wrong
EVIDENCE:  The same scratch probe with an out-of-range index and overflowing Float item printed ErrFloatOverflow, not ErrIndexOutOfRange. In `src/codegen/builtins/collections/func_insert.rs:152-160`, float observation and item materialization precede `lower_list_insert_collection`, which performs the bounds check.
SUGGESTED: "An out-of-range index raises ErrIndexOutOfRange."