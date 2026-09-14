### 1. List parameters omit empty-list and matching-length semantics
UNIT:      man-page:math/max  
CLAIM:     "The first value, or a list of them."  
VERDICT:   incomplete  
EVIDENCE:  `/tmp/plan-125-scratch/C-phase1/man-page-math-max` compiled and ran `math::max([], [])`, printing `0` for the result length. `src/codegen/builtins/vector/builder_simd_math.rs:lower_simd_binary` checks equal lengths and raises `ErrInvalidArgument` before producing the result.  
SUGGESTED: The first numeric value, or an Integer, Float, or Fixed list. Empty lists are allowed; when using lists, `b` must have the same element type and length.

### 2. Second list parameter omits the condition that raises the documented error
UNIT:      man-page:math/max  
CLAIM:     "The second value, or a list of them. Must be the same type as the first."  
VERDICT:   incomplete  
EVIDENCE:  A scratch program calling `math::max([1], [4, 2])` built successfully and printed `Error: 7-705-0002 Argument value is not valid for the requested operation.` at runtime. `src/codegen/builtins/vector/builder_simd_math.rs:lower_simd_binary` raises `ErrInvalidArgument` when the two list counts differ.  
SUGGESTED: The second numeric value, or an Integer, Float, or Fixed list. It must have the same type as `a`; list arguments must also have equal lengths.

### 3. List result does not state that inputs are left unchanged
UNIT:      man-page:math/max  
CLAIM:     "The `List OF Integer`/`Float`/`Fixed` array forms take two equal-length lists and return the element-wise maximum; mismatched lengths raise `ErrInvalidArgument`."  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/vector/builder_simd_math.rs:lower_simd_binary` calls `emit_alloc_result_list` and writes results to that output list, while reading both input lists. The scratch probe printed `4,7,9` for `math::max([1,7,3], [4,2,9])`.  
SUGGESTED: The `List OF Integer`, `List OF Float`, and `List OF Fixed` forms require equal-length lists and return a new list containing the element-wise maximum; the input lists are unchanged. Mismatched lengths raise `ErrInvalidArgument`.

### 4. Errors table incorrectly assigns `ErrInvalidArgument` to scalar overloads
UNIT:      man-page:math/max  
CLAIM:     "ErrInvalidArgument — Overloads 1, 2, 3, 4, 5, 6, 7"  
VERDICT:   wrong  
EVIDENCE:  `src/codegen/builtins/math/gen_math.rs:lower_math_min_max` handles valid scalar Integer, Float, Fixed, and Money arguments solely by comparison and selection; it contains no `raise_error_bare` call. The only `ErrInvalidArgument` path for `max` is the unequal-list-length branch in `src/codegen/builtins/vector/builder_simd_math.rs:lower_simd_binary`.  
SUGGESTED: Restrict `ErrInvalidArgument` to list overloads 1–3, with the condition: “The two lists have different lengths.”