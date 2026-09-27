### 1. Truncated predicate contract
UNIT:      man-page:collections/filter
CLAIM:     “This is enforced both when the call is resolved and again in the.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/collections/func_filter.rs:lower_filter` independently rejects a non-Boolean callable return; the existing invalid-call fixture reports `TYPE_CALL_ARGUMENT_MISMATCH` for `FUNC(Integer) AS Integer`.
SUGGESTED: The predicate must accept exactly one element and return Boolean. Calls with another callable shape are rejected.

### 2. Callback failures are not the only possible failure
UNIT:      man-page:collections/filter
CLAIM:     “It can fail only because a failing predicate propagates”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/collections/func_filter.rs:lower_filter` creates the result through `lower_reserved_list`; `src/codegen/builtins/collections/gen_mutate.rs:CodeBuilder::lower_reserved_list` raises `ErrOutOfMemory` if result-list creation fails.
SUGGESTED: filter has no function-specific error. An error raised by predicate propagates unchanged; runtime errors such as insufficient memory can also stop the call.

### 3. Irrelevant closing claim for a list value
UNIT:      man-page:collections/filter
CLAIM:     “value is neither modified nor closed; the result is a new list.”
VERDICT:   out-of-scope
EVIDENCE:  The parameter is `List OF T` in `src/codegen/builtins/collections/func_filter.rs:register`; closing is a resource concern and provides no useful contract for this value parameter. The scratch probe preserved `xs` after filtering, printing `source=4`.
SUGGESTED: value is not modified; filter returns a new list.