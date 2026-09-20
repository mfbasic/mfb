//! `math::abs` — absolute value of a numeric value or list.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float, Integer, Money};
const INTRO: &str = r#"Absolute value of a numeric value or list."#;
const DESC: &str = r#"`abs` returns the magnitude of `value`: the value with its sign removed. It accepts
`Integer`, `Float`, `Fixed`, and `Money` scalars (echoing the operand type) and the
`List OF Integer`/`Float`/`Fixed` array forms, which return a new list of the same
type holding each element's magnitude in the original order; the input list is
unchanged. `abs(INT64_MIN)`-class inputs have no representable magnitude and
raise `ErrOverflow`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::abs(-7)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary_typed_errors(
        "abs",
        INTRO,
        DESC,
        EX,
        "Integer | Float | Fixed | Money",
        "The number to take the magnitude of, or a list of them. The most negative `Integer`, `Fixed`, or `Money` has no positive counterpart and raises `ErrOverflow`.",
        &[Integer, Float, Fixed, Money],
        &[Integer, Float, Fixed],
        // bug-617: the `Float` forms clear the sign bit with a single `fabs` and
        // have no error path at all (`lower_math_abs`'s float arm; the SIMD
        // `AbsFloat` kernel declares no error). Only the two's-complement types can
        // meet a most-negative value with no positive counterpart.
        &[],
        &[
            (Integer, &["ErrOverflow"]),
            (Fixed, &["ErrOverflow"]),
            (Money, &["ErrOverflow"]),
        ],
        lower_math_abs,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::abs`, delegating to the shared
/// `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_abs(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("abs", args)
}
