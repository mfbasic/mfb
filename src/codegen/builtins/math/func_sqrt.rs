//! `math::sqrt` — square root of a `Float`/`Fixed` value or list.

use crate::codegen::registry::RegistryPackage;
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
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

pub(super) fn register(pkg: &mut RegistryPackage) {
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

/// Target-generic call-site lowering for `math::sqrt`. Slice B shim.
pub(crate) fn lower_math_sqrt(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder.lower_math_call("sqrt", args)
}
