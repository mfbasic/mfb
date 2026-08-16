//! `math::log` — natural logarithm of a `Float`/`Fixed` value or list.

use crate::codegen::registry::RegistryPackage;
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType::{Fixed, Float};

const INTRO: &str = r#"Natural logarithm of a Float or Fixed value or list."#;
const DESC: &str = r#"`log` returns the natural logarithm (base `e`) of `value`, echoing the operand type
(`Float` or `Fixed`), plus the `List OF Float`/`List OF Fixed` forms. A non-positive
argument is outside the domain and raises `ErrFloatDomain` (scalar) or
`ErrInvalidArgument` (array)."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::log(math::e)))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary(
        "log",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        &[Float, Fixed],
        &[Float, Fixed],
        &["ErrFloatDomain", "ErrInvalidArgument"],
        lower_math_log,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::log`. Slice B shim.
pub(crate) fn lower_math_log(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder.lower_math_call("log", args)
}
