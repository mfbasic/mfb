//! `math::abs` — absolute value of a numeric value or list.

use crate::codegen::registry::RegistryPackage;
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType::{Fixed, Float, Integer, Money};

const INTRO: &str = r#"Absolute value of a numeric value or list."#;
const DESC: &str = r#"`abs` returns the magnitude of `value`: the value with its sign removed. It accepts
`Integer`, `Float`, `Fixed`, and `Money` scalars (echoing the operand type) and the
`List OF Integer`/`Float`/`Fixed` array forms (element-wise, returning a new list of
the same type). `abs(INT64_MIN)`-class inputs have no representable magnitude and
raise `ErrOverflow`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::abs(-7)))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary(
        "abs",
        INTRO,
        DESC,
        EX,
        "Integer | Float | Fixed | Money",
        &[Integer, Float, Fixed, Money],
        &[Integer, Float, Fixed],
        &["ErrOverflow"],
        lower_math_abs,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::abs`. Slice B shim → the
/// `lower_math_call` dispatcher (relocated into `common/` in Slice C).
pub(crate) fn lower_math_abs(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder.lower_math_call("abs", args)
}
