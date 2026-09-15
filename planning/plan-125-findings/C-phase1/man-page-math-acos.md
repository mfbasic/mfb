### 1. Domain endpoints and zero are not explicit
UNIT:      man-page:math/acos
CLAIM:     “The cosine to invert, or a list of them. Must be within -1 through 1; outside that there is no angle and the call raises.”
VERDICT:   incomplete
EVIDENCE:  `/tmp/plan-125-scratch/C-phase1/man-page-math-acos/probe` printed `float-1=value=3.14`, `float0=value=1.57`, and `float1=value=0.00`; both bounds and zero are accepted. The description does not explicitly state inclusivity or zero’s result.
SUGGESTED: The cosine to invert, or a list of them. Values from -1 to 1 inclusive are valid; 0 returns pi/2. Values outside that range raise an error.

### 2. ErrInvalidArgument is assigned to incorrect overloads
UNIT:      man-page:math/acos
CLAIM:     “ErrInvalidArgument … Overloads 1, 2, 3”
VERDICT:   wrong
EVIDENCE:  The probe printed `float-out=error=77050012`, `list-out=error=77050012`, and `fixed-out=error=77050002`. `src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_asin` raises `ErrInvalidArgument` for an out-of-domain Fixed input.
SUGGESTED: Render `ErrInvalidArgument` for overload 3 only (Fixed).

### 3. ErrFloatDomain is assigned to an incorrect overload
UNIT:      man-page:math/acos
CLAIM:     “ErrFloatDomain … Overloads 1, 2, 3”
VERDICT:   wrong
EVIDENCE:  The probe printed `float-out=error=77050012` and `list-out=error=77050012`, while Fixed printed `fixed-out=error=77050002`. `src/codegen/builtins/vector/builder_simd_float_math.rs:FloatKernel::errors` and `emit_float_error_reduce` map Float acos domain failures to `ErrFloatDomain`.
SUGGESTED: Render `ErrFloatDomain` for overloads 1 and 2 only (List OF Float and Float).

### 4. List result semantics are omitted
UNIT:      man-page:math/acos
CLAIM:     “`acos` returns the arccosine of `value` in radians, echoing the operand type (Float or Fixed), plus the List OF Float vectorized form.”
VERDICT:   incomplete
EVIDENCE:  The probe printed `original0=0.00`, `original1=0.50`, `result0=1.57`, and `result1=1.05`: the input list remains unchanged and the returned list contains corresponding results in input order. The page never states this mutation and ordering behavior.
SUGGESTED: For a `List OF Float`, returns a new list containing each input’s arccosine in the same order; the input list is unchanged.

### 5. The principal-result range is omitted
UNIT:      man-page:math/acos
CLAIM:     “Arccosine (inverse cosine), returning radians.”
VERDICT:   incomplete
EVIDENCE:  The probe printed `float-1=value=3.14`, `float0=value=1.57`, and `float1=value=0.00`, establishing the implemented principal range from 0 through pi.
SUGGESTED: Arccosine (inverse cosine), returning an angle from 0 through pi radians.