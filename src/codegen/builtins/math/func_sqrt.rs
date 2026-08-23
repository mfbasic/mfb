//! `math::sqrt` — square root of a `Float`/`Fixed` value or list.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Square root of a Float or Fixed value or list."#;
const DESC: &str = r#"`sqrt` returns the non-negative square root of `value`, echoing the operand type
(`Float` or `Fixed`), plus the `List OF Float`/`List OF Fixed` vectorized forms. A
negative argument is outside the domain and raises `ErrFloatDomain` (scalar `Float`)
or `ErrInvalidArgument` (`Fixed` and the array forms)."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::sqrt(2.0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary(
        "sqrt",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        &[Float, Fixed],
        &[Float, Fixed],
        &["ErrFloatDomain", "ErrInvalidArgument"],
        lower_math_sqrt,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::sqrt`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_sqrt(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("sqrt", args)
}
