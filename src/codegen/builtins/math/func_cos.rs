//! `math::cos` — cosine of a `Float`/`Fixed` value or `Float` list (radians).

use crate::codegen::registry::RegistryPackage;
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType::{Fixed, Float};

const INTRO: &str = r#"Cosine of an angle in radians."#;
const DESC: &str = r#"`cos` returns the cosine of `value` (an angle in radians), echoing the operand type
(`Float` or `Fixed`), plus the `List OF Float` vectorized form."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::cos(0.0)))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary(
        "cos",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        &[Float, Fixed],
        &[Float],
        &["ErrFloatNaN"],
        lower_math_cos,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::cos`. Slice B shim.
pub(crate) fn lower_math_cos(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder.lower_math_call("cos", args)
}
