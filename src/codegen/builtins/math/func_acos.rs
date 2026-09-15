//! `math::acos` — arccosine of a `Float`/`Fixed` value or `Float` list (radians).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Arccosine (inverse cosine), returning an angle from 0 through pi radians."#;
const DESC: &str = r#"`acos` returns the arccosine of `value` in radians, echoing the operand type
(`Float` or `Fixed`), plus a `List OF Float` form that returns a new list of each
element's arccosine in the same order, leaving the input unchanged. `value` must be in
`[-1, 1]`; outside that domain a `Float` or `List OF Float` argument raises
`ErrFloatDomain`, and a `Fixed` argument raises `ErrInvalidArgument`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::acos(1.0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary(
        "acos",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        "The cosine to invert, or a list of them. Values from -1 through 1 inclusive are valid (0 gives pi/2); outside that there is no angle and the call raises.",
        &[Float, Fixed],
        &[Float],
        &["ErrFloatDomain", "ErrInvalidArgument"],
        lower_math_acos,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::acos`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_acos(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("acos", args)
}
