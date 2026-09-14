### 1. List overload type rule is misstated
UNIT:      man-page:math/clamp
CLAIM:     “All three arguments must be the same numeric type (`Integer`, `Float`, `Fixed`, or `Money`), echoing that type.”
VERDICT:   misleading
EVIDENCE:  The rendered overloads and `src/codegen/builtins/math/func_clamp.rs:register` accept `value AS List OF Integer` with scalar `low AS Integer` and `high AS Integer`; the compiled probe passed `[-3, 0, 4, 9]` to `math::clamp(values, 0, 4)` and printed `0,4`.
SUGGESTED: For scalar values, all three arguments must have the same numeric type. For a list, `low` and `high` must be scalar values of the list’s element type.

### 2. List result and empty-list behavior are omitted
UNIT:      man-page:math/clamp
CLAIM:     “The array form clamps a `List OF Integer`/`Float`/`Fixed` against two scalar bounds of the element type.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/math/gen_math.rs:lower_math_clamp_array` calls `lower_simd_clamp`, which creates a result list through `emit_alloc_result_list`; the probe printed the original list endpoints as `-3,9` and the result endpoints as `0,4`. A separate compiled empty-list probe printed `0`.
SUGGESTED: For a `List OF Integer`, `Float`, or `Fixed`, `clamp` returns a new list with each element restricted to the scalar bounds; the input list is unchanged. An empty input returns an empty list.

### 3. The Errors table omits allocation failure for list calls
UNIT:      man-page:math/clamp
CLAIM:     “`low` must not exceed `high`, else `ErrInvalidArgument`.”
VERDICT:   incomplete
EVIDENCE:  This condition is correct: the scalar invalid-bounds probe `math::clamp(5, 10, 2)` printed `Error: 7-705-0002`. But list lowering in `src/codegen/builtins/vector/builder_simd_math.rs:emit_alloc_result_list` raises `ErrOutOfMemory` when creating the returned list fails; the rendered Errors table lists only `ErrInvalidArgument`.
SUGGESTED: `low` must not exceed `high`, else `ErrInvalidArgument`. List calls may also raise `ErrOutOfMemory` if the result list cannot be created.