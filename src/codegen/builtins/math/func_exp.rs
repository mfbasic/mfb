//! `math::exp` — natural exponential of a `Float`/`Fixed` value or `Float` list.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::RegistryPackage;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Natural exponential (e raised to the power) of a value or list."#;
const DESC: &str = r#"`exp` returns `e ** value`, echoing the operand type (`Float` or `Fixed`), plus the
`List OF Float` vectorized form. An argument large enough to overflow the finite
range raises `ErrOverflow`/`ErrFloatInf`; a non-finite result raises `ErrFloatNaN`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::exp(1.0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary(
        "exp",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        &[Float, Fixed],
        &[Float],
        &["ErrFloatInf", "ErrFloatNaN", "ErrOverflow"],
        lower_math_exp,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::exp`. Slice B shim.
pub(crate) fn lower_math_exp(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder.lower_math_call("exp", args)
}
