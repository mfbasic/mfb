//! `math::tan` — tangent of a `Float`/`Fixed` value or `Float` list (radians).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Tangent of an angle in radians."#;
const DESC: &str = r#"`tan` returns the tangent of `value` (an angle in radians), echoing the operand type
(`Float` or `Fixed`), plus the `List OF Float` vectorized form. An argument near an
odd multiple of pi/2 can overflow to a non-finite result (`ErrFloatInf`/
`ErrFloatNaN`)."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::tan(0.0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary(
        "tan",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        "The angle in radians, or a list of them. Near an odd multiple of pi/2 the result grows without bound.",
        &[Float, Fixed],
        &[Float],
        &["ErrFloatInf", "ErrFloatNaN", "ErrInvalidArgument"],
        lower_math_tan,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::tan`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_tan(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("tan", args)
}
