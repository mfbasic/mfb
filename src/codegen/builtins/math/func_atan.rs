//! `math::atan` — arctangent of a `Float`/`Fixed` value or `Float` list (radians).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::RegistryPackage;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Arctangent (inverse tangent), returning radians."#;
const DESC: &str = r#"`atan` returns the arctangent of `value` in radians (in `[-pi/2, pi/2]`), echoing
the operand type (`Float` or `Fixed`), plus the `List OF Float` vectorized form. For
the two-argument form that takes the quadrant into account, see `math::atan2`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::atan(1.0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary(
        "atan",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        &[Float, Fixed],
        &[Float],
        &["ErrFloatNaN"],
        lower_math_atan,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::atan`. Slice B shim.
pub(crate) fn lower_math_atan(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder.lower_math_call("atan", args)
}
