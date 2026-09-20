//! `math::sin` — sine of a `Float`/`Fixed` value or `Float` list (radians).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Sine of an angle in radians."#;
const DESC: &str = r#"`sin` returns the sine of `value` (an angle in radians), echoing the operand type
(`Float` or `Fixed`), plus a `List OF Float` form that returns a new list of each
angle's sine in the same order, leaving the input unchanged; an empty list gives an
empty list.

A `Float` angle of any size is measured against pi/2 exactly, so a huge angle is as
accurate as a small one and the answer always lies in `[-1.0, 1.0]`:
`math::sin(100000000000000000000.0)` is about `-0.645`, even though `Float` values
that large are more than a full turn apart."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::sin(math::pi2)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary_typed_errors(
        "sin",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        "The angle in radians, or a `List OF Float` of them.",
        &[Float, Fixed],
        &[Float],
        // bug-617: `ErrFloatNaN` is the `Float` kernel's own guard. The `Fixed`
        // path has NO raise site at all (the five raise sites in
        // `money/gen_fixed_math.rs` are tan-overflow, asin/acos-domain,
        // scale-by-power-of-two, log-domain and pow — none in sin/cos/atan2), so
        // the `Fixed` form declares nothing.
        &[],
        &[(Float, &["ErrFloatNaN"])],
        lower_math_sin,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::sin`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_sin(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("sin", args)
}
