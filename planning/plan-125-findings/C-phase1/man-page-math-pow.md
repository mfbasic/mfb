### 1. Scalar overload parameter says it accepts lists
UNIT:      man-page:math/pow
CLAIM:     "The base, or a list of them."
VERDICT:   misleading
EVIDENCE:  The rendered table repeats this description for scalar `Float` and `Fixed` overloads. `src/codegen/builtins/math/func_pow.rs:register` registers lists only for `Float`; compiling `math::pow([2.0F], [3.0F])` produced `TYPE_CALL_ARGUMENT_MISMATCH`, expected `Float | Fixed, same type`.
SUGGESTED: For the list overload: "The Float base values. It must have the same length as exponent." For scalar overloads: "The scalar base."

### 2. Exponent description omits list and negative-exponent rules
UNIT:      man-page:math/pow
CLAIM:     "The exponent, or a list of them. Must be the same type as the base."
VERDICT:   incomplete
EVIDENCE:  The same `List OF Fixed` probe fails at compile time; only `List OF Float` is accepted. A mismatched `List OF Float` probe (`[2.0]`, `[3.0, 4.0]`) built and exited with `7-705-0002 ErrInvalidArgument`. Negative exponents are accepted for valid bases: `math::pow(2.0, -2.0)` printed `0.25`.
SUGGESTED: For the list overload: "The Float exponents. It must have the same length as base." For scalar overloads: "The scalar exponent. Negative exponents produce reciprocals when the result is representable."

### 3. Error and zero-edge behavior is not stated precisely
UNIT:      man-page:math/pow
CLAIM:     "A result that overflows the finite range or is otherwise non-finite raises ErrOverflow/ErrFloatInf/ErrFloatNaN; a negative base with a non-integer exponent is outside the domain."
VERDICT:   incomplete
EVIDENCE:  Probes against the release binary produced: `math::pow(-2.0, 0.5)` → `7-705-0013 ErrFloatNaN`; `math::pow(-2.0F, 0.5F)` → `7-705-0002 ErrInvalidArgument`; `math::pow(0.0, -1.0)` → `7-705-0014 ErrFloatInf`; `math::pow(0.0F, -1.0F)` → `7-705-0010 ErrOverflow`. A valid-edge probe printed `1.00`, `0.00`, `0.25`, `8.00`, `8.00`, `2.00`, `0` for `0 ** 0`, `0 ** 2`, `2 ** -2`, Fixed `2 ** 3`, two list elements, and an empty-list result length.
SUGGESTED: "Lists of different lengths raise ErrInvalidArgument. A negative base requires a whole-number exponent: a Float fractional exponent raises ErrFloatNaN, while a Fixed fractional exponent raises ErrInvalidArgument. Float overflow, including zero to a negative exponent, raises ErrFloatInf; Fixed overflow raises ErrOverflow. Zero to the zero power is 1, and two empty Float lists produce an empty result."