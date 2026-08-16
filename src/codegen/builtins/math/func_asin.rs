//! `math::asin` — arcsine of a `Float`/`Fixed` value or `Float` list (radians).

use crate::codegen::registry::RegistryPackage;
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
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

pub(super) fn register(pkg: &mut RegistryPackage) {
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

/// Target-generic call-site lowering for `math::asin`. Slice B shim.
pub(crate) fn lower_math_asin(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder.lower_math_call("asin", args)
}
