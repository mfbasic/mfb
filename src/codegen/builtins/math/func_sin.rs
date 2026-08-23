//! `math::sin` — sine of a `Float`/`Fixed` value or `Float` list (radians).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Sine of an angle in radians."#;
const DESC: &str = r#"`sin` returns the sine of `value` (an angle in radians), echoing the operand type
(`Float` or `Fixed`), plus the `List OF Float` vectorized form."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::sin(math::pi2)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary(
        "sin",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        &[Float, Fixed],
        &[Float],
        &["ErrFloatNaN"],
        lower_math_sin,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::sin`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_sin(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("sin", args)
}
