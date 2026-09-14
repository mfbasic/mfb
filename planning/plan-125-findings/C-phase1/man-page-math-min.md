### 1. Errors table assigns a runtime error to scalar overloads
UNIT:      man-page:math/min
CLAIM:     `ErrInvalidArgument | 1, 2, 3, 4, 5, 6, 7`
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/math/func_min.rs:20-38` registers `ErrInvalidArgument` for all overloads, but `lower_math_min_max` in `src/codegen/builtins/math/gen_math.rs:629-687` has no error path for valid scalar Integer, Float, Fixed, or Money inputs. The release-binary probe with unequal integer lists printed `Error: 7-705-0002`; that path is emitted only by `lower_simd_binary` in `src/codegen/builtins/vector/builder_simd_math.rs:649-654`.
SUGGESTED: Associate `ErrInvalidArgument` only with the three list overloads: “For list inputs, raises `ErrInvalidArgument` when the lists have different lengths.”

### 2. List-result mutation semantics are omitted
UNIT:      man-page:math/min
CLAIM:     “The `List OF Integer`/`Float`/`Fixed` array forms take two equal-length lists and return the element-wise minimum; mismatched lengths raise `ErrInvalidArgument`.”
VERDICT:   incomplete
EVIDENCE:  `lower_simd_binary` in `src/codegen/builtins/vector/builder_simd_math.rs:657-695` creates a separate result list and writes output values there; it only reads both input lists. The release-binary probe printed `3`, `-5`, `1` for `math::min([3, -5, 7], [4, -2, 1])`, confirming element-wise output.
SUGGESTED: “The list forms return a new list containing the element-wise minimum and leave both input lists unchanged. The lists must have equal lengths; otherwise they raise `ErrInvalidArgument`.”

### 3. Parameter descriptions omit valid zero, negative, and empty-list cases
UNIT:      man-page:math/min
CLAIM:     “The first value, or a list of them.” / “The second value, or a list of them. Must be the same type as the first.”
VERDICT:   incomplete
EVIDENCE:  The release-binary probe evaluated negative list elements and printed `3`, `-5`, `1`; it also evaluated `math::min([], [])` and printed `0` for the result length. `lower_simd_binary` compares only the two list lengths before producing its result (`src/codegen/builtins/vector/builder_simd_math.rs:649-695`), so empty equal-length lists are valid.
SUGGESTED: “The first scalar value, including zero or a negative value, or a list of values. An empty list is valid when `b` is also empty.” For `b`: “The second scalar value, including zero or a negative value, or a list of values of the same type as `a`. Lists must have equal lengths; empty lists are valid only together.”