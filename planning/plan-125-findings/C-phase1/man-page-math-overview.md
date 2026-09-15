### 1. Fixed lists are not accepted by every listed transcendental

UNIT:      man-page:math/overview  
CLAIM:     “Most members take a single number or a List OF that number and give back the same type they were given: abs, min/max/clamp, sqrt, the transcendentals, pow, and atan2.”  
VERDICT:   misleading  
EVIDENCE:  `src/codegen/builtins/math/func_exp.rs:register` registers scalar `Float`/`Fixed` but only `List OF Float`. A probe declaring `LET xs AS List OF Fixed = [1F]` and calling `math::exp(xs)` fails to build: `Call to math.exp has argument type(s) (List OF Fixed), expected Float | Fixed.` `pow` and `atan2` likewise register only `List OF Float` forms.  
SUGGESTED: “Most members preserve their input type. Some list forms accept only `List OF Float`; see the individual function page for accepted list element types.”

### 2. The ErrFloatDomain condition omits type-dependent behavior

UNIT:      man-page:math/overview  
CLAIM:     “Floating-point operation domain is invalid (negative sqrt, non-positive log/log10, out-of-range asin/acos, a non-whole or negative ^ exponent, or a Float MOD 0).”  
VERDICT:   misleading  
EVIDENCE:  `src/codegen/builtins/math/func_sqrt.rs:DESC` specifies `ErrFloatDomain` only for scalar `Float`; `Fixed` and array forms raise `ErrInvalidArgument`. The probe `math::sqrt(-1F)` built and ran, printing `Error: 7-705-0002` / `Argument value is not valid for the requested operation.` The Float probe `math::sqrt(-1.0)` printed `Error: 7-705-0012`.  
SUGGESTED: “For Float operations, the domain is invalid for negative sqrt, non-positive log/log10, out-of-range asin/acos, a non-whole or negative exponent, or Float MOD 0. Fixed and some list forms report ErrInvalidArgument for the corresponding invalid input.”

### 3. Overview has no runnable example

UNIT:      man-page:math/overview  
CLAIM:     (No Examples section is rendered.)  
VERDICT:   incomplete  
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man math` renders Description, Functions, and Errors only. `src/codegen/builtins/math/mod.rs:MODULE_DESC` contains no fenced example.  
SUGGESTED: Add a small compiled example, such as importing `math` and `io`, printing `math::pow(2.0, 10.0)`, and demonstrating `math::seed` followed by `math::rand(1, 6)`.