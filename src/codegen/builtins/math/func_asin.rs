//! `math::asin` — arcsine of a `Float`/`Fixed` value or `Float` list (radians).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Arcsine (inverse sine), returning radians."#;
const DESC: &str = r#"`asin` returns the arcsine of `value` in radians, echoing the operand type (`Float`
or `Fixed`), plus the `List OF Float` vectorized form. `value` must be in `[-1, 1]`;
outside that domain it raises `ErrFloatDomain` (scalar) or `ErrInvalidArgument`
(array)."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::asin(1.0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary(
        "asin",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        &[Float, Fixed],
        &[Float],
        &["ErrFloatDomain", "ErrInvalidArgument"],
        lower_math_asin,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::asin`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_asin(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("asin", args)
}
